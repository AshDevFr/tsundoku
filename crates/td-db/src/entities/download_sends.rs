use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Append-only audit of every send-to-client attempt, including failures (which
/// previously surfaced only as a transient 502 and were never recorded). The
/// denormalized "latest send" the badge reads still lives on the `releases` row
/// (`sent_to_client_at` / `sent_to_client_label`); this table is the log.
/// `source` is `torrent` | `magnet`.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "download_sends")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub release_id: String,
    pub sent_at: i64,
    pub label: Option<String>,
    pub source: String,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
