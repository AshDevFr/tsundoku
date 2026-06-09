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
use td_db::repos::{releases_repo, tagging_repo};
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
/// Set `allow_manual_overwrite = true` only when the operator has
/// explicitly asked to overwrite the row (e.g. the per-series refresh
/// endpoint). The resolver, the link-by-external-id path, and the bulk
/// refresh job all pass `false` so that `metadata_source = "manual"`
/// rows stay sticky: provider events (a new release arriving, a cron
/// tick) do not erase operator-curated metadata. When the manual lock
/// fires we still upsert `series_external_ids` (cheap, idempotent, and
/// it keeps cross-provider resolution working) but skip the series
/// `UPDATE` *and* the genre/tag re-sync.
///
/// The whole operation runs in a single transaction; partial failure
/// leaves the catalog consistent.
pub async fn upsert_series_from_metadata(
    db: &DatabaseConnection,
    provider_id: &str,
    metadata: &SeriesMetadata,
    release_posted_at: i64,
    now: DateTime<Utc>,
    allow_manual_overwrite: bool,
) -> Result<UpsertResult> {
    let txn = db.begin().await?;
    let fetched_at = now.timestamp();

    // Try to find an existing series by the active provider's own
    // (provider, external_id). If miss, also try every foreign id —
    // a previous release for the same series may have come in via a
    // different provider's link.
    //
    // When following a foreign id, reject candidates that already have a
    // *different* active-provider mapping. Upstream dumps occasionally
    // list the same foreign id (e.g. AniList 105778) under two distinct
    // provider series; collapsing those into one local series would
    // silently merge two real series. Falling through creates a fresh
    // series row instead.
    let mut series_id = find_series_by_id(&txn, provider_id, &metadata.external_id).await?;
    if series_id.is_none() {
        for fid in &metadata.foreign_ids {
            if let Some(id) = find_series_by_id(&txn, &fid.provider, &fid.id).await? {
                let existing_active = find_external_id_for_series(&txn, id, provider_id).await?;
                match existing_active {
                    Some(ref existing) if existing != &metadata.external_id => continue,
                    _ => {
                        series_id = Some(id);
                        break;
                    }
                }
            }
        }
    }

    let metadata_json = serde_json::to_string(&metadata.raw)?;
    let alternate_titles_json = if metadata.alternate_titles.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&metadata.alternate_titles)?)
    };

    let (series_id, unchanged, manual_lock) = match series_id {
        Some(id) => {
            // Hash-skip: if the stored metadata_hash matches incoming,
            // skip the UPDATE entirely. Manual-lock: if the row is
            // operator-curated and the caller didn't opt in to
            // overwrite, also skip. Either way we still proceed to
            // upsert the external_ids fan-out (cheap; tolerates
            // duplicates).
            let existing = series::Entity::find_by_id(id).one(&txn).await?;
            let hash_unchanged = matches!(
                existing.as_ref().and_then(|e| e.metadata_hash.as_deref()),
                Some(h) if h == metadata.content_hash
            );
            let manual_lock = !allow_manual_overwrite
                && existing
                    .as_ref()
                    .map(|e| e.metadata_source.as_str() == "manual")
                    .unwrap_or(false);
            if manual_lock {
                tracing::debug!(
                    series_id = id,
                    provider = provider_id,
                    "skipping series UPDATE: metadata_source='manual' and caller did not opt in to overwrite"
                );
            }
            let unchanged = hash_unchanged || manual_lock;
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
                    description: Set(metadata.description.clone()),
                    total_volumes: Set(metadata.total_volumes),
                    total_chapters: Set(metadata.total_chapters),
                    rating: Set(metadata.rating),
                    metadata_json: Set(Some(metadata_json)),
                    metadata_source: Set(METADATA_SOURCE_DEFAULT.into()),
                    metadata_hash: Set(Some(metadata.content_hash.clone())),
                    metadata_fetched_at: Set(fetched_at),
                    // first_seen_at is immutable after insert
                    first_seen_at: NotSet,
                    last_release_at: Set(last_release_at),
                    highest_volume: NotSet,
                    highest_chapter: NotSet,
                    // Release-derived coverage + its change timestamp are owned
                    // by `recompute_series_coverage`, not metadata: a refresh
                    // must never bump `updated_at` or the feed would re-emit
                    // every series each provider-cache cycle.
                    volume_coverage_json: NotSet,
                    chapter_coverage_json: NotSet,
                    updated_at: NotSet,
                    owned: NotSet,
                    // Operator-owned flag: leave it alone so a provider
                    // re-fetch never resets a manually-set ignore.
                    ignore_completion: NotSet,
                };
                series::Entity::update(model).exec(&txn).await?;
            }
            (id, unchanged, manual_lock)
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
                description: Set(metadata.description.clone()),
                total_volumes: Set(metadata.total_volumes),
                total_chapters: Set(metadata.total_chapters),
                rating: Set(metadata.rating),
                metadata_json: Set(Some(metadata_json)),
                metadata_source: Set(METADATA_SOURCE_DEFAULT.into()),
                metadata_hash: Set(Some(metadata.content_hash.clone())),
                metadata_fetched_at: Set(fetched_at),
                first_seen_at: Set(fetched_at),
                last_release_at: Set(release_posted_at),
                highest_volume: Set(None),
                highest_chapter: Set(None),
                // Empty until a release links and `recompute_series_coverage`
                // fills it; `updated_at = 0` until that first real change.
                volume_coverage_json: Set(None),
                chapter_coverage_json: Set(None),
                updated_at: Set(0),
                owned: Set(0),
                ignore_completion: Set(false),
            };
            let inserted = series::Entity::insert(model).exec(&txn).await?;
            (inserted.last_insert_id, false, false)
        }
    };

    // Always upsert the active provider's own ID first, then fan out.
    // `upsert_external_id` itself warn-skips ambiguous mappings (existing
    // row points at a different series, or this series already has a
    // different external_id for the provider), so the only errors that
    // can bubble out here are real DB faults.
    upsert_external_id(
        &txn,
        series_id,
        provider_id,
        &metadata.external_id,
        fetched_at,
    )
    .await?;
    for fid in &metadata.foreign_ids {
        upsert_foreign_id(&txn, series_id, fid, fetched_at).await?;
    }

    txn.commit().await?;

    // Sync the normalized genre/tag join tables. Failures here don't roll
    // back the series row: the next persist will re-sync from the full set,
    // and the catalog stays usable in the meantime. Skipped under manual
    // lock so a provider event can't overwrite operator-curated tags.
    if !manual_lock {
        if let Err(e) = tagging_repo::set_series_genres(db, series_id, &metadata.genres).await {
            tracing::warn!(error = ?e, series_id, "failed to sync series_genres; will retry on next persist");
        }
        if let Err(e) = tagging_repo::set_series_tags(db, series_id, &metadata.tags).await {
            tracing::warn!(error = ?e, series_id, "failed to sync series_tags; will retry on next persist");
        }
    }

    Ok(UpsertResult {
        series_id,
        unchanged,
    })
}

/// Stamp the title cleaner's output onto the release. Runs once per
/// resolve cycle regardless of which step ultimately matches (or none) —
/// the review UI surfaces the cleaned queries + applied rule names even
/// for releases that resolved via foreign-id, so the operator can see
/// what *would* have been searched.
pub async fn persist_search_queries(
    db: &DatabaseConnection,
    release_id: &str,
    queries: &[String],
    rules_applied: &[String],
) -> Result<()> {
    let queries_json = serde_json::to_string(queries)?;
    let rules_json = serde_json::to_string(rules_applied)?;
    let model = releases::ActiveModel {
        id: Set(release_id.to_string()),
        search_queries: Set(Some(queries_json)),
        cleanup_rules_applied: Set(Some(rules_json)),
        ..Default::default()
    };
    releases::Entity::update(model).exec(db).await?;
    Ok(())
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
    // Stamp resolved_at exactly once, the first time a release transitions
    // to status='resolved'. Anchors the time-to-resolution percentiles the
    // admin metrics view surfaces. Manual retries that re-resolve an
    // already-resolved release preserve the original timestamp.
    let resolved_at = if status == "resolved" {
        sea_orm::ActiveValue::Set(Some(
            current
                .as_ref()
                .and_then(|r| r.resolved_at)
                .unwrap_or(attempted_at),
        ))
    } else {
        sea_orm::ActiveValue::NotSet
    };
    let model = releases::ActiveModel {
        id: Set(release_id.to_string()),
        series_id: Set(series_id),
        resolution_path: Set(path.map(str::to_string)),
        resolution_confidence: Set(confidence),
        resolution_status: Set(status.to_string()),
        resolution_attempts: Set(attempts),
        last_resolve_attempt_at: Set(Some(attempted_at)),
        resolved_at,
        ..Default::default()
    };

    // The release link and the coverage recompute it triggers must commit
    // together or not at all. If they split — link persists, recompute fails —
    // the series is left with a linked release but `updated_at = 0` and empty
    // coverage, which silently drops it out of the release feed (the feed gates
    // on `updated_at > 0`) with no automatic recovery. One transaction closes
    // that gap: a recompute failure rolls back the link too, so the release
    // stays in its prior state and the resolver retries it on the next tick.
    let txn = db.begin().await?;
    releases::Entity::update(model).exec(&txn).await?;

    // Rebuild the merged coverage + `highest_*` of every series this (re)link
    // touched, bumping each one's `updated_at` only when it actually moved. A
    // re-link affects two series (the one losing this release and the one
    // gaining it); a reject/keep affects only the old one. Unlike the previous
    // monotonic bump, this re-merges from scratch, so coverage *shrinks*
    // correctly when a release moves away.
    let old_series_id = current.as_ref().and_then(|r| r.series_id);
    for sid in affected_series(old_series_id, series_id) {
        releases_repo::recompute_series_coverage(&txn, sid, attempted_at).await?;
    }
    txn.commit().await?;
    Ok(())
}

/// The distinct, non-null series ids affected by a (re)link: the release's
/// previous series and its new one.
fn affected_series(old: Option<i32>, new: Option<i32>) -> Vec<i32> {
    let mut ids: Vec<i32> = [old, new].into_iter().flatten().collect();
    ids.sort_unstable();
    ids.dedup();
    ids
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

async fn find_external_id_for_series<C: sea_orm::ConnectionTrait>(
    db: &C,
    series_id: i32,
    provider: &str,
) -> Result<Option<String>> {
    let row = series_external_ids::Entity::find()
        .filter(series_external_ids::Column::SeriesId.eq(series_id))
        .filter(series_external_ids::Column::Provider.eq(provider))
        .one(db)
        .await?;
    Ok(row.map(|r| r.external_id))
}

/// Write a `(provider, external_id) → series_id` mapping, refusing to
/// overwrite or duplicate an existing one. The table has two UNIQUE
/// constraints — `(provider, external_id)` and `(series_id, provider)` —
/// so a naive INSERT … ON CONFLICT covers only one of them, and using
/// ON CONFLICT … DO UPDATE on `(provider, external_id)` would silently
/// steal a foreign-id mapping from another series. Instead, branch on
/// the current state and warn-skip the ambiguous cases.
async fn upsert_external_id<C: sea_orm::ConnectionTrait>(
    db: &C,
    series_id: i32,
    provider: &str,
    external_id: &str,
    fetched_at: i64,
) -> Result<()> {
    // Case 1: (provider, external_id) already exists.
    let by_external = series_external_ids::Entity::find()
        .filter(series_external_ids::Column::Provider.eq(provider))
        .filter(series_external_ids::Column::ExternalId.eq(external_id))
        .one(db)
        .await?;
    if let Some(existing) = by_external {
        if existing.series_id != series_id {
            // Pointing this external_id at a new series would orphan the
            // old series's mapping. Treat as ambiguous, surface a warning.
            tracing::warn!(
                provider,
                external_id,
                existing_series_id = existing.series_id,
                attempted_series_id = series_id,
                "external id already maps to a different series; leaving existing mapping intact"
            );
            return Ok(());
        }
        // Same series: refresh fetched_at.
        let model = series_external_ids::ActiveModel {
            provider: Set(provider.to_string()),
            external_id: Set(external_id.to_string()),
            series_id: Set(series_id),
            fetched_at: Set(fetched_at),
        };
        series_external_ids::Entity::update(model).exec(db).await?;
        return Ok(());
    }

    // Case 2: (series_id, provider) already exists with a different
    // external_id. Adding another would violate UNIQUE(series_id,
    // provider). Same provenance question as case 1 — refuse silently
    // with a warning rather than crashing the whole persist.
    if let Some(other) = find_external_id_for_series(db, series_id, provider).await? {
        tracing::warn!(
            series_id,
            provider,
            existing_external_id = %other,
            attempted_external_id = external_id,
            "series already has a different external id for this provider; not adding duplicate"
        );
        return Ok(());
    }

    let model = series_external_ids::ActiveModel {
        provider: Set(provider.to_string()),
        external_id: Set(external_id.to_string()),
        series_id: Set(series_id),
        fetched_at: Set(fetched_at),
    };
    series_external_ids::Entity::insert(model).exec(db).await?;
    Ok(())
}

async fn upsert_foreign_id<C: sea_orm::ConnectionTrait>(
    db: &C,
    series_id: i32,
    fid: &ForeignId,
    fetched_at: i64,
) -> Result<()> {
    upsert_external_id(db, series_id, &fid.provider, &fid.id, fetched_at).await
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
    use td_source::{Span, spans_to_json};

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
            total_volumes: Some(11),
            total_chapters: Some(97),
            rating: Some(8.5),
            description: Some("Denji is a teenage devil-hunter.".into()),
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
        let result = upsert_series_from_metadata(
            &db,
            "mangabaka",
            &sample_metadata(),
            1_700_000_000,
            now,
            false,
        )
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
        let first = upsert_series_from_metadata(&db, "mangabaka", &m, 1_700_000_000, now, false)
            .await
            .unwrap();
        let second = upsert_series_from_metadata(&db, "mangabaka", &m, 1_700_000_000, now, false)
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
        let res = upsert_series_from_metadata(
            &db,
            "mangabaka",
            &sample_metadata(),
            1_700_000_000,
            now,
            false,
        )
        .await
        .unwrap();

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
        let first = upsert_series_from_metadata(&db, "anilist", &pseudo, 1_700_000_000, now, false)
            .await
            .unwrap();

        // Now the MangaBaka resolver finds the same series via its own
        // ID. Its metadata payload includes the anilist foreign_id, so
        // the second call should reuse `first.series_id` instead of
        // making a brand new row.
        let mb = sample_metadata();
        let second = upsert_series_from_metadata(&db, "mangabaka", &mb, 1_700_000_000, now, false)
            .await
            .unwrap();
        assert_eq!(first.series_id, second.series_id);

        // Total rows in `series`: still 1.
        let all = series::Entity::find().all(&db).await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn upsert_does_not_merge_when_foreign_id_chain_crosses_series() {
        // Upstream dump has two MangaBaka rows (AAA, BBB) that both list
        // the same AniList foreign id. They are different series and must
        // not be collapsed into one local row. The second upsert should
        // create a fresh series for BBB, and the existing series's
        // (mangabaka=AAA) and (anilist=200) mappings must stay intact.
        let db = fresh_db().await;
        let now = Utc::now();

        let aaa = SeriesMetadata {
            external_id: "AAA".into(),
            canonical_title: "Series One".into(),
            content_hash: "hash-aaa".into(),
            foreign_ids: vec![ForeignId {
                provider: "anilist".into(),
                id: "200".into(),
                url: None,
            }],
            ..sample_metadata()
        };
        let first = upsert_series_from_metadata(&db, "mangabaka", &aaa, 1_700_000_000, now, false)
            .await
            .unwrap();

        let bbb = SeriesMetadata {
            external_id: "BBB".into(),
            canonical_title: "Series Two".into(),
            content_hash: "hash-bbb".into(),
            foreign_ids: vec![ForeignId {
                provider: "anilist".into(),
                id: "200".into(),
                url: None,
            }],
            ..sample_metadata()
        };
        let second = upsert_series_from_metadata(&db, "mangabaka", &bbb, 1_700_000_000, now, false)
            .await
            .unwrap();

        assert_ne!(first.series_id, second.series_id);

        let series_rows = series::Entity::find().all(&db).await.unwrap();
        assert_eq!(series_rows.len(), 2);

        // First series keeps its mangabaka + anilist mappings intact.
        let first_mappings = series_external_ids::Entity::find()
            .filter(series_external_ids::Column::SeriesId.eq(first.series_id))
            .all(&db)
            .await
            .unwrap();
        let mut first_pairs: Vec<(String, String)> = first_mappings
            .into_iter()
            .map(|r| (r.provider, r.external_id))
            .collect();
        first_pairs.sort();
        assert_eq!(
            first_pairs,
            vec![
                ("anilist".into(), "200".into()),
                ("mangabaka".into(), "AAA".into()),
            ]
        );

        // Second series has only its own mangabaka mapping; the anilist
        // foreign-id fan-out must not steal series 1's anilist mapping.
        let second_mappings = series_external_ids::Entity::find()
            .filter(series_external_ids::Column::SeriesId.eq(second.series_id))
            .all(&db)
            .await
            .unwrap();
        let second_pairs: Vec<(String, String)> = second_mappings
            .into_iter()
            .map(|r| (r.provider, r.external_id))
            .collect();
        assert_eq!(second_pairs, vec![("mangabaka".into(), "BBB".into())]);
    }

    /// Flip an existing series to `metadata_source = "manual"` and stamp a
    /// distinct canonical_title so we can detect whether a subsequent upsert
    /// overwrote it.
    async fn mark_manual(db: &DatabaseConnection, series_id: i32, title: &str) {
        let model = series::ActiveModel {
            id: Set(series_id),
            canonical_title: Set(title.into()),
            metadata_source: Set("manual".into()),
            // Bust the hash so the hash-skip path doesn't mask a real
            // overwrite/skip — we want to observe the manual guard, not
            // the hash optimization.
            metadata_hash: Set(Some("operator-stamp".into())),
            ..Default::default()
        };
        series::Entity::update(model).exec(db).await.unwrap();
    }

    #[tokio::test]
    async fn upsert_with_manual_lock_skips_series_update_but_still_fans_out_external_ids() {
        let db = fresh_db().await;
        let now = Utc::now();
        let m = sample_metadata();
        let first = upsert_series_from_metadata(&db, "mangabaka", &m, 1_700_000_000, now, false)
            .await
            .unwrap();

        // Operator edits the row.
        mark_manual(&db, first.series_id, "Operator Title").await;

        // A second upsert under manual-lock semantics: must not clobber
        // the title or source. The fan-out is still cheap and idempotent;
        // we expect the existing mappings to stay (refreshed fetched_at
        // is acceptable, but content stays put).
        let m2 = SeriesMetadata {
            canonical_title: "Provider Title".into(),
            content_hash: "new-hash".into(),
            ..m.clone()
        };
        let result = upsert_series_from_metadata(&db, "mangabaka", &m2, 1_700_000_001, now, false)
            .await
            .unwrap();
        assert_eq!(result.series_id, first.series_id);
        assert!(
            result.unchanged,
            "manual lock should report unchanged so callers can short-circuit"
        );

        let row = series::Entity::find_by_id(first.series_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.canonical_title, "Operator Title", "title preserved");
        assert_eq!(row.metadata_source, "manual", "source preserved");
        assert_eq!(
            row.metadata_hash.as_deref(),
            Some("operator-stamp"),
            "hash preserved"
        );
    }

    #[tokio::test]
    async fn upsert_with_manual_overwrite_allowed_replaces_row() {
        let db = fresh_db().await;
        let now = Utc::now();
        let m = sample_metadata();
        let first = upsert_series_from_metadata(&db, "mangabaka", &m, 1_700_000_000, now, false)
            .await
            .unwrap();
        mark_manual(&db, first.series_id, "Operator Title").await;

        // Explicit operator action (per-series refresh endpoint) passes
        // `true` and gets the new metadata.
        let m2 = SeriesMetadata {
            canonical_title: "Provider Title".into(),
            content_hash: "new-hash".into(),
            ..m.clone()
        };
        let result = upsert_series_from_metadata(&db, "mangabaka", &m2, 1_700_000_001, now, true)
            .await
            .unwrap();
        assert_eq!(result.series_id, first.series_id);
        assert!(!result.unchanged);

        let row = series::Entity::find_by_id(first.series_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.canonical_title, "Provider Title");
        assert_eq!(row.metadata_source, "api");
    }

    #[tokio::test]
    async fn metadata_refresh_preserves_operator_ignore_completion_flag() {
        let db = fresh_db().await;
        let now = Utc::now();
        let m = sample_metadata();
        let first = upsert_series_from_metadata(&db, "mangabaka", &m, 1_700_000_000, now, false)
            .await
            .unwrap();

        // Operator sets the ignore-completion flag directly on the row.
        series::Entity::update(series::ActiveModel {
            id: Set(first.series_id),
            ignore_completion: Set(true),
            ..Default::default()
        })
        .exec(&db)
        .await
        .unwrap();

        // A later provider refresh with changed content forces the UPDATE
        // branch (new hash, no manual lock). The operator flag must survive
        // because the refresh UPDATE leaves `ignore_completion` `NotSet`.
        let refreshed = SeriesMetadata {
            canonical_title: "Refreshed Title".into(),
            content_hash: "refreshed-hash".into(),
            ..m.clone()
        };
        let second =
            upsert_series_from_metadata(&db, "mangabaka", &refreshed, 1_700_000_100, now, false)
                .await
                .unwrap();
        assert_eq!(second.series_id, first.series_id);
        assert!(!second.unchanged, "changed hash must hit the UPDATE branch");

        let row = series::Entity::find_by_id(first.series_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.canonical_title, "Refreshed Title",
            "provider columns were refreshed"
        );
        assert!(
            row.ignore_completion,
            "operator ignore flag preserved across refresh"
        );
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
            false,
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
            comment_suggested_links_json: Set(None),
            information_url: Set(None),
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
            resolved_at: Set(None),
            search_queries: Set(None),
            cleanup_rules_applied: Set(None),
            sent_to_client_at: Set(None),
            sent_to_client_label: Set(None),
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

    /// Insert a minimal release carrying volume / chapter spans so the
    /// link-time bump has something to read.
    async fn insert_release_with_spans(
        db: &DatabaseConnection,
        id: &str,
        volume: Option<Span>,
        chapter: Option<Span>,
    ) {
        let row = releases::ActiveModel {
            id: Set(id.into()),
            source_kind: Set("nyaa".into()),
            source_name: Set("test".into()),
            external_id: Set(id.into()),
            title: Set("X".into()),
            link: Set(format!("https://example.com/{id}")),
            magnet: Set(None),
            torrent_url: Set(None),
            ddl_url: Set(None),
            info_hash: Set(None),
            size_bytes: Set(None),
            files_json: Set(None),
            description_html: Set(None),
            extracted_links_json: Set(None),
            comment_suggested_links_json: Set(None),
            information_url: Set(None),
            posted_at: Set(1),
            observed_at: Set(2),
            series_id: Set(None),
            resolution_path: Set(None),
            resolution_confidence: Set(None),
            resolution_status: Set("unresolved".into()),
            resolution_attempts: Set(0),
            last_resolve_attempt_at: Set(None),
            volume_span_json: Set(spans_to_json(&volume.into_iter().collect::<Vec<_>>())),
            chapter_span_json: Set(spans_to_json(&chapter.into_iter().collect::<Vec<_>>())),
            resolved_at: Set(None),
            search_queries: Set(None),
            cleanup_rules_applied: Set(None),
            sent_to_client_at: Set(None),
            sent_to_client_label: Set(None),
        };
        releases::Entity::insert(row).exec(db).await.unwrap();
    }

    #[tokio::test]
    async fn link_release_bumps_series_highest_volume_and_chapter() {
        let db = fresh_db().await;
        let upsert = upsert_series_from_metadata(
            &db,
            "mangabaka",
            &sample_metadata(),
            1_700_000_000,
            Utc::now(),
            false,
        )
        .await
        .unwrap();

        insert_release_with_spans(
            &db,
            "nyaa:vol",
            Some(Span {
                start: 1.0,
                end: 5.0,
            }),
            Some(Span {
                start: 1.0,
                end: 40.0,
            }),
        )
        .await;
        link_release(
            &db,
            "nyaa:vol",
            Some(upsert.series_id),
            Some("manual"),
            Some(1.0),
            "resolved",
            1_700_000_000,
        )
        .await
        .unwrap();

        let row = series::Entity::find_by_id(upsert.series_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.highest_volume, Some(5.0));
        assert_eq!(row.highest_chapter, Some(40.0));
    }

    #[tokio::test]
    async fn link_release_does_not_lower_existing_series_highest() {
        let db = fresh_db().await;
        let upsert = upsert_series_from_metadata(
            &db,
            "mangabaka",
            &sample_metadata(),
            1_700_000_000,
            Utc::now(),
            false,
        )
        .await
        .unwrap();

        // First release reaches volume 10 / chapter 90.
        insert_release_with_spans(
            &db,
            "nyaa:hi",
            Some(Span {
                start: 1.0,
                end: 10.0,
            }),
            Some(Span {
                start: 1.0,
                end: 90.0,
            }),
        )
        .await;
        link_release(
            &db,
            "nyaa:hi",
            Some(upsert.series_id),
            Some("manual"),
            Some(1.0),
            "resolved",
            1_700_000_000,
        )
        .await
        .unwrap();

        // A later, smaller release must not pull the marks back down, but a
        // higher chapter still lifts only that column.
        insert_release_with_spans(
            &db,
            "nyaa:lo",
            Some(Span {
                start: 1.0,
                end: 2.0,
            }),
            Some(Span {
                start: 91.0,
                end: 120.0,
            }),
        )
        .await;
        link_release(
            &db,
            "nyaa:lo",
            Some(upsert.series_id),
            Some("manual"),
            Some(1.0),
            "resolved",
            1_700_000_100,
        )
        .await
        .unwrap();

        let row = series::Entity::find_by_id(upsert.series_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.highest_volume, Some(10.0), "volume not lowered");
        assert_eq!(row.highest_chapter, Some(120.0), "chapter raised");
    }

    #[tokio::test]
    async fn link_release_backfills_span_from_files_when_json_absent() {
        let db = fresh_db().await;
        let upsert = upsert_series_from_metadata(
            &db,
            "mangabaka",
            &sample_metadata(),
            1_700_000_000,
            Utc::now(),
            false,
        )
        .await
        .unwrap();

        // Legacy-shaped row: no span JSON, but a file list to parse from.
        let row = releases::ActiveModel {
            id: Set("nyaa:legacy".into()),
            source_kind: Set("nyaa".into()),
            source_name: Set("test".into()),
            external_id: Set("legacy".into()),
            title: Set("Some Series".into()),
            link: Set("https://example.com/legacy".into()),
            magnet: Set(None),
            torrent_url: Set(None),
            ddl_url: Set(None),
            info_hash: Set(None),
            size_bytes: Set(None),
            files_json: Set(Some(
                serde_json::to_string(&["Some Series v01-04.cbz"]).unwrap(),
            )),
            description_html: Set(None),
            extracted_links_json: Set(None),
            comment_suggested_links_json: Set(None),
            information_url: Set(None),
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
            resolved_at: Set(None),
            search_queries: Set(None),
            cleanup_rules_applied: Set(None),
            sent_to_client_at: Set(None),
            sent_to_client_label: Set(None),
        };
        releases::Entity::insert(row).exec(&db).await.unwrap();

        link_release(
            &db,
            "nyaa:legacy",
            Some(upsert.series_id),
            Some("manual"),
            Some(1.0),
            "resolved",
            1_700_000_000,
        )
        .await
        .unwrap();

        let series_row = series::Entity::find_by_id(upsert.series_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(series_row.highest_volume, Some(4.0));
    }

    #[tokio::test]
    async fn link_release_without_series_id_does_not_touch_highest() {
        let db = fresh_db().await;
        insert_release_with_spans(
            &db,
            "nyaa:none",
            Some(Span {
                start: 1.0,
                end: 3.0,
            }),
            None,
        )
        .await;
        // Reject path: series_id is None, so there is nothing to bump.
        link_release(
            &db,
            "nyaa:none",
            None,
            Some("rejected"),
            None,
            "rejected",
            1,
        )
        .await
        .unwrap();
        // No series rows exist; the call simply must not error.
        assert_eq!(series::Entity::find().all(&db).await.unwrap().len(), 0);
    }

    /// A second, distinct series: different external id and no shared foreign
    /// ids, so the upsert can't collapse it into the first one.
    fn other_metadata() -> SeriesMetadata {
        SeriesMetadata {
            external_id: "67890".into(),
            canonical_title: "Spy x Family".into(),
            alternate_titles: vec![],
            foreign_ids: vec![],
            raw: serde_json::json!({"id": 67890}),
            content_hash: "hash-other".into(),
            ..sample_metadata()
        }
    }

    #[tokio::test]
    async fn relink_moves_coverage_between_series() {
        let db = fresh_db().await;
        let a = upsert_series_from_metadata(
            &db,
            "mangabaka",
            &sample_metadata(),
            1_700_000_000,
            Utc::now(),
            false,
        )
        .await
        .unwrap()
        .series_id;
        let b = upsert_series_from_metadata(
            &db,
            "mangabaka",
            &other_metadata(),
            1_700_000_000,
            Utc::now(),
            false,
        )
        .await
        .unwrap()
        .series_id;
        assert_ne!(a, b);

        insert_release_with_spans(
            &db,
            "nyaa:mover",
            Some(Span {
                start: 1.0,
                end: 5.0,
            }),
            None,
        )
        .await;

        // Assign to A.
        link_release(
            &db,
            "nyaa:mover",
            Some(a),
            Some("manual"),
            Some(1.0),
            "resolved",
            1_000,
        )
        .await
        .unwrap();
        let row_a = series::Entity::find_by_id(a)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row_a.highest_volume, Some(5.0));
        assert_eq!(row_a.updated_at, 1_000);

        // Re-link to B: A must lose the coverage, B must gain it, and BOTH
        // `updated_at`s must reflect the move.
        link_release(
            &db,
            "nyaa:mover",
            Some(b),
            Some("manual"),
            Some(1.0),
            "resolved",
            2_000,
        )
        .await
        .unwrap();
        let row_a = series::Entity::find_by_id(a)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let row_b = series::Entity::find_by_id(b)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row_a.highest_volume, None, "A lost the release");
        assert_eq!(row_a.volume_coverage_json, None);
        assert_eq!(
            row_a.updated_at, 2_000,
            "A re-stamped when it lost coverage"
        );
        assert_eq!(row_b.highest_volume, Some(5.0), "B gained the release");
        assert_eq!(row_b.updated_at, 2_000);
    }

    #[tokio::test]
    async fn metadata_refresh_does_not_bump_updated_at() {
        let db = fresh_db().await;
        let sid = upsert_series_from_metadata(
            &db,
            "mangabaka",
            &sample_metadata(),
            1_700_000_000,
            Utc::now(),
            false,
        )
        .await
        .unwrap()
        .series_id;

        // A linked release stamps coverage + updated_at.
        insert_release_with_spans(
            &db,
            "nyaa:r",
            Some(Span {
                start: 1.0,
                end: 3.0,
            }),
            None,
        )
        .await;
        link_release(
            &db,
            "nyaa:r",
            Some(sid),
            Some("manual"),
            Some(1.0),
            "resolved",
            5_000,
        )
        .await
        .unwrap();
        assert_eq!(
            series::Entity::find_by_id(sid)
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .updated_at,
            5_000
        );

        // A metadata refresh that changes the row (new hash) must NOT touch
        // `updated_at` — otherwise the feed would re-emit every series each
        // provider-cache cycle.
        let refreshed = SeriesMetadata {
            description: Some("Updated synopsis.".into()),
            content_hash: "hash-v2".into(),
            ..sample_metadata()
        };
        upsert_series_from_metadata(
            &db,
            "mangabaka",
            &refreshed,
            1_700_000_500,
            Utc::now(),
            false,
        )
        .await
        .unwrap();
        let row = series::Entity::find_by_id(sid)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.description.as_deref(), Some("Updated synopsis."));
        assert_eq!(row.updated_at, 5_000, "refresh must not bump updated_at");
    }
}
