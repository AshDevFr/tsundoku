//! Release read/write helpers.

use anyhow::Result;
use chrono::DateTime;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter,
    QueryOrder, QuerySelect, Set, Statement, TransactionTrait,
};

use crate::repos::tagging_repo::NameUsage;
use td_source::{
    DiscoveredRelease, Span, detect_formats, detect_spans, merge_spans, spans_from_json,
    spans_max_end, spans_to_json,
};

use crate::entities::{release_formats, releases, review_candidates, series};

pub use releases::Model;

/// Compute the stable internal id for a release. Mirrors the
/// `UNIQUE(source_kind, external_id)` constraint on the `releases` table: a
/// single nyaa post surfaced by two different uploader feeds is one row, not
/// two, so `source_name` must not appear in the id. Including it would
/// produce a fresh id on the second poll while the upsert keeps the
/// original row's primary key, leaving the format-attach step's FK
/// reference pointing at a non-existent id.
pub fn id_for(source_kind: &str, external_id: &str) -> String {
    format!("{source_kind}:{external_id}")
}

/// Persist one [`DiscoveredRelease`] into the storage layer: upsert the
/// `releases` row, then idempotently attach every detected format. Returns
/// the internal `releases.id` so callers can chain into the resolution
/// pipeline.
///
/// Idempotency: `releases` upserts on `(source_kind, external_id)` (already
/// enforced by the schema's unique constraint), and `release_formats`
/// upserts on its composite primary key. Re-running the poll on the same
/// upstream is a no-op apart from refreshing the mutable columns (title,
/// magnet, posted_at, size, ...).
pub async fn persist_discovered<C: ConnectionTrait>(
    db: &C,
    release: &DiscoveredRelease,
    observed_at: i64,
) -> Result<String> {
    let id = id_for(&release.source_kind, &release.external_id);
    let active = to_active_model(release, &id, observed_at)?;

    // Both upsert and add_format are idempotent on their unique key, so a
    // partial-failure recovery is a re-poll. The single-writer SQLite pool
    // makes the interleaving here serial in practice. Generic over
    // `ConnectionTrait` so the caller can group several `persist_discovered`
    // calls into one transaction (the batched-write path in
    // `poll_source::run_tick`).
    upsert(db, active).await?;
    for fmt in detect_formats(&release.files) {
        add_format(db, &id, fmt.as_str()).await?;
    }
    Ok(id)
}

/// Reconstruct a [`DiscoveredRelease`] from a persisted row. The inverse of
/// [`to_active_model`] for the source-derived fields: resolution state lives
/// on the row but not on `DiscoveredRelease`, so it is intentionally dropped.
/// Used by the re-enrich job to feed an already-stored release back through
/// `DiscoverySource::enrich` + [`persist_discovered`] (the upsert preserves
/// resolution columns, so re-enriching a resolved row keeps its link).
pub fn model_to_discovered(m: &Model) -> DiscoveredRelease {
    let files = m
        .files_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default();
    let external_links = m
        .extracted_links_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let comment_suggested_links = m
        .comment_suggested_links_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    DiscoveredRelease {
        source_kind: m.source_kind.clone(),
        source_name: m.source_name.clone(),
        external_id: m.external_id.clone(),
        title: m.title.clone(),
        link: m.link.clone(),
        magnet: m.magnet.clone(),
        torrent_url: m.torrent_url.clone(),
        ddl_url: m.ddl_url.clone(),
        info_hash: m.info_hash.clone(),
        size_bytes: m.size_bytes.map(|n| n as u64),
        files,
        description_html: m.description_html.clone(),
        external_links,
        comment_suggested_links,
        information_url: m.information_url.clone(),
        posted_at: DateTime::from_timestamp(m.posted_at, 0).unwrap_or_default(),
    }
}

/// Rows for a given source whose `resolution_status` is in `statuses`, most
/// recently observed first, capped at `limit`. Drives the re-enrich job's
/// status-targeted walk. An empty `statuses` slice matches nothing.
pub async fn select_for_reenrich(
    db: &DatabaseConnection,
    source_kind: &str,
    source_name: &str,
    statuses: &[String],
    limit: u64,
) -> Result<Vec<Model>> {
    if statuses.is_empty() {
        return Ok(Vec::new());
    }
    Ok(releases::Entity::find()
        .filter(releases::Column::SourceKind.eq(source_kind))
        .filter(releases::Column::SourceName.eq(source_name))
        .filter(releases::Column::ResolutionStatus.is_in(statuses.iter().cloned()))
        .order_by_desc(releases::Column::ObservedAt)
        .limit(limit)
        .all(db)
        .await?)
}

/// Map a [`DiscoveredRelease`] into the sea-orm ActiveModel used for upsert.
/// Kept private — callers go through [`persist_discovered`] so the formats
/// attach step is not accidentally skipped.
fn to_active_model(
    release: &DiscoveredRelease,
    id: &str,
    observed_at: i64,
) -> Result<releases::ActiveModel> {
    let extracted_links_json = if release.external_links.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&release.external_links)?)
    };
    let comment_suggested_links_json = if release.comment_suggested_links.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&release.comment_suggested_links)?)
    };
    let files_json = if release.files.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&release.files)?)
    };

    // Volume / chapter spans parsed from the file names (falling back to the
    // title). Stored as gap-preserving JSON lists so the series-link step can
    // bump `series.highest_*` without re-parsing, and so the catalog/feed can
    // show the actual available ranges (with gaps) rather than a single span.
    let spans = detect_spans(&release.files, &release.title);
    let volume_span_json = spans_to_json(&spans.volumes);
    let chapter_span_json = spans_to_json(&spans.chapters);

    Ok(releases::ActiveModel {
        id: Set(id.to_string()),
        source_kind: Set(release.source_kind.clone()),
        source_name: Set(release.source_name.clone()),
        external_id: Set(release.external_id.clone()),
        title: Set(release.title.clone()),
        link: Set(release.link.clone()),
        magnet: Set(release.magnet.clone()),
        torrent_url: Set(release.torrent_url.clone()),
        ddl_url: Set(release.ddl_url.clone()),
        info_hash: Set(release.info_hash.clone()),
        size_bytes: Set(release.size_bytes.map(|n| n as i64)),
        files_json: Set(files_json),
        description_html: Set(release.description_html.clone()),
        extracted_links_json: Set(extracted_links_json),
        comment_suggested_links_json: Set(comment_suggested_links_json),
        information_url: Set(release.information_url.clone()),
        posted_at: Set(release.posted_at.timestamp()),
        observed_at: Set(observed_at),
        series_id: Set(None),
        resolution_path: Set(None),
        resolution_confidence: Set(None),
        resolution_status: Set("unresolved".into()),
        resolution_attempts: Set(0),
        last_resolve_attempt_at: Set(None),
        volume_span_json: Set(volume_span_json),
        chapter_span_json: Set(chapter_span_json),
        resolved_at: Set(None),
        search_queries: Set(None),
        cleanup_rules_applied: Set(None),
        sent_to_client_at: Set(None),
        sent_to_client_label: Set(None),
    })
}

pub async fn upsert<C: ConnectionTrait>(db: &C, model: releases::ActiveModel) -> Result<()> {
    releases::Entity::insert(model)
        .on_conflict(
            OnConflict::columns([releases::Column::SourceKind, releases::Column::ExternalId])
                .update_columns([
                    releases::Column::Title,
                    releases::Column::Link,
                    releases::Column::Magnet,
                    releases::Column::TorrentUrl,
                    releases::Column::DdlUrl,
                    releases::Column::InfoHash,
                    releases::Column::SizeBytes,
                    releases::Column::FilesJson,
                    releases::Column::DescriptionHtml,
                    releases::Column::ExtractedLinksJson,
                    releases::Column::CommentSuggestedLinksJson,
                    releases::Column::InformationUrl,
                    releases::Column::PostedAt,
                    releases::Column::VolumeSpanJson,
                    releases::Column::ChapterSpanJson,
                ])
                .to_owned(),
        )
        .exec(db)
        .await?;
    Ok(())
}

pub async fn find_by_id(db: &DatabaseConnection, id: &str) -> Result<Option<Model>> {
    Ok(releases::Entity::find_by_id(id.to_string()).one(db).await?)
}

/// Most recent `external_id`s for a `(source_kind, source_name)`. Used by
/// the scheduler / one-shot poll to populate `PollContext.recently_seen`
/// so sources can drop overlapping items before per-item enrichment runs.
pub async fn recent_external_ids(
    db: &DatabaseConnection,
    source_kind: &str,
    source_name: &str,
    limit: u64,
) -> Result<Vec<String>> {
    let rows = releases::Entity::find()
        .select_only()
        .column(releases::Column::ExternalId)
        .filter(releases::Column::SourceKind.eq(source_kind))
        .filter(releases::Column::SourceName.eq(source_name))
        .order_by_desc(releases::Column::PostedAt)
        .limit(limit)
        .into_tuple::<String>()
        .all(db)
        .await?;
    Ok(rows)
}

pub async fn list_by_status(
    db: &DatabaseConnection,
    status: &str,
    limit: u64,
) -> Result<Vec<Model>> {
    Ok(releases::Entity::find()
        .filter(releases::Column::ResolutionStatus.eq(status))
        .order_by_desc(releases::Column::ObservedAt)
        .limit(limit)
        .all(db)
        .await?)
}

/// Variant of [`list_by_status`] used by the "retry all (including
/// resolved)" path. Excludes rows whose `resolution_path` is `'manual'`:
/// those represent operator decisions made via `POST /releases/{id}/link`
/// and must not be silently overwritten by a bulk re-resolve.
pub async fn list_by_status_excluding_manual(
    db: &DatabaseConnection,
    status: &str,
    limit: u64,
) -> Result<Vec<Model>> {
    Ok(releases::Entity::find()
        .filter(releases::Column::ResolutionStatus.eq(status))
        .filter(
            releases::Column::ResolutionPath
                .ne("manual")
                .or(releases::Column::ResolutionPath.is_null()),
        )
        .order_by_desc(releases::Column::ObservedAt)
        .limit(limit)
        .all(db)
        .await?)
}

/// Record resolution outcome on a release. Does not touch the format rows.
pub async fn set_resolution(
    db: &DatabaseConnection,
    id: &str,
    series_id: Option<i32>,
    path: Option<String>,
    confidence: Option<f64>,
    status: &str,
    attempted_at: i64,
) -> Result<()> {
    let model = releases::ActiveModel {
        id: Set(id.to_string()),
        series_id: Set(series_id),
        resolution_path: Set(path),
        resolution_confidence: Set(confidence),
        resolution_status: Set(status.to_string()),
        last_resolve_attempt_at: Set(Some(attempted_at)),
        ..Default::default()
    };
    releases::Entity::update(model).exec(db).await?;
    Ok(())
}

/// Discovery sources that have surfaced at least one *linked* release, with
/// the count of distinct series each one resolved to. Sorted by descending
/// series count, then name. Powers the admin-only source filter's dropdown;
/// a source with only unresolved releases (`series_id IS NULL`) never appears,
/// since it can't narrow the series list anyway.
pub async fn list_sources_with_series_counts(db: &DatabaseConnection) -> Result<Vec<NameUsage>> {
    let backend = db.get_database_backend();
    let sql = "SELECT source_name AS name, COUNT(DISTINCT series_id) AS series_count
               FROM releases
               WHERE series_id IS NOT NULL
               GROUP BY source_name
               ORDER BY series_count DESC, name ASC";
    let stmt = Statement::from_sql_and_values(backend, sql, []);
    let rows = NameUsage::find_by_statement(stmt).all(db).await?;
    Ok(rows)
}

/// Record that a release was pushed to the operator's torrent client: stamp
/// `sent_to_client_at` (epoch seconds) and the `label` that was used (the
/// resolved per-send label, which may be `None`). Drives the "Sent" badge.
pub async fn mark_sent_to_client(
    db: &DatabaseConnection,
    id: &str,
    at: i64,
    label: Option<String>,
) -> Result<()> {
    let model = releases::ActiveModel {
        id: Set(id.to_string()),
        sent_to_client_at: Set(Some(at)),
        sent_to_client_label: Set(label),
        ..Default::default()
    };
    releases::Entity::update(model).exec(db).await?;
    Ok(())
}

/// Reject every release in `ids` in one shot: pin status to `rejected`,
/// clear the linked series, bump the attempt counter, and drop their review
/// candidates. Mirrors the single-release reject path (see
/// `td_resolution::persist::link_release`) but as a set-based update so the
/// bulk action doesn't loop per row. Returns the number of releases updated.
///
/// `resolved_at` is intentionally left untouched: it's only stamped on a
/// transition to `resolved`, and a rejected release was never resolved.
pub async fn bulk_reject(db: &DatabaseConnection, ids: &[String], now: i64) -> Result<u64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let txn = db.begin().await?;
    // Capture the series these releases were linked to *before* we clear the
    // link, so we can shrink their coverage afterward. This path bypasses
    // `link_release`, so it owns its own coverage maintenance.
    let mut affected: Vec<i32> = releases::Entity::find()
        .filter(releases::Column::Id.is_in(ids.iter().cloned()))
        .filter(releases::Column::SeriesId.is_not_null())
        .all(&txn)
        .await?
        .into_iter()
        .filter_map(|r| r.series_id)
        .collect();
    affected.sort_unstable();
    affected.dedup();

    let res = releases::Entity::update_many()
        .col_expr(
            releases::Column::ResolutionStatus,
            Expr::value("rejected".to_string()),
        )
        .col_expr(
            releases::Column::ResolutionPath,
            Expr::value("rejected".to_string()),
        )
        .col_expr(
            releases::Column::ResolutionConfidence,
            Expr::value(Option::<f64>::None),
        )
        .col_expr(releases::Column::SeriesId, Expr::value(Option::<i32>::None))
        .col_expr(releases::Column::LastResolveAttemptAt, Expr::value(now))
        .col_expr(
            releases::Column::ResolutionAttempts,
            Expr::col(releases::Column::ResolutionAttempts).add(1),
        )
        .filter(releases::Column::Id.is_in(ids.iter().cloned()))
        .exec(&txn)
        .await?;
    review_candidates::Entity::delete_many()
        .filter(review_candidates::Column::ReleaseId.is_in(ids.iter().cloned()))
        .exec(&txn)
        .await?;
    // Now that the links are cleared, recompute each affected series' coverage
    // (it can only shrink) and bump `updated_at` where it moved.
    for sid in affected {
        recompute_series_coverage(&txn, sid, now).await?;
    }
    txn.commit().await?;
    Ok(res.rows_affected)
}

/// Tallies from [`recompute_all_spans`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SpanRecompute {
    /// Releases whose stored `volume_span_json` / `chapter_span_json` was
    /// rewritten because re-parsing produced a different value.
    pub releases_rewritten: u64,
    /// Series rows whose coverage / `highest_*` changed (and whose
    /// `updated_at` was therefore bumped).
    pub series_updated: u64,
}

/// The four series columns derived from a set of per-release spans.
struct Coverage {
    volume_json: Option<String>,
    chapter_json: Option<String>,
    highest_volume: Option<f64>,
    highest_chapter: Option<f64>,
}

/// Merge accumulated per-release spans into a series' coverage fields. The
/// `highest_*` marks are just the max end of the merged lists, so coverage and
/// the marks can never drift apart.
fn coverage_of(mut volumes: Vec<Span>, mut chapters: Vec<Span>) -> Coverage {
    merge_spans(&mut volumes);
    merge_spans(&mut chapters);
    Coverage {
        highest_volume: spans_max_end(&volumes),
        highest_chapter: spans_max_end(&chapters),
        volume_json: spans_to_json(&volumes),
        chapter_json: spans_to_json(&chapters),
    }
}

/// True when `row` already stores exactly this coverage, so no write — and no
/// `updated_at` bump — is needed.
fn coverage_unchanged(row: &series::Model, cov: &Coverage) -> bool {
    row.volume_coverage_json == cov.volume_json
        && row.chapter_coverage_json == cov.chapter_json
        && row.highest_volume == cov.highest_volume
        && row.highest_chapter == cov.highest_chapter
}

/// Recompute one series' merged volume/chapter coverage and `highest_*` from
/// its currently-linked releases' stored spans, bumping `updated_at` to `now`
/// **only when something actually changed**. Returns `true` if the row was
/// rewritten.
///
/// This is the authoritative per-series maintenance invoked on every assign /
/// reject / re-link. Unlike a monotonic bump it can *lower* coverage when a
/// release is unlinked, because it re-merges from scratch over the releases
/// that are linked *now*. A re-link is two calls (old series + new series).
/// The no-op short-circuit is what keeps `updated_at` — and the release feed —
/// from churning when nothing about coverage moved.
pub async fn recompute_series_coverage<C: ConnectionTrait>(
    db: &C,
    series_id: i32,
    now: i64,
) -> Result<bool> {
    let Some(row) = series::Entity::find_by_id(series_id).one(db).await? else {
        return Ok(false);
    };
    let linked = releases::Entity::find()
        .filter(releases::Column::SeriesId.eq(series_id))
        .all(db)
        .await?;
    let mut volumes = Vec::new();
    let mut chapters = Vec::new();
    for r in &linked {
        if r.volume_span_json.is_none() && r.chapter_span_json.is_none() {
            // Legacy row persisted before span detection: re-derive from the
            // stored file list (falling back to the title) so linking one still
            // contributes coverage without waiting for a `recompute-spans`
            // backfill. Marker-less rows yield nothing, which is correct.
            let files: Vec<String> = r
                .files_json
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default();
            let spans = detect_spans(&files, &r.title);
            volumes.extend(spans.volumes);
            chapters.extend(spans.chapters);
        } else {
            volumes.extend(spans_from_json(r.volume_span_json.as_deref()));
            chapters.extend(spans_from_json(r.chapter_span_json.as_deref()));
        }
    }
    let cov = coverage_of(volumes, chapters);
    if coverage_unchanged(&row, &cov) {
        return Ok(false);
    }
    let model = series::ActiveModel {
        id: Set(series_id),
        volume_coverage_json: Set(cov.volume_json),
        chapter_coverage_json: Set(cov.chapter_json),
        highest_volume: Set(cov.highest_volume),
        highest_chapter: Set(cov.highest_chapter),
        updated_at: Set(now),
        ..Default::default()
    };
    series::Entity::update(model).exec(db).await?;
    Ok(true)
}

/// Authoritatively recompute every release's volume/chapter span and every
/// series' coverage + `highest_*` marks from scratch. Unlike the per-link
/// `recompute_series_coverage`, this re-derives each release's span from its
/// stored file list (so it also corrects an earlier, more eager parse) and
/// then rebuilds coverage for every series. `updated_at` is bumped to `now`
/// only for series whose coverage/marks actually move. Run it after a
/// parsing-strategy change or to backfill a catalog that predates span
/// detection or this coverage column.
///
/// Pure DB + lexical parsing: makes no network calls and does not touch
/// resolution state. Idempotent — running it twice in a row leaves the
/// second run reporting zero changes.
pub async fn recompute_all_spans(db: &DatabaseConnection, now: i64) -> Result<SpanRecompute> {
    use std::collections::HashMap;

    let mut summary = SpanRecompute::default();
    // series_id -> accumulated (unmerged) volume/chapter spans across its
    // currently-linked releases.
    let mut per_series: HashMap<i32, (Vec<Span>, Vec<Span>)> = HashMap::new();

    // Walk every release once, re-deriving its span from the stored file
    // list (falling back to the title) and rewriting the columns when the
    // re-parse disagrees with what's on disk.
    let all = releases::Entity::find().all(db).await?;
    for rel in &all {
        let files: Vec<String> = rel
            .files_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        let spans = detect_spans(&files, &rel.title);
        let vol_json = spans_to_json(&spans.volumes);
        let chap_json = spans_to_json(&spans.chapters);

        if vol_json != rel.volume_span_json || chap_json != rel.chapter_span_json {
            let model = releases::ActiveModel {
                id: Set(rel.id.clone()),
                volume_span_json: Set(vol_json),
                chapter_span_json: Set(chap_json),
                ..Default::default()
            };
            releases::Entity::update(model).exec(db).await?;
            summary.releases_rewritten += 1;
        }

        if let Some(sid) = rel.series_id {
            let entry = per_series.entry(sid).or_default();
            entry.0.extend(spans.volumes.iter().copied());
            entry.1.extend(spans.chapters.iter().copied());
        }
    }

    // Rebuild each series' coverage from the freshly-aggregated spans. Series
    // with no linked release that parses sink to empty/NULL. Only rows that
    // actually change are written (and only those bump `updated_at`).
    let series_rows = series::Entity::find().all(db).await?;
    for s in series_rows {
        let (volumes, chapters) = per_series.remove(&s.id).unwrap_or_default();
        let cov = coverage_of(volumes, chapters);
        if coverage_unchanged(&s, &cov) {
            continue;
        }
        let model = series::ActiveModel {
            id: Set(s.id),
            volume_coverage_json: Set(cov.volume_json),
            chapter_coverage_json: Set(cov.chapter_json),
            highest_volume: Set(cov.highest_volume),
            highest_chapter: Set(cov.highest_chapter),
            updated_at: Set(now),
            ..Default::default()
        };
        series::Entity::update(model).exec(db).await?;
        summary.series_updated += 1;
    }

    Ok(summary)
}

/// Idempotently attach a format tag to a release.
pub async fn add_format<C: ConnectionTrait>(db: &C, release_id: &str, format: &str) -> Result<()> {
    let row = release_formats::ActiveModel {
        release_id: Set(release_id.to_string()),
        format: Set(format.to_string()),
    };
    release_formats::Entity::insert(row)
        .on_conflict(
            OnConflict::columns([
                release_formats::Column::ReleaseId,
                release_formats::Column::Format,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(db)
        .await?;
    Ok(())
}

pub async fn list_formats(db: &DatabaseConnection, release_id: &str) -> Result<Vec<String>> {
    let rows = release_formats::Entity::find()
        .filter(release_formats::Column::ReleaseId.eq(release_id))
        .all(db)
        .await?;
    Ok(rows.into_iter().map(|r| r.format).collect())
}

#[derive(Debug, FromQueryResult)]
struct SeriesIdCount {
    series_id: i32,
    n: i64,
}

/// Batch count releases per series id, returned as a map. Series ids with
/// no releases are omitted from the map (callers should treat absence as
/// zero). One SELECT used by the series list endpoint to avoid N+1.
///
/// The COUNT includes every release linked to the series row regardless
/// of `resolution_status`, matching what `GET /releases?seriesId=…`
/// returns (which is what the detail page renders) so the badge and the
/// detail page can't disagree.
pub async fn count_by_series_ids(
    db: &DatabaseConnection,
    series_ids: &[i32],
) -> Result<std::collections::HashMap<i32, i64>> {
    if series_ids.is_empty() {
        return Ok(Default::default());
    }
    let placeholders = vec!["?"; series_ids.len()].join(",");
    let sql = format!(
        "SELECT series_id AS series_id, COUNT(*) AS n FROM releases \
         WHERE series_id IN ({placeholders}) GROUP BY series_id"
    );
    let backend = db.get_database_backend();
    let values: Vec<sea_orm::Value> = series_ids.iter().map(|id| (*id as i64).into()).collect();
    let stmt = sea_orm::Statement::from_sql_and_values(backend, &sql, values);
    let rows = SeriesIdCount::find_by_statement(stmt).all(db).await?;
    Ok(rows.into_iter().map(|r| (r.series_id, r.n)).collect())
}

/// All releases linked to a batch of series, grouped by `series_id` and
/// ordered newest-first (`posted_at` desc) within each series. Series with
/// no linked releases are omitted (callers treat absence as an empty list).
/// Used by the catalog export's optional `includeReleases` to avoid an N+1.
pub async fn list_by_series_ids(
    db: &DatabaseConnection,
    series_ids: &[i32],
) -> Result<std::collections::HashMap<i32, Vec<Model>>> {
    if series_ids.is_empty() {
        return Ok(Default::default());
    }
    let rows = releases::Entity::find()
        .filter(releases::Column::SeriesId.is_in(series_ids.iter().copied()))
        .order_by_desc(releases::Column::PostedAt)
        .all(db)
        .await?;
    let mut map: std::collections::HashMap<i32, Vec<Model>> = std::collections::HashMap::new();
    for row in rows {
        // `series_id` is guaranteed Some by the IN filter above.
        if let Some(sid) = row.series_id {
            map.entry(sid).or_default().push(row);
        }
    }
    Ok(map)
}

pub use releases::{ActiveModel, Column, Entity};

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use migration::{Migrator, MigratorTrait};
    use sea_orm::Database;
    use td_source::{DiscoveredRelease, ExternalLinks};

    async fn fresh_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db
    }

    fn sample(source_name: &str) -> DiscoveredRelease {
        DiscoveredRelease {
            source_kind: "nyaa".into(),
            source_name: source_name.into(),
            external_id: "2095990".into(),
            title: "Some Manga v01 (Digital)".into(),
            link: "https://nyaa.si/view/2095990".into(),
            magnet: None,
            torrent_url: None,
            ddl_url: None,
            info_hash: None,
            size_bytes: None,
            files: vec!["Some Manga v01.cbz".into()],
            description_html: None,
            external_links: ExternalLinks::default(),
            comment_suggested_links: ExternalLinks::default(),
            information_url: None,
            posted_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
        }
    }

    /// Regression: two uploader feeds (different `source_name`) can surface
    /// the same nyaa post. The unique constraint `UNIQUE(source_kind,
    /// external_id)` means it's one row in `releases`; the synthetic id
    /// must therefore be derivable from `(source_kind, external_id)` alone,
    /// or the format-attach step's FK reference goes stale on the second
    /// poll.
    #[tokio::test]
    async fn duplicate_post_under_two_source_names_is_idempotent() {
        let db = fresh_db().await;
        let first = sample("nyaa-uploaderA");
        let second = sample("nyaa-uploaderB");

        let id_first = persist_discovered(&db, &first, 1_700_000_100)
            .await
            .unwrap();
        let id_second = persist_discovered(&db, &second, 1_700_000_200)
            .await
            .unwrap();

        assert_eq!(
            id_first, id_second,
            "the same (source_kind, external_id) must produce the same release id regardless of source_name"
        );

        let row_count = releases::Entity::find()
            .filter(releases::Column::SourceKind.eq("nyaa"))
            .filter(releases::Column::ExternalId.eq("2095990"))
            .all(&db)
            .await
            .unwrap()
            .len();
        assert_eq!(row_count, 1, "duplicate poll must not create a second row");

        let formats = list_formats(&db, &id_second).await.unwrap();
        assert_eq!(
            formats,
            vec!["cbz"],
            "format must attach to the surviving row"
        );
    }

    #[tokio::test]
    async fn list_by_series_ids_groups_orders_desc_and_omits_empties() {
        use crate::entities::series;
        use sea_orm::{ActiveModelTrait, Set};

        let db = fresh_db().await;
        let mk_series = |title: &str| {
            let title = title.to_string();
            let db = &db;
            async move {
                series::ActiveModel {
                    canonical_title: Set(title),
                    metadata_source: Set("test".into()),
                    metadata_fetched_at: Set(1),
                    first_seen_at: Set(1),
                    last_release_at: Set(1),
                    owned: Set(0),
                    ..Default::default()
                }
                .insert(db)
                .await
                .unwrap()
                .id
            }
        };
        let s1 = mk_series("S1").await;
        let s2 = mk_series("S2").await;

        // Two releases linked to s1 with distinct posted_at; one (newer) and
        // one (older). s2 gets no releases.
        let mut older = sample("feed");
        older.external_id = "old".into();
        older.link = "https://nyaa.si/view/old".into();
        older.posted_at = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let id_old = persist_discovered(&db, &older, 1).await.unwrap();

        let mut newer = sample("feed");
        newer.external_id = "new".into();
        newer.link = "https://nyaa.si/view/new".into();
        newer.posted_at = Utc.timestamp_opt(1_700_009_999, 0).unwrap();
        let id_new = persist_discovered(&db, &newer, 1).await.unwrap();

        for id in [&id_old, &id_new] {
            releases::ActiveModel {
                id: Set(id.clone()),
                series_id: Set(Some(s1)),
                ..Default::default()
            }
            .update(&db)
            .await
            .unwrap();
        }

        let map = list_by_series_ids(&db, &[s1, s2]).await.unwrap();
        assert_eq!(map.len(), 1, "series with no linked releases are omitted");
        let s1_rows = &map[&s1];
        assert_eq!(s1_rows.len(), 2);
        assert_eq!(
            s1_rows[0].id, id_new,
            "newest (highest posted_at) comes first"
        );
        assert_eq!(s1_rows[1].id, id_old);
        assert!(!map.contains_key(&s2));
    }

    #[tokio::test]
    async fn list_sources_with_series_counts_ranks_by_distinct_series_and_skips_unlinked() {
        use crate::entities::series;
        use sea_orm::{ActiveModelTrait, Set};

        let db = fresh_db().await;
        let mk_series = |title: &str| {
            let title = title.to_string();
            let db = &db;
            async move {
                series::ActiveModel {
                    canonical_title: Set(title),
                    metadata_source: Set("test".into()),
                    metadata_fetched_at: Set(1),
                    first_seen_at: Set(1),
                    last_release_at: Set(1),
                    owned: Set(0),
                    ..Default::default()
                }
                .insert(db)
                .await
                .unwrap()
                .id
            }
        };
        let s1 = mk_series("S1").await;
        let s2 = mk_series("S2").await;

        // Persist releases on three feeds, then link some to series:
        //   alpha -> s1 and s2 (2 distinct series)
        //   beta  -> s1 only   (1 series)
        //   gamma -> unlinked  (must not appear at all)
        let link = |external_id: &str, feed: &str, series_id: Option<i32>| {
            let external_id = external_id.to_string();
            let feed = feed.to_string();
            let db = &db;
            async move {
                let mut r = sample(&feed);
                r.external_id = external_id.clone();
                r.link = format!("https://nyaa.si/view/{external_id}");
                let id = persist_discovered(db, &r, 1).await.unwrap();
                if let Some(sid) = series_id {
                    releases::ActiveModel {
                        id: Set(id.clone()),
                        series_id: Set(Some(sid)),
                        ..Default::default()
                    }
                    .update(db)
                    .await
                    .unwrap();
                }
                id
            }
        };
        link("a1", "alpha", Some(s1)).await;
        link("a2", "alpha", Some(s2)).await;
        link("b1", "beta", Some(s1)).await;
        link("g1", "gamma", None).await;

        let rows = list_sources_with_series_counts(&db).await.unwrap();
        assert_eq!(
            rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            vec!["alpha", "beta"],
            "alpha (2 series) ranks before beta (1); gamma (unlinked) is omitted"
        );
        assert_eq!(rows[0].series_count, 2);
        assert_eq!(rows[1].series_count, 1);
    }

    #[tokio::test]
    async fn list_by_series_ids_empty_input_is_empty_map() {
        let db = fresh_db().await;
        let map = list_by_series_ids(&db, &[]).await.unwrap();
        assert!(map.is_empty());
    }

    #[tokio::test]
    async fn bulk_reject_sets_status_increments_attempts_and_clears_candidates() {
        use crate::entities::{review_candidates, series};
        use sea_orm::{ActiveModelTrait, Set};

        let db = fresh_db().await;
        let a = persist_discovered(&db, &sample("feed"), 1_700_000_100)
            .await
            .unwrap();
        let mut second = sample("feed");
        second.external_id = "999".into();
        second.link = "https://nyaa.si/view/999".into();
        let b = persist_discovered(&db, &second, 1_700_000_200)
            .await
            .unwrap();

        // A candidate row on `a` must be cleared by the reject.
        let series_id = series::ActiveModel {
            canonical_title: Set("Cand".into()),
            metadata_source: Set("test".into()),
            metadata_fetched_at: Set(1),
            first_seen_at: Set(1),
            last_release_at: Set(1),
            owned: Set(0),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap()
        .id;
        review_candidates::ActiveModel {
            release_id: Set(a.clone()),
            series_id: Set(series_id),
            score: Set(0.5),
            reason: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();

        let rejected = bulk_reject(&db, std::slice::from_ref(&a), 1_700_000_500)
            .await
            .unwrap();
        assert_eq!(rejected, 1, "only the targeted release is rejected");

        let row_a = find_by_id(&db, &a).await.unwrap().unwrap();
        assert_eq!(row_a.resolution_status, "rejected");
        assert_eq!(row_a.resolution_path.as_deref(), Some("rejected"));
        assert_eq!(row_a.series_id, None);
        assert_eq!(row_a.last_resolve_attempt_at, Some(1_700_000_500));
        assert_eq!(row_a.resolution_attempts, 1, "attempts incremented");
        assert!(
            review_candidates::Entity::find()
                .filter(review_candidates::Column::ReleaseId.eq(a.as_str()))
                .all(&db)
                .await
                .unwrap()
                .is_empty(),
            "candidates cleared for rejected release"
        );

        // The untouched release keeps its original status.
        let row_b = find_by_id(&db, &b).await.unwrap().unwrap();
        assert_eq!(row_b.resolution_status, "unresolved");

        // Empty id list is a no-op.
        assert_eq!(bulk_reject(&db, &[], 1).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn recompute_all_spans_sets_series_highest_and_is_idempotent() {
        use crate::entities::series;
        use sea_orm::{ActiveModelTrait, Set};

        let db = fresh_db().await;
        let series_id = series::ActiveModel {
            canonical_title: Set("Recompute Me".into()),
            metadata_source: Set("test".into()),
            metadata_fetched_at: Set(1),
            first_seen_at: Set(1),
            last_release_at: Set(1),
            owned: Set(0),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap()
        .id;

        // Two linked releases: v01-03 and v05 → series tops out at volume 5.
        let mut r1 = sample("feed");
        r1.external_id = "r1".into();
        r1.link = "https://nyaa.si/view/r1".into();
        r1.files = vec!["Series v01-03.cbz".into()];
        let id1 = persist_discovered(&db, &r1, 1).await.unwrap();
        let mut r2 = sample("feed");
        r2.external_id = "r2".into();
        r2.link = "https://nyaa.si/view/r2".into();
        r2.files = vec!["Series v05 c050.cbz".into()];
        let id2 = persist_discovered(&db, &r2, 1).await.unwrap();
        for id in [&id1, &id2] {
            set_resolution(
                &db,
                id,
                Some(series_id),
                Some("test".into()),
                None,
                "resolved",
                1,
            )
            .await
            .unwrap();
        }

        let first = recompute_all_spans(&db, 1_700_000_000).await.unwrap();
        assert_eq!(first.series_updated, 1);
        let row = series::Entity::find_by_id(series_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.highest_volume, Some(5.0));
        assert_eq!(row.highest_chapter, Some(50.0));
        // Coverage is populated and merged across the two releases. v01-03 and
        // v05 stay disjoint (volume 4 is missing), and `updated_at` is stamped.
        assert_eq!(
            td_source::spans_from_json(row.volume_coverage_json.as_deref()),
            vec![
                td_source::Span {
                    start: 1.0,
                    end: 3.0,
                },
                td_source::Span {
                    start: 5.0,
                    end: 5.0,
                },
            ],
        );
        assert_eq!(
            td_source::spans_from_json(row.chapter_coverage_json.as_deref()),
            vec![td_source::Span {
                start: 50.0,
                end: 50.0,
            }],
        );
        assert_eq!(row.updated_at, 1_700_000_000);

        // Second run is a no-op: spans already stored, series already correct,
        // so `updated_at` is left untouched.
        let second = recompute_all_spans(&db, 1_700_000_999).await.unwrap();
        assert_eq!(second, SpanRecompute::default());
        let row = series::Entity::find_by_id(series_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.updated_at, 1_700_000_000,
            "no-op must not bump updated_at"
        );
    }

    #[tokio::test]
    async fn recompute_rewrites_legacy_spans_to_arrays_and_preserves_gaps() {
        use crate::entities::releases;
        use sea_orm::{ActiveModelTrait, Set};

        let db = fresh_db().await;

        // Files cover v01-04 and v06-09 — volume 5 is genuinely missing.
        let mut r = sample("feed");
        r.external_id = "gap".into();
        r.link = "https://nyaa.si/view/gap".into();
        r.files = vec!["Series v01-04.cbz".into(), "Series v06-09.cbz".into()];
        let id = persist_discovered(&db, &r, 1).await.unwrap();

        // Simulate a row written before spans became lists: overwrite the
        // column with the legacy single-object shape.
        releases::ActiveModel {
            id: Set(id.clone()),
            volume_span_json: Set(Some(r#"{"start":1.0,"end":9.0}"#.into())),
            ..Default::default()
        }
        .update(&db)
        .await
        .unwrap();

        // Recompute re-derives from the file list (still intact) and rewrites
        // the column to the gap-preserving array form — no re-poll needed.
        let summary = recompute_all_spans(&db, 1_700_000_000).await.unwrap();
        assert_eq!(summary.releases_rewritten, 1);

        let row = find_by_id(&db, &id).await.unwrap().unwrap();
        let raw = row.volume_span_json.as_deref().unwrap();
        assert!(raw.starts_with('['), "stored as a JSON array, got {raw}");
        assert_eq!(
            td_source::spans_from_json(Some(raw)),
            vec![
                td_source::Span {
                    start: 1.0,
                    end: 4.0,
                },
                td_source::Span {
                    start: 6.0,
                    end: 9.0,
                },
            ],
            "the gap at volume 5 survives as two entries",
        );

        // Idempotent once the array shape is stored.
        let second = recompute_all_spans(&db, 1_700_000_000).await.unwrap();
        assert_eq!(second.releases_rewritten, 0);
    }

    /// Insert a series and two releases linked to it, the first covering
    /// `v01-04` and the second `v06-09` (volume 5 missing). Returns the series
    /// id and the two release ids.
    async fn series_with_two_gapped_releases(db: &DatabaseConnection) -> (i32, String, String) {
        use crate::entities::series;
        use sea_orm::{ActiveModelTrait, Set};

        let sid = series::ActiveModel {
            canonical_title: Set("Cov".into()),
            metadata_source: Set("test".into()),
            metadata_fetched_at: Set(1),
            first_seen_at: Set(1),
            last_release_at: Set(1),
            owned: Set(0),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap()
        .id;

        let mut r1 = sample("feed");
        r1.external_id = "c1".into();
        r1.link = "https://nyaa.si/view/c1".into();
        r1.files = vec!["Series v01-04.cbz".into()];
        let id1 = persist_discovered(db, &r1, 1).await.unwrap();

        let mut r2 = sample("feed");
        r2.external_id = "c2".into();
        r2.link = "https://nyaa.si/view/c2".into();
        r2.files = vec!["Series v06-09.cbz".into()];
        let id2 = persist_discovered(db, &r2, 1).await.unwrap();

        for id in [&id1, &id2] {
            set_resolution(db, id, Some(sid), Some("test".into()), None, "resolved", 1)
                .await
                .unwrap();
        }
        (sid, id1, id2)
    }

    async fn series_row(db: &DatabaseConnection, sid: i32) -> crate::entities::series::Model {
        crate::entities::series::Entity::find_by_id(sid)
            .one(db)
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn recompute_series_coverage_merges_links_and_no_op_skips_bump() {
        let db = fresh_db().await;
        let (sid, _id1, _id2) = series_with_two_gapped_releases(&db).await;

        let changed = recompute_series_coverage(&db, sid, 1_700_000_000)
            .await
            .unwrap();
        assert!(changed);
        let row = series_row(&db, sid).await;
        assert_eq!(
            td_source::spans_from_json(row.volume_coverage_json.as_deref()),
            vec![
                td_source::Span {
                    start: 1.0,
                    end: 4.0,
                },
                td_source::Span {
                    start: 6.0,
                    end: 9.0,
                },
            ],
            "the gap at volume 5 survives in series coverage",
        );
        assert_eq!(row.highest_volume, Some(9.0));
        assert_eq!(row.updated_at, 1_700_000_000);

        // No-op: nothing changed, so `updated_at` must NOT move.
        let again = recompute_series_coverage(&db, sid, 1_700_009_999)
            .await
            .unwrap();
        assert!(!again);
        assert_eq!(series_row(&db, sid).await.updated_at, 1_700_000_000);
    }

    #[tokio::test]
    async fn recompute_series_coverage_shrinks_when_a_release_unlinks() {
        use sea_orm::{ActiveModelTrait, Set};
        let db = fresh_db().await;
        let (sid, _id1, id2) = series_with_two_gapped_releases(&db).await;
        recompute_series_coverage(&db, sid, 1_700_000_000)
            .await
            .unwrap();

        // Unlink the second release; coverage (and `highest_volume`) must drop,
        // which the old monotonic bump could never do.
        releases::ActiveModel {
            id: Set(id2),
            series_id: Set(None),
            ..Default::default()
        }
        .update(&db)
        .await
        .unwrap();

        let changed = recompute_series_coverage(&db, sid, 1_700_000_500)
            .await
            .unwrap();
        assert!(changed);
        let row = series_row(&db, sid).await;
        assert_eq!(
            td_source::spans_from_json(row.volume_coverage_json.as_deref()),
            vec![td_source::Span {
                start: 1.0,
                end: 4.0,
            }],
        );
        assert_eq!(row.highest_volume, Some(4.0), "highest dropped on unlink");
        assert_eq!(row.updated_at, 1_700_000_500);
    }

    #[tokio::test]
    async fn bulk_reject_shrinks_affected_series_coverage() {
        let db = fresh_db().await;
        let (sid, _id1, id2) = series_with_two_gapped_releases(&db).await;
        recompute_series_coverage(&db, sid, 1_700_000_000)
            .await
            .unwrap();

        // Rejecting the second release clears its link; bulk_reject owns its
        // own coverage maintenance (it bypasses link_release).
        bulk_reject(&db, &[id2], 1_700_000_700).await.unwrap();

        let row = series_row(&db, sid).await;
        assert_eq!(
            td_source::spans_from_json(row.volume_coverage_json.as_deref()),
            vec![td_source::Span {
                start: 1.0,
                end: 4.0,
            }],
        );
        assert_eq!(row.highest_volume, Some(4.0));
        assert_eq!(row.updated_at, 1_700_000_700);
    }

    #[tokio::test]
    async fn mark_sent_to_client_stamps_time_and_label() {
        let db = fresh_db().await;
        let id = persist_discovered(&db, &sample("feed"), 1).await.unwrap();

        // Fresh rows are unsent.
        let before = find_by_id(&db, &id).await.unwrap().unwrap();
        assert!(before.sent_to_client_at.is_none());
        assert!(before.sent_to_client_label.is_none());

        mark_sent_to_client(&db, &id, 1_700_000_500, Some("manga".into()))
            .await
            .unwrap();
        let after = find_by_id(&db, &id).await.unwrap().unwrap();
        assert_eq!(after.sent_to_client_at, Some(1_700_000_500));
        assert_eq!(after.sent_to_client_label.as_deref(), Some("manga"));

        // A label-less send still stamps the timestamp and clears the label.
        mark_sent_to_client(&db, &id, 1_700_000_600, None)
            .await
            .unwrap();
        let relabeled = find_by_id(&db, &id).await.unwrap().unwrap();
        assert_eq!(relabeled.sent_to_client_at, Some(1_700_000_600));
        assert!(relabeled.sent_to_client_label.is_none());
    }
}
