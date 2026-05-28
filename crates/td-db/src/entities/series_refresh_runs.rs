use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "series_refresh_runs")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub provider_id: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub status: String,
    #[sea_orm(column_name = "trigger")]
    pub trigger: String,
    /// Size of the batch the selection query returned at the start of the
    /// tick. `refreshed + unchanged + not_found + errored` <= this value;
    /// an early provider-error abort leaves the difference unaccounted for.
    pub considered_count: Option<i32>,
    /// Series whose row got an actual UPDATE because the new payload
    /// differed.
    pub refreshed_count: Option<i32>,
    /// Hash-matched, no UPDATE needed.
    pub unchanged_count: Option<i32>,
    /// `MetadataProvider::get` returned `Ok(None)`; we bumped
    /// `metadata_fetched_at` so the row rotates out of the next batch.
    pub not_found_count: Option<i32>,
    /// `MetadataProvider::get` returned `Err` for this row. Currently the
    /// tick aborts the batch on the first error, so this counts the row
    /// that triggered the abort.
    pub errored_count: Option<i32>,
    /// Total wall-clock time spent in `MetadataProvider::get()` calls
    /// over the tick, in milliseconds. Per-call latency isn't broken out.
    pub fetch_duration_ms: Option<i64>,
    pub error_message: Option<String>,
    pub error_kind: Option<String>,
    /// Live-progress fields. `progress_total` is typically the batch size
    /// the selection query returned; `progress_current` advances per row.
    pub progress_current: Option<i64>,
    pub progress_total: Option<i64>,
    pub progress_phase: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
