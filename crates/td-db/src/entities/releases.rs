use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "releases")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub source_kind: String,
    pub source_name: String,
    pub external_id: String,
    pub title: String,
    pub link: String,
    pub magnet: Option<String>,
    pub torrent_url: Option<String>,
    pub ddl_url: Option<String>,
    pub info_hash: Option<String>,
    pub size_bytes: Option<i64>,
    pub files_json: Option<String>,
    pub description_html: Option<String>,
    pub extracted_links_json: Option<String>,
    /// Provider links found in the post's comment section (serialized
    /// `ExternalLinks`). Untrusted — never fed to the resolver; the review UI
    /// surfaces them as operator-confirmable suggestions. Distinct from
    /// `extracted_links_json` (the uploader's links).
    pub comment_suggested_links_json: Option<String>,
    /// URL from the post's "Information" field, verbatim. Kept even when it
    /// is not a provider link we resolve against, so the review UI can show
    /// the uploader's cited source.
    pub information_url: Option<String>,
    pub posted_at: i64,
    pub observed_at: i64,
    pub series_id: Option<i32>,
    pub resolution_path: Option<String>,
    pub resolution_confidence: Option<f64>,
    pub resolution_status: String,
    pub resolution_attempts: i32,
    pub last_resolve_attempt_at: Option<i64>,
    pub volume_span_json: Option<String>,
    pub chapter_span_json: Option<String>,
    /// Set when `resolution_status` transitions to `resolved`. Anchors the
    /// time-to-resolution histogram surfaced on the admin metrics view.
    pub resolved_at: Option<i64>,
    /// JSON array of search queries the title cleaner produced (longest
    /// first). NULL on rows persisted before the cleaner shipped; the
    /// next resolve cycle backfills.
    pub search_queries: Option<String>,
    /// JSON array of stable rule names that fired during title cleanup.
    /// Surfaced as badge chips on the review card so the operator can
    /// audit what surgery happened.
    pub cleanup_rules_applied: Option<String>,
    /// Epoch seconds the release was pushed to the operator's torrent client,
    /// or `None` if never sent. Anchors the "Sent" badge.
    pub sent_to_client_at: Option<i64>,
    /// Label applied when the release was sent to the torrent client, or
    /// `None`. Records what was sent alongside `sent_to_client_at`.
    pub sent_to_client_label: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::series::Entity",
        from = "Column::SeriesId",
        to = "super::series::Column::Id"
    )]
    Series,
    #[sea_orm(has_many = "super::release_formats::Entity")]
    ReleaseFormats,
    #[sea_orm(has_many = "super::review_candidates::Entity")]
    ReviewCandidates,
}

impl Related<super::series::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Series.def()
    }
}

impl Related<super::release_formats::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::ReleaseFormats.def()
    }
}

impl Related<super::review_candidates::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::ReviewCandidates.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
