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
            enable_h3: crate::config::common::resolve_enable_h3(env_bool("ENABLE_H3", true)),
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
        if (self.tls_cert_path.is_some() || self.tls_key_path.is_some())
            && !(self.tls_cert_path.is_some() && self.tls_key_path.is_some())
        {
            ensure_tls_pair(&self.tls_cert_path, &self.tls_key_path, "TLS")?;
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

/// Admin config DB backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbBackendKind {
    Sqlite,
    Scylla,
}

impl DbBackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Scylla => "scylla",
        }
    }
}

/// Resolve `PERTISK_DB_BACKEND` (`sqlite` default; `scylla` / `scylladb` / `cassandra` accepted).
pub fn resolve_db_backend() -> DbBackendKind {
    match std::env::var("PERTISK_DB_BACKEND")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "" | "sqlite" | "rusqlite" => DbBackendKind::Sqlite,
        "scylla" | "scylladb" | "cassandra" => DbBackendKind::Scylla,
        other => {
            tracing::warn!(
                backend = %other,
                "unknown PERTISK_DB_BACKEND; using sqlite"
            );
            DbBackendKind::Sqlite
        }
    }
}

/// Scylla / Cassandra contact settings for the admin config DB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScyllaDbConfig {
    pub hosts: Vec<String>,
    pub keyspace: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub local_dc: Option<String>,
}

impl ScyllaDbConfig {
    pub fn from_env() -> Self {
        let hosts = resolve_scylla_hosts();
        let keyspace = std::env::var("PERTISK_SCYLLA_KEYSPACE")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "pertisk_proxy".to_string());
        let username = std::env::var("PERTISK_SCYLLA_USER")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let password = std::env::var("PERTISK_SCYLLA_PASSWORD").ok();
        let local_dc = std::env::var("PERTISK_SCYLLA_DC")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        Self {
            hosts,
            keyspace,
            username,
            password,
            local_dc,
        }
    }
}

/// Parse `PERTISK_SCYLLA_HOSTS` (comma-separated). Default `127.0.0.1:9042`.
pub fn resolve_scylla_hosts() -> Vec<String> {
    let raw = std::env::var("PERTISK_SCYLLA_HOSTS").unwrap_or_default();
    let hosts: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if hosts.is_empty() {
        vec!["127.0.0.1:9042".to_string()]
    } else {
        hosts
    }
}

/// Default on-disk data directory (certs, etc.) when not derived from a SQLite path.
pub fn default_data_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        PathBuf::from("/var/lib/pertisk-proxy")
    }
    #[cfg(not(target_os = "linux"))]
    {
        PathBuf::from("./data")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn restore(key: &str, prev: Option<String>) {
        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn resolve_db_backend_defaults_to_sqlite() {
        let _g = ENV_LOCK.lock().unwrap();
        let prev = std::env::var("PERTISK_DB_BACKEND").ok();
        std::env::remove_var("PERTISK_DB_BACKEND");
        assert_eq!(resolve_db_backend(), DbBackendKind::Sqlite);
        std::env::set_var("PERTISK_DB_BACKEND", "scylla");
        assert_eq!(resolve_db_backend(), DbBackendKind::Scylla);
        std::env::set_var("PERTISK_DB_BACKEND", "SCYLLADB");
        assert_eq!(resolve_db_backend(), DbBackendKind::Scylla);
        std::env::set_var("PERTISK_DB_BACKEND", "nope");
        assert_eq!(resolve_db_backend(), DbBackendKind::Sqlite);
        restore("PERTISK_DB_BACKEND", prev);
    }

    #[test]
    fn resolve_scylla_hosts_parses_csv_and_defaults() {
        let _g = ENV_LOCK.lock().unwrap();
        let prev = std::env::var("PERTISK_SCYLLA_HOSTS").ok();
        std::env::remove_var("PERTISK_SCYLLA_HOSTS");
        assert_eq!(resolve_scylla_hosts(), vec!["127.0.0.1:9042".to_string()]);
        std::env::set_var("PERTISK_SCYLLA_HOSTS", " 10.0.0.1:9042 , 10.0.0.2:9042 ");
        assert_eq!(
            resolve_scylla_hosts(),
            vec!["10.0.0.1:9042".to_string(), "10.0.0.2:9042".to_string()]
        );
        restore("PERTISK_SCYLLA_HOSTS", prev);
    }

    #[test]
    fn scylla_config_from_env_defaults() {
        let _g = ENV_LOCK.lock().unwrap();
        let keys = [
            "PERTISK_SCYLLA_HOSTS",
            "PERTISK_SCYLLA_KEYSPACE",
            "PERTISK_SCYLLA_USER",
            "PERTISK_SCYLLA_PASSWORD",
            "PERTISK_SCYLLA_DC",
        ];
        let prev: Vec<_> = keys
            .iter()
            .map(|k| (*k, std::env::var(k).ok()))
            .collect();
        for k in keys {
            std::env::remove_var(k);
        }
        let cfg = ScyllaDbConfig::from_env();
        assert_eq!(cfg.hosts, vec!["127.0.0.1:9042".to_string()]);
        assert_eq!(cfg.keyspace, "pertisk_proxy");
        assert!(cfg.username.is_none());
        assert!(cfg.password.is_none());
        assert!(cfg.local_dc.is_none());
        for (k, v) in prev {
            restore(k, v);
        }
    }

    #[test]
    fn scylla_config_from_env_reads_auth_and_dc() {
        let _g = ENV_LOCK.lock().unwrap();
        let keys = [
            "PERTISK_SCYLLA_HOSTS",
            "PERTISK_SCYLLA_KEYSPACE",
            "PERTISK_SCYLLA_USER",
            "PERTISK_SCYLLA_PASSWORD",
            "PERTISK_SCYLLA_DC",
        ];
        let prev: Vec<_> = keys
            .iter()
            .map(|k| (*k, std::env::var(k).ok()))
            .collect();
        std::env::set_var("PERTISK_SCYLLA_HOSTS", "db.example:9042");
        std::env::set_var("PERTISK_SCYLLA_KEYSPACE", "ks1");
        std::env::set_var("PERTISK_SCYLLA_USER", "u1");
        std::env::set_var("PERTISK_SCYLLA_PASSWORD", "p1");
        std::env::set_var("PERTISK_SCYLLA_DC", "dc1");
        let cfg = ScyllaDbConfig::from_env();
        assert_eq!(cfg.hosts, vec!["db.example:9042".to_string()]);
        assert_eq!(cfg.keyspace, "ks1");
        assert_eq!(cfg.username.as_deref(), Some("u1"));
        assert_eq!(cfg.password.as_deref(), Some("p1"));
        assert_eq!(cfg.local_dc.as_deref(), Some("dc1"));
        for (k, v) in prev {
            restore(k, v);
        }
    }

    #[test]
    fn db_backend_kind_as_str() {
        assert_eq!(DbBackendKind::Sqlite.as_str(), "sqlite");
        assert_eq!(DbBackendKind::Scylla.as_str(), "scylla");
    }
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
    #[cfg(all(
        target_os = "macos",
        not(feature = "h3-quinn"),
        not(feature = "h3-quiche")
    ))]
    {
        tracing::warn!("ENABLE_H3 is set but HTTP/3 was not compiled in");
        return false;
    }
    true
}

fn parse_port(listen: &str) -> Option<u16> {
    listen.rsplit(':').next()?.parse().ok()
}

fn ensure_tls_pair(cert: &Option<PathBuf>, key: &Option<PathBuf>, reason: &str) -> Result<()> {
    if cert.is_some() && key.is_some() {
        Ok(())
    } else {
        anyhow::bail!("{reason} requires TLS_CERT_PATH and TLS_KEY_PATH to be set")
    }
}
