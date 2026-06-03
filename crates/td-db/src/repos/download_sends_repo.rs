//! Append-only audit of send-to-client attempts (`download_sends`). Every send,
//! successful or failed, lands here; the badge-driving "latest send" columns
//! stay on the `releases` row.

use anyhow::Result;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, QueryOrder, QuerySelect, Set};

use crate::entities::download_sends;

pub use download_sends::{ActiveModel, Column, Entity, Model};

/// Record one send attempt. `source` is `torrent` | `magnet`; `success`
/// distinguishes a completed add from a client rejection (whose message is in
/// `error`).
#[allow(clippy::too_many_arguments)]
pub async fn insert(
    db: &DatabaseConnection,
    release_id: &str,
    sent_at: i64,
    label: Option<&str>,
    source: &str,
    success: bool,
    error: Option<&str>,
) -> Result<()> {
    ActiveModel {
        release_id: Set(release_id.to_string()),
        sent_at: Set(sent_at),
        label: Set(label.map(str::to_string)),
        source: Set(source.to_string()),
        success: Set(success),
        error: Set(error.map(str::to_string)),
        ..Default::default()
    }
    .insert(db)
    .await?;
    Ok(())
}

/// The most recent send attempts across all releases, newest first.
pub async fn list_recent(db: &DatabaseConnection, limit: u64) -> Result<Vec<Model>> {
    Ok(Entity::find()
        .order_by_desc(Column::Id)
        .limit(limit)
        .all(db)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::{ActiveModelTrait, Database, Set};

    async fn fresh_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db
    }

    /// `download_sends.release_id` is a FK to `releases(id)`, so a row must
    /// exist first. Set only the NOT NULL columns.
    async fn seed_release(db: &DatabaseConnection, id: &str) {
        crate::entities::releases::ActiveModel {
            id: Set(id.to_string()),
            source_kind: Set("nyaa".to_string()),
            source_name: Set("test".to_string()),
            external_id: Set(id.to_string()),
            title: Set("Test Release".to_string()),
            link: Set(format!("https://example/{id}")),
            posted_at: Set(1_700_000_000),
            observed_at: Set(1_700_000_100),
            resolution_status: Set("unresolved".to_string()),
            resolution_attempts: Set(0),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn records_success_and_failure_newest_first() {
        let db = fresh_db().await;
        seed_release(&db, "r1").await;

        insert(&db, "r1", 100, Some("manga"), "torrent", true, None)
            .await
            .unwrap();
        insert(
            &db,
            "r1",
            200,
            None,
            "magnet",
            false,
            Some("rejected: bad magnet"),
        )
        .await
        .unwrap();

        let recent = list_recent(&db, 10).await.unwrap();
        assert_eq!(recent.len(), 2);
        // Newest first.
        assert_eq!(recent[0].sent_at, 200);
        assert!(!recent[0].success);
        assert_eq!(recent[0].error.as_deref(), Some("rejected: bad magnet"));
        assert_eq!(recent[1].sent_at, 100);
        assert!(recent[1].success);
        assert_eq!(recent[1].label.as_deref(), Some("manga"));
    }
}
