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
                    series::Column::PublishedStartDate,
                    series::Column::PublishedEndDate,
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

/// Outcome of [`set_wishlisted`].
pub enum SetWishlistedOutcome {
    /// The flag was written; the boxed model reflects the new value.
    Updated(Box<Model>),
    /// No series row with this id.
    NotFound,
}

/// Set or clear the operator `wishlisted_at` flag on *any* series, provider-
/// backed or manual.
///
/// `wishlisted = true` stamps the column with `now` (epoch seconds);
/// `false` clears it to NULL. Like [`set_ignore_completion`], only the one
/// operator-owned column is written — every provider/descriptive field is left
/// as-is, and a metadata refresh never clobbers it. Setting an already-set flag
/// re-stamps the timestamp (idempotent in effect, fresh "clipped at").
pub async fn set_wishlisted(
    db: &DatabaseConnection,
    id: i32,
    wishlisted: bool,
    now: i64,
) -> Result<SetWishlistedOutcome> {
    use sea_orm::{ActiveModelTrait, ActiveValue::Set};

    let Some(row) = series::Entity::find_by_id(id).one(db).await? else {
        return Ok(SetWishlistedOutcome::NotFound);
    };
    let mut active: series::ActiveModel = row.into();
    active.wishlisted_at = Set(wishlisted.then_some(now));
    let updated = active.update(db).await?;
    Ok(SetWishlistedOutcome::Updated(Box::new(updated)))
}

/// Set or clear `wishlisted_at` on every listed series in one UPDATE.
///
/// Same column semantics as [`set_wishlisted`] (`true` stamps `now`, `false`
/// clears to NULL, works on any series regardless of provenance); ids with no
/// series row are simply absent from the returned affected count — the bulk
/// caller treats them as already gone, not as an error.
pub async fn set_wishlisted_bulk(
    db: &DatabaseConnection,
    ids: &[i32],
    wishlisted: bool,
    now: i64,
) -> Result<u64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let res = series::Entity::update_many()
        .col_expr(
            series::Column::WishlistedAt,
            Expr::value(wishlisted.then_some(now)),
        )
        .filter(series::Column::Id.is_in(ids.iter().copied()))
        .exec(db)
        .await?;
    Ok(res.rows_affected)
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
/// `external_ids` optionally narrows the page to series carrying one of the
/// given `(provider, external_id)` mappings — a consumer (e.g. a Codex release
/// plugin) sending the subset it tracks so it only receives changes it cares
/// about. Empty ⇒ no filter. The filter composes with the cursor; note that a
/// cursor already advanced past a *newly*-added id won't replay that series'
/// older changes, so adding an id wants a one-off unfiltered backfill for it.
///
/// Keyset, not offset: stable and gap-free while series are being re-stamped
/// concurrently. A series that gets re-stamped jumps to the tail (higher
/// `updated_at`) and is re-delivered, so callers must upsert by `series_id`.
/// Fetch `limit + 1` to detect whether more pages remain.
pub async fn feed_after(
    db: &DatabaseConnection,
    after_updated_at: i64,
    after_id: i32,
    external_ids: &[(String, String)],
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
    let mut query = series::Entity::find()
        .filter(series::Column::UpdatedAt.gt(0))
        .filter(keyset);
    if !external_ids.is_empty() {
        query = query.filter(series::Column::Id.in_subquery(external_ids_subquery(external_ids)));
    }
    Ok(query
        .order_by_asc(series::Column::UpdatedAt)
        .order_by_asc(series::Column::Id)
        .limit(limit)
        .all(db)
        .await?)
}

/// Subquery selecting `series_id`s that carry any of the given
/// `(provider, external_id)` mappings. Grouped by provider into one
/// `provider = ? AND external_id IN (...)` clause each (usually a single
/// provider), so thousands of ids stay a bounded `IN` list rather than a deep
/// `OR` chain.
fn external_ids_subquery(external_ids: &[(String, String)]) -> sea_orm::sea_query::SelectStatement {
    use std::collections::HashMap;
    let mut by_provider: HashMap<&str, Vec<&str>> = HashMap::new();
    for (provider, external_id) in external_ids {
        by_provider
            .entry(provider.as_str())
            .or_default()
            .push(external_id.as_str());
    }
    let mut cond = Condition::any();
    for (provider, ids) in by_provider {
        cond = cond.add(
            Condition::all()
                .add(series_external_ids::Column::Provider.eq(provider))
                .add(series_external_ids::Column::ExternalId.is_in(ids)),
        );
    }
    Query::select()
        .column(series_external_ids::Column::SeriesId)
        .from(series_external_ids::Entity)
        .cond_where(cond)
        .to_owned()
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

/// Every series id whose FTS5 entry matches `match_expr` — the whole match
/// set, deliberately unlimited.
///
/// The catalog-wide search treats an FTS hit as a match in its own right, so
/// truncating this set silently drops candidates: a one-word query scores far
/// below the Dice floor against a long title, leaving FTS as the only thing
/// keeping that row alive. A `LIMIT 200` here dropped roughly half the match
/// set for common tokens like "World" (374 matches) or "Girl" (305).
///
/// Unbounded is safe because the result is bounded by the catalog itself and
/// carries ids only — the pathological "matches everything" query returns a
/// few KB of `i32`, against a Dice scan that already walks every row. Ordering
/// is not returned: the caller ranks by its own score, using membership here
/// only as a boost and a floor bypass.
///
/// The caller supplies a raw FTS5 MATCH expression; the function does not
/// escape the input.
pub async fn search_fts_ids(db: &DatabaseConnection, match_expr: &str) -> Result<Vec<i32>> {
    #[derive(FromQueryResult)]
    struct IdRow {
        id: i32,
    }
    let backend = db.get_database_backend();
    let sql = "SELECT f.rowid AS id
               FROM series_fts f
               WHERE series_fts MATCH ?1";
    let stmt = Statement::from_sql_and_values(backend, sql, [match_expr.into()]);
    Ok(IdRow::find_by_statement(stmt)
        .all(db)
        .await?
        .into_iter()
        .map(|r| r.id)
        .collect())
}

/// A series row with nothing left pointing at it, safe to delete.
///
/// The predicate is deliberately five-way, and every arm is load-bearing —
/// "has no releases" alone is nowhere near sufficient:
///
/// - **No linked releases.** The obvious arm, and the only one the operator
///   usually thinks of.
/// - **Not a review candidate.** The resolver has to persist a `series` row
///   for every candidate it records, so most release-less series are the
///   *options* the review queue is currently offering. Deleting them empties
///   the "pick the right match" panel.
/// - **No Codex link.** Deleting one destroys the operator's mapping to their
///   library.
/// - **Not owned.**
/// - **Not wishlisted**, when `exclude_wishlisted` — deliberately a toggle,
///   since a wishlisted orphan may be a series the operator added by hand
///   and is waiting on.
///
/// The first three matter more than they look: `review_candidates.series_id`
/// and `codex_series_link.series_id` both declare `ON DELETE CASCADE`, so a
/// wrong predicate does not fail loudly — it succeeds and silently takes the
/// child rows with it. SQLite will not protect this operation; the query has
/// to.
///
/// Shared verbatim by the dry run and the purge so the count the operator
/// confirms is the set that actually gets deleted.
pub fn orphan_series_condition(exclude_wishlisted: bool) -> Condition {
    use sea_orm::sea_query::{Expr, Query};

    let has_release = Query::select()
        .expr(Expr::val(1))
        .from(crate::entities::releases::Entity)
        .and_where(
            Expr::col((
                crate::entities::releases::Entity,
                crate::entities::releases::Column::SeriesId,
            ))
            .equals((series::Entity, series::Column::Id)),
        )
        .to_owned();
    let is_candidate = Query::select()
        .expr(Expr::val(1))
        .from(crate::entities::review_candidates::Entity)
        .and_where(
            Expr::col((
                crate::entities::review_candidates::Entity,
                crate::entities::review_candidates::Column::SeriesId,
            ))
            .equals((series::Entity, series::Column::Id)),
        )
        .to_owned();
    let has_codex_link = Query::select()
        .expr(Expr::val(1))
        .from(crate::entities::codex_series_link::Entity)
        .and_where(
            Expr::col((
                crate::entities::codex_series_link::Entity,
                crate::entities::codex_series_link::Column::SeriesId,
            ))
            .equals((series::Entity, series::Column::Id)),
        )
        .to_owned();

    let mut cond = Condition::all()
        .add(Expr::exists(has_release).not())
        .add(Expr::exists(is_candidate).not())
        .add(Expr::exists(has_codex_link).not())
        .add(series::Column::Owned.eq(0));
    if exclude_wishlisted {
        cond = cond.add(series::Column::WishlistedAt.is_null());
    }
    cond
}

/// How many series the purge would delete, and a sample to show the operator
/// before they commit. Same predicate as [`purge_orphan_series`].
pub async fn count_orphan_series(db: &DatabaseConnection, exclude_wishlisted: bool) -> Result<u64> {
    Ok(series::Entity::find()
        .filter(orphan_series_condition(exclude_wishlisted))
        .count(db)
        .await?)
}

/// A bounded sample of the rows [`count_orphan_series`] counted, oldest ids
/// first so the listing is stable between the dry run and the purge.
pub async fn sample_orphan_series(
    db: &DatabaseConnection,
    exclude_wishlisted: bool,
    limit: u64,
) -> Result<Vec<Model>> {
    Ok(series::Entity::find()
        .filter(orphan_series_condition(exclude_wishlisted))
        .order_by_asc(series::Column::Id)
        .limit(limit)
        .all(db)
        .await?)
}

/// Delete every series matching [`orphan_series_condition`]. Returns how many
/// rows went.
///
/// Child rows in `series_external_ids` / `series_genres` / `series_tags` are
/// deleted explicitly rather than left to `ON DELETE CASCADE`. Cascade needs
/// `PRAGMA foreign_keys=ON`, which is per-connection and set by the pool — an
/// invariant that holds today but is exactly the kind of thing a future
/// refactor drops silently. For an irreversible operation, leaving stale
/// `series_external_ids` rows behind would be nasty: their
/// `UNIQUE(provider, external_id)` would then reject the series ever being
/// rediscovered. Belt and braces is three statements.
///
/// The whole thing runs in one transaction so a partial purge cannot leave
/// dangling children.
pub async fn purge_orphan_series(db: &DatabaseConnection, exclude_wishlisted: bool) -> Result<u64> {
    use sea_orm::TransactionTrait;

    let txn = db.begin().await?;
    let ids: Vec<i32> = series::Entity::find()
        .filter(orphan_series_condition(exclude_wishlisted))
        .select_only()
        .column(series::Column::Id)
        .into_tuple::<i32>()
        .all(&txn)
        .await?;
    if ids.is_empty() {
        txn.commit().await?;
        return Ok(0);
    }

    crate::entities::series_external_ids::Entity::delete_many()
        .filter(crate::entities::series_external_ids::Column::SeriesId.is_in(ids.iter().copied()))
        .exec(&txn)
        .await?;
    crate::entities::series_genres::Entity::delete_many()
        .filter(crate::entities::series_genres::Column::SeriesId.is_in(ids.iter().copied()))
        .exec(&txn)
        .await?;
    crate::entities::series_tags::Entity::delete_many()
        .filter(crate::entities::series_tags::Column::SeriesId.is_in(ids.iter().copied()))
        .exec(&txn)
        .await?;
    // The `series_ad` trigger keeps `series_fts` in step with this delete.
    let res = series::Entity::delete_many()
        .filter(series::Column::Id.is_in(ids.iter().copied()))
        .exec(&txn)
        .await?;
    txn.commit().await?;
    Ok(res.rows_affected)
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
    use sea_orm::{ActiveModelTrait, Database, Set};

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
    async fn set_wishlisted_stamps_and_clears_for_any_source() {
        let db = fresh().await;
        let id = seed(&db, "Provider Backed", "api", Some("hash")).await;

        let model = match set_wishlisted(&db, id, true, 1234).await.unwrap() {
            SetWishlistedOutcome::Updated(m) => *m,
            SetWishlistedOutcome::NotFound => panic!("expected Updated"),
        };
        assert_eq!(model.wishlisted_at, Some(1234), "stamps the clip time");
        // Provider provenance is untouched.
        assert_eq!(model.metadata_source, "api");
        assert_eq!(model.metadata_hash.as_deref(), Some("hash"));

        // Clearing nulls the column back out.
        match set_wishlisted(&db, id, false, 5678).await.unwrap() {
            SetWishlistedOutcome::Updated(m) => assert_eq!(m.wishlisted_at, None),
            SetWishlistedOutcome::NotFound => panic!("expected Updated"),
        }
    }

    #[tokio::test]
    async fn set_wishlisted_reports_not_found() {
        let db = fresh().await;
        assert!(matches!(
            set_wishlisted(&db, 9999, true, 1).await.unwrap(),
            SetWishlistedOutcome::NotFound
        ));
    }

    #[tokio::test]
    async fn set_wishlisted_bulk_stamps_and_clears_counting_only_existing() {
        let db = fresh().await;
        let provider_backed = seed(&db, "Provider Backed", "api", Some("hash")).await;
        let manual = seed(&db, "Manual Row", "manual", None).await;

        // Unknown ids are silently dropped from the count.
        let updated = set_wishlisted_bulk(&db, &[provider_backed, manual, 9999], true, 1234)
            .await
            .unwrap();
        assert_eq!(updated, 2);
        for id in [provider_backed, manual] {
            let row = series::Entity::find_by_id(id)
                .one(&db)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.wishlisted_at, Some(1234), "stamps every listed row");
        }

        // Clearing one row leaves the other clipped.
        let updated = set_wishlisted_bulk(&db, &[provider_backed], false, 5678)
            .await
            .unwrap();
        assert_eq!(updated, 1);
        let cleared = series::Entity::find_by_id(provider_backed)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cleared.wishlisted_at, None);
        let untouched = series::Entity::find_by_id(manual)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(untouched.wishlisted_at, Some(1234));
    }

    #[tokio::test]
    async fn set_wishlisted_bulk_empty_ids_is_a_no_op() {
        let db = fresh().await;
        assert_eq!(set_wishlisted_bulk(&db, &[], true, 1).await.unwrap(), 0);
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

        let page = feed_after(&db, 0, 0, &[], 10).await.unwrap();
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
        let p1 = feed_after(&db, 0, 0, &[], 1).await.unwrap();
        assert_eq!(p1.iter().map(|s| s.id).collect::<Vec<_>>(), vec![d]);
        let p2 = feed_after(&db, 50, d, &[], 1).await.unwrap();
        assert_eq!(p2.iter().map(|s| s.id).collect::<Vec<_>>(), vec![e1]);
        // Same-second tie: resuming after (100, e1) yields e2, not a re-deliver.
        let p3 = feed_after(&db, 100, e1, &[], 10).await.unwrap();
        assert_eq!(p3.iter().map(|s| s.id).collect::<Vec<_>>(), vec![e2]);
        let p4 = feed_after(&db, 100, e2, &[], 10).await.unwrap();
        assert!(p4.is_empty(), "caught up");
    }

    #[tokio::test]
    async fn feed_after_redelivers_a_rebumped_series() {
        let db = fresh().await;
        let b = seed_with_updated(&db, "B", 100).await;
        // Consumer has walked past B (cursor at (100, b)).
        assert!(feed_after(&db, 100, b, &[], 10).await.unwrap().is_empty());

        // B's coverage changes again -> updated_at jumps to the tail.
        series::Entity::update(series::ActiveModel {
            id: Set(b),
            updated_at: Set(300),
            ..Default::default()
        })
        .exec(&db)
        .await
        .unwrap();

        let page = feed_after(&db, 100, b, &[], 10).await.unwrap();
        assert_eq!(
            page.iter().map(|s| s.id).collect::<Vec<_>>(),
            vec![b],
            "a re-stamped series re-appears past the old cursor",
        );
    }

    #[tokio::test]
    async fn feed_after_filters_by_external_ids() {
        let db = fresh().await;
        let a = seed_with_updated(&db, "A", 100).await;
        let b = seed_with_updated(&db, "B", 200).await;
        let c = seed_with_updated(&db, "C", 300).await;
        link(&db, a, "mangabaka", "1").await;
        link(&db, b, "mangabaka", "2").await;
        link(&db, c, "mangabaka", "3").await;
        // A second provider mapping on B, to prove provider is matched too.
        link(&db, b, "anilist", "999").await;

        // Filter to A and C only.
        let want = [
            ("mangabaka".to_string(), "1".to_string()),
            ("mangabaka".to_string(), "3".to_string()),
        ];
        let page = feed_after(&db, 0, 0, &want, 10).await.unwrap();
        assert_eq!(page.iter().map(|s| s.id).collect::<Vec<_>>(), vec![a, c]);

        // The provider half of the pair matters: anilist:1 matches nothing.
        let none = [("anilist".to_string(), "1".to_string())];
        assert!(feed_after(&db, 0, 0, &none, 10).await.unwrap().is_empty());

        // The filter composes with the cursor: resuming after A yields only C.
        let after_a = feed_after(&db, 100, a, &want, 10).await.unwrap();
        assert_eq!(after_a.iter().map(|s| s.id).collect::<Vec<_>>(), vec![c]);
    }

    /// Insert a series with a `description`, exercising the FTS triggers so the
    /// `series_fts.description` column is populated.
    async fn seed_with_description(db: &DatabaseConnection, title: &str, description: &str) -> i32 {
        let now = Utc::now().timestamp();
        series::Entity::insert(series::ActiveModel {
            canonical_title: Set(title.into()),
            description: Set(Some(description.into())),
            metadata_source: Set("api".into()),
            metadata_fetched_at: Set(now),
            first_seen_at: Set(now),
            last_release_at: Set(now),
            owned: Set(0),
            ..Default::default()
        })
        .exec_with_returning(db)
        .await
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn fts_description_scoping_gates_synopsis_matches() {
        let db = fresh().await;
        // Term "dragon" appears only in the synopsis, never in the title.
        let id = seed_with_description(&db, "Solo Leveling", "A hunter fights a dragon").await;

        // Title-scoped expression (default search mode) must NOT surface a
        // description-only hit — reproducing pre-description-column behavior.
        let scoped = search_fts_ids(&db, "{title alternate_titles} : (\"dragon\"*)")
            .await
            .unwrap();
        assert!(
            scoped.is_empty(),
            "title-scoped match should ignore the description"
        );

        // Unscoped expression (toggle on) now spans the description column.
        let unscoped = search_fts_ids(&db, "\"dragon\"*").await.unwrap();
        assert_eq!(unscoped, vec![id]);

        // A genuine title term still matches in the scoped mode.
        let title_hit = search_fts_ids(&db, "{title alternate_titles} : (\"solo\"*)")
            .await
            .unwrap();
        assert_eq!(title_hit, vec![id]);
    }

    #[tokio::test]
    async fn fts_description_tracks_updates() {
        let db = fresh().await;
        let id = seed_with_description(&db, "Untitled", "no keyword yet").await;
        assert!(
            search_fts_ids(&db, "\"griffin\"*")
                .await
                .unwrap()
                .is_empty()
        );

        // Update the description; the AFTER UPDATE trigger must refresh the FTS row.
        let mut row: series::ActiveModel = series::Entity::find_by_id(id)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .into();
        row.description = Set(Some("now mentions a griffin".into()));
        row.update(&db).await.unwrap();

        let hit = search_fts_ids(&db, "\"griffin\"*").await.unwrap();
        assert_eq!(hit, vec![id]);
    }
}

#[cfg(test)]
mod orphan_purge_tests {
    use super::*;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::{ActiveModelTrait, ConnectionTrait, Database, EntityTrait, Set};

    /// Unlike the other test harnesses here, this one turns **foreign keys on**
    /// — SQLite defaults them off, and the production pool pins the pragma. A
    /// purge test against a database without FK enforcement is testing a
    /// different database from the one that ships, and would quietly miss the
    /// cascade behaviour that makes a wrong predicate destructive.
    async fn fresh_with_fks() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db.execute_unprepared("PRAGMA foreign_keys=ON")
            .await
            .unwrap();
        db
    }

    async fn seed_series(db: &DatabaseConnection, title: &str) -> i32 {
        series::ActiveModel {
            canonical_title: Set(title.into()),
            metadata_source: Set("api".into()),
            metadata_fetched_at: Set(0),
            first_seen_at: Set(0),
            last_release_at: Set(0),
            owned: Set(0),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap()
        .id
    }

    async fn seed_release(db: &DatabaseConnection, id: &str, series_id: Option<i32>) {
        crate::entities::releases::ActiveModel {
            id: Set(id.into()),
            source_kind: Set("nyaa".into()),
            source_name: Set("feed".into()),
            external_id: Set(id.into()),
            title: Set("A release".into()),
            link: Set(format!("https://nyaa.si/view/{id}")),
            posted_at: Set(0),
            observed_at: Set(0),
            series_id: Set(series_id),
            resolution_status: Set("resolved".into()),
            resolution_attempts: Set(0),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap();
    }

    /// Every exclusion arm, asserted independently. A series that satisfies
    /// four of the five must still be spared.
    #[tokio::test]
    async fn orphan_predicate_spares_every_referenced_series() {
        let db = fresh_with_fks().await;

        let safe = seed_series(&db, "Unreferenced").await;

        let with_release = seed_series(&db, "Has a release").await;
        seed_release(&db, "r1", Some(with_release)).await;

        let candidate = seed_series(&db, "Is a review candidate").await;
        seed_release(&db, "r2", None).await;
        crate::entities::review_candidates::ActiveModel {
            release_id: Set("r2".into()),
            series_id: Set(candidate),
            score: Set(0.5),
            reason: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();

        let codex_linked = seed_series(&db, "Linked to Codex").await;
        crate::entities::codex_series_link::ActiveModel {
            series_id: Set(codex_linked),
            codex_series_uuid: Set("codex-uuid".into()),
            link_kind: Set("auto".into()),
            synced_at: Set(0),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap();

        let owned = seed_series(&db, "Owned").await;
        series::ActiveModel {
            id: Set(owned),
            owned: Set(1),
            ..Default::default()
        }
        .update(&db)
        .await
        .unwrap();

        let wishlisted = seed_series(&db, "Wishlisted").await;
        series::ActiveModel {
            id: Set(wishlisted),
            wishlisted_at: Set(Some(100)),
            ..Default::default()
        }
        .update(&db)
        .await
        .unwrap();

        let ids: Vec<i32> = sample_orphan_series(&db, true, 100)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(
            ids,
            vec![safe],
            "only the genuinely unreferenced series is selectable",
        );
        assert_eq!(count_orphan_series(&db, true).await.unwrap(), 1);

        // The toggle is the one arm that can be relaxed.
        let mut relaxed: Vec<i32> = sample_orphan_series(&db, false, 100)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        relaxed.sort();
        assert_eq!(relaxed, vec![safe, wishlisted]);
    }

    /// The purge deletes exactly what the dry run counted, and leaves no
    /// dangling child rows behind it.
    #[tokio::test]
    async fn purge_removes_only_orphans_and_cleans_their_children() {
        let db = fresh_with_fks().await;
        let doomed = seed_series(&db, "Doomed").await;
        let kept = seed_series(&db, "Kept").await;
        seed_release(&db, "r1", Some(kept)).await;

        crate::repos::series_external_ids_repo::upsert(&db, doomed, "mangabaka", "111", 0)
            .await
            .unwrap();
        crate::repos::series_external_ids_repo::upsert(&db, kept, "mangabaka", "222", 0)
            .await
            .unwrap();
        crate::repos::tagging_repo::set_series_genres(&db, doomed, &["action".into()])
            .await
            .unwrap();
        crate::repos::tagging_repo::set_series_tags(&db, doomed, &["isekai".into()])
            .await
            .unwrap();

        let expected = count_orphan_series(&db, true).await.unwrap();
        assert_eq!(expected, 1);
        assert_eq!(purge_orphan_series(&db, true).await.unwrap(), expected);

        let remaining: Vec<i32> = series::Entity::find()
            .all(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(remaining, vec![kept]);

        // No stale mapping rows: a leftover `series_external_ids` row would
        // make the series impossible to rediscover, since its
        // `UNIQUE(provider, external_id)` would reject the re-insert.
        let mappings = crate::entities::series_external_ids::Entity::find()
            .all(&db)
            .await
            .unwrap();
        assert_eq!(mappings.len(), 1);
        assert_eq!(mappings[0].series_id, kept);
        assert!(
            crate::entities::series_genres::Entity::find()
                .all(&db)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            crate::entities::series_tags::Entity::find()
                .all(&db)
                .await
                .unwrap()
                .is_empty()
        );

        // Idempotent: a second run finds nothing left to do.
        assert_eq!(purge_orphan_series(&db, true).await.unwrap(), 0);
    }

    /// The regression that matters most: a review candidate's series row must
    /// survive a purge, because `review_candidates` cascades and would be
    /// stripped silently along with it.
    #[tokio::test]
    async fn purge_never_strips_a_live_review_candidate() {
        let db = fresh_with_fks().await;
        let candidate = seed_series(&db, "Candidate").await;
        seed_release(&db, "r1", None).await;
        crate::entities::review_candidates::ActiveModel {
            release_id: Set("r1".into()),
            series_id: Set(candidate),
            score: Set(0.9),
            reason: Set(Some("fuzzy_title:0.900".into())),
        }
        .insert(&db)
        .await
        .unwrap();

        assert_eq!(purge_orphan_series(&db, true).await.unwrap(), 0);
        assert_eq!(
            crate::entities::review_candidates::Entity::find()
                .all(&db)
                .await
                .unwrap()
                .len(),
            1,
            "the review queue's candidate must be untouched",
        );
    }
}
