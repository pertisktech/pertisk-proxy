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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::RuntimeMode;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn runtime_config(resolved_mode: RuntimeMode) -> RuntimeConfig {
        RuntimeConfig {
            requested_mode: resolved_mode,
            resolved_mode,
            worker_threads: 2,
            max_blocking_threads: 2,
        }
    }

    fn restore_env_var(key: &str, previous: Option<String>) {
        match previous {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    fn clear_pool_env() -> Vec<(&'static str, Option<String>)> {
        let keys = [
            "PERTISK_H3_UPSTREAM_POOL_MAX_IDLE",
            "PERTISK_H3_UPSTREAM_POOL_IDLE_TIMEOUT_SECS",
            "PERTISK_H3_UPSTREAM_TCP_KEEPALIVE_SECS",
        ];
        let previous = keys
            .iter()
            .map(|key| (*key, std::env::var(key).ok()))
            .collect::<Vec<_>>();
        for key in keys {
            std::env::remove_var(key);
        }
        previous
    }

    fn restore_pool_env(previous: Vec<(&'static str, Option<String>)>) {
        for (key, value) in previous {
            restore_env_var(key, value);
        }
    }

    #[test]
    fn upstream_pool_config_uses_standard_defaults() {
        let _guard = ENV_LOCK.lock().unwrap();
        let previous = clear_pool_env();

        let cfg = upstream_pool_config(&runtime_config(RuntimeMode::Standard));

        assert_eq!(cfg.max_idle_per_host, 64);
        assert_eq!(cfg.idle_timeout_secs, 90);
        assert_eq!(cfg.tcp_keepalive_secs, 60);

        restore_pool_env(previous);
    }

    #[test]
    fn upstream_pool_config_uses_performance_defaults() {
        let _guard = ENV_LOCK.lock().unwrap();
        let previous = clear_pool_env();

        let cfg = upstream_pool_config(&runtime_config(RuntimeMode::Performance));

        assert_eq!(cfg.max_idle_per_host, 256);
        assert_eq!(cfg.idle_timeout_secs, 120);
        assert_eq!(cfg.tcp_keepalive_secs, 60);

        restore_pool_env(previous);
    }

    #[test]
    fn upstream_pool_config_uses_env_overrides() {
        let _guard = ENV_LOCK.lock().unwrap();
        let previous = clear_pool_env();
        std::env::set_var("PERTISK_H3_UPSTREAM_POOL_MAX_IDLE", "10");
        std::env::set_var("PERTISK_H3_UPSTREAM_POOL_IDLE_TIMEOUT_SECS", "11");
        std::env::set_var("PERTISK_H3_UPSTREAM_TCP_KEEPALIVE_SECS", "12");

        let cfg = upstream_pool_config(&runtime_config(RuntimeMode::Standard));

        assert_eq!(cfg.max_idle_per_host, 10);
        assert_eq!(cfg.idle_timeout_secs, 11);
        assert_eq!(cfg.tcp_keepalive_secs, 12);

        restore_pool_env(previous);
    }

    #[test]
    fn upstream_pool_config_filters_invalid_and_zero_env_values() {
        let _guard = ENV_LOCK.lock().unwrap();
        let previous = clear_pool_env();
        std::env::set_var("PERTISK_H3_UPSTREAM_POOL_MAX_IDLE", "0");
        std::env::set_var("PERTISK_H3_UPSTREAM_POOL_IDLE_TIMEOUT_SECS", "invalid");
        std::env::set_var("PERTISK_H3_UPSTREAM_TCP_KEEPALIVE_SECS", "0");

        let cfg = upstream_pool_config(&runtime_config(RuntimeMode::Standard));

        assert_eq!(cfg.max_idle_per_host, 64);
        assert_eq!(cfg.idle_timeout_secs, 90);
        assert_eq!(cfg.tcp_keepalive_secs, 60);

        restore_pool_env(previous);
    }

    #[test]
    fn build_upstream_client_succeeds() {
        let _guard = ENV_LOCK.lock().unwrap();
        let previous = clear_pool_env();

        let client = build_upstream_client(&runtime_config(RuntimeMode::Standard));

        assert!(client.is_ok());

        restore_pool_env(previous);
    }
}
