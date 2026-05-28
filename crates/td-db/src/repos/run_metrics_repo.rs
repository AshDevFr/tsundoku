//! History tables for scheduler ticks + manual triggers.
//!
//! Two parallel pipelines, mirrored on purpose so the admin UI can render
//! both with the same shape:
//!
//! - `poll_runs`: one row per source-poll attempt (cron or manual).
//! - `provider_refreshes`: one row per provider-refresh attempt.
//!
//! Status moves `running → success | failure | skipped`. Wrappers in
//! `td-scheduler` insert a row at the start and finalise it at the end,
//! so an aborted process leaves a `running` row that the admin UI can
//! surface as "in flight" (it'll never auto-resolve, that's intentional).
//!
//! Aggregate readers (`source_summary`, `source_buckets`, and their provider
//! counterparts) compute everything in SQL: success rate over a window,
//! latest run timestamp, and counts per time bucket so the frontend can
//! render runs-over-time without re-bucketing on the client.

use anyhow::Result;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter,
    QueryOrder, QuerySelect, Set, Statement,
};
use serde::Serialize;

use crate::entities::{poll_runs, provider_refreshes, series_refresh_runs};

/// Status discriminator. Stringly typed to keep the column readable in
/// SQLite, but tightly bounded so the UI can switch on it cleanly.
pub mod status {
    pub const RUNNING: &str = "running";
    pub const SUCCESS: &str = "success";
    pub const FAILURE: &str = "failure";
    pub const SKIPPED: &str = "skipped";
}

/// Trigger discriminator.
pub mod trigger {
    pub const CRON: &str = "cron";
    pub const MANUAL: &str = "manual";
    /// One-shot refresh kicked off at server startup when an offline cache
    /// is configured but no dump exists on disk yet.
    pub const STARTUP: &str = "startup";
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PollRunCounts {
    pub fetched: Option<i32>,
    pub new: Option<i32>,
    pub resolved: Option<i32>,
    /// Wall-clock duration of `DiscoverySource::poll()` in milliseconds.
    pub fetch_duration_ms: Option<i64>,
    /// One counter per `ResolutionOutcome` variant (see
    /// `td-resolution::pipeline::ResolutionPath`/`ResolutionStatus`).
    pub outcome_known_id: Option<i32>,
    pub outcome_foreign_id: Option<i32>,
    pub outcome_fuzzy: Option<i32>,
    pub outcome_review: Option<i32>,
    pub outcome_failed: Option<i32>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProviderRefreshCounts {
    pub bytes_downloaded: Option<i64>,
    pub record_count: Option<i64>,
    /// Wall-clock duration of `MetadataProvider::refresh_cache()` in ms.
    pub fetch_duration_ms: Option<i64>,
}

/// Per-tick counters for the bulk series-metadata refresh job. `considered`
/// is the batch size the selection query returned; the four outcome
/// counters partition (a subset of) it depending on what
/// `MetadataProvider::get` returned per row. `fetch_duration_ms` totals
/// all the per-row provider calls in the tick.
#[derive(Debug, Clone, Copy, Default)]
pub struct SeriesRefreshCounts {
    pub considered: Option<i32>,
    pub refreshed: Option<i32>,
    pub unchanged: Option<i32>,
    pub not_found: Option<i32>,
    pub errored: Option<i32>,
    pub fetch_duration_ms: Option<i64>,
}

/// Insert a `running` row for a poll tick. Returns the row id so the
/// caller can finalise it once the tick completes.
pub async fn start_poll_run<C: ConnectionTrait>(
    db: &C,
    source_name: &str,
    source_kind: &str,
    started_at: i64,
    trigger: &str,
) -> Result<i64> {
    let model = poll_runs::ActiveModel {
        source_name: Set(source_name.to_string()),
        source_kind: Set(source_kind.to_string()),
        started_at: Set(started_at),
        finished_at: Set(None),
        status: Set(status::RUNNING.into()),
        fetched_count: Set(None),
        new_count: Set(None),
        resolved_count: Set(None),
        error_message: Set(None),
        trigger: Set(trigger.to_string()),
        ..Default::default()
    };
    let res = poll_runs::Entity::insert(model).exec(db).await?;
    Ok(res.last_insert_id)
}

/// Update an already-inserted poll-run row with its terminal status.
/// Status is one of [`status::SUCCESS`], [`status::FAILURE`],
/// [`status::SKIPPED`]; failures must supply an `error_message`.
pub async fn finalize_poll_run<C: ConnectionTrait>(
    db: &C,
    id: i64,
    finished_at: i64,
    status: &str,
    counts: PollRunCounts,
    error_message: Option<&str>,
    error_kind: Option<&str>,
) -> Result<()> {
    poll_runs::Entity::update_many()
        .filter(poll_runs::Column::Id.eq(id))
        .col_expr(poll_runs::Column::FinishedAt, Expr::value(finished_at))
        .col_expr(poll_runs::Column::Status, Expr::value(status))
        .col_expr(poll_runs::Column::FetchedCount, Expr::value(counts.fetched))
        .col_expr(poll_runs::Column::NewCount, Expr::value(counts.new))
        .col_expr(
            poll_runs::Column::ResolvedCount,
            Expr::value(counts.resolved),
        )
        .col_expr(
            poll_runs::Column::FetchDurationMs,
            Expr::value(counts.fetch_duration_ms),
        )
        .col_expr(
            poll_runs::Column::OutcomeKnownId,
            Expr::value(counts.outcome_known_id),
        )
        .col_expr(
            poll_runs::Column::OutcomeForeignId,
            Expr::value(counts.outcome_foreign_id),
        )
        .col_expr(
            poll_runs::Column::OutcomeFuzzy,
            Expr::value(counts.outcome_fuzzy),
        )
        .col_expr(
            poll_runs::Column::OutcomeReview,
            Expr::value(counts.outcome_review),
        )
        .col_expr(
            poll_runs::Column::OutcomeFailed,
            Expr::value(counts.outcome_failed),
        )
        .col_expr(
            poll_runs::Column::ErrorMessage,
            Expr::value(error_message.map(str::to_string)),
        )
        .col_expr(
            poll_runs::Column::ErrorKind,
            Expr::value(error_kind.map(str::to_string)),
        )
        .exec(db)
        .await?;
    Ok(())
}

pub async fn start_provider_refresh<C: ConnectionTrait>(
    db: &C,
    provider_id: &str,
    started_at: i64,
    trigger: &str,
) -> Result<i64> {
    let model = provider_refreshes::ActiveModel {
        provider_id: Set(provider_id.to_string()),
        started_at: Set(started_at),
        finished_at: Set(None),
        status: Set(status::RUNNING.into()),
        bytes_downloaded: Set(None),
        record_count: Set(None),
        error_message: Set(None),
        trigger: Set(trigger.to_string()),
        ..Default::default()
    };
    let res = provider_refreshes::Entity::insert(model).exec(db).await?;
    Ok(res.last_insert_id)
}

pub async fn finalize_provider_refresh<C: ConnectionTrait>(
    db: &C,
    id: i64,
    finished_at: i64,
    status: &str,
    counts: ProviderRefreshCounts,
    error_message: Option<&str>,
    error_kind: Option<&str>,
) -> Result<()> {
    provider_refreshes::Entity::update_many()
        .filter(provider_refreshes::Column::Id.eq(id))
        .col_expr(
            provider_refreshes::Column::FinishedAt,
            Expr::value(finished_at),
        )
        .col_expr(provider_refreshes::Column::Status, Expr::value(status))
        .col_expr(
            provider_refreshes::Column::BytesDownloaded,
            Expr::value(counts.bytes_downloaded),
        )
        .col_expr(
            provider_refreshes::Column::RecordCount,
            Expr::value(counts.record_count),
        )
        .col_expr(
            provider_refreshes::Column::FetchDurationMs,
            Expr::value(counts.fetch_duration_ms),
        )
        .col_expr(
            provider_refreshes::Column::ErrorMessage,
            Expr::value(error_message.map(str::to_string)),
        )
        .col_expr(
            provider_refreshes::Column::ErrorKind,
            Expr::value(error_kind.map(str::to_string)),
        )
        .exec(db)
        .await?;
    Ok(())
}

pub async fn start_series_refresh_run<C: ConnectionTrait>(
    db: &C,
    provider_id: &str,
    started_at: i64,
    trigger: &str,
) -> Result<i64> {
    let model = series_refresh_runs::ActiveModel {
        provider_id: Set(provider_id.to_string()),
        started_at: Set(started_at),
        finished_at: Set(None),
        status: Set(status::RUNNING.into()),
        trigger: Set(trigger.to_string()),
        considered_count: Set(None),
        refreshed_count: Set(None),
        unchanged_count: Set(None),
        not_found_count: Set(None),
        errored_count: Set(None),
        fetch_duration_ms: Set(None),
        error_message: Set(None),
        error_kind: Set(None),
        ..Default::default()
    };
    let res = series_refresh_runs::Entity::insert(model).exec(db).await?;
    Ok(res.last_insert_id)
}

pub async fn finalize_series_refresh_run<C: ConnectionTrait>(
    db: &C,
    id: i64,
    finished_at: i64,
    status: &str,
    counts: SeriesRefreshCounts,
    error_message: Option<&str>,
    error_kind: Option<&str>,
) -> Result<()> {
    series_refresh_runs::Entity::update_many()
        .filter(series_refresh_runs::Column::Id.eq(id))
        .col_expr(
            series_refresh_runs::Column::FinishedAt,
            Expr::value(finished_at),
        )
        .col_expr(series_refresh_runs::Column::Status, Expr::value(status))
        .col_expr(
            series_refresh_runs::Column::ConsideredCount,
            Expr::value(counts.considered),
        )
        .col_expr(
            series_refresh_runs::Column::RefreshedCount,
            Expr::value(counts.refreshed),
        )
        .col_expr(
            series_refresh_runs::Column::UnchangedCount,
            Expr::value(counts.unchanged),
        )
        .col_expr(
            series_refresh_runs::Column::NotFoundCount,
            Expr::value(counts.not_found),
        )
        .col_expr(
            series_refresh_runs::Column::ErroredCount,
            Expr::value(counts.errored),
        )
        .col_expr(
            series_refresh_runs::Column::FetchDurationMs,
            Expr::value(counts.fetch_duration_ms),
        )
        .col_expr(
            series_refresh_runs::Column::ErrorMessage,
            Expr::value(error_message.map(str::to_string)),
        )
        .col_expr(
            series_refresh_runs::Column::ErrorKind,
            Expr::value(error_kind.map(str::to_string)),
        )
        .exec(db)
        .await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, FromQueryResult)]
#[serde(rename_all = "camelCase")]
pub struct SourceSummaryRow {
    pub source_name: String,
    pub total_runs: i64,
    pub success_count: i64,
    pub failure_count: i64,
    pub skipped_count: i64,
    pub fetched_sum: Option<i64>,
    pub new_sum: Option<i64>,
    pub resolved_sum: Option<i64>,
    pub outcome_known_id_sum: Option<i64>,
    pub outcome_foreign_id_sum: Option<i64>,
    pub outcome_fuzzy_sum: Option<i64>,
    pub outcome_review_sum: Option<i64>,
    pub outcome_failed_sum: Option<i64>,
    pub last_started_at: Option<i64>,
    pub last_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromQueryResult)]
#[serde(rename_all = "camelCase")]
pub struct ErrorKindRow {
    pub error_kind: Option<String>,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, FromQueryResult)]
#[serde(rename_all = "camelCase")]
pub struct FetchLatencyRow {
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub max_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, FromQueryResult)]
#[serde(rename_all = "camelCase")]
pub struct TimeToResolutionRow {
    pub p50_seconds: Option<f64>,
    pub p95_seconds: Option<f64>,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, FromQueryResult)]
#[serde(rename_all = "camelCase")]
pub struct SourceBucketRow {
    pub bucket_start: i64,
    pub success_count: i64,
    pub failure_count: i64,
    pub skipped_count: i64,
    pub fetched_sum: Option<i64>,
    pub new_sum: Option<i64>,
}

#[derive(Debug, Clone, Serialize, FromQueryResult)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRefreshSummaryRow {
    pub provider_id: String,
    pub total_runs: i64,
    pub success_count: i64,
    pub failure_count: i64,
    pub skipped_count: i64,
    pub bytes_sum: Option<i64>,
    pub last_started_at: Option<i64>,
    pub last_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromQueryResult)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRefreshBucketRow {
    pub bucket_start: i64,
    pub success_count: i64,
    pub failure_count: i64,
    pub skipped_count: i64,
}

/// Per-source aggregates over `[since, until)`. Returns one row per source
/// the operator has data for.
pub async fn source_summary(
    db: &DatabaseConnection,
    since: i64,
    until: i64,
) -> Result<Vec<SourceSummaryRow>> {
    // Window function (SQLite 3.25+) lets us aggregate counts and pick the
    // most-recent row's status in one pass per source. Outer MAX is fine on
    // started_at; last_status is reduced via MAX over a "first row wins"
    // CASE, since rn=1 occurs exactly once per group.
    let sql = "WITH ranked AS (
            SELECT
                source_name,
                status,
                started_at,
                fetched_count,
                new_count,
                resolved_count,
                outcome_known_id,
                outcome_foreign_id,
                outcome_fuzzy,
                outcome_review,
                outcome_failed,
                ROW_NUMBER() OVER (
                    PARTITION BY source_name ORDER BY started_at DESC
                ) AS rn
            FROM poll_runs
            WHERE started_at >= ?1 AND started_at < ?2
        )
        SELECT
            source_name AS source_name,
            COUNT(*) AS total_runs,
            SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS success_count,
            SUM(CASE WHEN status = 'failure' THEN 1 ELSE 0 END) AS failure_count,
            SUM(CASE WHEN status = 'skipped' THEN 1 ELSE 0 END) AS skipped_count,
            SUM(COALESCE(fetched_count, 0)) AS fetched_sum,
            SUM(COALESCE(new_count, 0)) AS new_sum,
            SUM(COALESCE(resolved_count, 0)) AS resolved_sum,
            SUM(COALESCE(outcome_known_id, 0)) AS outcome_known_id_sum,
            SUM(COALESCE(outcome_foreign_id, 0)) AS outcome_foreign_id_sum,
            SUM(COALESCE(outcome_fuzzy, 0)) AS outcome_fuzzy_sum,
            SUM(COALESCE(outcome_review, 0)) AS outcome_review_sum,
            SUM(COALESCE(outcome_failed, 0)) AS outcome_failed_sum,
            MAX(started_at) AS last_started_at,
            MAX(CASE WHEN rn = 1 THEN status END) AS last_status
        FROM ranked
        GROUP BY source_name
        ORDER BY source_name ASC";
    let stmt = Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        [since.into(), until.into()],
    );
    Ok(SourceSummaryRow::find_by_statement(stmt).all(db).await?)
}

/// Per-bucket run counts for one source. `bucket_seconds` must divide the
/// `[since, until)` window cleanly to produce a stable bar chart.
pub async fn source_buckets(
    db: &DatabaseConnection,
    source_name: &str,
    since: i64,
    until: i64,
    bucket_seconds: i64,
) -> Result<Vec<SourceBucketRow>> {
    let bucket = bucket_seconds.max(1);
    let sql = "SELECT
            ((started_at - ?2) / ?4) * ?4 + ?2 AS bucket_start,
            SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS success_count,
            SUM(CASE WHEN status = 'failure' THEN 1 ELSE 0 END) AS failure_count,
            SUM(CASE WHEN status = 'skipped' THEN 1 ELSE 0 END) AS skipped_count,
            SUM(COALESCE(fetched_count, 0)) AS fetched_sum,
            SUM(COALESCE(new_count, 0)) AS new_sum
        FROM poll_runs
        WHERE source_name = ?1
          AND started_at >= ?2 AND started_at < ?3
        GROUP BY bucket_start
        ORDER BY bucket_start ASC";
    let stmt = Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        [
            source_name.into(),
            since.into(),
            until.into(),
            bucket.into(),
        ],
    );
    Ok(SourceBucketRow::find_by_statement(stmt).all(db).await?)
}

pub async fn provider_refresh_summary(
    db: &DatabaseConnection,
    since: i64,
    until: i64,
) -> Result<Vec<ProviderRefreshSummaryRow>> {
    let sql = "WITH ranked AS (
            SELECT
                provider_id,
                status,
                started_at,
                bytes_downloaded,
                ROW_NUMBER() OVER (
                    PARTITION BY provider_id ORDER BY started_at DESC
                ) AS rn
            FROM provider_refreshes
            WHERE started_at >= ?1 AND started_at < ?2
        )
        SELECT
            provider_id AS provider_id,
            COUNT(*) AS total_runs,
            SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS success_count,
            SUM(CASE WHEN status = 'failure' THEN 1 ELSE 0 END) AS failure_count,
            SUM(CASE WHEN status = 'skipped' THEN 1 ELSE 0 END) AS skipped_count,
            SUM(COALESCE(bytes_downloaded, 0)) AS bytes_sum,
            MAX(started_at) AS last_started_at,
            MAX(CASE WHEN rn = 1 THEN status END) AS last_status
        FROM ranked
        GROUP BY provider_id
        ORDER BY provider_id ASC";
    let stmt = Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        [since.into(), until.into()],
    );
    Ok(ProviderRefreshSummaryRow::find_by_statement(stmt)
        .all(db)
        .await?)
}

pub async fn provider_refresh_buckets(
    db: &DatabaseConnection,
    provider_id: &str,
    since: i64,
    until: i64,
    bucket_seconds: i64,
) -> Result<Vec<ProviderRefreshBucketRow>> {
    let bucket = bucket_seconds.max(1);
    let sql = "SELECT
            ((started_at - ?2) / ?4) * ?4 + ?2 AS bucket_start,
            SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS success_count,
            SUM(CASE WHEN status = 'failure' THEN 1 ELSE 0 END) AS failure_count,
            SUM(CASE WHEN status = 'skipped' THEN 1 ELSE 0 END) AS skipped_count
        FROM provider_refreshes
        WHERE provider_id = ?1
          AND started_at >= ?2 AND started_at < ?3
        GROUP BY bucket_start
        ORDER BY bucket_start ASC";
    let stmt = Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        [
            provider_id.into(),
            since.into(),
            until.into(),
            bucket.into(),
        ],
    );
    Ok(ProviderRefreshBucketRow::find_by_statement(stmt)
        .all(db)
        .await?)
}

/// Most recent `limit` poll runs for one source, ordered newest-first.
/// Used by the admin UI's "recent runs" list.
pub async fn recent_source_runs(
    db: &DatabaseConnection,
    source_name: &str,
    limit: u64,
) -> Result<Vec<poll_runs::Model>> {
    Ok(poll_runs::Entity::find()
        .filter(poll_runs::Column::SourceName.eq(source_name))
        .order_by_desc(poll_runs::Column::StartedAt)
        .limit(limit)
        .all(db)
        .await?)
}

/// Histogram of failure causes for `source_name` over the window.
pub async fn source_error_kinds(
    db: &DatabaseConnection,
    source_name: &str,
    since: i64,
    until: i64,
) -> Result<Vec<ErrorKindRow>> {
    let sql = "SELECT
            error_kind AS error_kind,
            COUNT(*) AS count
        FROM poll_runs
        WHERE source_name = ?1
          AND started_at >= ?2 AND started_at < ?3
          AND status = 'failure'
        GROUP BY error_kind
        ORDER BY count DESC";
    let stmt = Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        [source_name.into(), since.into(), until.into()],
    );
    Ok(ErrorKindRow::find_by_statement(stmt).all(db).await?)
}

/// P50/P95 of `fetch_duration_ms` for `source_name`. Uses SQLite's
/// `PERCENTILE_CONT`-equivalent via `NTILE`-rank arithmetic (SQLite lacks a
/// built-in percentile aggregate, so we approximate via sorted positions in
/// a window function CTE — cheap for the row counts the admin view returns).
pub async fn source_fetch_latency(
    db: &DatabaseConnection,
    source_name: &str,
    since: i64,
    until: i64,
) -> Result<FetchLatencyRow> {
    let sql = "WITH samples AS (
            SELECT fetch_duration_ms AS ms,
                   ROW_NUMBER() OVER (ORDER BY fetch_duration_ms ASC) AS rn,
                   COUNT(*) OVER () AS total
            FROM poll_runs
            WHERE source_name = ?1
              AND started_at >= ?2 AND started_at < ?3
              AND fetch_duration_ms IS NOT NULL
        )
        SELECT
            (SELECT CAST(ms AS REAL) FROM samples WHERE rn = CAST(0.5 * total + 0.5 AS INTEGER) LIMIT 1) AS p50_ms,
            (SELECT CAST(ms AS REAL) FROM samples WHERE rn = CAST(0.95 * total + 0.5 AS INTEGER) LIMIT 1) AS p95_ms,
            (SELECT MAX(ms) FROM samples) AS max_ms";
    let stmt = Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        [source_name.into(), since.into(), until.into()],
    );
    Ok(FetchLatencyRow::find_by_statement(stmt)
        .one(db)
        .await?
        .unwrap_or(FetchLatencyRow {
            p50_ms: None,
            p95_ms: None,
            max_ms: None,
        }))
}

/// P50/P95 time-to-resolution (in seconds) for releases that originated
/// from `source_name` and got resolved during `[since, until)`.
///
/// `releases.resolved_at` is the anchor; we subtract `observed_at` (when
/// the discovery layer first saw the release) to get latency. The query
/// filters on `resolved_at` so only releases that closed in the window
/// contribute — long-pending unresolved rows are excluded by design.
pub async fn source_time_to_resolution(
    db: &DatabaseConnection,
    source_name: &str,
    since: i64,
    until: i64,
) -> Result<TimeToResolutionRow> {
    let sql = "WITH samples AS (
            SELECT (resolved_at - observed_at) AS ttr,
                   ROW_NUMBER() OVER (ORDER BY (resolved_at - observed_at) ASC) AS rn,
                   COUNT(*) OVER () AS total
            FROM releases
            WHERE source_name = ?1
              AND resolved_at IS NOT NULL
              AND resolved_at >= ?2 AND resolved_at < ?3
        )
        SELECT
            (SELECT CAST(ttr AS REAL) FROM samples WHERE rn = CAST(0.5 * total + 0.5 AS INTEGER) LIMIT 1) AS p50_seconds,
            (SELECT CAST(ttr AS REAL) FROM samples WHERE rn = CAST(0.95 * total + 0.5 AS INTEGER) LIMIT 1) AS p95_seconds,
            (SELECT COUNT(*) FROM samples) AS count";
    let stmt = Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        [source_name.into(), since.into(), until.into()],
    );
    Ok(TimeToResolutionRow::find_by_statement(stmt)
        .one(db)
        .await?
        .unwrap_or(TimeToResolutionRow {
            p50_seconds: None,
            p95_seconds: None,
            count: 0,
        }))
}

/// Aggregate of `provider_refreshes.bytes_downloaded` over the window
/// per provider, useful for the provider metrics card.
pub async fn provider_fetch_latency(
    db: &DatabaseConnection,
    provider_id: &str,
    since: i64,
    until: i64,
) -> Result<FetchLatencyRow> {
    let sql = "WITH samples AS (
            SELECT fetch_duration_ms AS ms,
                   ROW_NUMBER() OVER (ORDER BY fetch_duration_ms ASC) AS rn,
                   COUNT(*) OVER () AS total
            FROM provider_refreshes
            WHERE provider_id = ?1
              AND started_at >= ?2 AND started_at < ?3
              AND fetch_duration_ms IS NOT NULL
        )
        SELECT
            (SELECT CAST(ms AS REAL) FROM samples WHERE rn = CAST(0.5 * total + 0.5 AS INTEGER) LIMIT 1) AS p50_ms,
            (SELECT CAST(ms AS REAL) FROM samples WHERE rn = CAST(0.95 * total + 0.5 AS INTEGER) LIMIT 1) AS p95_ms,
            (SELECT MAX(ms) FROM samples) AS max_ms";
    let stmt = Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        [provider_id.into(), since.into(), until.into()],
    );
    Ok(FetchLatencyRow::find_by_statement(stmt)
        .one(db)
        .await?
        .unwrap_or(FetchLatencyRow {
            p50_ms: None,
            p95_ms: None,
            max_ms: None,
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::Database;

    async fn fresh_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db
    }

    #[tokio::test]
    async fn start_then_finalize_marks_status_and_counts() {
        let db = fresh_db().await;
        let id = start_poll_run(&db, "feed-a", "nyaa", 100, trigger::CRON)
            .await
            .unwrap();
        finalize_poll_run(
            &db,
            id,
            120,
            status::SUCCESS,
            PollRunCounts {
                fetched: Some(5),
                new: Some(3),
                resolved: Some(2),
                ..Default::default()
            },
            None,
            None,
        )
        .await
        .unwrap();

        let row = poll_runs::Entity::find_by_id(id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, "success");
        assert_eq!(row.finished_at, Some(120));
        assert_eq!(row.fetched_count, Some(5));
        assert_eq!(row.new_count, Some(3));
        assert_eq!(row.resolved_count, Some(2));
        assert_eq!(row.error_message, None);
    }

    #[tokio::test]
    async fn failure_finalize_captures_error_message() {
        let db = fresh_db().await;
        let id = start_poll_run(&db, "feed-b", "nyaa", 200, trigger::MANUAL)
            .await
            .unwrap();
        finalize_poll_run(
            &db,
            id,
            201,
            status::FAILURE,
            PollRunCounts {
                fetched: None,
                new: None,
                resolved: None,
                ..Default::default()
            },
            Some("boom"),
            None,
        )
        .await
        .unwrap();
        let row = poll_runs::Entity::find_by_id(id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, "failure");
        assert_eq!(row.error_message.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn source_summary_aggregates_within_window() {
        let db = fresh_db().await;
        // 3 success, 1 failure for "feed-a"; 1 success for "feed-b". One
        // older "feed-a" row outside the window must not be counted.
        for (name, started, status, fetched) in [
            ("feed-a", 100_i64, "success", Some(5_i32)),
            ("feed-a", 110, "success", Some(2)),
            ("feed-a", 120, "failure", None),
            ("feed-a", 130, "success", Some(7)),
            ("feed-b", 140, "success", Some(1)),
            ("feed-a", 10, "success", Some(99)), // out of window
        ] {
            let id = start_poll_run(&db, name, "nyaa", started, trigger::CRON)
                .await
                .unwrap();
            finalize_poll_run(
                &db,
                id,
                started + 1,
                status,
                PollRunCounts {
                    fetched,
                    new: fetched,
                    resolved: fetched,
                    ..Default::default()
                },
                None,
                None,
            )
            .await
            .unwrap();
        }
        let rows = source_summary(&db, 100, 200).await.unwrap();
        let feed_a = rows.iter().find(|r| r.source_name == "feed-a").unwrap();
        assert_eq!(feed_a.total_runs, 4);
        assert_eq!(feed_a.success_count, 3);
        assert_eq!(feed_a.failure_count, 1);
        assert_eq!(feed_a.fetched_sum, Some(14));
        assert_eq!(feed_a.last_started_at, Some(130));
        assert_eq!(feed_a.last_status.as_deref(), Some("success"));
    }

    #[tokio::test]
    async fn source_buckets_groups_by_window() {
        let db = fresh_db().await;
        // Window [0, 60), bucket=20. Three rows at 5, 25, 45.
        for (idx, started, status) in [
            (1, 5_i64, "success"),
            (2, 25, "failure"),
            (3, 45, "success"),
        ] {
            let id = start_poll_run(&db, "feed-a", "nyaa", started, trigger::CRON)
                .await
                .unwrap();
            finalize_poll_run(
                &db,
                id,
                started + 1,
                status,
                PollRunCounts {
                    fetched: Some(idx),
                    new: Some(idx),
                    resolved: Some(idx),
                    ..Default::default()
                },
                None,
                None,
            )
            .await
            .unwrap();
        }
        let buckets = source_buckets(&db, "feed-a", 0, 60, 20).await.unwrap();
        assert_eq!(buckets.len(), 3);
        assert_eq!(buckets[0].bucket_start, 0);
        assert_eq!(buckets[0].success_count, 1);
        assert_eq!(buckets[1].bucket_start, 20);
        assert_eq!(buckets[1].failure_count, 1);
        assert_eq!(buckets[2].bucket_start, 40);
    }

    #[tokio::test]
    async fn recent_source_runs_returns_newest_first() {
        let db = fresh_db().await;
        for started in [10_i64, 30, 20] {
            let id = start_poll_run(&db, "feed-a", "nyaa", started, trigger::CRON)
                .await
                .unwrap();
            finalize_poll_run(
                &db,
                id,
                started + 1,
                status::SUCCESS,
                PollRunCounts {
                    fetched: Some(1),
                    new: Some(1),
                    resolved: Some(1),
                    ..Default::default()
                },
                None,
                None,
            )
            .await
            .unwrap();
        }
        let rows = recent_source_runs(&db, "feed-a", 5).await.unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows[0].started_at >= rows[1].started_at);
        assert!(rows[1].started_at >= rows[2].started_at);
    }

    #[tokio::test]
    async fn series_refresh_run_round_trips_through_start_and_finalize() {
        let db = fresh_db().await;
        let id = start_series_refresh_run(&db, "mangabaka", 100, trigger::CRON)
            .await
            .unwrap();
        finalize_series_refresh_run(
            &db,
            id,
            150,
            status::SUCCESS,
            SeriesRefreshCounts {
                considered: Some(50),
                refreshed: Some(40),
                unchanged: Some(9),
                not_found: Some(1),
                errored: Some(0),
                fetch_duration_ms: Some(8_400),
            },
            None,
            None,
        )
        .await
        .unwrap();

        let row = series_refresh_runs::Entity::find_by_id(id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, "success");
        assert_eq!(row.finished_at, Some(150));
        assert_eq!(row.considered_count, Some(50));
        assert_eq!(row.refreshed_count, Some(40));
        assert_eq!(row.unchanged_count, Some(9));
        assert_eq!(row.not_found_count, Some(1));
        assert_eq!(row.errored_count, Some(0));
        assert_eq!(row.fetch_duration_ms, Some(8_400));
        assert_eq!(row.trigger, "cron");
    }

    #[tokio::test]
    async fn series_refresh_run_failure_captures_error_fields() {
        let db = fresh_db().await;
        let id = start_series_refresh_run(&db, "mangabaka", 200, trigger::MANUAL)
            .await
            .unwrap();
        finalize_series_refresh_run(
            &db,
            id,
            205,
            status::FAILURE,
            SeriesRefreshCounts {
                considered: Some(50),
                errored: Some(1),
                ..Default::default()
            },
            Some("provider timeout"),
            Some("network"),
        )
        .await
        .unwrap();
        let row = series_refresh_runs::Entity::find_by_id(id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, "failure");
        assert_eq!(row.error_message.as_deref(), Some("provider timeout"));
        assert_eq!(row.error_kind.as_deref(), Some("network"));
        assert_eq!(row.errored_count, Some(1));
    }

    #[tokio::test]
    async fn provider_refresh_paths_record_bytes_and_records() {
        let db = fresh_db().await;
        let id = start_provider_refresh(&db, "mangabaka", 100, trigger::CRON)
            .await
            .unwrap();
        finalize_provider_refresh(
            &db,
            id,
            105,
            status::SUCCESS,
            ProviderRefreshCounts {
                bytes_downloaded: Some(1024 * 1024),
                record_count: Some(585_000),
                ..Default::default()
            },
            None,
            None,
        )
        .await
        .unwrap();
        let rows = provider_refresh_summary(&db, 0, i64::MAX).await.unwrap();
        let mb = rows.iter().find(|r| r.provider_id == "mangabaka").unwrap();
        assert_eq!(mb.total_runs, 1);
        assert_eq!(mb.success_count, 1);
        assert_eq!(mb.bytes_sum, Some(1024 * 1024));
        assert_eq!(mb.last_status.as_deref(), Some("success"));
    }
}
