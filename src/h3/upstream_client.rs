use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::Client;

use crate::runtime::{is_performance_mode, RuntimeConfig};

#[derive(Debug, Clone, Copy)]
pub struct H3UpstreamPoolConfig {
    pub max_idle_per_host: usize,
    pub idle_timeout_secs: u64,
    pub tcp_keepalive_secs: u64,
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|n| *n > 0)
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|n| *n > 0)
}

/// HTTP client for H3 → upstream hops. Does not follow redirects so 302/307 responses
/// (e.g. OIDC login to Auth0) reach the browser unchanged, matching Pingora/H2 behavior.
///
/// Pool size scales with `PERTISK_*_MODE=performance` (or overrides below).
/// - `PERTISK_H3_UPSTREAM_POOL_MAX_IDLE` — max idle connections per host
/// - `PERTISK_H3_UPSTREAM_POOL_IDLE_TIMEOUT_SECS` — idle pool timeout
/// - `PERTISK_H3_UPSTREAM_TCP_KEEPALIVE_SECS` — TCP keepalive interval
pub fn upstream_pool_config(runtime_cfg: &RuntimeConfig) -> H3UpstreamPoolConfig {
    let performance = is_performance_mode(runtime_cfg);
    H3UpstreamPoolConfig {
        max_idle_per_host: env_usize("PERTISK_H3_UPSTREAM_POOL_MAX_IDLE")
            .unwrap_or(if performance { 256 } else { 64 }),
        idle_timeout_secs: env_u64("PERTISK_H3_UPSTREAM_POOL_IDLE_TIMEOUT_SECS")
            .unwrap_or(if performance { 120 } else { 90 }),
        tcp_keepalive_secs: env_u64("PERTISK_H3_UPSTREAM_TCP_KEEPALIVE_SECS").unwrap_or(60),
    }
}

pub fn build_upstream_client(runtime_cfg: &RuntimeConfig) -> Result<Client> {
    let pool = upstream_pool_config(runtime_cfg);

    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .pool_max_idle_per_host(pool.max_idle_per_host)
        .pool_idle_timeout(Duration::from_secs(pool.idle_timeout_secs))
        .tcp_keepalive(Duration::from_secs(pool.tcp_keepalive_secs))
        .danger_accept_invalid_certs(true)
        .build()
        .context("build H3 upstream client")
}
