//! Persistence helpers used by the resolution pipeline.
//!
//! Two operations live here:
//!
//! - [`upsert_series_from_metadata`]: given a [`SeriesMetadata`] from the
//!   active provider, find-or-create a `series` row and fan out every
//!   foreign ID into `series_external_ids`. Returns the internal
//!   `series.id` and a flag indicating whether the row already existed
//!   with an unchanged `metadata_hash` (the resolver can use this to
//!   short-circuit re-runs).
//! - [`link_release`]: write the resolution outcome onto the `releases`
//!   row (series_id, path, confidence, status, attempts++).
//!
//! Kept in this crate rather than `td-db` because the orchestration logic
//! (lookup-by-external-id, fan out foreign IDs, recompute last_release_at)
//! is resolver-specific, not a generic CRUD primitive.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, NotSet, QueryFilter, Set, TransactionTrait,
};
use td_db::entities::{releases, series, series_external_ids};
use td_db::repos::tagging_repo;
use td_metadata::{ForeignId, SeriesKind, SeriesMetadata, SeriesStatus};

/// Source-of-truth provenance for the `series.metadata_source` column.
/// `offline_cache` means "provider served the response from a local dump";
/// `api` means "live HTTP fetch". The resolver doesn't observe this
/// directly today, but the column is non-nullable so we set a sensible
/// default at write time. Operators can sharpen it later by threading a
/// flag through `MetadataProvider::get`.
pub const METADATA_SOURCE_DEFAULT: &str = "api";

/// Outcome of [`upsert_series_from_metadata`].
#[derive(Debug, Clone)]
pub struct UpsertResult {
    pub series_id: i32,
    /// `true` when the existing row's `metadata_hash` matches the incoming
    /// metadata, so we skipped the actual series UPDATE. We still upsert
    /// `series_external_ids` (cheap, idempotent) so a newly-discovered
    /// foreign ID still gets registered.
    pub unchanged: bool,
}

/// Find or create a `series` row from `metadata` produced by the provider
/// with id `provider_id`. Fans out every cross-reference in
/// `metadata.foreign_ids` into `series_external_ids` so future releases
/// linking to any of them short-circuit to the same series.
///
/// The whole operation runs in a single transaction; partial failure
/// leaves the catalog consistent.
pub async fn upsert_series_from_metadata(
    db: &DatabaseConnection,
    provider_id: &str,
    metadata: &SeriesMetadata,
    release_posted_at: i64,
    now: DateTime<Utc>,
) -> Result<UpsertResult> {
    let txn = db.begin().await?;
    let fetched_at = now.timestamp();

    // Try to find an existing series by the active provider's own
    // (provider, external_id). If miss, also try every foreign id —
    // a previous release for the same series may have come in via a
    // different provider's link.
    let mut series_id = find_series_by_id(&txn, provider_id, &metadata.external_id).await?;
    if series_id.is_none() {
        for fid in &metadata.foreign_ids {
            if let Some(id) = find_series_by_id(&txn, &fid.provider, &fid.id).await? {
                series_id = Some(id);
                break;
            }
        }
    }

    let metadata_json = serde_json::to_string(&metadata.raw)?;
    let alternate_titles_json = if metadata.alternate_titles.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&metadata.alternate_titles)?)
    };
    let genres_json = if metadata.genres.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&metadata.genres)?)
    };

    let (series_id, unchanged) = match series_id {
        Some(id) => {
            // Hash-skip: if the stored metadata_hash matches incoming,
            // skip the UPDATE entirely. We still proceed to upsert the
            // external_ids fan-out (cheap; tolerates duplicates).
            let existing = series::Entity::find_by_id(id).one(&txn).await?;
            let unchanged = matches!(
                existing.as_ref().and_then(|e| e.metadata_hash.as_deref()),
                Some(h) if h == metadata.content_hash
            );
            if !unchanged {
                let mut last_release_at = release_posted_at;
                if let Some(e) = existing.as_ref() {
                    last_release_at = last_release_at.max(e.last_release_at);
                }
                let model = series::ActiveModel {
                    id: Set(id),
                    canonical_title: Set(metadata.canonical_title.clone()),
                    alternate_titles_json: Set(alternate_titles_json),
                    cover_url: Set(metadata.cover_url.clone()),
                    kind: Set(metadata.kind.as_ref().map(kind_to_db)),
                    status: Set(metadata.status.as_ref().map(status_to_db)),
                    year: Set(metadata.year),
                    genres_json: Set(genres_json),
                    metadata_json: Set(Some(metadata_json)),
                    metadata_source: Set(METADATA_SOURCE_DEFAULT.into()),
                    metadata_hash: Set(Some(metadata.content_hash.clone())),
                    metadata_fetched_at: Set(fetched_at),
                    // first_seen_at is immutable after insert
                    first_seen_at: NotSet,
                    last_release_at: Set(last_release_at),
                    highest_volume: NotSet,
                    highest_chapter: NotSet,
                    owned: NotSet,
                };
                series::Entity::update(model).exec(&txn).await?;
            }
            (id, unchanged)
        }
        None => {
            let model = series::ActiveModel {
                id: NotSet,
                canonical_title: Set(metadata.canonical_title.clone()),
                alternate_titles_json: Set(alternate_titles_json),
                cover_url: Set(metadata.cover_url.clone()),
                kind: Set(metadata.kind.as_ref().map(kind_to_db)),
                status: Set(metadata.status.as_ref().map(status_to_db)),
                year: Set(metadata.year),
                genres_json: Set(genres_json),
                metadata_json: Set(Some(metadata_json)),
                metadata_source: Set(METADATA_SOURCE_DEFAULT.into()),
                metadata_hash: Set(Some(metadata.content_hash.clone())),
                metadata_fetched_at: Set(fetched_at),
                first_seen_at: Set(fetched_at),
                last_release_at: Set(release_posted_at),
                highest_volume: Set(None),
                highest_chapter: Set(None),
                owned: Set(0),
            };
            let inserted = series::Entity::insert(model).exec(&txn).await?;
            (inserted.last_insert_id, false)
        }
    };

    // Always upsert the active provider's own ID first, then fan out.
    upsert_external_id(
        &txn,
        series_id,
        provider_id,
        &metadata.external_id,
        metadata.external_url.as_deref(),
        fetched_at,
    )
    .await?;
    for fid in &metadata.foreign_ids {
        if let Err(e) = upsert_foreign_id(&txn, series_id, fid, fetched_at).await {
            // A UNIQUE conflict on (series_id, provider) means another
            // row already claims that provider for this series under a
            // different external_id; that's a data anomaly the review UI
            // should surface, not a fatal error here.
            tracing::warn!(
                error = ?e,
                series_id,
                provider = %fid.provider,
                external_id = %fid.id,
                "skipping foreign-id upsert; existing row conflicts"
            );
        }
    }

    txn.commit().await?;

    // Sync the normalized genre/tag join tables. We keep `series.genres_json`
    // writes in the loop above as a fallback for one release; the canonical
    // source the UI reads from is the join tables. Failures here don't roll
    // back the series row: the next persist will re-sync from the full set,
    // and the catalog stays usable in the meantime.
    if let Err(e) = tagging_repo::set_series_genres(db, series_id, &metadata.genres).await {
        tracing::warn!(error = ?e, series_id, "failed to sync series_genres; will retry on next persist");
    }
    if let Err(e) = tagging_repo::set_series_tags(db, series_id, &metadata.tags).await {
        tracing::warn!(error = ?e, series_id, "failed to sync series_tags; will retry on next persist");
    }

    Ok(UpsertResult {
        series_id,
        unchanged,
    })
}

/// Resolve the link between a release and its series. Writes
/// `series_id`, `resolution_path`, `resolution_confidence`,
/// `resolution_status`, bumps `resolution_attempts`, and sets
/// `last_resolve_attempt_at`. Other release columns are untouched.
pub async fn link_release(
    db: &DatabaseConnection,
    release_id: &str,
    series_id: Option<i32>,
    path: Option<&str>,
    confidence: Option<f64>,
    status: &str,
    attempted_at: i64,
) -> Result<()> {
    let current = releases::Entity::find_by_id(release_id.to_string())
        .one(db)
        .await?;
    let attempts = current.as_ref().map(|r| r.resolution_attempts).unwrap_or(0) + 1;
    let model = releases::ActiveModel {
        id: Set(release_id.to_string()),
        series_id: Set(series_id),
        resolution_path: Set(path.map(str::to_string)),
        resolution_confidence: Set(confidence),
        resolution_status: Set(status.to_string()),
        resolution_attempts: Set(attempts),
        last_resolve_attempt_at: Set(Some(attempted_at)),
        ..Default::default()
    };
    releases::Entity::update(model).exec(db).await?;
    Ok(())
}

async fn find_series_by_id<C: sea_orm::ConnectionTrait>(
    db: &C,
    provider: &str,
    external_id: &str,
) -> Result<Option<i32>> {
    let row = series_external_ids::Entity::find()
        .filter(series_external_ids::Column::Provider.eq(provider))
        .filter(series_external_ids::Column::ExternalId.eq(external_id))
        .one(db)
        .await?;
    Ok(row.map(|r| r.series_id))
}

async fn upsert_external_id<C: sea_orm::ConnectionTrait>(
    db: &C,
    series_id: i32,
    provider: &str,
    external_id: &str,
    external_url: Option<&str>,
    fetched_at: i64,
) -> Result<()> {
    use sea_orm::sea_query::OnConflict;
    let model = series_external_ids::ActiveModel {
        provider: Set(provider.to_string()),
        external_id: Set(external_id.to_string()),
        series_id: Set(series_id),
        external_url: Set(external_url.map(str::to_string)),
        fetched_at: Set(fetched_at),
    };
    series_external_ids::Entity::insert(model)
        .on_conflict(
            OnConflict::columns([
                series_external_ids::Column::Provider,
                series_external_ids::Column::ExternalId,
            ])
            .update_columns([
                series_external_ids::Column::SeriesId,
                series_external_ids::Column::ExternalUrl,
                series_external_ids::Column::FetchedAt,
            ])
            .to_owned(),
        )
        .exec(db)
        .await?;
    Ok(())
}

async fn upsert_foreign_id<C: sea_orm::ConnectionTrait>(
    db: &C,
    series_id: i32,
    fid: &ForeignId,
    fetched_at: i64,
) -> Result<()> {
    upsert_external_id(
        db,
        series_id,
        &fid.provider,
        &fid.id,
        fid.url.as_deref(),
        fetched_at,
    )
    .await
}

fn kind_to_db(kind: &SeriesKind) -> String {
    match kind {
        SeriesKind::Manga => "manga".into(),
        SeriesKind::Manhwa => "manhwa".into(),
        SeriesKind::Manhua => "manhua".into(),
        SeriesKind::Novel => "novel".into(),
        SeriesKind::OneShot => "one_shot".into(),
        SeriesKind::Oel => "oel".into(),
        SeriesKind::Other(s) => s.clone(),
    }
}

fn status_to_db(status: &SeriesStatus) -> String {
    match status {
        SeriesStatus::Ongoing => "ongoing".into(),
        SeriesStatus::Completed => "completed".into(),
        SeriesStatus::Hiatus => "hiatus".into(),
        SeriesStatus::Cancelled => "cancelled".into(),
        SeriesStatus::Upcoming => "upcoming".into(),
        SeriesStatus::Unknown => "unknown".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::Database;
    use td_metadata::{ForeignId, SeriesMetadata};

    async fn fresh_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db
    }

    fn sample_metadata() -> SeriesMetadata {
        SeriesMetadata {
            external_id: "12345".into(),
            canonical_title: "Chainsaw Man".into(),
            alternate_titles: vec!["チェンソーマン".into()],
            kind: Some(SeriesKind::Manga),
            status: Some(SeriesStatus::Ongoing),
            year: Some(2018),
            cover_url: Some("https://example.com/c.jpg".into()),
            external_url: Some("https://api.mangabaka.dev/v1/series/12345".into()),
            genres: vec!["action".into(), "horror".into()],
            tags: vec!["devil hunter".into(), "gore".into()],
            foreign_ids: vec![
                ForeignId {
                    provider: "mangaupdates".into(),
                    id: "ylx5wzn".into(),
                    url: Some("https://www.mangaupdates.com/series/ylx5wzn".into()),
                },
                ForeignId {
                    provider: "anilist".into(),
                    id: "105778".into(),
                    url: None,
                },
            ],
            raw: serde_json::json!({"id": 12345, "title": "Chainsaw Man"}),
            content_hash: "hash-v1".into(),
        }
    }

    #[tokio::test]
    async fn upsert_creates_series_and_fans_out_foreign_ids() {
        let db = fresh_db().await;
        let now = Utc::now();
        let result =
            upsert_series_from_metadata(&db, "mangabaka", &sample_metadata(), 1_700_000_000, now)
                .await
                .unwrap();
        assert!(result.series_id > 0);
        assert!(!result.unchanged);

        // Active provider's own ID + every foreign_id is mapped.
        let map = series_external_ids::Entity::find()
            .filter(series_external_ids::Column::SeriesId.eq(result.series_id))
            .all(&db)
            .await
            .unwrap();
        let mut providers: Vec<&str> = map.iter().map(|r| r.provider.as_str()).collect();
        providers.sort();
        assert_eq!(providers, vec!["anilist", "mangabaka", "mangaupdates"]);
    }

    #[tokio::test]
    async fn second_upsert_with_same_hash_is_unchanged() {
        let db = fresh_db().await;
        let now = Utc::now();
        let m = sample_metadata();
        let first = upsert_series_from_metadata(&db, "mangabaka", &m, 1_700_000_000, now)
            .await
            .unwrap();
        let second = upsert_series_from_metadata(&db, "mangabaka", &m, 1_700_000_000, now)
            .await
            .unwrap();
        assert_eq!(first.series_id, second.series_id);
        assert!(!first.unchanged);
        assert!(second.unchanged);
    }

    #[tokio::test]
    async fn upsert_writes_normalized_genre_and_tag_tables() {
        let db = fresh_db().await;
        let now = Utc::now();
        let res =
            upsert_series_from_metadata(&db, "mangabaka", &sample_metadata(), 1_700_000_000, now)
                .await
                .unwrap();

        // Both genres_json (fallback) and the join tables get populated.
        let row = series::Entity::find_by_id(res.series_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert!(row.genres_json.as_deref().unwrap().contains("action"));

        let genres = td_db::repos::tagging_repo::list_genres_for_series(&db, res.series_id)
            .await
            .unwrap();
        assert_eq!(genres.len(), 2);
        let tags = td_db::repos::tagging_repo::list_tags_for_series(&db, res.series_id)
            .await
            .unwrap();
        assert_eq!(tags.len(), 2);
        assert!(tags.iter().any(|t| t.contains("devil")));
    }

    #[tokio::test]
    async fn upsert_finds_existing_series_via_foreign_id_lookup() {
        let db = fresh_db().await;
        let now = Utc::now();

        // First insert: a series under "anilist" only (a release arrived
        // via an AniList link, say, and a hypothetical anilist provider
        // wrote the row).
        let pseudo = SeriesMetadata {
            external_id: "105778".into(),
            canonical_title: "Chainsaw Man (anilist row)".into(),
            content_hash: "hash-anilist".into(),
            ..sample_metadata()
        };
        let first = upsert_series_from_metadata(&db, "anilist", &pseudo, 1_700_000_000, now)
            .await
            .unwrap();

        // Now the MangaBaka resolver finds the same series via its own
        // ID. Its metadata payload includes the anilist foreign_id, so
        // the second call should reuse `first.series_id` instead of
        // making a brand new row.
        let mb = sample_metadata();
        let second = upsert_series_from_metadata(&db, "mangabaka", &mb, 1_700_000_000, now)
            .await
            .unwrap();
        assert_eq!(first.series_id, second.series_id);

        // Total rows in `series`: still 1.
        let all = series::Entity::find().all(&db).await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn link_release_writes_resolution_columns_and_increments_attempts() {
        let db = fresh_db().await;
        // Need a series row to satisfy the FK on releases.series_id.
        let upsert = upsert_series_from_metadata(
            &db,
            "mangabaka",
            &sample_metadata(),
            1_700_000_000,
            Utc::now(),
        )
        .await
        .unwrap();
        // Insert a minimal releases row first.
        let row = releases::ActiveModel {
            id: Set("nyaa:test:1".into()),
            source_kind: Set("nyaa".into()),
            source_name: Set("test".into()),
            external_id: Set("1".into()),
            title: Set("X".into()),
            link: Set("https://example.com/1".into()),
            magnet: Set(None),
            torrent_url: Set(None),
            ddl_url: Set(None),
            info_hash: Set(None),
            size_bytes: Set(None),
            files_json: Set(None),
            description_html: Set(None),
            extracted_links_json: Set(None),
            posted_at: Set(1),
            observed_at: Set(2),
            series_id: Set(None),
            resolution_path: Set(None),
            resolution_confidence: Set(None),
            resolution_status: Set("unresolved".into()),
            resolution_attempts: Set(0),
            last_resolve_attempt_at: Set(None),
            volume_span_json: Set(None),
            chapter_span_json: Set(None),
        };
        releases::Entity::insert(row).exec(&db).await.unwrap();

        link_release(
            &db,
            "nyaa:test:1",
            Some(upsert.series_id),
            Some("known_external_id"),
            Some(1.0),
            "resolved",
            1_700_000_000,
        )
        .await
        .unwrap();

        let stored = releases::Entity::find_by_id("nyaa:test:1".to_string())
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.series_id, Some(upsert.series_id));
        assert_eq!(stored.resolution_path.as_deref(), Some("known_external_id"));
        assert_eq!(stored.resolution_confidence, Some(1.0));
        assert_eq!(stored.resolution_status, "resolved");
        assert_eq!(stored.resolution_attempts, 1);
        assert_eq!(stored.last_resolve_attempt_at, Some(1_700_000_000));

        // A second call increments attempts.
        link_release(
            &db,
            "nyaa:test:1",
            Some(upsert.series_id),
            Some("known_external_id"),
            Some(1.0),
            "resolved",
            1_700_000_100,
        )
        .await
        .unwrap();
        let stored2 = releases::Entity::find_by_id("nyaa:test:1".to_string())
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored2.resolution_attempts, 2);
    }
}
