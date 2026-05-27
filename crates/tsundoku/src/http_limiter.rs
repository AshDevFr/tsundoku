//! Build the process-wide [`td_http::HttpLimiter`] from the figment-shaped
//! [`td_config::HttpConfig`]. Lives in the binary crate so `td-config` and
//! `td-http` stay independent of one another (same pattern we already use
//! for `NyaaSourceConfig` vs `td_config::NyaaSourceOptions`).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use td_config::HttpConfig;
use td_http::{HostPolicy, HttpLimiter};

pub fn build(cfg: &HttpConfig) -> Arc<HttpLimiter> {
    let default_policy = HostPolicy {
        concurrency: cfg.default_concurrency.max(1) as usize,
        min_gap: Duration::from_millis(cfg.default_min_gap_ms),
    };
    let mut overrides: HashMap<String, HostPolicy> = HashMap::with_capacity(cfg.hosts.len());
    for host in &cfg.hosts {
        overrides.insert(
            host.host.clone(),
            HostPolicy {
                concurrency: host.concurrency.max(1) as usize,
                min_gap: Duration::from_millis(host.min_gap_ms),
            },
        );
    }
    Arc::new(HttpLimiter::new(default_policy, overrides))
}
