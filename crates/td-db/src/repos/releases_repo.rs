//! Release read/write helpers.

use anyhow::Result;
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};

use crate::entities::{release_formats, releases};

pub use releases::Model;

pub async fn upsert(db: &DatabaseConnection, model: releases::ActiveModel) -> Result<()> {
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

/// Idempotently attach a format tag to a release.
pub async fn add_format(db: &DatabaseConnection, release_id: &str, format: &str) -> Result<()> {
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

pub use releases::{ActiveModel, Column, Entity};
