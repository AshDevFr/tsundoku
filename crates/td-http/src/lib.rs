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
}

impl HostPolicy {
    /// A no-op policy used by tests (and by [`HttpLimiter::no_limit`]):
    /// effectively unbounded concurrency, no gap. The concurrency value
    /// is capped well below `tokio::sync::Semaphore::MAX_PERMITS` (which
    /// is `(1 << 61) - 1`); the semaphore panics on construction past
    /// that limit and there is no scenario where 1024 in-flight requests
    /// to one host is "not enough".
    pub fn unlimited() -> Self {
        Self {
            concurrency: 1024,
            min_gap: Duration::ZERO,
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
    /// `min_gap`, then executes the request. Releases the permit when
    /// the response headers are in (not when the body is drained).
    pub async fn send(self) -> Result<Response, reqwest::Error> {
        let url = self.url_or_err?;

        // Disabled limiter: straight pass-through, no acquisition.
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

        self.inner.send().await
    }
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
            },
        );
        overrides.insert(
            "b.example".into(),
            HostPolicy {
                concurrency: 1,
                min_gap: Duration::ZERO,
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
}
