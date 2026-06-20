use std::path::PathBuf;

use anyhow::{Context, Result};

use super::common::{env_bool, env_bool_parse, ServerConfig};

/// Configuration for the standalone `pertisk-proxy` binary (proxy mode).
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    pub server: ServerConfig,
    pub routes_config: PathBuf,
    pub routes_watch: bool,
    pub auto_https: bool,
}

impl ProxyConfig {
    pub fn from_env() -> Result<Self> {
        let server = ServerConfig::from_env_proxy_defaults();

        let routes_config = std::env::var("ROUTES_CONFIG")
            .or_else(|_| std::env::var("PERTISK_ROUTES_CONFIG"))
            .context("ROUTES_CONFIG is required for pertisk-proxy (proxy mode)")?
            .into();

        let auto_https = std::env::var("AUTO_HTTPS")
            .ok()
            .map(|v| env_bool_parse(&v))
            .unwrap_or(false);

        let env_tls_configured =
            server.tls_cert_path.is_some() && server.tls_key_path.is_some();

        if server.enable_h3 && !env_tls_configured {
            // Site TLS from routes.yaml may still enable H3; main.rs resolves that after load.
            if server.tls_cert_path.is_some() || server.tls_key_path.is_some() {
                anyhow::bail!("TLS_CERT_PATH and TLS_KEY_PATH must both be set when using global TLS");
            }
        }

        if (server.tls_cert_path.is_some() || server.tls_key_path.is_some()) && !env_tls_configured
        {
            anyhow::bail!("TLS_CERT_PATH and TLS_KEY_PATH must both be set when using global TLS");
        }

        Ok(Self {
            server,
            routes_config,
            routes_watch: std::env::var("ROUTES_WATCH")
                .ok()
                .map(|v| env_bool_parse(&v))
                .unwrap_or_else(|| env_bool("ROUTES_CONFIG_WATCH", true)),
            auto_https,
        })
    }
}
