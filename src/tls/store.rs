use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

use anyhow::{Context, Result};
use tracing::{info, warn};

use super::config::{TlsConfig, TlsSource};
use super::validate::{self, validate_cert_pair as validate_tls_pair, validate_cert_pair_pem};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertPaths {
    pub cert: PathBuf,
    pub key: PathBuf,
}

/// Certificate material held in the store (on disk or in memory).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoredCert {
    File(CertPaths),
    Pem { cert: Vec<u8>, key: Vec<u8> },
}

impl StoredCert {
    pub fn read_pem(&self) -> Result<(Vec<u8>, Vec<u8>)> {
        match self {
            StoredCert::File(paths) => {
                let cert = std::fs::read(&paths.cert)
                    .with_context(|| format!("read cert {}", paths.cert.display()))?;
                let key = std::fs::read(&paths.key)
                    .with_context(|| format!("read key {}", paths.key.display()))?;
                Ok((cert, key))
            }
            StoredCert::Pem { cert, key } => Ok((cert.clone(), key.clone())),
        }
    }
}

#[derive(Default)]
struct CertStoreInner {
    by_host: HashMap<String, StoredCert>,
    default: Option<StoredCert>,
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
                g.default = Some(StoredCert::File(paths));
            }
        }
        Ok(())
    }

    /// Replace site TLS entries from `routes.yaml` `tls:` blocks.
    pub fn reload_from_configs(&self, configs: &[TlsConfig]) -> Result<()> {
        let mut by_host = HashMap::new();
        let mut default: Option<StoredCert> = None;
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
                        default = Some(StoredCert::File(paths.clone()));
                    }
                    for host in &entry.hosts {
                        by_host.insert(normalize_host(host), StoredCert::File(paths.clone()));
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

    pub fn default_cert(&self) -> Option<StoredCert> {
        let g = self.inner.read().ok()?;
        g.default
            .clone()
            .or_else(|| g.by_host.values().next().cloned())
    }

    pub fn default_paths(&self) -> Option<CertPaths> {
        let g = self.inner.read().ok()?;
        match g
            .default
            .as_ref()
            .or_else(|| g.by_host.values().next())
        {
            Some(StoredCert::File(paths)) => Some(paths.clone()),
            _ => None,
        }
    }

    pub fn has_cert_for_host(&self, host: &str) -> bool {
        self.lookup_sni(host).is_some()
    }

    /// True when any certificate is loaded (per-host or global default).
    pub fn has_any_cert(&self) -> bool {
        self.host_count() > 0 || self.default_paths().is_some()
    }

    /// Exact hostname or wildcard match only (no global default fallback).
    pub fn lookup_sni(&self, host: &str) -> Option<StoredCert> {
        let g = self.inner.read().ok()?;
        let host = normalize_host(host);
        if let Some(cert) = g.by_host.get(&host) {
            return Some(cert.clone());
        }

        let mut best: Option<(usize, StoredCert)> = None;
        for (key, cert) in &g.by_host {
            if let Some(suffix) = key.strip_prefix("*.") {
                let suffix = format!(".{suffix}");
                if host.ends_with(&suffix) && host.len() > suffix.len() {
                    let len = suffix.len();
                    if best.as_ref().map(|(l, _)| *l).unwrap_or(0) < len {
                        best = Some((len, cert.clone()));
                    }
                }
            }
        }
        best.map(|(_, cert)| cert)
    }

    fn insert_stored_for_hosts(&self, hosts: &[String], cert: StoredCert) {
        if let Ok(mut g) = self.inner.write() {
            if g.default.is_none() {
                g.default = Some(cert.clone());
            }
            for host in hosts {
                let host = host.trim();
                if !host.is_empty() {
                    g.by_host.insert(normalize_host(host), cert.clone());
                }
            }
        }
    }

    /// Register certificate file paths for the given hostnames.
    pub fn insert_paths_for_hosts(&self, hosts: &[String], paths: CertPaths) {
        self.insert_stored_for_hosts(hosts, StoredCert::File(paths));
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

    /// Keep PEM material in memory (for Kubernetes Secrets; no filesystem write).
    pub fn insert_pem_in_memory_for_hosts(
        &self,
        hosts: &[String],
        cert_pem: &[u8],
        key_pem: &[u8],
    ) -> Result<()> {
        validate_cert_pair_pem(cert_pem, key_pem).context("invalid certificate PEM")?;
        validate::warn_host_cert_mismatch_pem(cert_pem, hosts)?;
        self.insert_stored_for_hosts(
            hosts,
            StoredCert::Pem {
                cert: cert_pem.to_vec(),
                key: key_pem.to_vec(),
            },
        );
        Ok(())
    }

    pub fn get(&self, host: &str) -> Option<StoredCert> {
        self.lookup_sni(host)
            .or_else(|| self.default_cert())
    }
}

fn normalize_host(host: &str) -> String {
    host.split(':').next().unwrap_or(host).to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{CertificateParams, KeyPair};
    use crate::tls::config::{TlsConfig, TlsSource};

    fn test_pem() -> (Vec<u8>, Vec<u8>) {
        let key_pair = KeyPair::generate().unwrap();
        let params = CertificateParams::new(vec!["example.com".to_string()]).unwrap();
        let cert = params.self_signed(&key_pair).unwrap();
        (cert.pem().into_bytes(), key_pair.serialize_pem().into_bytes())
    }

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
            store.lookup_sni("gitlab.apps.pertisk.com").unwrap(),
            StoredCert::File(CertPaths {
                cert: "/apps.pem".into(),
                key: "/apps.key".into(),
            })
        );
        assert!(store.lookup_sni("unknown.example.com").is_none());
        assert!(store.get("unknown.example.com").is_some());
    }

    #[test]
    fn store_basic_operations() {
        let store = CertStore::new();
        assert!(store.is_empty());
        assert_eq!(store.host_count(), 0);

        store.insert_paths_for_hosts(
            &["app.example.com".into()],
            CertPaths {
                cert: "/c.pem".into(),
                key: "/k.key".into(),
            },
        );
        assert!(!store.is_empty());
        assert_eq!(store.host_count(), 1);
        assert!(store.has_cert_for_host("app.example.com"));
        assert_eq!(store.default_paths().unwrap().cert, PathBuf::from("/c.pem"));

        store.remove_for_hosts(&["app.example.com".into()]);
        assert!(store.is_empty());
    }

    #[test]
    fn insert_pem_in_memory() {
        let (cert, key) = test_pem();
        let store = CertStore::new();
        store
            .insert_pem_in_memory_for_hosts(
                &["mem.example.com".into()],
                &cert,
                &key,
            )
            .unwrap();
        let stored = store.lookup_sni("mem.example.com").unwrap();
        let (c, k) = stored.read_pem().unwrap();
        assert_eq!(c, cert);
        assert_eq!(k, key);
        assert!(store.default_cert().is_some());
    }

    #[test]
    fn insert_pem_to_disk() {
        let (cert, key) = test_pem();
        let store = CertStore::new();
        let dir = tempfile::tempdir().unwrap();
        store
            .insert_pem_for_hosts(
                &["disk.example.com".into()],
                &cert,
                &key,
                dir.path(),
                "site-1",
            )
            .unwrap();
        assert!(store.lookup_sni("disk.example.com").is_some());
    }

    #[test]
    fn reload_from_configs_skips_empty_hosts() {
        let (cert, key) = test_pem();
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");
        std::fs::write(&cert_path, &cert).unwrap();
        std::fs::write(&key_path, &key).unwrap();

        let store = CertStore::new();
        store
            .reload_from_configs(&[
                TlsConfig {
                    hosts: vec![],
                    source: TlsSource::File {
                        cert: cert_path.clone(),
                        key: key_path.clone(),
                    },
                    expires_at: None,
                },
                TlsConfig {
                    hosts: vec!["tls.example.com".into()],
                    source: TlsSource::File {
                        cert: cert_path,
                        key: key_path,
                    },
                    expires_at: None,
                },
                TlsConfig {
                    hosts: vec!["acme.example.com".into()],
                    source: TlsSource::Acme {
                        email: None,
                        challenge: "http01".into(),
                        dns_provider: None,
                        dns_provider_type: None,
                        dns_credentials: None,
                    },
                    expires_at: None,
                },
                TlsConfig {
                    hosts: vec!["k8s.example.com".into()],
                    source: TlsSource::Kubernetes,
                    expires_at: None,
                },
            ])
            .unwrap();
        assert!(store.has_cert_for_host("tls.example.com"));
        assert!(!store.has_cert_for_host("acme.example.com"));
    }

    #[test]
    fn set_global_fallback_and_read_file_pem() {
        let (cert, key) = test_pem();
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");
        std::fs::write(&cert_path, &cert).unwrap();
        std::fs::write(&key_path, &key).unwrap();

        let store = CertStore::new();
        store
            .set_global_fallback(cert_path.clone(), key_path.clone())
            .unwrap();
        let stored = store.default_cert().unwrap();
        let (c, k) = stored.read_pem().unwrap();
        assert_eq!(c, cert);
        assert_eq!(k, key);
    }

    #[test]
    fn default_paths_none_for_in_memory_pem() {
        let (cert, key) = test_pem();
        let store = CertStore::new();
        store
            .insert_pem_in_memory_for_hosts(&["pem.example.com".into()], &cert, &key)
            .unwrap();
        assert!(store.default_paths().is_none());
        assert!(store.default_cert().is_some());
    }
}
