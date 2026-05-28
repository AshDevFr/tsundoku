//! Release read/write helpers.

use anyhow::Result;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter,
    QueryOrder, QuerySelect, Set, TransactionTrait,
};
use td_source::{DiscoveredRelease, detect_formats};

use crate::entities::{release_formats, releases, review_candidates};

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
    let files_json = if release.files.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&release.files)?)
    };

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
        posted_at: Set(release.posted_at.timestamp()),
        observed_at: Set(observed_at),
        series_id: Set(None),
        resolution_path: Set(None),
        resolution_confidence: Set(None),
        resolution_status: Set("unresolved".into()),
        resolution_attempts: Set(0),
        last_resolve_attempt_at: Set(None),
        volume_span_json: Set(None),
        chapter_span_json: Set(None),
        resolved_at: Set(None),
        search_queries: Set(None),
        cleanup_rules_applied: Set(None),
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
    txn.commit().await?;
    Ok(res.rows_affected)
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
}
