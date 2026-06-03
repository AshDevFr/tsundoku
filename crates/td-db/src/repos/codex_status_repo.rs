//! Single-row Codex connection-health record (`codex_status`, `id = 1`).
//!
//! Each setter upserts the singleton and rewrites only the columns it owns, so
//! a preflight update doesn't stomp the last-success timestamp and vice versa.
//! The pool is pinned to one connection, so the read-free upsert pattern is
//! race-free here.

use anyhow::Result;
use sea_orm::sea_query::OnConflict;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, QueryOrder, QuerySelect, Set};

use crate::entities::{codex_health_checks, codex_status};

pub use codex_status::{ActiveModel, Column, Entity, Model};

use super::TRIGGER_MANUAL;

/// The fixed primary key of the singleton row.
const ROW_ID: i32 = 1;

/// `auth_state` discriminants.
pub const AUTH_UNKNOWN: &str = "unknown";
pub const AUTH_OK: &str = "ok";
pub const AUTH_UNAUTHORIZED: &str = "unauthorized";
pub const AUTH_FORBIDDEN: &str = "forbidden";

/// The current status row, or `None` if no tick has ever recorded one.
pub async fn get(db: &DatabaseConnection) -> Result<Option<Model>> {
    Ok(Entity::find_by_id(ROW_ID).one(db).await?)
}

/// Record the outcome of an `/info` preflight: reachability plus the Codex
/// name/version (when reachable). Leaves auth/sweep fields untouched.
pub async fn set_preflight(
    db: &DatabaseConnection,
    reachable: bool,
    codex_name: Option<&str>,
    codex_version: Option<&str>,
    at: i64,
) -> Result<()> {
    let model = ActiveModel {
        id: Set(ROW_ID),
        reachable: Set(reachable),
        codex_name: Set(codex_name.map(str::to_string)),
        codex_version: Set(codex_version.map(str::to_string)),
        last_preflight_at: Set(Some(at)),
        ..Default::default()
    };
    Entity::insert(model)
        .on_conflict(
            OnConflict::column(Column::Id)
                .update_columns([
                    Column::Reachable,
                    Column::CodexName,
                    Column::CodexVersion,
                    Column::LastPreflightAt,
                ])
                .to_owned(),
        )
        .exec(db)
        .await?;
    Ok(())
}

/// Record a failed preflight: Codex was unreachable (or `/info` returned a
/// non-200). Marks `reachable = false`, stamps `last_preflight_at`, and writes
/// `last_error`. Leaves `auth_state` and the last-success fields untouched —
/// an outage isn't an auth verdict, and stale links stay valid.
pub async fn set_preflight_unreachable(
    db: &DatabaseConnection,
    last_error: &str,
    at: i64,
) -> Result<()> {
    let model = ActiveModel {
        id: Set(ROW_ID),
        reachable: Set(false),
        last_preflight_at: Set(Some(at)),
        last_error: Set(Some(last_error.to_string())),
        ..Default::default()
    };
    Entity::insert(model)
        .on_conflict(
            OnConflict::column(Column::Id)
                .update_columns([
                    Column::Reachable,
                    Column::LastPreflightAt,
                    Column::LastError,
                ])
                .to_owned(),
        )
        .exec(db)
        .await?;
    Ok(())
}

/// Record the auth outcome of the first `external-index` page: `ok`,
/// `unauthorized` (401), or `forbidden` (403). `last_error` is a
/// human-readable message on failure, or `None` to clear it on success.
pub async fn set_auth_state(
    db: &DatabaseConnection,
    auth_state: &str,
    last_error: Option<&str>,
) -> Result<()> {
    let model = ActiveModel {
        id: Set(ROW_ID),
        auth_state: Set(auth_state.to_string()),
        last_error: Set(last_error.map(str::to_string)),
        ..Default::default()
    };
    Entity::insert(model)
        .on_conflict(
            OnConflict::column(Column::Id)
                .update_columns([Column::AuthState, Column::LastError])
                .to_owned(),
        )
        .exec(db)
        .await?;
    Ok(())
}

/// Record a non-auth sweep failure (transport error, 5xx, decode failure):
/// writes `last_error` only, leaving `auth_state`, reachability, and the
/// last-success fields untouched. 401/403 use [`set_auth_state`] instead.
pub async fn set_error(db: &DatabaseConnection, last_error: &str) -> Result<()> {
    let model = ActiveModel {
        id: Set(ROW_ID),
        last_error: Set(Some(last_error.to_string())),
        ..Default::default()
    };
    Entity::insert(model)
        .on_conflict(
            OnConflict::column(Column::Id)
                .update_columns([Column::LastError])
                .to_owned(),
        )
        .exec(db)
        .await?;
    Ok(())
}

/// Record a fully-successful sweep: stamps `last_success_at` (which drives the
/// UI's `codexSyncedAt` badge guard), the fetched + linked counts, clears
/// `last_error`, and marks auth `ok`.
pub async fn set_success(
    db: &DatabaseConnection,
    fetched_count: i64,
    linked_count: i64,
    at: i64,
) -> Result<()> {
    let model = ActiveModel {
        id: Set(ROW_ID),
        auth_state: Set(AUTH_OK.to_string()),
        last_success_at: Set(Some(at)),
        fetched_count: Set(Some(fetched_count)),
        linked_count: Set(Some(linked_count)),
        last_error: Set(None),
        ..Default::default()
    };
    Entity::insert(model)
        .on_conflict(
            OnConflict::column(Column::Id)
                .update_columns([
                    Column::AuthState,
                    Column::LastSuccessAt,
                    Column::FetchedCount,
                    Column::LinkedCount,
                    Column::LastError,
                ])
                .to_owned(),
        )
        .exec(db)
        .await?;
    Ok(())
}

/// Record a preflight outcome *and* maintain the reachability history. Wraps
/// [`set_preflight`] / [`set_preflight_unreachable`] for the snapshot, then
/// appends a `codex_health_checks` row when reachability transitions (or the
/// very first probe) — plus always on a manual trigger. Returns whether
/// reachability transitioned. Mirrors `download_status_repo::record_check`.
pub async fn record_preflight(
    db: &DatabaseConnection,
    reachable: bool,
    codex_name: Option<&str>,
    codex_version: Option<&str>,
    error: Option<&str>,
    at: i64,
    trigger: &str,
) -> Result<bool> {
    let previous = get(db).await?.map(|m| m.reachable);
    let transitioned = previous != Some(reachable);

    if reachable {
        set_preflight(db, true, codex_name, codex_version, at).await?;
    } else {
        set_preflight_unreachable(db, error.unwrap_or("unreachable"), at).await?;
    }

    if transitioned || trigger == TRIGGER_MANUAL {
        codex_health_checks::ActiveModel {
            checked_at: Set(at),
            reachable: Set(reachable),
            error: Set(error.map(str::to_string)),
            trigger: Set(trigger.to_string()),
            ..Default::default()
        }
        .insert(db)
        .await?;
    }

    Ok(transitioned)
}

/// The most recent reachability-history rows, newest first.
pub async fn list_recent_checks(
    db: &DatabaseConnection,
    limit: u64,
) -> Result<Vec<codex_health_checks::Model>> {
    Ok(codex_health_checks::Entity::find()
        .order_by_desc(codex_health_checks::Column::Id)
        .limit(limit)
        .all(db)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::super::{TRIGGER_CRON, TRIGGER_LAUNCH, TRIGGER_MANUAL};
    use super::*;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::Database;

    async fn fresh_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db
    }

    #[tokio::test]
    async fn get_is_none_before_any_write() {
        let db = fresh_db().await;
        assert!(get(&db).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn setters_target_disjoint_fields_on_the_singleton() {
        let db = fresh_db().await;

        set_preflight(&db, true, Some("codex"), Some("1.2.3"), 100)
            .await
            .unwrap();
        let row = get(&db).await.unwrap().unwrap();
        assert_eq!(row.id, ROW_ID);
        assert!(row.reachable);
        assert_eq!(row.codex_version.as_deref(), Some("1.2.3"));
        // Untouched fields keep their DB defaults.
        assert_eq!(row.auth_state, AUTH_UNKNOWN);
        assert!(row.last_success_at.is_none());

        // An auth failure updates only auth fields; preflight info survives.
        set_auth_state(&db, AUTH_UNAUTHORIZED, Some("api_key rejected (401)"))
            .await
            .unwrap();
        let row = get(&db).await.unwrap().unwrap();
        assert_eq!(row.auth_state, AUTH_UNAUTHORIZED);
        assert_eq!(row.last_error.as_deref(), Some("api_key rejected (401)"));
        assert!(row.reachable, "preflight reachability preserved");
        assert_eq!(row.codex_version.as_deref(), Some("1.2.3"));

        // A later success flips auth back to ok, clears the error, stamps the
        // sweep, and does not disturb the name/version.
        set_success(&db, 412, 42, 200).await.unwrap();
        let row = get(&db).await.unwrap().unwrap();
        assert_eq!(row.auth_state, AUTH_OK);
        assert!(row.last_error.is_none());
        assert_eq!(row.last_success_at, Some(200));
        assert_eq!(row.fetched_count, Some(412));
        assert_eq!(row.linked_count, Some(42));
        assert_eq!(row.codex_name.as_deref(), Some("codex"));

        // Still exactly one row.
        assert_eq!(Entity::find().all(&db).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn preflight_unreachable_marks_down_without_touching_auth_or_success() {
        let db = fresh_db().await;
        // Seed a prior good state.
        set_preflight(&db, true, Some("codex"), Some("1.0.0"), 100)
            .await
            .unwrap();
        set_success(&db, 20, 5, 150).await.unwrap();

        // Codex goes down: reachable flips, error recorded, but the last
        // successful sweep + auth verdict survive so stale links stay valid.
        set_preflight_unreachable(&db, "connection refused", 200)
            .await
            .unwrap();
        let row = get(&db).await.unwrap().unwrap();
        assert!(!row.reachable);
        assert_eq!(row.last_error.as_deref(), Some("connection refused"));
        assert_eq!(row.last_preflight_at, Some(200));
        assert_eq!(row.auth_state, AUTH_OK, "auth verdict preserved");
        assert_eq!(row.last_success_at, Some(150), "last success preserved");
        assert_eq!(row.linked_count, Some(5));
    }

    #[tokio::test]
    async fn record_preflight_keeps_history_on_transitions_and_manual() {
        let db = fresh_db().await;

        // First probe is a transition and updates the snapshot.
        assert!(
            record_preflight(
                &db,
                true,
                Some("codex"),
                Some("1.0.0"),
                None,
                100,
                TRIGGER_LAUNCH
            )
            .await
            .unwrap()
        );
        assert!(get(&db).await.unwrap().unwrap().reachable);
        assert_eq!(list_recent_checks(&db, 10).await.unwrap().len(), 1);

        // Steady-state cron probe: snapshot refreshes, no new history row.
        assert!(
            !record_preflight(
                &db,
                true,
                Some("codex"),
                Some("1.0.0"),
                None,
                200,
                TRIGGER_CRON
            )
            .await
            .unwrap()
        );
        assert_eq!(list_recent_checks(&db, 10).await.unwrap().len(), 1);

        // Manual probe always appends, even unchanged.
        assert!(
            !record_preflight(
                &db,
                true,
                Some("codex"),
                Some("1.0.0"),
                None,
                250,
                TRIGGER_MANUAL
            )
            .await
            .unwrap()
        );
        assert_eq!(list_recent_checks(&db, 10).await.unwrap().len(), 2);

        // A flip to unreachable appends and marks the snapshot down without
        // disturbing the preserved auth/success fields.
        assert!(
            record_preflight(&db, false, None, None, Some("refused"), 300, TRIGGER_CRON)
                .await
                .unwrap()
        );
        let recent = list_recent_checks(&db, 10).await.unwrap();
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].checked_at, 300);
        assert!(!recent[0].reachable);
        assert_eq!(recent[0].error.as_deref(), Some("refused"));
        assert!(!get(&db).await.unwrap().unwrap().reachable);
    }
}
