//! Review-candidate read/write helpers.

use anyhow::Result;
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};

use crate::entities::review_candidates;

pub use review_candidates::Model;

/// Replace the candidate set for a release. Any prior candidates not present
/// in `candidates` are removed; the new set is upserted in one transaction.
pub async fn replace_for_release(
    db: &DatabaseConnection,
    release_id: &str,
    candidates: Vec<review_candidates::ActiveModel>,
) -> Result<()> {
    use sea_orm::TransactionTrait;
    let txn = db.begin().await?;
    review_candidates::Entity::delete_many()
        .filter(review_candidates::Column::ReleaseId.eq(release_id))
        .exec(&txn)
        .await?;
    if !candidates.is_empty() {
        review_candidates::Entity::insert_many(candidates)
            .on_conflict(
                OnConflict::columns([
                    review_candidates::Column::ReleaseId,
                    review_candidates::Column::MangabakaId,
                ])
                .update_columns([
                    review_candidates::Column::Score,
                    review_candidates::Column::Reason,
                ])
                .to_owned(),
            )
            .exec(&txn)
            .await?;
    }
    txn.commit().await?;
    Ok(())
}

pub async fn list_for_release(db: &DatabaseConnection, release_id: &str) -> Result<Vec<Model>> {
    Ok(review_candidates::Entity::find()
        .filter(review_candidates::Column::ReleaseId.eq(release_id))
        .order_by_desc(review_candidates::Column::Score)
        .all(db)
        .await?)
}

pub use review_candidates::{ActiveModel, Column, Entity};
