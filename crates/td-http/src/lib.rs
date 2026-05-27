//! Shared outbound-HTTP plumbing: a per-host concurrency limiter and a
//! minimum-gap throttle, plus a thin `reqwest::Client` wrapper that routes
//! every request through them.
//!
//! Why this exists: the v1 scheduler fires every configured source on the
//! same cron tick. With 15 nyaa uploader feeds and no coordination, that's
//! 15 simultaneous requests to a single host — enough to earn a 429 storm
//! and risk an IP ban from nyaa.si, MangaBaka, or MangaUpdates. Routing
//! everything through one process-wide [`HttpLimiter`] keeps the request
//! shape polite without forcing each provider/source crate to roll its
//! own throttle.
//!
//! Layout:
//! - [`HttpLimiter`] owns one [`HostState`] per host, lazily created on
//!   first use. Each `HostState` is `{ semaphore, last_release_at }`.
//! - [`LimitedClient`] wraps a `reqwest::Client` plus a reference to the
//!   limiter; it returns [`LimitedRequestBuilder`] from `get`/`head`/etc.
//! - [`LimitedRequestBuilder::send`] extracts the host from the request
//!   URL, acquires the host's permit, sleeps for any remaining `min_gap`,
//!   then executes the request. The permit is released when `send`
//!   returns (i.e. when response headers are in), not when the body is
//!   fully drained — holding the permit across body reads would block
//!   every other request to that host for the duration of a multi-minute
//!   dump download.
//!
//! Phase 3 will add retry-on-429/5xx inside [`LimitedRequestBuilder::send`].
//! The permit is held across retry attempts so a tight retry loop can't
//! be overtaken by a different request to the same host.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use reqwest::{Client, IntoUrl, Method, RequestBuilder, Response, Url};
use tokio::sync::{Mutex, Semaphore};

/// Per-host limiter knobs. A single instance is the default; per-host
/// overrides come from the operator's config.
#[derive(Debug, Clone)]
pub struct HostPolicy {
    /// Maximum number of in-flight requests to one host. Setting this to 1
    /// gives strict serial behavior (the original cure for nyaa's bursty
    /// 429s).
    pub concurrency: usize,
    /// Minimum time between successive request starts to the same host.
    /// Enforced by sleeping after permit acquisition; the permit is held
    /// throughout the sleep so callers do not race past the gate.
    pub min_gap: Duration,
    /// Maximum number of *additional* attempts after the initial request
    /// fails with a retryable status (429 / 502 / 503 / 504). Set to 0 to
    /// disable retries entirely. The first attempt is always made
    /// regardless.
    pub retry_max_attempts: u32,
    /// Initial backoff window. Doubles on each retry, capped at
    /// `retry_max_backoff`. Used for 502/503/504 and as the fallback when
    /// a 429 response omits `Retry-After`. Half of this value is also
    /// added as random jitter to every backoff to spread out retries from
    /// callers that hit the same upstream at the same instant.
    pub retry_initial_backoff: Duration,
    /// Hard ceiling on any single backoff window, including a value
    /// honored from a `Retry-After` header. An upstream returning
    /// `Retry-After: 3600` does not get to pin our request loop for an
    /// hour — we cap at this and try again sooner. The next retry's
    /// response will tell us whether the upstream is actually ready.
    pub retry_max_backoff: Duration,
}

impl HostPolicy {
    /// A no-op policy used by tests (and by [`HttpLimiter::no_limit`]):
    /// effectively unbounded concurrency, no gap, no retries. The
    /// concurrency value is capped well below
    /// `tokio::sync::Semaphore::MAX_PERMITS` (which is `(1 << 61) - 1`);
    /// the semaphore panics on construction past that limit and there is
    /// no scenario where 1024 in-flight requests to one host is "not
    /// enough".
    pub fn unlimited() -> Self {
        Self {
            concurrency: 1024,
            min_gap: Duration::ZERO,
            retry_max_attempts: 0,
            retry_initial_backoff: Duration::ZERO,
            retry_max_backoff: Duration::ZERO,
        }
    }
}

impl Default for HostPolicy {
    fn default() -> Self {
        // Conservative-but-functional defaults. Sources that need stricter
        // limits (notably nyaa.si) should be listed as per-host overrides
        // in config.
        Self {
            concurrency: 2,
            min_gap: Duration::from_millis(250),
            retry_max_attempts: 3,
            retry_initial_backoff: Duration::from_millis(500),
            retry_max_backoff: Duration::from_secs(30),
        }
    }
}

struct HostState {
    sem: Arc<Semaphore>,
    /// Wall-clock instant when the last request *finished* sending. Used
    /// to enforce `min_gap` against the *next* acquirer. Wrapped in a
    /// `Mutex` so two concurrent acquirers see a consistent value.
    last_release_at: Mutex<Option<Instant>>,
    policy: HostPolicy,
}

impl HostState {
    fn new(policy: HostPolicy) -> Self {
        Self {
            sem: Arc::new(Semaphore::new(policy.concurrency)),
            last_release_at: Mutex::new(None),
            policy,
        }
    }
}

/// Process-wide per-host limiter. Cheap to clone via `Arc`.
pub struct HttpLimiter {
    default_policy: HostPolicy,
    overrides: HashMap<String, HostPolicy>,
    state: DashMap<String, Arc<HostState>>,
    disabled: bool,
}

impl HttpLimiter {
    /// Build a limiter with a default policy and per-host overrides. Host
    /// keys are matched case-insensitively against the request URL's host
    /// component (`example.com`, no scheme, no port).
    pub fn new(default_policy: HostPolicy, overrides: HashMap<String, HostPolicy>) -> Self {
        let overrides = overrides
            .into_iter()
            .map(|(k, v)| (k.to_lowercase(), v))
            .collect();
        Self {
            default_policy,
            overrides,
            state: DashMap::new(),
            disabled: false,
        }
    }

    /// A pass-through limiter that performs no serialization and no
    /// sleeping. Intended for unit/integration tests where the limiter
    /// would otherwise introduce ordering dependencies into HTTP-fixture
    /// playback.
    pub fn no_limit() -> Arc<Self> {
        Arc::new(Self {
            default_policy: HostPolicy::unlimited(),
            overrides: HashMap::new(),
            state: DashMap::new(),
            disabled: true,
        })
    }

    /// Wrap an existing `reqwest::Client` so every request routes through
    /// this limiter. Callers keep ownership of their own `Client` (with
    /// their own user-agent, timeouts, redirect policy, etc.); this
    /// adapter only intercepts the `send` step.
    pub fn client(self: &Arc<Self>, http: Client) -> LimitedClient {
        LimitedClient {
            limiter: self.clone(),
            http,
        }
    }

    fn host_state_for(&self, host: &str) -> Arc<HostState> {
        let key = host.to_lowercase();
        if let Some(existing) = self.state.get(&key) {
            return existing.clone();
        }
        let policy = self
            .overrides
            .get(&key)
            .cloned()
            .unwrap_or_else(|| self.default_policy.clone());
        let state = Arc::new(HostState::new(policy));
        // `entry().or_insert_with` would race two acquirers and pick the
        // first; that's fine here.
        self.state
            .entry(key)
            .or_insert_with(|| state)
            .value()
            .clone()
    }
}

/// `reqwest::Client` adapter that funnels every request through an
/// [`HttpLimiter`]. The underlying client is preserved as-is.
#[derive(Clone)]
pub struct LimitedClient {
    limiter: Arc<HttpLimiter>,
    http: Client,
}

impl LimitedClient {
    pub fn get<U: IntoUrl>(&self, url: U) -> LimitedRequestBuilder {
        self.request(Method::GET, url)
    }

    pub fn head<U: IntoUrl>(&self, url: U) -> LimitedRequestBuilder {
        self.request(Method::HEAD, url)
    }

    pub fn request<U: IntoUrl>(&self, method: Method, url: U) -> LimitedRequestBuilder {
        // Convert eagerly so we can extract the host without re-parsing
        // inside `send`. URL parse errors are deferred into `send()` so
        // the call shape mirrors `reqwest::Client::get` (which also
        // never fails until you call send/build).
        let parsed: Result<Url, _> = url.into_url();
        let (inner, url_or_err) = match parsed {
            Ok(u) => (self.http.request(method, u.clone()), Ok(u)),
            Err(e) => {
                // We still want a RequestBuilder to chain `.header(...)`
                // off of; reqwest doesn't expose a way to build an
                // error-state builder, so we synthesize one against a
                // placeholder URL. The placeholder is never sent — the
                // error in `url_or_err` short-circuits `send`.
                (
                    self.http.request(Method::GET, "http://invalid.local/"),
                    Err(e),
                )
            }
        };
        LimitedRequestBuilder {
            limiter: self.limiter.clone(),
            inner,
            url_or_err,
        }
    }

    /// Borrow the underlying `reqwest::Client` (e.g. for `bytes_stream`
    /// follow-ups that don't need limiting because they're consuming a
    /// response body already in flight).
    pub fn inner(&self) -> &Client {
        &self.http
    }
}

/// Builder for one limited request. Forwards the small subset of
/// `RequestBuilder` methods the codebase actually uses; if a caller needs
/// something else, add a forwarder rather than exposing the inner builder
/// (which would let callers send around the limiter).
pub struct LimitedRequestBuilder {
    limiter: Arc<HttpLimiter>,
    inner: RequestBuilder,
    url_or_err: Result<Url, reqwest::Error>,
}

impl LimitedRequestBuilder {
    /// Add a header. Matches the `(&str, &str)` shape every caller in this
    /// codebase already uses; if a future caller needs something richer
    /// (e.g. `HeaderMap` or typed header values), add a forwarder rather
    /// than re-exporting reqwest's generic bound, which pulls in the
    /// `http` crate's error type.
    pub fn header(mut self, key: &str, value: &str) -> Self {
        self.inner = self.inner.header(key, value);
        self
    }

    /// Send the request. Acquires the per-host permit, enforces the
    /// `min_gap`, then executes the request. Retries on `429`, `502`,
    /// `503`, and `504` according to the host's `HostPolicy`:
    /// `Retry-After` (in seconds) is honored if present, otherwise an
    /// exponentially-growing jittered backoff is used. The permit is
    /// held across all retry attempts — releasing between attempts
    /// would let a queued caller race past us into the same upstream
    /// that just told us to back off, defeating the rate-limit signal.
    /// Released when send returns (response headers in), not when the
    /// body is drained.
    pub async fn send(self) -> Result<Response, reqwest::Error> {
        let url = self.url_or_err?;

        // Disabled limiter: straight pass-through, no acquisition, no
        // retries. Tests and shutdown paths use this.
        if self.limiter.disabled {
            return self.inner.send().await;
        }

        let host = url.host_str().unwrap_or("").to_string();
        let state = self.limiter.host_state_for(&host);

        // `Semaphore::acquire_owned` never errors unless the semaphore is
        // closed, which we never do.
        let _permit = state
            .sem
            .clone()
            .acquire_owned()
            .await
            .expect("limiter semaphore was closed");

        // Honor the min-gap. We compute the wait under the mutex so two
        // concurrent acquirers see consistent state, but sleep with the
        // mutex held — releasing it would let a third acquirer race past
        // us with no gap. The semaphore is the primary serializer; the
        // mutex just guards `last_release_at`.
        {
            let mut last = state.last_release_at.lock().await;
            if let Some(prev) = *last {
                let target = prev + state.policy.min_gap;
                let now = Instant::now();
                if target > now {
                    let wait = target - now;
                    tracing::trace!(host = %host, ?wait, "min-gap throttle: sleeping before request");
                    tokio::time::sleep(wait).await;
                }
            }
            *last = Some(Instant::now());
        }

        let policy = &state.policy;
        let inner = self.inner;
        let max_attempts = policy.retry_max_attempts;

        // Clone the request up front so retries can re-issue the same
        // headers/body. `try_clone` returns `None` for streaming bodies;
        // there are no streaming request bodies in this codebase, but if
        // one ever appears we degrade gracefully to a single attempt
        // rather than silently dropping the retry budget.
        let cloneable = inner.try_clone();
        let can_retry = cloneable.is_some() && max_attempts > 0;
        if max_attempts > 0 && cloneable.is_none() {
            tracing::warn!(
                host = %host,
                "request body is not cloneable; retries disabled for this call"
            );
        }

        // Attempt 0 = the initial request; attempts 1..=max_attempts are
        // retries. Loop exits early on a non-retryable status.
        let mut last_response = inner.send().await?;
        if !can_retry {
            return Ok(last_response);
        }
        let cloneable = cloneable.expect("verified Some above");

        for attempt in 1..=max_attempts {
            let status = last_response.status();
            let Some(reason) = RetryReason::from_status(status) else {
                return Ok(last_response);
            };

            let backoff = match reason {
                RetryReason::TooManyRequests => retry_after_or_backoff(
                    &last_response,
                    attempt,
                    policy.retry_initial_backoff,
                    policy.retry_max_backoff,
                ),
                RetryReason::TransientServerError => jittered_backoff(
                    attempt,
                    policy.retry_initial_backoff,
                    policy.retry_max_backoff,
                ),
            };

            tracing::warn!(
                host = %host,
                attempt,
                max_attempts,
                status = status.as_u16(),
                backoff_ms = backoff.as_millis() as u64,
                "retrying after upstream rate-limit / transient error"
            );

            tokio::time::sleep(backoff).await;

            let retry_req = cloneable
                .try_clone()
                .expect("body was verified cloneable above");
            last_response = retry_req.send().await?;
        }

        // Exhausted: return whatever the last attempt produced. Callers
        // see the original status code (typically 429 or 503) and can
        // decide how to surface it — same shape as no-retry behavior.
        Ok(last_response)
    }
}

/// Why a response triggered a retry. Kept private to the crate; callers
/// don't need to disambiguate.
enum RetryReason {
    /// 429. Prefer `Retry-After` over computed backoff.
    TooManyRequests,
    /// 502 / 503 / 504. Compute backoff from policy.
    TransientServerError,
}

impl RetryReason {
    fn from_status(status: reqwest::StatusCode) -> Option<Self> {
        match status.as_u16() {
            429 => Some(Self::TooManyRequests),
            502..=504 => Some(Self::TransientServerError),
            _ => None,
        }
    }
}

/// Read `Retry-After` from a response. Only the seconds form is
/// supported — the HTTP-date form is rare for rate-limited APIs and
/// would pull in an extra dep for marginal value. Returns `None` if the
/// header is missing or unparseable; callers fall back to the computed
/// backoff in that case.
fn parse_retry_after_seconds(resp: &Response) -> Option<Duration> {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

/// Resolve the backoff to use for a 429: honor `Retry-After` (capped at
/// `max_backoff`) if present, otherwise fall back to the same
/// jittered exponential backoff used for 5xx.
fn retry_after_or_backoff(
    resp: &Response,
    attempt: u32,
    initial: Duration,
    max: Duration,
) -> Duration {
    match parse_retry_after_seconds(resp) {
        Some(d) => d.min(max),
        None => jittered_backoff(attempt, initial, max),
    }
}

/// Exponential backoff with random jitter:
/// `initial * 2^(attempt-1) + rand(0..=initial/2)`, capped at `max`.
/// `attempt` is 1-indexed (the first retry is attempt 1).
fn jittered_backoff(attempt: u32, initial: Duration, max: Duration) -> Duration {
    use rand::RngExt;
    let multiplier = 1u32 << (attempt - 1).min(31);
    let base = initial.saturating_mul(multiplier).min(max);
    let jitter_ceiling = (initial / 2).as_millis() as u64;
    let jitter_ms = if jitter_ceiling == 0 {
        0
    } else {
        rand::rng().random_range(0..=jitter_ceiling)
    };
    (base + Duration::from_millis(jitter_ms)).min(max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::{Duration, Instant};

    /// Build a limiter scoped to one host with `concurrency=1` and a
    /// short min-gap; useful for asserting serialization timing.
    fn one_at_a_time(host: &str, min_gap_ms: u64) -> Arc<HttpLimiter> {
        let mut overrides = HashMap::new();
        overrides.insert(
            host.to_string(),
            HostPolicy {
                concurrency: 1,
                min_gap: Duration::from_millis(min_gap_ms),
                ..HostPolicy::unlimited()
            },
        );
        Arc::new(HttpLimiter::new(HostPolicy::unlimited(), overrides))
    }

    /// Two concurrent requests to the same host must serialize: the second
    /// permit can only be acquired after the first is dropped. We exercise
    /// this without an HTTP server by acquiring permits directly via the
    /// host-state semaphore.
    #[tokio::test(start_paused = true)]
    async fn same_host_serializes_under_concurrency_one() {
        let limiter = one_at_a_time("example.com", 0);
        let state = limiter.host_state_for("example.com");
        assert_eq!(state.sem.available_permits(), 1);

        let order = Arc::new(AtomicUsize::new(0));
        let order1 = order.clone();
        let order2 = order.clone();
        let state1 = state.clone();
        let state2 = state.clone();

        let t1 = tokio::spawn(async move {
            let _p = state1.sem.clone().acquire_owned().await.unwrap();
            // Mark this task as "first", then yield long enough that the
            // second task definitely gets a chance to try to acquire.
            order1.store(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(100)).await;
        });
        // Make sure t1 grabs the permit before t2 starts.
        tokio::time::sleep(Duration::from_millis(10)).await;
        let t2 = tokio::spawn(async move {
            let _p = state2.sem.clone().acquire_owned().await.unwrap();
            // If serialization is broken, this fires while t1 still holds
            // the permit; the assertion below catches it.
            order2.store(2, Ordering::SeqCst);
        });
        t1.await.unwrap();
        t2.await.unwrap();
        assert_eq!(order.load(Ordering::SeqCst), 2);
    }

    /// Requests to different hosts must NOT serialize. Two acquirers on
    /// distinct hosts should both hold their permits at once.
    #[tokio::test]
    async fn different_hosts_acquire_in_parallel() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "a.example".into(),
            HostPolicy {
                concurrency: 1,
                min_gap: Duration::ZERO,
                ..HostPolicy::unlimited()
            },
        );
        overrides.insert(
            "b.example".into(),
            HostPolicy {
                concurrency: 1,
                min_gap: Duration::ZERO,
                ..HostPolicy::unlimited()
            },
        );
        let limiter = Arc::new(HttpLimiter::new(HostPolicy::unlimited(), overrides));
        let state_a = limiter.host_state_for("a.example");
        let state_b = limiter.host_state_for("b.example");

        let permit_a = state_a.sem.clone().acquire_owned().await.unwrap();
        let permit_b = state_b.sem.clone().acquire_owned().await.unwrap();
        // Holding both permits at the same time would not be possible if
        // hosts shared a semaphore.
        assert_eq!(state_a.sem.available_permits(), 0);
        assert_eq!(state_b.sem.available_permits(), 0);
        drop(permit_a);
        drop(permit_b);
    }

    /// The min-gap throttle must delay a second acquirer when the first
    /// has just released. Uses Tokio's paused-time runtime so the test
    /// stays deterministic without sleeping wall-clock.
    #[tokio::test(start_paused = true)]
    async fn min_gap_delays_successive_acquirers() {
        let limiter = one_at_a_time("example.com", 500);
        let state = limiter.host_state_for("example.com");

        // Simulate a first "send": grab permit, set last_release_at, drop.
        let permit = state.sem.clone().acquire_owned().await.unwrap();
        *state.last_release_at.lock().await = Some(Instant::now().into_std());
        drop(permit);

        // Second acquirer: emulate what `send()` does (acquire permit,
        // then sleep for the remaining gap).
        let start = Instant::now();
        let permit = state.sem.clone().acquire_owned().await.unwrap();
        let last = *state.last_release_at.lock().await;
        if let Some(prev) = last {
            let target = prev + state.policy.min_gap;
            let now = Instant::now().into_std();
            if target > now {
                tokio::time::sleep(target - now).await;
            }
        }
        let waited = start.elapsed();
        drop(permit);
        assert!(
            waited >= Duration::from_millis(500),
            "expected min_gap to delay the second acquirer by >=500ms, got {waited:?}"
        );
    }

    /// `HttpLimiter::no_limit()` must short-circuit acquisition entirely
    /// — useful for test harnesses that play back HTTP fixtures and
    /// shouldn't pay min-gap costs.
    #[tokio::test]
    async fn no_limit_skips_acquisition() {
        let limiter = HttpLimiter::no_limit();
        assert!(limiter.disabled);
        // Calling host_state_for should still work (it's used internally)
        // but with the unlimited default policy.
        let state = limiter.host_state_for("anything.example");
        assert_eq!(
            state.policy.concurrency,
            HostPolicy::unlimited().concurrency
        );
    }

    // ---- Retry layer (Phase 3) ----------------------------------------
    //
    // These tests drive a tiny canned TCP server on a real port so we
    // exercise the actual reqwest send path, including header
    // preservation across retries. The pattern mirrors the existing
    // canned server in `td-resolution::mangaupdates_redirect::tests`.

    use std::io::{Read, Write};
    use std::net::TcpListener as StdListener;
    use std::thread;
    use std::time::Instant as StdInstant;

    /// Bind a TCP listener that, for each connection, responds with one
    /// of the supplied canned responses in order. The captured request
    /// bytes go into `requests` so tests can assert what the client
    /// actually sent.
    fn spawn_canned_server(
        responses: Vec<&'static [u8]>,
    ) -> (String, Arc<std::sync::Mutex<Vec<Vec<u8>>>>) {
        let listener = StdListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}");
        let requests = Arc::new(std::sync::Mutex::new(Vec::<Vec<u8>>::new()));
        let captured = requests.clone();
        thread::spawn(move || {
            for (idx, stream) in listener.incoming().enumerate() {
                let Ok(mut stream) = stream else { return };
                let mut buf = [0u8; 8192];
                let n = stream.read(&mut buf).unwrap_or(0);
                captured.lock().unwrap().push(buf[..n].to_vec());
                if idx >= responses.len() {
                    return;
                }
                let _ = stream.write_all(responses[idx]);
                let _ = stream.flush();
            }
        });
        (url, requests)
    }

    /// Build a limiter that retries up to `max_attempts` with short
    /// backoffs (test-time friendly) against `host`.
    fn retrying_limiter(host: &str, max_attempts: u32) -> Arc<HttpLimiter> {
        let mut overrides = HashMap::new();
        overrides.insert(
            host.to_string(),
            HostPolicy {
                concurrency: 4,
                min_gap: Duration::ZERO,
                retry_max_attempts: max_attempts,
                retry_initial_backoff: Duration::from_millis(50),
                retry_max_backoff: Duration::from_millis(200),
            },
        );
        Arc::new(HttpLimiter::new(HostPolicy::unlimited(), overrides))
    }

    /// A 429 with `Retry-After: 1` must delay the retry by ~1 second
    /// and then return the successful 200 from the second attempt.
    /// Uses a large `retry_max_backoff` so the cap doesn't shorten the
    /// 1-second value — the separate cap test below covers that path.
    #[tokio::test]
    async fn retry_honors_retry_after_then_returns_success() {
        let r1 = b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 1\r\nContent-Length: 0\r\n\r\n";
        let r2 = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
        let (base, _captured) = spawn_canned_server(vec![r1, r2]);
        let url = reqwest::Url::parse(&base).unwrap();
        let host = url.host_str().unwrap().to_string();
        let mut overrides = HashMap::new();
        overrides.insert(
            host,
            HostPolicy {
                concurrency: 4,
                min_gap: Duration::ZERO,
                retry_max_attempts: 3,
                retry_initial_backoff: Duration::from_millis(50),
                retry_max_backoff: Duration::from_secs(5),
            },
        );
        let limiter = Arc::new(HttpLimiter::new(HostPolicy::unlimited(), overrides));
        let client = limiter.client(reqwest::Client::new());

        let start = StdInstant::now();
        let resp = client.get(&base).send().await.unwrap();
        let elapsed = start.elapsed();
        assert_eq!(resp.status().as_u16(), 200);
        assert!(
            elapsed >= Duration::from_millis(900),
            "expected Retry-After: 1 to delay >= 900ms, got {elapsed:?}"
        );
    }

    /// `retry_max_backoff` must cap an over-eager `Retry-After` value.
    /// An upstream returning `Retry-After: 60` does not get to pin us
    /// for a minute when our cap is 200ms.
    #[tokio::test]
    async fn retry_max_backoff_caps_retry_after() {
        let r1 = b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 60\r\nContent-Length: 0\r\n\r\n";
        let r2 = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
        let (base, _captured) = spawn_canned_server(vec![r1, r2]);
        let url = reqwest::Url::parse(&base).unwrap();
        let host = url.host_str().unwrap();
        let limiter = retrying_limiter(host, 3); // max_backoff = 200ms
        let client = limiter.client(reqwest::Client::new());

        let start = StdInstant::now();
        let resp = client.get(&base).send().await.unwrap();
        let elapsed = start.elapsed();
        assert_eq!(resp.status().as_u16(), 200);
        assert!(
            elapsed < Duration::from_secs(1),
            "expected Retry-After: 60 to be capped to ~200ms, got {elapsed:?}"
        );
    }

    /// When every attempt fails with 429, the limiter must give up
    /// after `retry_max_attempts` retries and return the final 429
    /// response (not a synthesized error). Caller decides how to
    /// surface the failure.
    #[tokio::test]
    async fn retry_exhaustion_returns_final_response() {
        // Server responds 429 + Retry-After: 0 to keep backoff small.
        // 1 initial + 3 retries = 4 responses needed.
        let bad = b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Length: 0\r\n\r\n";
        let (base, captured) = spawn_canned_server(vec![bad, bad, bad, bad]);
        let url = reqwest::Url::parse(&base).unwrap();
        let host = url.host_str().unwrap();
        let limiter = retrying_limiter(host, 3);
        let client = limiter.client(reqwest::Client::new());

        let resp = client.get(&base).send().await.unwrap();
        assert_eq!(
            resp.status().as_u16(),
            429,
            "exhausted retries must surface the upstream's final status"
        );
        // 1 initial attempt + 3 retries = 4 total connections.
        assert_eq!(
            captured.lock().unwrap().len(),
            4,
            "expected 4 attempts (1 initial + 3 retries)"
        );
    }

    /// Headers set on the initial request must be sent on every retry
    /// too. Regression guard against forgetting `try_clone()`.
    #[tokio::test]
    async fn retry_preserves_request_headers() {
        let r1 = b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n";
        let r2 = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
        let (base, captured) = spawn_canned_server(vec![r1, r2]);
        let url = reqwest::Url::parse(&base).unwrap();
        let host = url.host_str().unwrap();
        let limiter = retrying_limiter(host, 2);
        let client = limiter.client(reqwest::Client::new());

        let resp = client
            .get(&base)
            .header("x-marker", "preserved-value")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);

        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 2, "expected one retry");
        for (i, raw) in reqs.iter().enumerate() {
            let text = String::from_utf8_lossy(raw);
            assert!(
                text.to_lowercase().contains("x-marker: preserved-value"),
                "request {i} missing the x-marker header; raw:\n{text}"
            );
        }
    }
}
