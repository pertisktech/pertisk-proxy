use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub http_listen: String,
    pub https_listen: String,
    pub h3_udp_listen: String,
    pub enable_h3: bool,
    pub tls_cert_path: Option<PathBuf>,
    pub tls_key_path: Option<PathBuf>,
    pub ingress_class: Option<String>,
    pub watch_all_namespaces: bool,
    pub watch_namespace: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let enable_h3 = env_bool("ENABLE_H3", true);
        let tls_cert_path = env::var("TLS_CERT_PATH").ok().map(PathBuf::from);
        let tls_key_path = env::var("TLS_KEY_PATH").ok().map(PathBuf::from);

        if enable_h3 {
            match (&tls_cert_path, &tls_key_path) {
                (Some(_), Some(_)) => {}
                _ => anyhow::bail!(
                    "ENABLE_H3 requires TLS_CERT_PATH and TLS_KEY_PATH to be set"
                ),
            }
        }

        Ok(Self {
            http_listen: env::var("LISTEN_HTTP").unwrap_or_else(|_| "0.0.0.0:8080".into()),
            https_listen: env::var("LISTEN_HTTPS").unwrap_or_else(|_| "0.0.0.0:8443".into()),
            h3_udp_listen: env::var("LISTEN_H3_UDP").unwrap_or_else(|_| "0.0.0.0:8443".into()),
            enable_h3,
            tls_cert_path,
            tls_key_path,
            ingress_class: env::var("INGRESS_CLASS").ok(),
            watch_all_namespaces: env_bool("WATCH_ALL_NAMESPACES", true),
            watch_namespace: env::var("WATCH_NAMESPACE").ok(),
        })
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

fn env_bool(key: &str, default: bool) -> bool {
    match env::var(key) {
        Ok(value) => matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"),
        Err(_) => default,
    }
}
