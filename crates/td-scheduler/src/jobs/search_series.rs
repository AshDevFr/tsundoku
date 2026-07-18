//! Per-series release search driver: one [`SearchSource`] entry walked
//! against every usable title of one series.
//!
//! Mirrors [`super::backfill_source`]: per hit it runs cheap-dedup →
//! enrich → persist → resolve, so search discoveries are indistinguishable
//! from polled ones and are *not* force-linked to the series that launched
//! the search (upstream search is substring-ish; the resolver and review
//! queue sort out what actually matches). Audited in `search_runs` rather
//! than the per-source `poll_runs` lane: a search belongs to a series and
//! a `[[search]]` entry, not to a `[[sources]]` instance.
//!
//! Callers handle contention (the API via `try_dispatch` on the
//! `search:<entry>` lock; the CLI by being its own process); `run` does
//! not acquire locks itself.

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use sea_orm::DatabaseConnection;
use td_config::IngestionConfig;
use td_db::repos::releases_repo::{self, id_for};
use td_db::repos::search_runs_repo::{self, SearchRunCounts};
use td_db::repos::series_repo;
use td_metadata::MetadataRegistry;
use td_resolution::Resolver;
use td_resolution::mangaupdates_redirect::MangaUpdatesRedirector;
use td_resolution::query_builder::QueryBuilder;
use td_source::SearchSource;

/// Queries per run are capped hard: alternate-title lists routinely hold
/// 20+ near-duplicate romanizations, and every query is a full page walk
/// against the upstream.
const MAX_QUERIES: usize = 12;
/// Queries shorter than this (in chars) match half of Nyaa and are noise.
/// CJK scripts pack far more information per character (킹덤, 呪術), so
/// non-ASCII titles get a lower floor.
const MIN_QUERY_CHARS_ASCII: usize = 3;
const MIN_QUERY_CHARS_NON_ASCII: usize = 2;

/// Per-run tallies, printed by the CLI and recorded in `search_runs`.
#[derive(Debug, Clone, Default)]
pub struct SearchSummary {
    pub queries_attempted: u32,
    /// Titles dropped by normalization (too short) or the query cap.
    pub queries_dropped: usize,
    /// `search_page` calls made (includes the empty page that ends a walk).
    pub pages_fetched: u32,
    /// Hits returned by the upstream across all pages, before any dedup.
    pub releases_seen: usize,
    pub releases_new: usize,
    /// Hits whose `(source_kind, external_id)` was already in the catalog.
    pub already_known: usize,
    /// Page-fetch and per-release persist/resolve failures (non-fatal).
    pub errors: usize,
}

/// Search one series against one `[[search]]` entry. `max_pages` is the
/// entry's per-query cap; `trigger` is `manual` | `cli`.
///
/// Errors only on setup faults (series missing, resolver construction).
/// Per-query and per-release failures are logged and counted; the run's
/// `search_runs` row is completed `error` only when every query failed
/// outright, so a partially-degraded upstream still records its work.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    source: Arc<dyn SearchSource>,
    max_pages: u32,
    db: DatabaseConnection,
    metadata: Arc<MetadataRegistry>,
    ingestion: IngestionConfig,
    query_builder: Arc<QueryBuilder>,
    mangaupdates_redirector: Option<Arc<MangaUpdatesRedirector>>,
    series_id: i32,
    trigger: &str,
) -> Result<SearchSummary> {
    let max_pages = max_pages.max(1);
    let entry_name = source.name().to_string();

    let series = series_repo::find_by_id(&db, series_id)
        .await
        .context("loading series")?
        .ok_or_else(|| anyhow!("series {series_id} not found"))?;

    let alternates: Vec<String> = series
        .alternate_titles_json
        .as_deref()
        .and_then(|j| serde_json::from_str(j).ok())
        .unwrap_or_default();
    let (queries, dropped) = build_query_set(&series.canonical_title, &alternates);
    if dropped > 0 {
        tracing::info!(
            series_id,
            dropped,
            kept = queries.len(),
            "query set truncated (short titles and/or over the cap)"
        );
    }
    if queries.is_empty() {
        return Err(anyhow!(
            "series {series_id} has no usable titles to search (all too short)"
        ));
    }

    // Audit row first so the UI can poll `running` while the walk is live.
    // Failure to insert is non-fatal (mirrors the backfill metrics row).
    let run_id = match search_runs_repo::insert_running(
        &db,
        Utc::now().timestamp(),
        &entry_name,
        series_id,
        trigger,
    )
    .await
    {
        Ok(id) => Some(id),
        Err(e) => {
            tracing::warn!(error = ?e, series_id, "failed to record search_runs start");
            None
        }
    };

    let mut resolver =
        Resolver::new(db.clone(), metadata, ingestion).with_query_builder(query_builder);
    if let Some(r) = mangaupdates_redirector {
        resolver = resolver.with_mangaupdates_redirector(r);
    }

    tracing::info!(
        search = %entry_name,
        series_id,
        series = %series.canonical_title,
        queries = queries.len(),
        max_pages,
        trigger = %trigger,
        "series search starting"
    );

    let mut totals = SearchSummary {
        queries_dropped: dropped,
        ..Default::default()
    };
    // Cross-query dedup: overlapping titles (romaji vs English) return the
    // same posts; process each hit once per run.
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut failed_queries = 0u32;
    let mut first_error: Option<String> = None;

    for query in &queries {
        totals.queries_attempted += 1;
        for page in 1..=max_pages {
            let hits = match source.search_page(query, page).await {
                Ok(h) => h,
                Err(e) => {
                    tracing::warn!(
                        error = ?e,
                        search = %entry_name,
                        query = %query,
                        page,
                        "search page failed; abandoning this query"
                    );
                    totals.errors += 1;
                    if page == 1 {
                        failed_queries += 1;
                    }
                    first_error.get_or_insert_with(|| e.to_string());
                    break;
                }
            };
            totals.pages_fetched += 1;
            if hits.is_empty() {
                break;
            }
            for mut release in hits {
                totals.releases_seen += 1;
                let id = id_for(&release.source_kind, &release.external_id);
                if !seen_ids.insert(id.clone()) {
                    continue;
                }
                // Cheap dedup against the catalog: skip the detail fetch +
                // resolver for rows any poll/backfill/search already saw.
                match releases_repo::find_by_id(&db, &id).await {
                    Ok(Some(_)) => {
                        totals.already_known += 1;
                        continue;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!(error = ?e, %id, "find_by_id failed; treating as new");
                    }
                }
                if let Err(e) = source.enrich(&mut release).await {
                    tracing::warn!(
                        error = ?e,
                        external_id = %release.external_id,
                        "enrich failed; persisting with listing-only data"
                    );
                }
                let persisted_id = match releases_repo::persist_discovered(
                    &db,
                    &release,
                    Utc::now().timestamp(),
                )
                .await
                {
                    Ok(id) => id,
                    Err(e) => {
                        tracing::error!(error = ?e, external_id = %release.external_id, "persist failed");
                        totals.errors += 1;
                        continue;
                    }
                };
                if let Err(e) = resolver.resolve_one(&persisted_id).await {
                    tracing::warn!(error = ?e, release_id = %persisted_id, "resolver failed; release left unresolved");
                    totals.errors += 1;
                }
                totals.releases_new += 1;
            }
        }
    }

    // Every query dying on its first page means the upstream (or the
    // config) is broken: surface that as a failed run. Anything less is a
    // partial success with its errors counted.
    let all_failed = failed_queries == totals.queries_attempted && totals.queries_attempted > 0;
    let (outcome, error_msg) = if all_failed {
        (
            search_runs_repo::OUTCOME_ERROR,
            Some(first_error.unwrap_or_else(|| "every query failed".to_string())),
        )
    } else {
        (search_runs_repo::OUTCOME_SUCCESS, None)
    };

    if let Some(run_id) = run_id
        && let Err(e) = search_runs_repo::complete(
            &db,
            run_id,
            Utc::now().timestamp(),
            outcome,
            SearchRunCounts {
                queries_attempted: totals.queries_attempted as i64,
                pages_fetched: totals.pages_fetched as i64,
                releases_seen: totals.releases_seen as i64,
                releases_new: totals.releases_new as i64,
            },
            error_msg.as_deref(),
        )
        .await
    {
        tracing::warn!(error = ?e, series_id, "failed to finalize search_runs row");
    }

    tracing::info!(
        search = %entry_name,
        series_id,
        queries = totals.queries_attempted,
        pages = totals.pages_fetched,
        seen = totals.releases_seen,
        new = totals.releases_new,
        already_known = totals.already_known,
        errors = totals.errors,
        outcome,
        "series search complete"
    );

    Ok(totals)
}

/// Normalize the series' titles into the query list: whitespace collapsed,
/// case-insensitively deduped (canonical first, so it always survives),
/// too-short titles dropped (script-aware floor), capped at [`MAX_QUERIES`].
/// Non-Latin titles are deliberately preserved: they are what makes a
/// raw-category `[[search]]` entry productive. Returns `(queries, dropped)`
/// where `dropped` counts too-short and over-cap titles (exact duplicates
/// collapse silently).
fn build_query_set(canonical: &str, alternates: &[String]) -> (Vec<String>, usize) {
    let mut queries: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    let mut dropped = 0usize;
    for raw in std::iter::once(canonical).chain(alternates.iter().map(String::as_str)) {
        let cleaned = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        let min_chars = if cleaned.is_ascii() {
            MIN_QUERY_CHARS_ASCII
        } else {
            MIN_QUERY_CHARS_NON_ASCII
        };
        if cleaned.chars().count() < min_chars {
            dropped += 1;
            continue;
        }
        if !seen.insert(cleaned.to_lowercase()) {
            continue;
        }
        if queries.len() >= MAX_QUERIES {
            dropped += 1;
            continue;
        }
        queries.push(cleaned);
    }
    (queries, dropped)
}

#[cfg(test)]
mod tests {
    use super::{MAX_QUERIES, build_query_set};

    fn alts(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn canonical_first_then_alternates() {
        let (q, dropped) = build_query_set("Solo Leveling", &alts(&["Only I Level Up"]));
        assert_eq!(q, vec!["Solo Leveling", "Only I Level Up"]);
        assert_eq!(dropped, 0);
    }

    #[test]
    fn dedupes_case_insensitively_keeping_first_spelling() {
        let (q, dropped) = build_query_set(
            "Solo Leveling",
            &alts(&["solo leveling", "SOLO   LEVELING", "Na Honjaman Level Up"]),
        );
        assert_eq!(q, vec!["Solo Leveling", "Na Honjaman Level Up"]);
        // Duplicates collapse silently; nothing was "dropped".
        assert_eq!(dropped, 0);
    }

    #[test]
    fn collapses_internal_whitespace() {
        let (q, _) = build_query_set("Solo   Leveling ", &[]);
        assert_eq!(q, vec!["Solo Leveling"]);
    }

    #[test]
    fn drops_short_titles_and_counts_them() {
        let (q, dropped) = build_query_set("Berserk", &alts(&["OP", "  ", "ワンパンマン"]));
        assert_eq!(q, vec!["Berserk", "ワンパンマン"]);
        assert_eq!(dropped, 2);
    }

    #[test]
    fn preserves_non_latin_titles() {
        let (q, _) = build_query_set("Frieren", &alts(&["葬送のフリーレン"]));
        assert!(q.contains(&"葬送のフリーレン".to_string()));
    }

    #[test]
    fn two_char_cjk_titles_survive_the_length_floor() {
        let (q, dropped) = build_query_set("Kingdom", &alts(&["킹덤", "呪術", "犬"]));
        assert_eq!(q, vec!["Kingdom", "킹덤", "呪術"]);
        // Single-char titles are still dropped, even CJK.
        assert_eq!(dropped, 1);
    }

    #[test]
    fn caps_the_query_count_and_counts_overflow() {
        let many: Vec<String> = (0..20).map(|i| format!("Title Number {i}")).collect();
        let (q, dropped) = build_query_set("Canonical Title", &many);
        assert_eq!(q.len(), MAX_QUERIES);
        assert_eq!(q[0], "Canonical Title");
        // 21 candidates, 12 kept.
        assert_eq!(dropped, 9);
    }
}
