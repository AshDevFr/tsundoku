//! Series read/write helpers.

use anyhow::Result;
use sea_orm::sea_query::{Expr, OnConflict, Query};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, EntityTrait, FromQueryResult,
    PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Statement, TransactionTrait,
};

use crate::entities::{series, series_external_ids};

/// Provenance string used in `series.metadata_source` for operator-authored
/// rows that have no provider behind them. Kept in the repo because every
/// bulk write needs to skip these (manual rows have no upstream to refetch
/// from, so a hash invalidation would do nothing useful).
const MANUAL_METADATA_SOURCE: &str = "manual";

/// Result of a bulk hash-invalidation call.
#[derive(Debug, Clone, Copy)]
pub struct InvalidateHashesOutcome {
    /// Number of rows whose `metadata_hash` was cleared.
    pub invalidated: u64,
    /// Number of rows that matched the scope but were skipped because
    /// `metadata_source = 'manual'`. Reported for operator transparency:
    /// "you have N manual rows we left alone."
    pub skipped_manual: u64,
}

pub use series::Model;

pub async fn upsert(db: &DatabaseConnection, model: series::ActiveModel) -> Result<series::Model> {
    let inserted = series::Entity::insert(model)
        .on_conflict(
            OnConflict::column(series::Column::Id)
                .update_columns([
                    series::Column::CanonicalTitle,
                    series::Column::AlternateTitlesJson,
                    series::Column::CoverUrl,
                    series::Column::Kind,
                    series::Column::Status,
                    series::Column::Year,
                    series::Column::MetadataJson,
                    series::Column::MetadataSource,
                    series::Column::MetadataHash,
                    series::Column::MetadataFetchedAt,
                    series::Column::LastReleaseAt,
                    series::Column::HighestVolume,
                    series::Column::HighestChapter,
                ])
                .to_owned(),
        )
        .exec_with_returning(db)
        .await?;
    Ok(inserted)
}

/// Insert a brand-new series row, returning the persisted model (with its
/// auto-assigned `id`). Unlike [`upsert`], this never updates an existing
/// row: the caller is creating a fresh series (e.g. an operator-authored
/// manual series with no provider mapping), so `id` must be left unset.
pub async fn create(db: &DatabaseConnection, model: series::ActiveModel) -> Result<series::Model> {
    Ok(series::Entity::insert(model)
        .exec_with_returning(db)
        .await?)
}

/// Editable descriptive fields of a manual series. All values are the new
/// desired state (a full replace, not a partial patch); the caller is
/// expected to have trimmed strings and mapped empties to `None` already.
/// `alternate_titles` is the full replacement list — an empty vec clears the
/// stored alternates (persisted as SQL `NULL`, matching "absent" elsewhere in
/// the schema).
#[derive(Debug, Clone)]
pub struct ManualSeriesEdit {
    pub canonical_title: String,
    pub alternate_titles: Vec<String>,
    pub kind: Option<String>,
    pub status: Option<String>,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
    pub description: Option<String>,
}

/// Result of [`update_manual_fields`]. The caller maps these to HTTP statuses:
/// `Updated` → 200, `NotManual` → 409, `NotFound` → 404.
#[derive(Debug)]
pub enum UpdateManualOutcome {
    /// The row was a manual series and its editable fields were rewritten.
    /// Boxed because `Model` is large relative to the unit variants.
    Updated(Box<Model>),
    /// A row with this id exists but is provider-backed (`metadata_source`
    /// is not `manual`), so it is owned by the provider and must not be
    /// hand-edited — a refresh would clobber the change.
    NotManual,
    /// No series row with this id.
    NotFound,
}

/// Overwrite the editable descriptive fields of a *manual* series.
///
/// Only `metadata_source = 'manual'` rows are editable: provider-backed rows
/// are the provider's to own and would have any edit overwritten on the next
/// metadata refresh, so they are rejected with [`UpdateManualOutcome::NotManual`]
/// and left untouched. Provider/metadata/provenance columns (`metadata_source`,
/// `metadata_hash`, `metadata_json`, `metadata_fetched_at`, the `*_at`
/// timestamps, the span/total/rating denormalizations, `owned`) are never
/// written here — only the operator-authored descriptive fields change.
pub async fn update_manual_fields(
    db: &DatabaseConnection,
    id: i32,
    edit: ManualSeriesEdit,
) -> Result<UpdateManualOutcome> {
    use sea_orm::{ActiveModelTrait, ActiveValue::Set};

    let Some(row) = series::Entity::find_by_id(id).one(db).await? else {
        return Ok(UpdateManualOutcome::NotFound);
    };
    if row.metadata_source != MANUAL_METADATA_SOURCE {
        return Ok(UpdateManualOutcome::NotManual);
    }

    let alternate_titles_json = if edit.alternate_titles.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&edit.alternate_titles)?)
    };

    let mut active: series::ActiveModel = row.into();
    active.canonical_title = Set(edit.canonical_title);
    active.alternate_titles_json = Set(alternate_titles_json);
    active.kind = Set(edit.kind);
    active.status = Set(edit.status);
    active.year = Set(edit.year);
    active.cover_url = Set(edit.cover_url);
    active.description = Set(edit.description);

    let updated = active.update(db).await?;
    Ok(UpdateManualOutcome::Updated(Box::new(updated)))
}

/// Result of [`set_ignore_completion`]. The caller maps these to HTTP statuses:
/// `Updated` → 200, `NotFound` → 404.
#[derive(Debug)]
pub enum SetIgnoreCompletionOutcome {
    /// The flag was written; the boxed model reflects the new value.
    Updated(Box<Model>),
    /// No series row with this id.
    NotFound,
}

/// Set the operator `ignore_completion` flag on *any* series, provider-backed
/// or manual.
///
/// Unlike [`update_manual_fields`], this is intentionally allowed on
/// provider-backed rows: the flag is operator-owned and the metadata refresh
/// leaves it alone (the refresh UPDATE writes `ignore_completion` as `NotSet`),
/// so there is nothing for a provider re-fetch to clobber. Only the one column
/// is written; every provider/descriptive field is left as-is.
pub async fn set_ignore_completion(
    db: &DatabaseConnection,
    id: i32,
    ignore: bool,
) -> Result<SetIgnoreCompletionOutcome> {
    use sea_orm::{ActiveModelTrait, ActiveValue::Set};

    let Some(row) = series::Entity::find_by_id(id).one(db).await? else {
        return Ok(SetIgnoreCompletionOutcome::NotFound);
    };
    let mut active: series::ActiveModel = row.into();
    active.ignore_completion = Set(ignore);
    let updated = active.update(db).await?;
    Ok(SetIgnoreCompletionOutcome::Updated(Box::new(updated)))
}

pub async fn find_by_id(db: &DatabaseConnection, id: i32) -> Result<Option<Model>> {
    Ok(series::Entity::find_by_id(id).one(db).await?)
}

pub async fn recent(db: &DatabaseConnection, limit: u64) -> Result<Vec<Model>> {
    Ok(series::Entity::find()
        .order_by_desc(series::Column::LastReleaseAt)
        .limit(limit)
        .all(db)
        .await?)
}

/// One page of the incremental release feed: series with real coverage
/// activity (`updated_at > 0`) whose keyset position is strictly after
/// `(after_updated_at, after_id)`, ordered ascending so a consumer can walk
/// forward from a stored cursor. Pass `(0, 0)` to start from the beginning.
///
/// Keyset, not offset: stable and gap-free while series are being re-stamped
/// concurrently. A series that gets re-stamped jumps to the tail (higher
/// `updated_at`) and is re-delivered, so callers must upsert by `series_id`.
/// Fetch `limit + 1` to detect whether more pages remain.
pub async fn feed_after(
    db: &DatabaseConnection,
    after_updated_at: i64,
    after_id: i32,
    limit: u64,
) -> Result<Vec<Model>> {
    // (updated_at, id) > (after_updated_at, after_id)
    let keyset = Condition::any()
        .add(series::Column::UpdatedAt.gt(after_updated_at))
        .add(
            Condition::all()
                .add(series::Column::UpdatedAt.eq(after_updated_at))
                .add(series::Column::Id.gt(after_id)),
        );
    Ok(series::Entity::find()
        .filter(series::Column::UpdatedAt.gt(0))
        .filter(keyset)
        .order_by_asc(series::Column::UpdatedAt)
        .order_by_asc(series::Column::Id)
        .limit(limit)
        .all(db)
        .await?)
}

/// Find the series whose `metadata_hash` matches the given hash, if any.
/// Used by the resolver to skip a write when an incoming metadata payload
/// is byte-for-byte identical to what's already on the row.
pub async fn find_by_hash(db: &DatabaseConnection, hash: &str) -> Result<Option<Model>> {
    Ok(series::Entity::find()
        .filter(series::Column::MetadataHash.eq(hash))
        .one(db)
        .await?)
}

/// FTS5 match against `series_fts`. Returns matching series rows ordered by
/// FTS relevance (best first). The caller supplies a raw FTS5 MATCH expression
/// (e.g. a quoted phrase or column-prefixed term); the function does not
/// escape the input.
pub async fn search_fts(
    db: &DatabaseConnection,
    match_expr: &str,
    limit: u64,
) -> Result<Vec<Model>> {
    let backend = db.get_database_backend();
    let sql = "SELECT s.*
               FROM series s
               JOIN series_fts f ON f.rowid = s.id
               WHERE series_fts MATCH ?1
               ORDER BY rank
               LIMIT ?2";
    let stmt =
        Statement::from_sql_and_values(backend, sql, [match_expr.into(), (limit as i64).into()]);
    let rows = Model::find_by_statement(stmt).all(db).await?;
    Ok(rows)
}

/// Clear `metadata_hash` for every provider-backed series row, so the
/// next provider refresh tick rewrites them instead of short-circuiting
/// on a hash match.
///
/// `provider` scopes the operation:
/// - `None`: every non-manual row is cleared (today this is equivalent
///   to "every series resolved by any provider", since v1 has one).
/// - `Some(id)`: only rows that have a `series_external_ids` entry for
///   that provider id are cleared.
///
/// Manual rows (`metadata_source = 'manual'`) are always skipped — they
/// have no provider behind them, so the hash has no meaning.
///
/// The count + UPDATE run in one transaction so the reported
/// `skipped_manual` is consistent with what the UPDATE actually
/// considered, even if a write lands concurrently. Returns
/// `(invalidated, skipped_manual)`.
pub async fn invalidate_metadata_hashes(
    db: &DatabaseConnection,
    provider: Option<&str>,
) -> Result<InvalidateHashesOutcome> {
    let txn = db.begin().await?;

    let mut manual_q =
        series::Entity::find().filter(series::Column::MetadataSource.eq(MANUAL_METADATA_SOURCE));
    if let Some(p) = provider {
        manual_q = manual_q.filter(provider_filter(p));
    }
    let skipped_manual = manual_q.count(&txn).await?;

    let mut update = series::Entity::update_many()
        .col_expr(
            series::Column::MetadataHash,
            Expr::value(Option::<String>::None),
        )
        .filter(series::Column::MetadataSource.ne(MANUAL_METADATA_SOURCE))
        // Avoid pointless writes on rows that already have a NULL hash
        // (a fresh series with no prior refresh, or one already cleared
        // by an earlier call). Keeps `rows_affected` honest.
        .filter(series::Column::MetadataHash.is_not_null());
    if let Some(p) = provider {
        update = update.filter(provider_filter(p));
    }
    let invalidated = update.exec(&txn).await?.rows_affected;

    txn.commit().await?;
    Ok(InvalidateHashesOutcome {
        invalidated,
        skipped_manual,
    })
}

/// `series.id IN (SELECT series_id FROM series_external_ids WHERE provider = ?)`.
/// Factored out so the count and the UPDATE apply the exact same scope.
fn provider_filter(provider: &str) -> sea_orm::sea_query::SimpleExpr {
    series::Column::Id.in_subquery(
        Query::select()
            .column(series_external_ids::Column::SeriesId)
            .from(series_external_ids::Entity)
            .and_where(series_external_ids::Column::Provider.eq(provider))
            .to_owned(),
    )
}

// re-export the entity column/active-model surface for callers that need to
// build their own ActiveModel literals without depending on td-db internals.
pub use series::{ActiveModel, Column, Entity};

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::{Database, Set};

    async fn fresh() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db
    }

    async fn seed(
        db: &DatabaseConnection,
        title: &str,
        metadata_source: &str,
        metadata_hash: Option<&str>,
    ) -> i32 {
        let now = Utc::now().timestamp();
        let model = series::ActiveModel {
            canonical_title: Set(title.into()),
            metadata_source: Set(metadata_source.into()),
            metadata_hash: Set(metadata_hash.map(str::to_owned)),
            metadata_fetched_at: Set(now),
            first_seen_at: Set(now),
            last_release_at: Set(now),
            owned: Set(0),
            ..Default::default()
        };
        series::Entity::insert(model)
            .exec_with_returning(db)
            .await
            .unwrap()
            .id
    }

    async fn link(db: &DatabaseConnection, series_id: i32, provider: &str, external_id: &str) {
        let now = Utc::now().timestamp();
        series_external_ids::Entity::insert(series_external_ids::ActiveModel {
            provider: Set(provider.into()),
            external_id: Set(external_id.into()),
            series_id: Set(series_id),
            fetched_at: Set(now),
        })
        .exec(db)
        .await
        .unwrap();
    }

    async fn hash_of(db: &DatabaseConnection, id: i32) -> Option<String> {
        series::Entity::find_by_id(id)
            .one(db)
            .await
            .unwrap()
            .unwrap()
            .metadata_hash
    }

    #[tokio::test]
    async fn clears_non_manual_hashes_and_skips_manual_rows() {
        let db = fresh().await;
        let a = seed(&db, "A", "api", Some("hash-a")).await;
        let b = seed(&db, "B", "offline_cache", Some("hash-b")).await;
        let manual = seed(&db, "M", "manual", Some("hash-m")).await;

        let out = invalidate_metadata_hashes(&db, None).await.unwrap();
        assert_eq!(out.invalidated, 2);
        assert_eq!(out.skipped_manual, 1);

        assert_eq!(hash_of(&db, a).await, None);
        assert_eq!(hash_of(&db, b).await, None);
        assert_eq!(hash_of(&db, manual).await, Some("hash-m".into()));
    }

    #[tokio::test]
    async fn ignores_rows_with_already_null_hash() {
        let db = fresh().await;
        let _stale = seed(&db, "stale", "api", None).await;
        let live = seed(&db, "live", "api", Some("hash-live")).await;

        let out = invalidate_metadata_hashes(&db, None).await.unwrap();
        assert_eq!(out.invalidated, 1, "only the row with a hash counts");
        assert_eq!(hash_of(&db, live).await, None);
    }

    fn sample_edit(title: &str) -> ManualSeriesEdit {
        ManualSeriesEdit {
            canonical_title: title.into(),
            alternate_titles: vec!["Alt One".into(), "Alt Two".into()],
            kind: Some("manga".into()),
            status: Some("ongoing".into()),
            year: Some(2021),
            cover_url: Some("https://example/cover.jpg".into()),
            description: Some("a synopsis".into()),
        }
    }

    #[tokio::test]
    async fn update_manual_fields_rewrites_a_manual_row() {
        let db = fresh().await;
        let id = seed(&db, "Old Title", "manual", None).await;

        let outcome = update_manual_fields(&db, id, sample_edit("New Title"))
            .await
            .unwrap();
        let model = match outcome {
            UpdateManualOutcome::Updated(m) => *m,
            other => panic!("expected Updated, got {other:?}"),
        };

        // Returned model reflects the edit...
        assert_eq!(model.canonical_title, "New Title");
        assert_eq!(model.kind.as_deref(), Some("manga"));
        assert_eq!(model.year, Some(2021));
        assert_eq!(
            model.cover_url.as_deref(),
            Some("https://example/cover.jpg")
        );
        assert_eq!(model.description.as_deref(), Some("a synopsis"));
        // ...and metadata provenance is untouched.
        assert_eq!(model.metadata_source, "manual");

        // Alternate titles round-trip through the JSON column.
        let persisted = series::Entity::find_by_id(id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let alts: Vec<String> =
            serde_json::from_str(persisted.alternate_titles_json.as_deref().unwrap()).unwrap();
        assert_eq!(alts, vec!["Alt One", "Alt Two"]);
    }

    #[tokio::test]
    async fn update_manual_fields_clears_alternates_with_empty_list() {
        let db = fresh().await;
        let id = seed(&db, "Title", "manual", None).await;
        // Seed some alternates first.
        update_manual_fields(&db, id, sample_edit("Title"))
            .await
            .unwrap();

        let mut edit = sample_edit("Title");
        edit.alternate_titles = vec![];
        update_manual_fields(&db, id, edit).await.unwrap();

        let persisted = series::Entity::find_by_id(id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            persisted.alternate_titles_json, None,
            "an empty alternate-title list clears the column to NULL"
        );
    }

    #[tokio::test]
    async fn update_manual_fields_refuses_provider_backed_rows() {
        let db = fresh().await;
        for source in ["api", "offline_cache"] {
            let id = seed(&db, "Provider Owned", source, Some("hash")).await;
            let outcome = update_manual_fields(&db, id, sample_edit("Hijacked"))
                .await
                .unwrap();
            assert!(
                matches!(outcome, UpdateManualOutcome::NotManual),
                "{source} row should be NotManual"
            );

            // The row is left exactly as it was.
            let row = series::Entity::find_by_id(id)
                .one(&db)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.canonical_title, "Provider Owned");
            assert_eq!(row.metadata_source, source);
            assert_eq!(row.metadata_hash.as_deref(), Some("hash"));
        }
    }

    #[tokio::test]
    async fn update_manual_fields_reports_not_found() {
        let db = fresh().await;
        let outcome = update_manual_fields(&db, 9999, sample_edit("Ghost"))
            .await
            .unwrap();
        assert!(matches!(outcome, UpdateManualOutcome::NotFound));
    }

    #[tokio::test]
    async fn set_ignore_completion_toggles_on_and_off_for_any_source() {
        let db = fresh().await;
        // Allowed on a provider-backed row (unlike manual-field edits).
        let id = seed(&db, "Provider Owned", "api", Some("hash")).await;

        let model = match set_ignore_completion(&db, id, true).await.unwrap() {
            SetIgnoreCompletionOutcome::Updated(m) => *m,
            other => panic!("expected Updated, got {other:?}"),
        };
        assert!(model.ignore_completion, "returned model reflects the flag");
        // Provider provenance is untouched.
        assert_eq!(model.metadata_source, "api");
        assert_eq!(model.metadata_hash.as_deref(), Some("hash"));

        // Persisted, and the inverse toggle clears it.
        assert!(
            series::Entity::find_by_id(id)
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .ignore_completion
        );
        match set_ignore_completion(&db, id, false).await.unwrap() {
            SetIgnoreCompletionOutcome::Updated(m) => assert!(!m.ignore_completion),
            other => panic!("expected Updated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn set_ignore_completion_reports_not_found() {
        let db = fresh().await;
        assert!(matches!(
            set_ignore_completion(&db, 9999, true).await.unwrap(),
            SetIgnoreCompletionOutcome::NotFound
        ));
    }

    #[tokio::test]
    async fn provider_filter_scopes_to_rows_with_matching_external_id() {
        let db = fresh().await;
        let mb = seed(&db, "MB-backed", "api", Some("hash-mb")).await;
        link(&db, mb, "mangabaka", "1").await;
        let other = seed(&db, "Other-backed", "api", Some("hash-other")).await;
        link(&db, other, "anilist", "999").await;

        let out = invalidate_metadata_hashes(&db, Some("mangabaka"))
            .await
            .unwrap();
        assert_eq!(out.invalidated, 1);
        assert_eq!(out.skipped_manual, 0);

        assert_eq!(hash_of(&db, mb).await, None);
        assert_eq!(
            hash_of(&db, other).await,
            Some("hash-other".into()),
            "anilist-backed row stays untouched when scoped to mangabaka",
        );
    }

    async fn seed_with_updated(db: &DatabaseConnection, title: &str, updated_at: i64) -> i32 {
        let id = seed(db, title, "api", None).await;
        series::Entity::update(series::ActiveModel {
            id: Set(id),
            updated_at: Set(updated_at),
            ..Default::default()
        })
        .exec(db)
        .await
        .unwrap();
        id
    }

    #[tokio::test]
    async fn feed_after_skips_inactive_and_walks_ascending() {
        let db = fresh().await;
        // updated_at = 0 (never had coverage activity) must not surface.
        let _inactive = seed(&db, "Inactive", "api", None).await;
        let b = seed_with_updated(&db, "B", 100).await;
        let c = seed_with_updated(&db, "C", 200).await;

        let page = feed_after(&db, 0, 0, 10).await.unwrap();
        let ids: Vec<i32> = page.iter().map(|s| s.id).collect();
        assert_eq!(
            ids,
            vec![b, c],
            "ascending by updated_at, inactive excluded"
        );
    }

    #[tokio::test]
    async fn feed_after_is_keyset_resumable_with_ties() {
        let db = fresh().await;
        let d = seed_with_updated(&db, "D", 50).await;
        // Two series share updated_at = 100; the id is the tiebreaker.
        let e1 = seed_with_updated(&db, "E1", 100).await;
        let e2 = seed_with_updated(&db, "E2", 100).await;
        assert!(e1 < e2);

        // Walk one at a time from the start.
        let p1 = feed_after(&db, 0, 0, 1).await.unwrap();
        assert_eq!(p1.iter().map(|s| s.id).collect::<Vec<_>>(), vec![d]);
        let p2 = feed_after(&db, 50, d, 1).await.unwrap();
        assert_eq!(p2.iter().map(|s| s.id).collect::<Vec<_>>(), vec![e1]);
        // Same-second tie: resuming after (100, e1) yields e2, not a re-deliver.
        let p3 = feed_after(&db, 100, e1, 10).await.unwrap();
        assert_eq!(p3.iter().map(|s| s.id).collect::<Vec<_>>(), vec![e2]);
        let p4 = feed_after(&db, 100, e2, 10).await.unwrap();
        assert!(p4.is_empty(), "caught up");
    }

    #[tokio::test]
    async fn feed_after_redelivers_a_rebumped_series() {
        let db = fresh().await;
        let b = seed_with_updated(&db, "B", 100).await;
        // Consumer has walked past B (cursor at (100, b)).
        assert!(feed_after(&db, 100, b, 10).await.unwrap().is_empty());

        // B's coverage changes again -> updated_at jumps to the tail.
        series::Entity::update(series::ActiveModel {
            id: Set(b),
            updated_at: Set(300),
            ..Default::default()
        })
        .exec(&db)
        .await
        .unwrap();

        let page = feed_after(&db, 100, b, 10).await.unwrap();
        assert_eq!(
            page.iter().map(|s| s.id).collect::<Vec<_>>(),
            vec![b],
            "a re-stamped series re-appears past the old cursor",
        );
    }
}
