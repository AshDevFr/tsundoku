//! Append-only log of metadata-provider cache refreshes.
//!
//! Each `refresh_cache()` call from a provider that maintains an offline
//! cache writes one row here. The most-recent row per provider answers
//! "when did we last refresh this provider?" for the API / UI.

use anyhow::Result;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
};

use crate::entities::provider_cache_state;

pub use provider_cache_state::Model;

pub async fn append(
    db: &DatabaseConnection,
    provider: &str,
    fetched_at: i64,
    cache_version: Option<&str>,
    record_count: Option<i64>,
    source_url: Option<&str>,
    bytes_downloaded: Option<i64>,
) -> Result<Model> {
    let row = provider_cache_state::ActiveModel {
        provider: Set(provider.to_string()),
        fetched_at: Set(fetched_at),
        cache_version: Set(cache_version.map(str::to_string)),
        record_count: Set(record_count),
        source_url: Set(source_url.map(str::to_string)),
        bytes_downloaded: Set(bytes_downloaded),
        ..Default::default()
    };
    Ok(row.insert(db).await?)
}

pub async fn latest(db: &DatabaseConnection, provider: &str) -> Result<Option<Model>> {
    Ok(provider_cache_state::Entity::find()
        .filter(provider_cache_state::Column::Provider.eq(provider))
        .order_by_desc(provider_cache_state::Column::FetchedAt)
        .one(db)
        .await?)
}

pub use provider_cache_state::{ActiveModel, Column, Entity};
