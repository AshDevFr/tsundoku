use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for stmt in UP {
            db.execute_unprepared(stmt).await?;
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Data-only correction; the stale values it replaced are not worth
        // restoring.
        Ok(())
    }
}

// `last_release_at` is meant to be the newest linked release's upstream post
// date, but the series upsert used to fold `max(existing, <caller's value>)`
// into it, and several callers passed a time that was not a release: the
// metadata refresh job its previous fetch time, the refresh button and
// add-from-provider `now`. Every refresh with a changed hash therefore dragged
// the value forward, so long-dormant series looked recent and sorted near the
// top. It is now derived alongside coverage from linked releases only; this
// recomputes it for every existing row. 0 means nothing is linked.
const UP: &[&str] = &["UPDATE series SET last_release_at = COALESCE(
         (SELECT MAX(r.posted_at) FROM releases r WHERE r.series_id = series.id),
         0
     )"];
