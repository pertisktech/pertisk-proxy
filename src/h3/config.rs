use crate::config::ServerConfig;

#[derive(Clone)]
pub struct H3Config {
    pub udp_listen: String,
    pub tls_cert_path: String,
    pub tls_key_path: String,
}

impl H3Config {
    pub fn from_tls_paths(cert: impl Into<String>, key: impl Into<String>, udp_listen: String) -> Self {
        Self {
            udp_listen,
            tls_cert_path: cert.into(),
            tls_key_path: key.into(),
        }
    }

    pub fn from_server(server: &ServerConfig) -> anyhow::Result<Self> {
        Ok(Self {
            udp_listen: server.h3_udp_listen.clone(),
            tls_cert_path: server.tls_cert_path()?.to_string(),
            tls_key_path: server.tls_key_path()?.to_string(),
        })
    }
}
