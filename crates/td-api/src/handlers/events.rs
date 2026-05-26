//! Server-Sent Events stream for manual-trigger lifecycle.
//!
//! Operators that click "Poll now" or "Refresh cache" are staring at the
//! screen, and waiting for the next TanStack-Query refetch tick to learn
//! whether the job finished feels laggy. This stream pushes a
//! `started` event the moment a manual trigger handler acquires the
//! per-key mutex and a `finished` event when the spawned tick returns.
//!
//! Intentionally narrow: cron-driven runs do **not** publish here. The
//! channel is the "user is staring at the screen" signal, not an audit
//! log; lists and metric panels continue to refresh via TanStack-Query
//! polling.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::response::Sse;
use axum::response::sse::{Event, KeepAlive};
use futures_util::stream::Stream;
use futures_util::stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::state::AppState;

/// SSE keepalive cadence. 15s comfortably under most proxy idle timers
/// and matches the standard nginx default. The comment frame is
/// ` ` (a single colon) so clients see a heartbeat without
/// dispatching a synthetic event.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// Subscribe to manual-trigger lifecycle events as an SSE stream.
///
/// Auth: inherits the read auth gate (`auth.read_requires_auth`). The
/// browser `EventSource` API cannot carry a custom `Authorization`
/// header, so installs that flip `read_requires_auth=true` will need
/// the operator's `api_key` to be sent via the
/// `Authorization` header through a fetch-based polyfill, or the gate
/// disabled for this single route. Default installs (`false`) work out
/// of the box.
#[utoipa::path(
    get,
    path = "/api/v1/events/jobs",
    tag = "system",
    operation_id = "events_jobs",
    responses(
        (status = 200, description = "text/event-stream of JobEvent frames", content_type = "text/event-stream")
    )
)]
pub async fn jobs(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.job_events.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|res| async move {
        // `BroadcastStream` yields `Err(BroadcastStreamRecvError::Lagged(n))`
        // when a slow client misses frames. Convert those to an `info`
        // comment so clients see a "you dropped N events" hint without
        // tearing down the connection. Genuine errors don't happen on
        // this stream type.
        match res {
            Ok(ev) => match Event::default().json_data(&ev) {
                Ok(frame) => Some(Ok::<_, Infallible>(frame)),
                Err(e) => {
                    tracing::warn!(?e, "failed to serialize job event");
                    None
                }
            },
            Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
                let frame = Event::default()
                    .event("lag")
                    .data(format!("dropped {n} events"));
                Some(Ok(frame))
            }
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(KEEPALIVE_INTERVAL))
}
