use anyhow::Result;

use super::common::{env_bool, ServerConfig};

/// Configuration for the `pertisk-proxy-ingress` binary (Kubernetes Ingress controller).
#[derive(Debug, Clone)]
pub struct IngressConfig {
    pub server: ServerConfig,
    pub ingress_class: Option<String>,
    pub watch_all_namespaces: bool,
    pub watch_namespace: Option<String>,
}

impl IngressConfig {
    pub fn from_env() -> Result<Self> {
        let server = ServerConfig::from_env_ingress_defaults();
        // TLS and HTTP/3 use certs reconciled from Kubernetes Secrets (CertStore).
        // TLS_CERT_PATH / TLS_KEY_PATH are optional bootstrap fallbacks only.

        Ok(Self {
            server,
            ingress_class: std::env::var("INGRESS_CLASS").ok(),
            watch_all_namespaces: env_bool("WATCH_ALL_NAMESPACES", true),
            watch_namespace: std::env::var("WATCH_NAMESPACE").ok(),
        })
    }
}
