//! TLS configuration types (re-exported from proxy config model).

pub use crate::proxy_config::{TlsConfig, TlsSource};

impl TlsSource {
    pub fn is_acme(&self) -> bool {
        matches!(self, Self::Acme { .. })
    }

    pub fn cert_path(&self) -> Option<&std::path::PathBuf> {
        match self {
            Self::File { cert, .. } => Some(cert),
            Self::Acme { .. } | Self::Kubernetes => None,
        }
    }

    pub fn key_path(&self) -> Option<&std::path::PathBuf> {
        match self {
            Self::File { key, .. } => Some(key),
            Self::Acme { .. } | Self::Kubernetes => None,
        }
    }
}
