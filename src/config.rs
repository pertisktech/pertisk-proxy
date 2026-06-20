use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::mode::{parse_operating_mode, OperatingMode, ProxyKind};

#[derive(Debug, Clone)]
pub struct Config {
    pub mode: OperatingMode,
    pub http_listen: String,
    pub https_listen: String,
    pub h3_udp_listen: String,
    pub enable_h3: bool,
    pub tls_cert_path: Option<PathBuf>,
    pub tls_key_path: Option<PathBuf>,
    /// Path to pertisk routes file (required in proxy modes).
    pub routes_config: Option<PathBuf>,
    /// Reload routes file when it changes (default varies by proxy kind).
    pub routes_watch: bool,
    /// Caddy-style automatic HTTP→HTTPS redirect.
    pub auto_https: bool,
    pub ingress_class: Option<String>,
    pub watch_all_namespaces: bool,
    pub watch_namespace: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let mode = parse_operating_mode()?;
        let enable_h3 = env_bool("ENABLE_H3", true);
        let tls_cert_path = std::env::var("TLS_CERT_PATH").ok().map(PathBuf::from);
        let tls_key_path = std::env::var("TLS_KEY_PATH").ok().map(PathBuf::from);

        let (routes_config, routes_watch, auto_https, http_listen, https_listen) =
            apply_mode_defaults(&mode)?;

        if enable_h3 {
            ensure_tls(&tls_cert_path, &tls_key_path, "ENABLE_H3")?;
        }

        let needs_tls = enable_h3
            || tls_cert_path.is_some()
            || tls_key_path.is_some()
            || auto_https;

        if needs_tls && (tls_cert_path.is_none() || tls_key_path.is_none()) {
            anyhow::bail!("TLS_CERT_PATH and TLS_KEY_PATH must both be set when TLS is enabled");
        }

        Ok(Self {
            mode,
            http_listen,
            https_listen: env::var("LISTEN_HTTPS").unwrap_or(https_listen),
            h3_udp_listen: env_or("LISTEN_H3_UDP", "0.0.0.0:8443"),
            enable_h3,
            tls_cert_path,
            tls_key_path,
            routes_config,
            routes_watch: env::var("ROUTES_WATCH")
                .ok()
                .map(|v| env_bool_parse(&v))
                .unwrap_or(routes_watch),
            auto_https: env::var("AUTO_HTTPS")
                .ok()
                .map(|v| env_bool_parse(&v))
                .unwrap_or(auto_https),
            ingress_class: std::env::var("INGRESS_CLASS").ok(),
            watch_all_namespaces: env_bool("WATCH_ALL_NAMESPACES", true),
            watch_namespace: std::env::var("WATCH_NAMESPACE").ok(),
        })
    }

    pub fn is_ingress_mode(&self) -> bool {
        matches!(self.mode, OperatingMode::Ingress)
    }

    pub fn https_port(&self) -> u16 {
        parse_port(&self.https_listen).unwrap_or(443)
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

fn apply_mode_defaults(
    mode: &OperatingMode,
) -> Result<(Option<PathBuf>, bool, bool, String, String)> {
    match mode {
        OperatingMode::Ingress => Ok((
            None,
            false,
            false,
            env_or("LISTEN_HTTP", "0.0.0.0:8080"),
            env_or("LISTEN_HTTPS", "0.0.0.0:8443"),
        )),
        OperatingMode::Proxy(kind) => {
            let routes_config = Some(
                std::env::var("ROUTES_CONFIG")
                    .or_else(|_| std::env::var("PERTISK_CONFIG"))
                    .context("ROUTES_CONFIG is required in proxy mode")?
                    .into(),
            );

            let (http_listen, https_listen, auto_https) = match kind {
                ProxyKind::Nginx => (
                    env_or("LISTEN_HTTP", "0.0.0.0:80"),
                    env_or("LISTEN_HTTPS", "0.0.0.0:443"),
                    false,
                ),
                ProxyKind::Caddy => (
                    env_or("LISTEN_HTTP", "0.0.0.0:80"),
                    env_or("LISTEN_HTTPS", "0.0.0.0:443"),
                    true,
                ),
                ProxyKind::Traefik => (
                    env_or("LISTEN_HTTP", "0.0.0.0:80"),
                    env_or("LISTEN_HTTPS", "0.0.0.0:443"),
                    false,
                ),
            };

            Ok((
                routes_config,
                kind.default_routes_watch(),
                auto_https,
                env::var("LISTEN_HTTP").unwrap_or(http_listen),
                https_listen,
            ))
        }
    }
}

fn parse_port(listen: &str) -> Option<u16> {
    listen.rsplit(':').next()?.parse().ok()
}

fn ensure_tls(
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

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.into())
}

fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(value) => env_bool_parse(&value),
        Err(_) => default,
    }
}

fn env_bool_parse(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes")
}
