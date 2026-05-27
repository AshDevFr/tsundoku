//! Resolve a MangaUpdates legacy numeric id to its modern alphanumeric
//! slug by issuing a HEAD against the legacy URL and reading the
//! permanent-redirect `Location` header.
//!
//! Constraints we honor:
//!
//! - MangaUpdates rate-limits. A semaphore-of-one mutex plus a one-second
//!   gap between releases keeps us safely under their bar at single-host
//!   personal scale.
//! - 429 triggers exponential backoff (1s, 2s, 4s, ... cap 300s). The
//!   backoff state lives on the redirector, so the next caller will wait
//!   automatically without each one re-discovering the limit.
//! - The redirector never *writes* the cache: callers (typically the
//!   resolution pipeline) own the persistence step. Keeping concerns
//!   separate lets tests inject canned responses without a database.
//!
//! Outcomes the caller should handle:
//!
//! - [`ResolveOutcome::Modern`] — `Location: /series/{slug}/{title}`. The
//!   slug is the modern id MangaBaka indexes on.
//! - [`ResolveOutcome::Tombstone`] — the redirect target does not look
//!   like a real series page (`/series` only, root, or non-series path).
//!   The legacy id has been retired; persist a tombstone and stop asking.
//! - [`ResolveError::Transient`] — 429, network failure, no `Location`
//!   header, etc. Do not write to the cache; retry on the next poll.

use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::{Client, StatusCode, redirect};
use td_http::{HttpLimiter, LimitedClient};
use thiserror::Error;
use tokio::sync::Mutex;
use tokio::time::sleep;

/// MU's web URL the legacy redirect lives at. Exposed for tests that
/// drive a local listener; production callers do not need to override.
pub const MANGAUPDATES_BASE_URL: &str = "https://www.mangaupdates.com";

/// Minimum delay between two HEAD requests to MangaUpdates.
const MIN_REQUEST_GAP: Duration = Duration::from_millis(1_000);

/// Starting backoff window after the first 429.
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);

/// Upper bound on the 429 backoff window. Keeps a long outage from
/// blocking the entire resolver loop forever.
const MAX_BACKOFF: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveOutcome {
    /// MangaUpdates redirected us to a real `/series/{slug}/` page.
    Modern(String),
    /// MU redirected us somewhere that is not a series page (typically
    /// `/series`). The legacy id is retired.
    Tombstone,
}

#[derive(Debug, Error)]
pub enum ResolveError {
    /// 429, network failure, malformed/missing Location header.
    /// Cache should not be updated; the next poll cycle retries.
    #[error("transient redirect failure: {0}")]
    Transient(String),
}

/// Wraps an HTTP client with the per-host throttle + 429 backoff state.
/// One instance per process is enough; clone the `Arc` to share it.
///
/// The redirector keeps its own backoff/throttle on top of the global
/// [`HttpLimiter`]: the limiter handles process-wide per-host
/// serialization, while this struct's `ThrottleState` carries the
/// MU-specific 429-driven exponential backoff (which has stricter
/// semantics than a flat min-gap).
pub struct MangaUpdatesRedirector {
    client: LimitedClient,
    base_url: String,
    throttle: Arc<Mutex<ThrottleState>>,
}

struct ThrottleState {
    next_allowed: Instant,
    backoff: Duration,
}

impl MangaUpdatesRedirector {
    /// Build a redirector against the live MangaUpdates host. Uses
    /// `redirect::Policy::none()` so we can read the `Location` header
    /// instead of transparently following the redirect.
    pub fn new(user_agent: &str, limiter: Arc<HttpLimiter>) -> reqwest::Result<Self> {
        let inner = Client::builder()
            .redirect(redirect::Policy::none())
            .user_agent(user_agent)
            .build()?;
        Ok(Self::with_client(
            limiter.client(inner),
            MANGAUPDATES_BASE_URL,
        ))
    }

    /// Build a redirector that targets a custom base URL. Used by the
    /// crate's own tests against a local listener.
    pub fn with_client(client: LimitedClient, base_url: impl Into<String>) -> Self {
        Self {
            client,
            base_url: base_url.into(),
            throttle: Arc::new(Mutex::new(ThrottleState {
                next_allowed: Instant::now(),
                backoff: INITIAL_BACKOFF,
            })),
        }
    }

    /// Resolve a single legacy id. Honors the throttle: concurrent calls
    /// serialize through the mutex with a one-second floor between
    /// releases, and a 429 from MU pushes the next-allowed-at out
    /// exponentially.
    pub async fn resolve_legacy(&self, legacy_id: i64) -> Result<ResolveOutcome, ResolveError> {
        let url = format!("{}/series.html?id={legacy_id}", self.base_url);
        // Serialize: only one MU request in flight at a time, with a 1s
        // floor between consecutive calls.
        let mut guard = self.throttle.lock().await;
        let now = Instant::now();
        if guard.next_allowed > now {
            let wait = guard.next_allowed - now;
            // Release the lock for the wait? No — we want hard serial
            // semantics; everyone queues up behind the gate.
            sleep(wait).await;
        }
        let response = self.client.head(&url).send().await;
        let send_completed = Instant::now();
        match response {
            Ok(resp) => {
                if resp.status() == StatusCode::TOO_MANY_REQUESTS {
                    // Bump backoff for the next caller.
                    guard.backoff = (guard.backoff * 2).min(MAX_BACKOFF);
                    guard.next_allowed = send_completed + guard.backoff;
                    return Err(ResolveError::Transient(format!(
                        "mangaupdates returned 429; backing off {:?}",
                        guard.backoff
                    )));
                }
                // Successful response: reset backoff to the floor and
                // arm the gate for the next caller.
                guard.backoff = INITIAL_BACKOFF;
                guard.next_allowed = send_completed + MIN_REQUEST_GAP;
                let location = resp
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string);
                drop(guard);
                Ok(parse_location(location.as_deref()))
            }
            Err(e) => {
                // Network error: don't bump backoff (we don't know if
                // MU is unhappy or our own network blinked) but still
                // hold the 1s floor.
                guard.next_allowed = send_completed + MIN_REQUEST_GAP;
                Err(ResolveError::Transient(format!(
                    "mangaupdates HEAD failed: {e}"
                )))
            }
        }
    }
}

/// Pure URL-parsing layer. Pulled out so the redirect contract is
/// unit-testable without an HTTP stack.
///
/// Returns:
/// - `ResolveOutcome::Modern(slug)` when `location` matches
///   `/series/{slug}[/{title}]` with a non-empty slug.
/// - `ResolveOutcome::Tombstone` when `location` is missing, empty, or
///   points anywhere else (typically `/series` after MU retired the id).
pub fn parse_location(location: Option<&str>) -> ResolveOutcome {
    let Some(loc) = location else {
        return ResolveOutcome::Tombstone;
    };
    let loc = loc.trim();
    if loc.is_empty() {
        return ResolveOutcome::Tombstone;
    }
    // Strip scheme + host if present; we only care about the path.
    let path = if let Some(rest) = loc.strip_prefix("https://") {
        match rest.find('/') {
            Some(i) => &rest[i..],
            None => "/",
        }
    } else if let Some(rest) = loc.strip_prefix("http://") {
        match rest.find('/') {
            Some(i) => &rest[i..],
            None => "/",
        }
    } else {
        loc
    };
    // Strip query / fragment.
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let trimmed = path.trim_start_matches('/');
    let mut segments = trimmed.split('/');
    if segments.next() != Some("series") {
        return ResolveOutcome::Tombstone;
    }
    match segments.next() {
        Some(slug) if !slug.is_empty() => ResolveOutcome::Modern(slug.to_string()),
        _ => ResolveOutcome::Tombstone,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener as StdListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    #[test]
    fn parse_location_modern_redirect_yields_slug() {
        let got = parse_location(Some(
            "https://www.mangaupdates.com/series/6z1uqw7/solo-leveling",
        ));
        assert_eq!(got, ResolveOutcome::Modern("6z1uqw7".into()));
    }

    #[test]
    fn parse_location_handles_path_only() {
        let got = parse_location(Some("/series/uu6yf6n/horimiya"));
        assert_eq!(got, ResolveOutcome::Modern("uu6yf6n".into()));
    }

    #[test]
    fn parse_location_modern_without_title_segment() {
        let got = parse_location(Some("/series/6z1uqw7"));
        assert_eq!(got, ResolveOutcome::Modern("6z1uqw7".into()));
    }

    #[test]
    fn parse_location_dead_id_redirects_to_listing_is_tombstone() {
        // What MU returns for ids it has retired.
        let got = parse_location(Some("/series"));
        assert_eq!(got, ResolveOutcome::Tombstone);
    }

    #[test]
    fn parse_location_unrelated_path_is_tombstone() {
        let got = parse_location(Some("/about"));
        assert_eq!(got, ResolveOutcome::Tombstone);
    }

    #[test]
    fn parse_location_missing_or_empty_is_tombstone() {
        assert_eq!(parse_location(None), ResolveOutcome::Tombstone);
        assert_eq!(parse_location(Some("   ")), ResolveOutcome::Tombstone);
    }

    #[test]
    fn parse_location_strips_query_and_fragment() {
        let got = parse_location(Some("/series/6z1uqw7/title?ref=foo#reviews"));
        assert_eq!(got, ResolveOutcome::Modern("6z1uqw7".into()));
    }

    // ---- HTTP layer: drive a local listener that returns canned bytes ----

    /// Bind a TCP listener that, for each connection, responds with one
    /// of the supplied canned responses in order. Returns the bound
    /// host:port and a thread join handle.
    fn spawn_canned_server(responses: Vec<&'static [u8]>) -> (String, thread::JoinHandle<()>) {
        let listener = StdListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}");
        let counter = Arc::new(AtomicUsize::new(0));
        let handle = thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let idx = counter.fetch_add(1, Ordering::SeqCst);
                if idx >= responses.len() {
                    return;
                }
                let _ = stream.write_all(responses[idx]);
                let _ = stream.flush();
            }
        });
        (url, handle)
    }

    fn build_redirector(base_url: String) -> MangaUpdatesRedirector {
        let inner = Client::builder()
            .redirect(redirect::Policy::none())
            .build()
            .unwrap();
        let client = HttpLimiter::no_limit().client(inner);
        MangaUpdatesRedirector::with_client(client, base_url)
    }

    #[tokio::test]
    async fn resolve_legacy_308_returns_modern_slug() {
        let resp =
            b"HTTP/1.1 308 Permanent Redirect\r\nContent-Length: 0\r\nLocation: https://www.mangaupdates.com/series/6z1uqw7/solo-leveling\r\n\r\n";
        let (url, _h) = spawn_canned_server(vec![resp]);
        let redirector = build_redirector(url);
        let got = redirector.resolve_legacy(151349).await.unwrap();
        assert_eq!(got, ResolveOutcome::Modern("6z1uqw7".into()));
    }

    #[tokio::test]
    async fn resolve_legacy_307_to_series_listing_is_tombstone() {
        let resp =
            b"HTTP/1.1 307 Temporary Redirect\r\nContent-Length: 0\r\nLocation: /series\r\n\r\n";
        let (url, _h) = spawn_canned_server(vec![resp]);
        let redirector = build_redirector(url);
        let got = redirector.resolve_legacy(99_999_999).await.unwrap();
        assert_eq!(got, ResolveOutcome::Tombstone);
    }

    #[tokio::test]
    async fn resolve_legacy_429_returns_transient_and_arms_backoff() {
        let resp = b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nRetry-After: 1\r\n\r\n";
        let (url, _h) = spawn_canned_server(vec![resp]);
        let redirector = build_redirector(url);
        let got = redirector.resolve_legacy(151349).await;
        assert!(matches!(got, Err(ResolveError::Transient(_))));
        // Backoff state must have advanced past the first window.
        let state = redirector.throttle.lock().await;
        assert!(state.backoff > INITIAL_BACKOFF);
    }

    #[tokio::test]
    async fn resolve_legacy_serializes_concurrent_calls() {
        // Two requests, both 308, with distinct slugs. The gap between
        // them on the wire should be at least one MIN_REQUEST_GAP.
        let r1 =
            b"HTTP/1.1 308 Permanent Redirect\r\nContent-Length: 0\r\nLocation: /series/aaa/one\r\n\r\n";
        let r2 =
            b"HTTP/1.1 308 Permanent Redirect\r\nContent-Length: 0\r\nLocation: /series/bbb/two\r\n\r\n";
        let (url, _h) = spawn_canned_server(vec![r1, r2]);
        let redirector = Arc::new(build_redirector(url));
        let start = Instant::now();
        let r_a = {
            let r = Arc::clone(&redirector);
            tokio::spawn(async move { r.resolve_legacy(1).await })
        };
        let r_b = {
            let r = Arc::clone(&redirector);
            tokio::spawn(async move { r.resolve_legacy(2).await })
        };
        let (a, b) = (r_a.await.unwrap(), r_b.await.unwrap());
        let elapsed = start.elapsed();
        // Both succeeded with distinct slugs.
        assert!(matches!(a, Ok(ResolveOutcome::Modern(_))));
        assert!(matches!(b, Ok(ResolveOutcome::Modern(_))));
        // Second call must have waited at least MIN_REQUEST_GAP before
        // hitting the wire.
        assert!(
            elapsed >= MIN_REQUEST_GAP,
            "expected concurrent calls to serialize through the 1s gate, took {elapsed:?}"
        );
    }
}
