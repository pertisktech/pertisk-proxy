use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use anyhow::{Context, Result};
use tracing::{info, warn};

use super::config::{TlsConfig, TlsSource};
use super::validate;

#[derive(Debug, Clone)]
pub struct CertPaths {
    pub cert: PathBuf,
    pub key: PathBuf,
}

#[derive(Default)]
struct CertStoreInner {
    by_host: HashMap<String, CertPaths>,
    default: Option<CertPaths>,
}

/// In-memory map of hostname -> certificate file paths.
#[derive(Clone, Default)]
pub struct CertStore {
    inner: std::sync::Arc<RwLock<CertStoreInner>>,
}

impl CertStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.inner
            .read()
            .map(|g| g.by_host.is_empty())
            .unwrap_or(true)
    }

    pub fn host_count(&self) -> usize {
        self.inner
            .read()
            .map(|g| g.by_host.len())
            .unwrap_or(0)
    }

    /// Load a global fallback certificate from environment variables.
    pub fn set_global_fallback(&self, cert: PathBuf, key: PathBuf) -> Result<()> {
        validate_cert_pair(&cert, &key).context("invalid global TLS_CERT_PATH/TLS_KEY_PATH")?;
        let paths = CertPaths { cert, key };
        if let Ok(mut g) = self.inner.write() {
            if g.default.is_none() {
                g.default = Some(paths);
            }
        }
        Ok(())
    }

    /// Replace site TLS entries from `routes.yaml` `tls:` blocks.
    pub fn reload_from_configs(&self, configs: &[TlsConfig]) -> Result<()> {
        let mut by_host = HashMap::new();
        let mut default = None;
        let mut acme_pending = 0usize;

        for entry in configs {
            if entry.hosts.is_empty() {
                warn!("tls entry has no hosts; skipping");
                continue;
            }

            match &entry.source {
                TlsSource::File { cert, key } => {
                    validate_cert_pair(cert, key)
                        .with_context(|| format!("invalid TLS file config for {:?}", entry.hosts))?;
                    validate::warn_host_cert_mismatch(cert, &entry.hosts)?;
                    let paths = CertPaths {
                        cert: cert.clone(),
                        key: key.clone(),
                    };
                    if default.is_none() {
                        default = Some(paths.clone());
                    }
                    for host in &entry.hosts {
                        by_host.insert(normalize_host(host), paths.clone());
                    }
                    info!(
                        hosts = ?entry.hosts,
                        cert = %cert.display(),
                        "loaded site TLS certificate"
                    );
                }
                TlsSource::Acme { .. } => {
                    acme_pending += entry.hosts.len();
                    warn!(
                        hosts = ?entry.hosts,
                        "ACME TLS is not implemented yet; use type: file or TLS_CERT_PATH until ACME lands"
                    );
                }
            }
        }

        if acme_pending > 0 {
            info!(
                hosts = acme_pending,
                "ACME certificates pending — automatic issuance will be added in a future release"
            );
        }

        if let Ok(mut g) = self.inner.write() {
            g.by_host = by_host;
            if default.is_some() {
                g.default = default;
            }
        }

        Ok(())
    }

    pub fn default_paths(&self) -> Option<CertPaths> {
        let g = self.inner.read().ok()?;
        g.default
            .clone()
            .or_else(|| g.by_host.values().next().cloned())
    }

    pub fn has_cert_for_host(&self, host: &str) -> bool {
        self.get(host).is_some()
    }

    pub fn get(&self, host: &str) -> Option<CertPaths> {
        let g = self.inner.read().ok()?;
        let host = normalize_host(host);
        if let Some(paths) = g.by_host.get(&host) {
            return Some(paths.clone());
        }

        let mut best: Option<(usize, CertPaths)> = None;
        for (key, paths) in &g.by_host {
            if let Some(suffix) = key.strip_prefix("*.") {
                let suffix = format!(".{suffix}");
                if host.ends_with(&suffix) && host.len() > suffix.len() {
                    let len = suffix.len();
                    if best.as_ref().map(|(l, _)| *l).unwrap_or(0) < len {
                        best = Some((len, paths.clone()));
                    }
                }
            }
        }
        if let Some((_, paths)) = best {
            return Some(paths);
        }

        g.default.clone()
    }
}

fn validate_cert_pair(cert: &Path, key: &Path) -> Result<()> {
    if !cert.is_file() {
        anyhow::bail!("certificate file not found: {}", cert.display());
    }
    if !key.is_file() {
        anyhow::bail!("private key file not found: {}", key.display());
    }
    Ok(())
}

fn normalize_host(host: &str) -> String {
    host.split(':').next().unwrap_or(host).to_ascii_lowercase()
}
