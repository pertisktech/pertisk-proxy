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
        server.validate_tls()?;

        if server.enable_h3
            && (server.tls_cert_path.is_none() || server.tls_key_path.is_none())
        {
            anyhow::bail!("ENABLE_H3 requires TLS_CERT_PATH and TLS_KEY_PATH");
        }

        Ok(Self {
            server,
            ingress_class: std::env::var("INGRESS_CLASS").ok(),
            watch_all_namespaces: env_bool("WATCH_ALL_NAMESPACES", true),
            watch_namespace: std::env::var("WATCH_NAMESPACE").ok(),
        })
    }
}
