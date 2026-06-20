//! Per-site TLS configuration (file certs today; ACME in a follow-up).

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

/// TLS block for one or more hostnames (wildcard sharing supported).
#[derive(Debug, Clone, Deserialize)]
pub struct TlsConfig {
    pub hosts: Vec<String>,
    pub source: TlsSource,
}

/// How certificate material is obtained for the listed hosts.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TlsSource {
    /// Paths to an existing certificate and private key.
    File { cert: PathBuf, key: PathBuf },
    /// Let's Encrypt / ACME (HTTP-01 or DNS-01) — not implemented yet.
    Acme {
        #[serde(default)]
        email: Option<String>,
        #[serde(default = "default_challenge")]
        challenge: String,
        #[serde(default)]
        dns_provider: Option<String>,
        #[serde(default)]
        dns_provider_type: Option<String>,
        #[serde(default)]
        dns_credentials: Option<HashMap<String, String>>,
    },
}

fn default_challenge() -> String {
    "http01".into()
}

impl TlsSource {
    pub fn is_acme(&self) -> bool {
        matches!(self, Self::Acme { .. })
    }

    pub fn cert_path(&self) -> Option<&PathBuf> {
        match self {
            Self::File { cert, .. } => Some(cert),
            Self::Acme { .. } => None,
        }
    }

    pub fn key_path(&self) -> Option<&PathBuf> {
        match self {
            Self::File { key, .. } => Some(key),
            Self::Acme { .. } => None,
        }
    }
}
