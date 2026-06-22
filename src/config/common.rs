//! Shared server settings (listen addresses, TLS, HTTP/3).

use std::path::PathBuf;

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub http_listen: String,
    pub https_listen: String,
    pub h3_udp_listen: String,
    pub enable_h3: bool,
    pub enable_h2: bool,
    pub tls_cert_path: Option<PathBuf>,
    pub tls_key_path: Option<PathBuf>,
}

impl ServerConfig {
    pub fn from_env_proxy_defaults() -> Self {
        Self {
            http_listen: env_listen("LISTEN_HTTP", "PERTISK_HTTP_ADDR", "[::]:80"),
            https_listen: env_listen("LISTEN_HTTPS", "PERTISK_HTTPS_ADDR", "[::]:443"),
            h3_udp_listen: env_listen("LISTEN_H3_UDP", "PERTISK_HTTP3_ADDR", "[::]:443"),
            enable_h3: crate::config::common::resolve_enable_h3(env_bool("ENABLE_H3", false)),
            enable_h2: env_bool("PERTISK_ENABLE_H2", true),
            tls_cert_path: std::env::var("TLS_CERT_PATH").ok().map(PathBuf::from),
            tls_key_path: std::env::var("TLS_KEY_PATH").ok().map(PathBuf::from),
        }
    }

    pub fn from_env_ingress_defaults() -> Self {
        Self {
            http_listen: env_listen("LISTEN_HTTP", "PERTISK_HTTP_ADDR", "0.0.0.0:8080"),
            https_listen: env_listen("LISTEN_HTTPS", "PERTISK_HTTPS_ADDR", "0.0.0.0:8443"),
            h3_udp_listen: env_listen("LISTEN_H3_UDP", "PERTISK_HTTP3_ADDR", "[::]:8443"),
            enable_h3: crate::config::common::resolve_enable_h3(env_bool("ENABLE_H3", true)),
            enable_h2: env_bool("PERTISK_ENABLE_H2", true),
            tls_cert_path: std::env::var("TLS_CERT_PATH").ok().map(PathBuf::from),
            tls_key_path: std::env::var("TLS_KEY_PATH").ok().map(PathBuf::from),
        }
    }

    pub fn validate_tls(&self) -> Result<()> {
        if self.enable_h3 {
            ensure_tls_pair(&self.tls_cert_path, &self.tls_key_path, "ENABLE_H3")?;
        }
        Ok(())
    }

    pub fn https_port(&self) -> u16 {
        parse_port(&self.https_listen).unwrap_or(443)
    }

    pub fn h3_port(&self) -> u16 {
        parse_port(&self.h3_udp_listen).unwrap_or(self.https_port())
    }

    /// Port clients should use for QUIC (Alt-Svc). May differ from the in-container UDP bind port.
    pub fn http3_advertised_port(&self) -> u16 {
        std::env::var("PERTISK_HTTP3_ADVERTISED_PORT")
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .filter(|p| *p > 0)
            .unwrap_or_else(|| self.https_port())
    }

    pub fn tls_cert_path(&self) -> Result<&str> {
        self.tls_cert_path
            .as_ref()
            .context("TLS_CERT_PATH is not configured")?
            .to_str()
            .context("TLS_CERT_PATH is not valid UTF-8")
    }

    pub fn tls_key_path(&self) -> Result<&str> {
        self.tls_key_path
            .as_ref()
            .context("TLS_KEY_PATH is not configured")?
            .to_str()
            .context("TLS_KEY_PATH is not valid UTF-8")
    }
}

pub fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.into())
}

/// Read listen address from `LISTEN_*` or legacy `PERTISK_*_ADDR` (Helm chart).
pub fn env_listen(listen_key: &str, pertisk_key: &str, default: &str) -> String {
    std::env::var(listen_key)
        .or_else(|_| std::env::var(pertisk_key))
        .unwrap_or_else(|_| default.into())
}

pub fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(value) => env_bool_parse(&value),
        Err(_) => default,
    }
}

pub fn env_bool_parse(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes")
}

/// Default SQLite path when `PERTISK_DB_PATH` is unset.
pub fn default_db_path() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        PathBuf::from("/var/lib/pertisk-proxy/proxy.sqlite")
    }
    #[cfg(not(target_os = "linux"))]
    {
        PathBuf::from("./data/proxy.sqlite")
    }
}

pub fn resolve_db_path() -> PathBuf {
    std::env::var("PERTISK_DB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_db_path())
}

/// HTTP/3 (Quiche/BoringSSL) conflicts with ACME (OpenSSL) when both are linked.
pub fn resolve_enable_h3(requested: bool) -> bool {
    if !requested {
        return false;
    }
    #[cfg(all(feature = "acme", feature = "h3-quiche"))]
    {
        tracing::warn!(
            "ENABLE_H3 is set but HTTP/3 (Quiche) is disabled when ACME is enabled; \
             rebuild with --no-default-features --features admin,h3-quiche for Quiche-only HTTP/3, \
             or use the default h3-quinn backend on Linux"
        );
        return false;
    }
    #[cfg(all(target_os = "macos", not(feature = "h3-quinn"), not(feature = "h3-quiche")))]
    {
        tracing::warn!("ENABLE_H3 is set but HTTP/3 was not compiled in");
        return false;
    }
    true
}

fn parse_port(listen: &str) -> Option<u16> {
    listen.rsplit(':').next()?.parse().ok()
}

fn ensure_tls_pair(
    cert: &Option<PathBuf>,
    key: &Option<PathBuf>,
    reason: &str,
) -> Result<()> {
    if cert.is_some() && key.is_some() {
        Ok(())
    } else {
        anyhow::bail!("{reason} requires TLS_CERT_PATH and TLS_KEY_PATH to be set")
    }
}
