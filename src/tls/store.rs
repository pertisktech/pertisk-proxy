use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

use anyhow::{Context, Result};
use tracing::{info, warn};

use super::config::{TlsConfig, TlsSource};
use super::validate::{self, validate_cert_pair as validate_tls_pair, validate_cert_pair_pem};

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
        validate_tls_pair(&cert, &key).context("invalid global TLS_CERT_PATH/TLS_KEY_PATH")?;
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
                    validate_tls_pair(cert, key)
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
                }
                TlsSource::Kubernetes => {}
            }
        }

        if acme_pending > 0 {
            info!(
                hosts = acme_pending,
                "ACME TLS configured; automatic issuance runs in background"
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
        self.lookup_sni(host).is_some()
    }

    /// Exact hostname or wildcard match only (no global default fallback).
    pub fn lookup_sni(&self, host: &str) -> Option<CertPaths> {
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
        best.map(|(_, paths)| paths)
    }

    /// Register certificate file paths for the given hostnames.
    pub fn insert_paths_for_hosts(&self, hosts: &[String], paths: CertPaths) {
        if let Ok(mut g) = self.inner.write() {
            if g.default.is_none() {
                g.default = Some(paths.clone());
            }
            for host in hosts {
                let host = host.trim();
                if !host.is_empty() {
                    g.by_host.insert(normalize_host(host), paths.clone());
                }
            }
        }
    }

    /// Remove host mappings (e.g. after deleting a DB certificate).
    pub fn remove_for_hosts(&self, hosts: &[String]) {
        if let Ok(mut g) = self.inner.write() {
            for host in hosts {
                g.by_host.remove(&normalize_host(host.trim()));
            }
        }
    }

    /// Write PEM material to disk and map it to hostnames.
    pub fn insert_pem_for_hosts(
        &self,
        hosts: &[String],
        cert_pem: &[u8],
        key_pem: &[u8],
        certs_dir: &std::path::Path,
        id: &str,
    ) -> Result<()> {
        validate_cert_pair_pem(cert_pem, key_pem).context("invalid certificate PEM")?;
        std::fs::create_dir_all(certs_dir)?;
        let cert_path = certs_dir.join(format!("{id}.pem"));
        let key_path = certs_dir.join(format!("{id}.key"));
        std::fs::write(&cert_path, cert_pem)?;
        std::fs::write(&key_path, key_pem)?;
        validate_tls_pair(&cert_path, &key_path)?;
        validate::warn_host_cert_mismatch(&cert_path, hosts)?;
        self.insert_paths_for_hosts(
            hosts,
            CertPaths {
                cert: cert_path,
                key: key_path,
            },
        );
        Ok(())
    }

    pub fn get(&self, host: &str) -> Option<CertPaths> {
        self.lookup_sni(host).or_else(|| {
            self.inner
                .read()
                .ok()
                .and_then(|g| g.default.clone())
        })
    }
}

fn normalize_host(host: &str) -> String {
    host.split(':').next().unwrap_or(host).to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_sni_does_not_use_unrelated_default() {
        let store = CertStore::new();
        store.insert_paths_for_hosts(
            &["*.amd.thaidevops.co".into()],
            CertPaths {
                cert: "/amd.pem".into(),
                key: "/amd.key".into(),
            },
        );
        store.insert_paths_for_hosts(
            &["*.apps.pertisk.com".into()],
            CertPaths {
                cert: "/apps.pem".into(),
                key: "/apps.key".into(),
            },
        );
        assert_eq!(
            store.lookup_sni("gitlab.apps.pertisk.com").unwrap().cert,
            PathBuf::from("/apps.pem")
        );
        assert!(store.lookup_sni("unknown.example.com").is_none());
        // get() still falls back to default for clients without a matching SNI entry.
        assert!(store.get("unknown.example.com").is_some());
    }
}
