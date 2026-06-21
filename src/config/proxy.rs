use std::path::PathBuf;

use anyhow::Result;

use super::common::{env_bool_parse, ServerConfig};

/// Configuration for the standalone `pertisk-proxy` binary (proxy mode).
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    pub server: ServerConfig,
    pub db_path: PathBuf,
    pub auto_https: bool,
    /// One-time migration from legacy routes.yaml when DB is empty.
    pub migrate_routes_path: Option<PathBuf>,
}

impl ProxyConfig {
    pub fn from_env() -> Result<Self> {
        let server = ServerConfig::from_env_proxy_defaults();

        let db_path = super::common::resolve_db_path();

        let migrate_routes_path = std::env::var("ROUTES_CONFIG")
            .or_else(|_| std::env::var("PERTISK_ROUTES_CONFIG"))
            .ok()
            .map(PathBuf::from)
            .filter(|p| p.is_file());

        let auto_https = std::env::var("AUTO_HTTPS")
            .ok()
            .map(|v| env_bool_parse(&v))
            .unwrap_or(false);

        let env_tls_configured =
            server.tls_cert_path.is_some() && server.tls_key_path.is_some();

        if (server.tls_cert_path.is_some() || server.tls_key_path.is_some()) && !env_tls_configured
        {
            anyhow::bail!("TLS_CERT_PATH and TLS_KEY_PATH must both be set when using global TLS");
        }

        Ok(Self {
            server,
            db_path,
            auto_https,
            migrate_routes_path,
        })
    }
}
