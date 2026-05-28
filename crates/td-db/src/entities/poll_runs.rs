use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "poll_runs")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub source_name: String,
    pub source_kind: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub status: String,
    pub fetched_count: Option<i32>,
    pub new_count: Option<i32>,
    pub resolved_count: Option<i32>,
    pub error_message: Option<String>,
    #[sea_orm(column_name = "trigger")]
    pub trigger: String,
    /// Wall-clock time spent inside `DiscoverySource::poll()` only, in
    /// milliseconds. `None` when the tick was skipped or the call hadn't
    /// returned before failure.
    pub fetch_duration_ms: Option<i64>,
    /// Coarse classification of the failure cause; see
    /// `td_scheduler::error_kind`.
    pub error_kind: Option<String>,
    pub outcome_known_id: Option<i32>,
    pub outcome_foreign_id: Option<i32>,
    pub outcome_fuzzy: Option<i32>,
    pub outcome_review: Option<i32>,
    pub outcome_failed: Option<i32>,
    /// Live-progress fields, written by `ProgressHandle` during the loop
    /// (throttled) and frozen at job end. `NULL` for jobs that don't
    /// report progress and on pre-migration rows.
    pub progress_current: Option<i64>,
    pub progress_total: Option<i64>,
    pub progress_phase: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
