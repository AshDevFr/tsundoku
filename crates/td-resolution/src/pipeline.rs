//! The resolution orchestrator.
//!
//! Given a single `releases` row, walks the priority chain and writes an
//! outcome back. The chain is deterministic and shared across the
//! scheduler poll path, the `tsundoku resolve` CLI, and the
//! `POST /api/v1/releases/{id}/retry` handler (Phase 7).
//!
//! Step order is documented at the crate root. Each step is implemented as
//! a private method here so the high-level [`Resolver::resolve_one`] reads
//! top-to-bottom like the PRD's "Resolution flow" pseudo-code.

use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use td_config::IngestionConfig;
use td_db::entities::releases as releases_entity;
use td_db::entities::review_candidates;
use td_db::repos::{
    mangaupdates_id_repo, releases_repo, review_repo, series_external_ids_repo, series_repo,
};
use td_metadata::{MetadataRegistry, SearchHit, SeriesMetadata};
use td_source::ExternalLinks;

use std::collections::HashMap;

use crate::foreign_id;
use crate::mangaupdates_redirect::{MangaUpdatesRedirector, ResolveOutcome};
use crate::persist::{self, UpsertResult};
use crate::query_builder::{CleanedQuery, QueryBuilder};
use crate::scoring::dice;
use crate::validation::{self, ValidationOutcome};

/// How a release was matched. Persisted to `releases.resolution_path`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionPath {
    KnownExternalId,
    ForeignIdLookup,
    FuzzyTitle,
}

impl ResolutionPath {
    pub fn as_str(self) -> &'static str {
        match self {
            ResolutionPath::KnownExternalId => "known_external_id",
            ResolutionPath::ForeignIdLookup => "foreign_id_lookup",
            ResolutionPath::FuzzyTitle => "fuzzy_title",
        }
    }
}

/// Outcome of a single resolver run. Mirrors the columns we write back to
/// the release: status + path + confidence. `series_id` is `None` for the
/// `unresolved` outcome.
#[derive(Debug, Clone)]
pub struct ResolutionOutcome {
    pub release_id: String,
    pub series_id: Option<i32>,
    pub path: Option<ResolutionPath>,
    pub confidence: Option<f64>,
    pub status: ResolutionStatus,
    /// Surfaced for tests and logging; not persisted directly.
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionStatus {
    Resolved,
    /// Format-type validation failed after a confident match. The release
    /// is linked to the series, but flagged for review.
    Ambiguous,
    /// No confident match, but at least one plausible candidate was
    /// recorded in `review_candidates`.
    ReviewPending,
    Unresolved,
}

impl ResolutionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ResolutionStatus::Resolved => "resolved",
            ResolutionStatus::Ambiguous => "ambiguous",
            ResolutionStatus::ReviewPending => "review_pending",
            ResolutionStatus::Unresolved => "unresolved",
        }
    }
}

/// Pipeline orchestrator. Hold one of these for the lifetime of the
/// process (or per-task in tests) and call [`Self::resolve_one`] or
/// [`Self::resolve_unresolved`] as needed.
pub struct Resolver {
    db: DatabaseConnection,
    registry: Arc<MetadataRegistry>,
    config: IngestionConfig,
    /// Title cleaner. Built once at startup from the built-in keyword
    /// list plus any `ingestion.cleanup.extra_format_keywords` from
    /// config. Defaults to a built-in-only cleaner when constructed via
    /// `Resolver::new` so existing call sites don't have to opt in.
    query_builder: Arc<QueryBuilder>,
    /// Optional redirector for translating MangaUpdates legacy numeric
    /// IDs to modern alphanumeric slugs. `None` in tests and CLIs that
    /// don't have network; legacy MU links are simply dropped in that
    /// case.
    mangaupdates_redirector: Option<Arc<MangaUpdatesRedirector>>,
}

impl Resolver {
    pub fn new(
        db: DatabaseConnection,
        registry: Arc<MetadataRegistry>,
        config: IngestionConfig,
    ) -> Self {
        Self {
            db,
            registry,
            config,
            query_builder: Arc::new(QueryBuilder::with_defaults()),
            mangaupdates_redirector: None,
        }
    }

    /// Attach a pre-built title cleaner. Production callers pass one
    /// built from the operator's `ingestion.cleanup.extra_format_keywords`;
    /// tests typically use the default.
    pub fn with_query_builder(mut self, query_builder: Arc<QueryBuilder>) -> Self {
        self.query_builder = query_builder;
        self
    }

    /// Attach a MangaUpdates redirect resolver so the pipeline can
    /// translate legacy `series.html?id=NNN` IDs into modern slugs.
    /// Without this, legacy MU links are silently dropped.
    pub fn with_mangaupdates_redirector(mut self, redirector: Arc<MangaUpdatesRedirector>) -> Self {
        self.mangaupdates_redirector = Some(redirector);
        self
    }

    /// Resolve the release with id `release_id`. Walks every step in the
    /// priority chain and writes the outcome back to the row. Returns the
    /// outcome for the caller's logging / CLI rendering.
    pub async fn resolve_one(&self, release_id: &str) -> Result<ResolutionOutcome> {
        let release = releases_repo::find_by_id(&self.db, release_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("release {release_id:?} not found"))?;
        self.resolve_release(&release).await
    }

    /// Re-run the resolver on every row whose status is `unresolved` or
    /// `ambiguous`. Returns a per-status tally so the CLI can render a
    /// summary table. Errors on individual releases are logged but don't
    /// abort the batch.
    pub async fn resolve_unresolved(&self, batch_limit: u64) -> Result<RetrySummary> {
        let mut summary = RetrySummary::default();
        let mut rows = releases_repo::list_by_status(&self.db, "unresolved", batch_limit).await?;
        rows.extend(releases_repo::list_by_status(&self.db, "ambiguous", batch_limit).await?);
        for row in rows {
            match self.resolve_release(&row).await {
                Ok(outcome) => summary.observe(outcome.status),
                Err(e) => {
                    tracing::warn!(error = ?e, release_id = %row.id, "resolver failed");
                    summary.errors += 1;
                }
            }
        }
        Ok(summary)
    }

    async fn resolve_release(&self, release: &releases_entity::Model) -> Result<ResolutionOutcome> {
        let now = Utc::now();
        let attempted_at = now.timestamp();
        let links = parse_external_links(release.extracted_links_json.as_deref());
        let normalized_pairs = self.normalize_external_links(&links).await;
        let formats = releases_repo::list_formats(&self.db, &release.id).await?;
        let active = self.registry.active().clone();
        let active_id = self.registry.active_id().to_string();

        // Clean the title once and persist immediately. Persisting on
        // every release (regardless of which step matches) gives the
        // review UI a consistent diagnostic surface and pre-populates
        // the search modal for manual relink.
        let cleaned = self.query_builder.clean(&release.title);
        if let Err(e) = persist::persist_search_queries(
            &self.db,
            &release.id,
            &cleaned.queries,
            &cleaned.rules_applied,
        )
        .await
        {
            tracing::warn!(
                error = ?e,
                release_id = %release.id,
                "failed to persist cleaned search queries; continuing resolve"
            );
        }

        // 1. Known external IDs.
        if let Some(series_id) = self.step_known_external_id(&normalized_pairs).await? {
            let outcome = self
                .finalize_known_match(
                    release,
                    series_id,
                    ResolutionPath::KnownExternalId,
                    &formats,
                )
                .await?;
            self.write_outcome(&outcome, attempted_at).await?;
            return Ok(outcome);
        }

        // 2. Foreign-ID lookup via the active provider.
        if let Some(metadata) = self
            .step_foreign_id_lookup(&active_id, active.as_ref(), &normalized_pairs)
            .await?
        {
            let outcome = self
                .finalize_metadata_match(
                    release,
                    &active_id,
                    metadata,
                    ResolutionPath::ForeignIdLookup,
                    1.0,
                    &formats,
                    now,
                )
                .await?;
            self.write_outcome(&outcome, attempted_at).await?;
            return Ok(outcome);
        }

        // 3. Fuzzy title via the active provider. Run one search per
        // cleaned query and Dice-rescore each candidate against the
        // closest matching query — fixes the prior behavior where
        // `dice(release.title, hit.title)` against the raw nyaa title
        // dragged noisy releases below the resolution threshold even
        // when the right candidate had been found.
        let fuzzy = self
            .step_fuzzy_title(&active_id, active.as_ref(), &cleaned)
            .await?;

        if let Some((hit, score)) = fuzzy.best_above_threshold(self.config.resolution_threshold) {
            // Above the confident threshold → fetch full metadata, persist,
            // run validation.
            let metadata = active
                .get(&hit.external_id)
                .await
                .map_err(|e| anyhow::anyhow!("active provider get({}): {e}", hit.external_id))?;
            if let Some(metadata) = metadata {
                let outcome = self
                    .finalize_metadata_match(
                        release,
                        &active_id,
                        metadata,
                        ResolutionPath::FuzzyTitle,
                        score as f64,
                        &formats,
                        now,
                    )
                    .await?;
                self.write_outcome(&outcome, attempted_at).await?;
                return Ok(outcome);
            }
        }

        // Below threshold: maybe queue for review.
        let outcome = self
            .finalize_low_confidence(release, &fuzzy.hits, &active_id, active.as_ref(), now)
            .await?;
        self.write_outcome(&outcome, attempted_at).await?;
        Ok(outcome)
    }

    /// Resolve any synthetic `mangaupdates-legacy` entries into modern
    /// MangaUpdates IDs using the cache (and the redirect resolver on
    /// cache miss). Entries that tombstone — or that miss with no
    /// redirector configured — are dropped from the returned list.
    async fn normalize_external_links(
        &self,
        links: &ExternalLinks,
    ) -> Vec<(&'static str, String, Option<String>)> {
        let mut out = Vec::with_capacity(4);
        for (provider, id, url) in foreign_id::pairs(links) {
            if provider == foreign_id::MANGAUPDATES_LEGACY {
                if let Some(modern) = self.translate_legacy_mu(&id).await {
                    out.push(("mangaupdates", modern, url));
                }
                // Tombstoned or transient: silently drop. A transient
                // failure leaves the cache untouched so the next poll
                // tries again.
                continue;
            }
            out.push((provider, id, url));
        }
        out
    }

    async fn translate_legacy_mu(&self, legacy_id_str: &str) -> Option<String> {
        let legacy_id: i64 = match legacy_id_str.parse() {
            Ok(n) => n,
            Err(_) => return None,
        };
        match mangaupdates_id_repo::lookup(&self.db, legacy_id).await {
            Ok(Some(Some(modern))) => return Some(modern),
            Ok(Some(None)) => return None,
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(error = ?e, legacy_id, "mangaupdates_id_map lookup failed");
                return None;
            }
        }
        let redirector = self.mangaupdates_redirector.as_ref()?;
        let now = Utc::now().timestamp();
        match redirector.resolve_legacy(legacy_id).await {
            Ok(ResolveOutcome::Modern(modern)) => {
                if let Err(e) =
                    mangaupdates_id_repo::record(&self.db, legacy_id, Some(&modern), now).await
                {
                    tracing::warn!(error = ?e, legacy_id, "failed to persist mu id mapping");
                }
                Some(modern)
            }
            Ok(ResolveOutcome::Tombstone) => {
                if let Err(e) = mangaupdates_id_repo::record(&self.db, legacy_id, None, now).await {
                    tracing::warn!(error = ?e, legacy_id, "failed to persist mu id tombstone");
                }
                None
            }
            Err(e) => {
                tracing::warn!(error = ?e, legacy_id, "mangaupdates redirect failed; will retry");
                None
            }
        }
    }

    async fn step_known_external_id(
        &self,
        pairs: &[(&'static str, String, Option<String>)],
    ) -> Result<Option<i32>> {
        for (provider, id, _) in pairs {
            if let Some(series_id) =
                series_external_ids_repo::find_series_id(&self.db, provider, id).await?
            {
                return Ok(Some(series_id));
            }
        }
        Ok(None)
    }

    async fn step_foreign_id_lookup(
        &self,
        active_id: &str,
        active: &dyn td_metadata::MetadataProvider,
        pairs: &[(&'static str, String, Option<String>)],
    ) -> Result<Option<SeriesMetadata>> {
        for (provider, id, _) in pairs {
            if *provider == active_id {
                // The active provider's own ID would already have hit
                // step 1 if it were known. A miss here means the row
                // genuinely doesn't exist locally; the provider's `get`
                // would be a better fit than `resolve_by_foreign_id`,
                // but the spec is explicit that step 2 only handles
                // *foreign* IDs. Skip and let step 3 handle it.
                continue;
            }
            match active.resolve_by_foreign_id(provider, id).await {
                Ok(Some(metadata)) => return Ok(Some(metadata)),
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(error = ?e, provider = %provider, id = %id,
                        "resolve_by_foreign_id failed; continuing chain");
                }
            }
        }
        Ok(None)
    }

    async fn step_fuzzy_title(
        &self,
        active_id: &str,
        active: &dyn td_metadata::MetadataProvider,
        cleaned: &CleanedQuery,
    ) -> Result<FuzzyResults> {
        // Run one search per cleaned query (typically one; up to ~3 when
        // the raw title had a romaji / English split). Dedupe hits by
        // external_id, keeping the max Dice score across all queries —
        // a candidate that matches the romaji half tightly should not be
        // demoted just because the English half also got searched.
        let mut by_id: HashMap<String, (SearchHit, f32)> = HashMap::new();
        for query in &cleaned.queries {
            let hits = match active.search(query, self.config.fuzzy_search_limit).await {
                Ok(h) => h,
                Err(e) => {
                    tracing::warn!(
                        error = ?e,
                        provider = %active_id,
                        query = %query,
                        "fuzzy search failed; continuing with remaining queries"
                    );
                    continue;
                }
            };
            for hit in hits {
                let score = cleaned
                    .queries
                    .iter()
                    .map(|q| dice(q, &hit.title))
                    .fold(0f32, f32::max);
                by_id
                    .entry(hit.external_id.clone())
                    .and_modify(|(_, s)| {
                        if score > *s {
                            *s = score;
                        }
                    })
                    .or_insert((hit, score));
            }
        }
        let mut scored: Vec<(SearchHit, f32)> = by_id.into_values().collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(FuzzyResults { hits: scored })
    }

    async fn finalize_known_match(
        &self,
        release: &releases_entity::Model,
        series_id: i32,
        path: ResolutionPath,
        formats: &[String],
    ) -> Result<ResolutionOutcome> {
        // Clear any prior review candidates for this release: a confident
        // match supersedes the prior ambiguous state.
        review_repo::replace_for_release(&self.db, &release.id, Vec::new()).await?;

        let series = series_repo::find_by_id(&self.db, series_id).await?;
        let kind = series
            .as_ref()
            .and_then(|s| s.kind.as_deref())
            .map(string_to_kind);
        let validation =
            validation::validate(&self.config.format_type_rules, formats, kind.as_ref());
        let (status, reason) = match validation {
            ValidationOutcome::Ok => (ResolutionStatus::Resolved, None),
            ValidationOutcome::Mismatch {
                offending_formats,
                required_kinds,
                series_kind,
            } => (
                ResolutionStatus::Ambiguous,
                Some(format_type_mismatch_reason(
                    &offending_formats,
                    &required_kinds,
                    series_kind.as_deref(),
                )),
            ),
        };
        Ok(ResolutionOutcome {
            release_id: release.id.clone(),
            series_id: Some(series_id),
            path: Some(path),
            confidence: Some(1.0),
            status,
            reason,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn finalize_metadata_match(
        &self,
        release: &releases_entity::Model,
        active_id: &str,
        metadata: SeriesMetadata,
        path: ResolutionPath,
        confidence: f64,
        formats: &[String],
        now: chrono::DateTime<Utc>,
    ) -> Result<ResolutionOutcome> {
        let UpsertResult { series_id, .. } = persist::upsert_series_from_metadata(
            &self.db,
            active_id,
            &metadata,
            release.posted_at,
            now,
        )
        .await
        .with_context(|| format!("upserting series for release {}", release.id))?;
        // Clear prior candidates: confident match.
        review_repo::replace_for_release(&self.db, &release.id, Vec::new()).await?;

        let validation = validation::validate(
            &self.config.format_type_rules,
            formats,
            metadata.kind.as_ref(),
        );
        let (status, reason) = match validation {
            ValidationOutcome::Ok => (ResolutionStatus::Resolved, None),
            ValidationOutcome::Mismatch {
                offending_formats,
                required_kinds,
                series_kind,
            } => (
                ResolutionStatus::Ambiguous,
                Some(format_type_mismatch_reason(
                    &offending_formats,
                    &required_kinds,
                    series_kind.as_deref(),
                )),
            ),
        };
        Ok(ResolutionOutcome {
            release_id: release.id.clone(),
            series_id: Some(series_id),
            path: Some(path),
            confidence: Some(confidence),
            status,
            reason,
        })
    }

    async fn finalize_low_confidence(
        &self,
        release: &releases_entity::Model,
        scored: &[(SearchHit, f32)],
        active_id: &str,
        active: &dyn td_metadata::MetadataProvider,
        now: chrono::DateTime<Utc>,
    ) -> Result<ResolutionOutcome> {
        // Filter to plausible candidates above review_threshold.
        let plausible: Vec<&(SearchHit, f32)> = scored
            .iter()
            .filter(|(_, s)| *s >= self.config.review_threshold)
            .take(5)
            .collect();

        if !self.config.queue_low_confidence || plausible.is_empty() {
            review_repo::replace_for_release(&self.db, &release.id, Vec::new()).await?;
            return Ok(ResolutionOutcome {
                release_id: release.id.clone(),
                series_id: None,
                path: None,
                confidence: scored.first().map(|(_, s)| *s as f64),
                status: ResolutionStatus::Unresolved,
                reason: Some("no_confident_match".into()),
            });
        }

        // Upsert each candidate as a stub series row (we have title from
        // SearchHit but not full metadata) → keep using the active provider's
        // namespace for the external_id. Fetch full metadata for each so the
        // review queue has cover art etc.
        let mut candidate_rows = Vec::with_capacity(plausible.len());
        for (hit, score) in &plausible {
            let metadata = match active.get(&hit.external_id).await {
                Ok(Some(m)) => m,
                Ok(None) => {
                    tracing::debug!(provider = %active_id, id = %hit.external_id,
                        "candidate present in search() but absent from get(); skipping");
                    continue;
                }
                Err(e) => {
                    tracing::warn!(error = ?e, provider = %active_id, id = %hit.external_id,
                        "fetching candidate metadata failed");
                    continue;
                }
            };
            let UpsertResult { series_id, .. } = persist::upsert_series_from_metadata(
                &self.db,
                active_id,
                &metadata,
                release.posted_at,
                now,
            )
            .await?;
            let s = *score;
            candidate_rows.push(review_candidates::ActiveModel {
                release_id: sea_orm::Set(release.id.clone()),
                series_id: sea_orm::Set(series_id),
                score: sea_orm::Set(s as f64),
                reason: sea_orm::Set(Some(format!("fuzzy_title:{:.3}", s))),
            });
        }

        if candidate_rows.is_empty() {
            review_repo::replace_for_release(&self.db, &release.id, Vec::new()).await?;
            return Ok(ResolutionOutcome {
                release_id: release.id.clone(),
                series_id: None,
                path: None,
                confidence: scored.first().map(|(_, s)| *s as f64),
                status: ResolutionStatus::Unresolved,
                reason: Some("candidate_fetch_failed".into()),
            });
        }

        review_repo::replace_for_release(&self.db, &release.id, candidate_rows).await?;
        Ok(ResolutionOutcome {
            release_id: release.id.clone(),
            series_id: None,
            path: None,
            confidence: plausible.first().map(|(_, s)| *s as f64),
            status: ResolutionStatus::ReviewPending,
            reason: Some("below_resolution_threshold".into()),
        })
    }

    async fn write_outcome(&self, outcome: &ResolutionOutcome, attempted_at: i64) -> Result<()> {
        persist::link_release(
            &self.db,
            &outcome.release_id,
            outcome.series_id,
            outcome.path.map(|p| p.as_str()),
            outcome.confidence,
            outcome.status.as_str(),
            attempted_at,
        )
        .await
    }
}

#[derive(Debug, Clone, Default)]
pub struct RetrySummary {
    pub resolved: usize,
    pub ambiguous: usize,
    pub review_pending: usize,
    pub unresolved: usize,
    pub errors: usize,
}

impl RetrySummary {
    fn observe(&mut self, status: ResolutionStatus) {
        match status {
            ResolutionStatus::Resolved => self.resolved += 1,
            ResolutionStatus::Ambiguous => self.ambiguous += 1,
            ResolutionStatus::ReviewPending => self.review_pending += 1,
            ResolutionStatus::Unresolved => self.unresolved += 1,
        }
    }

    pub fn total(&self) -> usize {
        self.resolved + self.ambiguous + self.review_pending + self.unresolved + self.errors
    }
}

#[derive(Debug, Default)]
struct FuzzyResults {
    hits: Vec<(SearchHit, f32)>,
}

impl FuzzyResults {
    fn best_above_threshold(&self, threshold: f32) -> Option<(&SearchHit, f32)> {
        self.hits
            .first()
            .filter(|(_, score)| *score >= threshold)
            .map(|(h, s)| (h, *s))
    }
}

fn parse_external_links(json: Option<&str>) -> ExternalLinks {
    json.and_then(|j| serde_json::from_str::<ExternalLinks>(j).ok())
        .unwrap_or_default()
}

fn string_to_kind(s: &str) -> td_metadata::SeriesKind {
    use td_metadata::SeriesKind::*;
    match s {
        "manga" => Manga,
        "manhwa" => Manhwa,
        "manhua" => Manhua,
        "novel" => Novel,
        "one_shot" => OneShot,
        "oel" => Oel,
        other => Other(other.to_string()),
    }
}

fn format_type_mismatch_reason(
    offending_formats: &[String],
    required_kinds: &[String],
    series_kind: Option<&str>,
) -> String {
    format!(
        "format_type_mismatch: {} requires kind in {:?}, got {:?}",
        offending_formats.join(","),
        required_kinds,
        series_kind.unwrap_or("<unknown>")
    )
}
