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
use crate::validation::{self, FormatKindGroups, ValidationOutcome};

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
        self.resolve_by_statuses(&["unresolved", "ambiguous"], batch_limit)
            .await
    }

    /// Re-run the resolver on every row currently surfaced in the review
    /// UI: `unresolved`, `ambiguous`, and `review_pending`. Used by the
    /// "retry all" button so the operator can re-evaluate the entire
    /// queue after a provider refresh or a config change without picking
    /// rows one at a time.
    pub async fn resolve_review_queue(&self, batch_limit: u64) -> Result<RetrySummary> {
        self.resolve_by_statuses(&["unresolved", "ambiguous", "review_pending"], batch_limit)
            .await
    }

    /// Like [`Self::resolve_review_queue`] but also walks rows currently
    /// marked `resolved`. Use after changing `format_type_rules`,
    /// `cleanup`, or the active provider's offline cache, when a
    /// previously-confident match needs to be re-evaluated against the
    /// new logic.
    ///
    /// Excluded by design:
    /// - `rejected` / `standalone` — operator decisions.
    /// - any row whose `resolution_path` is `'manual'` — set by
    ///   `POST /releases/{id}/link`; re-resolving would silently
    ///   overwrite the operator's manual relink. Operators that want to
    ///   force a manual re-resolve still have `POST /bulk/retry` with
    ///   explicit ids.
    pub async fn resolve_all(&self, batch_limit: u64) -> Result<RetrySummary> {
        let mut summary = RetrySummary::default();
        let mut rows = Vec::new();
        for status in ["unresolved", "ambiguous", "review_pending"] {
            rows.extend(releases_repo::list_by_status(&self.db, status, batch_limit).await?);
        }
        rows.extend(
            releases_repo::list_by_status_excluding_manual(&self.db, "resolved", batch_limit)
                .await?,
        );
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

    /// Re-run the resolver against an explicit set of release ids. Mirrors
    /// [`resolve_review_queue`] but targets the exact selection a bulk-retry
    /// request resolved from its filters, rather than a status sweep. An id
    /// that no longer exists (e.g. deleted between selection and execution)
    /// is counted as an error and does not abort the batch.
    pub async fn resolve_ids(&self, ids: &[String]) -> Result<RetrySummary> {
        let mut summary = RetrySummary::default();
        for id in ids {
            match self.resolve_one(id).await {
                Ok(outcome) => summary.observe(outcome.status),
                Err(e) => {
                    tracing::warn!(error = ?e, release_id = %id, "resolver failed");
                    summary.errors += 1;
                }
            }
        }
        Ok(summary)
    }

    async fn resolve_by_statuses(
        &self,
        statuses: &[&str],
        batch_limit: u64,
    ) -> Result<RetrySummary> {
        let mut summary = RetrySummary::default();
        let mut rows = Vec::new();
        for status in statuses {
            rows.extend(releases_repo::list_by_status(&self.db, status, batch_limit).await?);
        }
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

        // Format-to-kind rules influence candidate selection in two ways:
        //  - Mixed-format release (e.g. cbz + epub) with confident hits in
        //    more than one rule's required-kinds → operator must choose,
        //    so route directly to review with one candidate per bucket.
        //  - Single-format release (e.g. cbz only) → drop hits whose kind
        //    sits outside the firing rule's required-kinds before picking
        //    a winner, so a same-titled novel can't steal a CBZ release
        //    from its manga counterpart.
        // When no rule fires (no format the rules care about), the
        // filtering is a no-op and the legacy "best score wins" behavior
        // applies.
        let groups = validation::rule_groups(&self.config.format_type_rules, &formats);

        if let Some(top_per_bucket) =
            multi_bucket_confident_hits(&groups, &fuzzy.hits, self.config.resolution_threshold)
        {
            let outcome = self
                .finalize_multi_bucket_review(
                    release,
                    &top_per_bucket,
                    &active_id,
                    active.as_ref(),
                    now,
                )
                .await?;
            self.write_outcome(&outcome, attempted_at).await?;
            return Ok(outcome);
        }

        let filtered = filter_compatible_hits(&groups, &fuzzy.hits);
        let primary_hits: &[(SearchHit, f32)] = filtered.as_deref().unwrap_or(&fuzzy.hits);

        if let Some((hit, score)) =
            best_above_threshold(primary_hits, self.config.resolution_threshold)
        {
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

        // Below threshold: maybe queue for review. Use the full hit list
        // (including format-incompatible ones) so the operator sees every
        // candidate the fuzzy step found — the review UI can still flag
        // format mismatches via the candidate's own `kind`.
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
            let result = if *provider == active_id {
                // Active-provider's own ID extracted from the release
                // (e.g. a `mangabaka.org/{id}` link pasted into a Nyaa
                // post body). Step 1 would have caught it if it were
                // already in `series_external_ids`; a miss here means
                // it's just unknown locally, so we ask the provider
                // directly via `get` rather than skipping to fuzzy.
                active.get(id).await
            } else {
                active.resolve_by_foreign_id(provider, id).await
            };
            match result {
                Ok(Some(metadata)) => return Ok(Some(metadata)),
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(error = ?e, provider = %provider, id = %id,
                        "active-provider lookup failed; continuing chain");
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
                // Score against the best-matching query, scaling each by
                // its weight so a lossy subtitle-head query (weight < 1)
                // can't auto-resolve on a mediocre match.
                let score = cleaned
                    .queries
                    .iter()
                    .zip(&cleaned.query_weights)
                    .map(|(q, w)| dice(q, &hit.title) * w)
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
        // Resolver upsert: a release event is not a signal to overwrite
        // operator-curated metadata, so manual rows stay sticky.
        let UpsertResult { series_id, .. } = persist::upsert_series_from_metadata(
            &self.db,
            active_id,
            &metadata,
            release.posted_at,
            now,
            false,
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
                false,
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

    /// Persist one candidate per rule-bucket and mark the release
    /// `review_pending`. Called when the release's format set fires
    /// multiple format-type rules and high-confidence hits exist in more
    /// than one rule's required-kinds (e.g. a CBZ+EPUB release with a
    /// same-titled manga and novel both at score 1.0). The operator picks
    /// the correct kind in the review UI; the resolver refuses to guess.
    async fn finalize_multi_bucket_review(
        &self,
        release: &releases_entity::Model,
        scored: &[(SearchHit, f32)],
        active_id: &str,
        active: &dyn td_metadata::MetadataProvider,
        now: chrono::DateTime<Utc>,
    ) -> Result<ResolutionOutcome> {
        let mut candidate_rows = Vec::with_capacity(scored.len());
        for (hit, score) in scored {
            let metadata = match active.get(&hit.external_id).await {
                Ok(Some(m)) => m,
                Ok(None) => {
                    tracing::debug!(provider = %active_id, id = %hit.external_id,
                        "mixed-format candidate present in search() but absent from get(); skipping");
                    continue;
                }
                Err(e) => {
                    tracing::warn!(error = ?e, provider = %active_id, id = %hit.external_id,
                        "fetching mixed-format candidate metadata failed");
                    continue;
                }
            };
            let UpsertResult { series_id, .. } = persist::upsert_series_from_metadata(
                &self.db,
                active_id,
                &metadata,
                release.posted_at,
                now,
                false,
            )
            .await?;
            let s = *score;
            candidate_rows.push(review_candidates::ActiveModel {
                release_id: sea_orm::Set(release.id.clone()),
                series_id: sea_orm::Set(series_id),
                score: sea_orm::Set(s as f64),
                reason: sea_orm::Set(Some(format!("mixed_format:{:.3}", s))),
            });
        }

        if candidate_rows.is_empty() {
            // All `get()` calls failed; fall back to unresolved so the
            // next retry can rebuild candidates from a fresh fetch.
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
            confidence: scored.first().map(|(_, s)| *s as f64),
            status: ResolutionStatus::ReviewPending,
            reason: Some("mixed_format_multi_kind".into()),
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

/// First hit at-or-above `threshold`. The slice is assumed sorted in
/// descending score order (the fuzzy step does that).
fn best_above_threshold(scored: &[(SearchHit, f32)], threshold: f32) -> Option<(&SearchHit, f32)> {
    scored
        .first()
        .filter(|(_, score)| *score >= threshold)
        .map(|(h, s)| (h, *s))
}

/// When the release's format set fires multiple format-type rules,
/// return the top-scoring candidate per bucket — but only if at least
/// two buckets have a candidate at-or-above `threshold`. Returns `None`
/// in every other case (single bucket, no rules firing, only one bucket
/// has a confident hit) so the caller can take its normal "best wins"
/// path.
fn multi_bucket_confident_hits(
    groups: &FormatKindGroups,
    scored: &[(SearchHit, f32)],
    threshold: f32,
) -> Option<Vec<(SearchHit, f32)>> {
    if groups.groups.len() < 2 {
        return None;
    }
    let mut top: Vec<Option<(SearchHit, f32)>> = vec![None; groups.groups.len()];
    for (hit, score) in scored {
        if *score < threshold {
            // Slice is sorted descending; everything past here is below.
            break;
        }
        for idx in groups.bucket_indexes_for(hit.kind.as_ref()) {
            if top[idx].is_none() {
                top[idx] = Some((hit.clone(), *score));
            }
        }
    }
    let filled: Vec<(SearchHit, f32)> = top.into_iter().flatten().collect();
    (filled.len() >= 2).then_some(filled)
}

/// Drop hits whose `kind` is outside any firing rule's required-kinds.
/// Returns `None` when there's nothing to filter (unconstrained) or when
/// every hit would be filtered out (caller falls back to the original
/// list so post-match validation can still link-and-flag the best
/// available candidate as `ambiguous`).
fn filter_compatible_hits(
    groups: &FormatKindGroups,
    scored: &[(SearchHit, f32)],
) -> Option<Vec<(SearchHit, f32)>> {
    if groups.is_unconstrained() {
        return None;
    }
    let kept: Vec<(SearchHit, f32)> = scored
        .iter()
        .filter(|(h, _)| groups.is_kind_compatible(h.kind.as_ref()))
        .cloned()
        .collect();
    if kept.is_empty() { None } else { Some(kept) }
}
