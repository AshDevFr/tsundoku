//! Single-owner lock + spawn + event-lifecycle helper.
//!
//! Every job (cron-driven or manual-trigger) funnels through
//! [`try_dispatch`]. The dispatcher owns the per-key
//! [`tokio::sync::Mutex`] for the lifetime of the spawned task, so the
//! work body never tries to re-acquire it — eliminating the double-lock
//! bug where the inner `try_lock` would see the dispatcher's own guard
//! and skip the work.
//!
//! Lifecycle:
//! - **Uncontested:** emit `Started`, spawn the task holding the guard,
//!   await `work`, emit `Finished` with the returned [`JobResult`].
//! - **Contended:** emit a single `Finished { skipped: true }`, spawn
//!   `on_skipped` so the caller can record a "skipped" row in its
//!   metrics table without blocking the HTTP response or the cron tick.
//!
//! `on_skipped` runs detached. The cron path doesn't observe its
//! completion; the metrics row appears a few ms after the trigger
//! returns. That's fine — the row is operator-facing audit, not part of
//! the request/response contract.

use std::future::Future;
use std::sync::Arc;

use tokio::sync::{Mutex, broadcast};

use crate::events::{JobEvent, JobKind, JobResult};

/// Try to acquire `lock` and dispatch `work` under the held guard.
///
/// Returns `true` iff `work` was spawned. On contention, spawns
/// `on_skipped` (typically a "skipped" metrics-row writer) and returns
/// `false`. The lock is held for the lifetime of the `work` task, so
/// the work body must NOT try to acquire `lock` again — that would
/// deadlock against the dispatcher's own guard.
pub fn try_dispatch<F, Fut, S, Sfut>(
    events: &broadcast::Sender<JobEvent>,
    lock: Arc<Mutex<()>>,
    kind: JobKind,
    key: impl Into<String>,
    on_skipped: S,
    work: F,
) -> bool
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = JobResult> + Send + 'static,
    S: FnOnce() -> Sfut + Send + 'static,
    Sfut: Future<Output = ()> + Send + 'static,
{
    let key = key.into();
    let Ok(guard) = lock.try_lock_owned() else {
        let _ = events.send(JobEvent::finished(
            kind,
            key.clone(),
            JobResult {
                triggered: false,
                skipped: true,
                ..Default::default()
            },
        ));
        tokio::spawn(async move {
            on_skipped().await;
        });
        return false;
    };
    let _ = events.send(JobEvent::started(kind, key.clone()));
    let events_for_task = events.clone();
    tokio::spawn(async move {
        let _g = guard;
        let result = work().await;
        let _ = events_for_task.send(JobEvent::finished(kind, key, result));
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::sync::oneshot;
    use tokio::time::timeout;

    use crate::events::JobPhase;

    async fn collect_events(
        rx: &mut broadcast::Receiver<JobEvent>,
        expected: usize,
    ) -> Vec<JobEvent> {
        let mut out = Vec::with_capacity(expected);
        for i in 0..expected {
            match timeout(Duration::from_secs(2), rx.recv()).await {
                Ok(Ok(event)) => out.push(event),
                Ok(Err(e)) => panic!(
                    "broadcast channel closed while collecting event {}/{expected}: {e:?}; got so far: {out:?}",
                    i + 1
                ),
                Err(_) => panic!(
                    "timed out waiting for event {}/{expected}; got so far: {out:?}",
                    i + 1
                ),
            }
        }
        out
    }

    fn count_started(events: &[JobEvent]) -> usize {
        events
            .iter()
            .filter(|e| matches!(e.phase, JobPhase::Started))
            .count()
    }

    fn count_finished_with(events: &[JobEvent], skipped: bool) -> usize {
        events
            .iter()
            .filter(|e| matches!(e.phase, JobPhase::Finished))
            .filter(|e| {
                e.result
                    .as_ref()
                    .map(|r| r.skipped == skipped)
                    .unwrap_or(false)
            })
            .count()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn happy_path_emits_started_then_finished_with_work_result() {
        let (tx, mut rx) = broadcast::channel::<JobEvent>(16);
        let lock = Arc::new(Mutex::new(()));

        let triggered = try_dispatch(
            &tx,
            lock,
            JobKind::Provider,
            "mangabaka",
            || async {},
            || async {
                JobResult {
                    triggered: true,
                    skipped: false,
                    new: Some(42),
                    ..Default::default()
                }
            },
        );
        assert!(triggered, "uncontested dispatch should win the lock");

        let events = collect_events(&mut rx, 2).await;
        let started = events
            .iter()
            .find(|e| matches!(e.phase, JobPhase::Started))
            .expect("Started present");
        assert_eq!(started.id, "mangabaka");

        let finished = events
            .iter()
            .find(|e| matches!(e.phase, JobPhase::Finished))
            .expect("Finished present");
        let finished_result = finished.result.as_ref().expect("Finished carries result");
        assert!(finished_result.triggered);
        assert!(!finished_result.skipped);
        assert_eq!(finished_result.new, Some(42));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn contended_dispatch_emits_one_skipped_finished_and_runs_on_skipped() {
        let (tx, mut rx) = broadcast::channel::<JobEvent>(16);
        let lock = Arc::new(Mutex::new(()));

        let (lock_taken_tx, lock_taken_rx) = oneshot::channel::<()>();
        let (release_tx, release_rx) = oneshot::channel::<()>();
        let first = try_dispatch(
            &tx,
            lock.clone(),
            JobKind::Source,
            "nyaa",
            || async {},
            move || async move {
                let _ = lock_taken_tx.send(());
                let _ = release_rx.await;
                JobResult {
                    triggered: true,
                    skipped: false,
                    ..Default::default()
                }
            },
        );
        assert!(first, "first dispatch should win the lock");

        timeout(Duration::from_secs(1), lock_taken_rx)
            .await
            .expect("first work should reach its body within the test budget")
            .expect("first work should signal lock acquisition");

        let (skipped_called_tx, skipped_called_rx) = oneshot::channel::<()>();
        let second = try_dispatch(
            &tx,
            lock.clone(),
            JobKind::Source,
            "nyaa",
            move || async move {
                let _ = skipped_called_tx.send(());
            },
            || async {
                panic!("contended dispatch must not invoke work");
            },
        );
        assert!(!second, "second dispatch should report skipped");
        timeout(Duration::from_secs(1), skipped_called_rx)
            .await
            .expect("on_skipped should run within the test budget")
            .expect("on_skipped sender should signal");

        let _ = release_tx.send(());

        let events = collect_events(&mut rx, 3).await;
        assert_eq!(
            count_started(&events),
            1,
            "exactly one Started expected; got {events:?}"
        );
        assert_eq!(
            count_finished_with(&events, true),
            1,
            "exactly one Finished{{skipped:true}} expected; got {events:?}"
        );
        assert_eq!(
            count_finished_with(&events, false),
            1,
            "exactly one Finished{{skipped:false}} expected; got {events:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn distinct_locks_dispatch_independently() {
        let (tx, mut rx) = broadcast::channel::<JobEvent>(16);
        let lock_a = Arc::new(Mutex::new(()));
        let lock_b = Arc::new(Mutex::new(()));

        let a = try_dispatch(
            &tx,
            lock_a,
            JobKind::Source,
            "nyaa",
            || async {},
            || async {
                JobResult {
                    triggered: true,
                    skipped: false,
                    ..Default::default()
                }
            },
        );
        let b = try_dispatch(
            &tx,
            lock_b,
            JobKind::Source,
            "subsplease",
            || async {},
            || async {
                JobResult {
                    triggered: true,
                    skipped: false,
                    ..Default::default()
                }
            },
        );
        assert!(a && b, "two different locks should both dispatch");

        let events = collect_events(&mut rx, 4).await;
        assert_eq!(count_started(&events), 2);
        assert_eq!(count_finished_with(&events, false), 2);
        assert_eq!(count_finished_with(&events, true), 0);
    }
}
