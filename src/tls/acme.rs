//! ACME HTTP-01 / DNS-01 certificate acquisition (Let's Encrypt).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tracing::info;

use crate::proxy_config::{TlsConfig, TlsSource};

/// HTTP-01 challenge store: token -> key_authorization.
#[derive(Clone, Default)]
pub struct Http01ChallengeStore {
    inner: Arc<std::sync::RwLock<HashMap<String, String>>>,
}

impl Http01ChallengeStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_sync(&self, token: String, key_authorization: String) {
        if let Ok(mut g) = self.inner.write() {
            g.insert(token, key_authorization);
        }
    }

    pub fn get(&self, token: &str) -> Option<String> {
        self.inner.read().ok().and_then(|g| g.get(token).cloned())
    }

    pub fn remove_sync(&self, token: &str) {
        if let Ok(mut g) = self.inner.write() {
            g.remove(token);
        }
    }
}

#[cfg(not(feature = "acme"))]
pub struct AcmeManager {
    pub http01_store: Http01ChallengeStore,
}

#[cfg(not(feature = "acme"))]
impl AcmeManager {
    pub fn new(_cache_dir: PathBuf, _use_staging: bool, http01_store: Http01ChallengeStore) -> Self {
        Self { http01_store }
    }
}

#[cfg(feature = "acme")]
pub struct AcmeManager {
    pub http01_store: Http01ChallengeStore,
    cache_dir: PathBuf,
    use_staging: bool,
}

#[cfg(feature = "acme")]
impl AcmeManager {
    pub fn new(cache_dir: PathBuf, use_staging: bool, http01_store: Http01ChallengeStore) -> Self {
        Self {
            http01_store,
            cache_dir,
            use_staging,
        }
    }

    /// Blocking ACME obtain; run inside `spawn_blocking`.
    pub fn obtain_certs_sync(
        &self,
        tls_config: &TlsConfig,
        #[cfg(feature = "dns-challenge")]
        dns_solver: Option<Box<dyn super::dns_01::Dns01Solver>>,
        #[cfg(feature = "dns-challenge")]
        rt_handle: Option<tokio::runtime::Handle>,
    ) -> Result<(Vec<String>, Vec<u8>, Vec<u8>), String> {
        let TlsSource::Acme {
            email,
            challenge,
            ..
        } = &tls_config.source
        else {
            return Err("not an ACME config".to_string());
        };
        let challenge = challenge.to_lowercase();
        let is_dns01 = challenge == "dns01" || challenge == "dns-01";
        let is_http01 = challenge == "http01" || challenge == "http-01" || !is_dns01;
        let domains: Vec<String> = tls_config
            .hosts
            .iter()
            .filter_map(|h| {
                let s = h.trim();
                if s.is_empty() {
                    None
                } else if is_dns01 {
                    Some(s.to_string())
                } else if s.starts_with('*') {
                    None
                } else {
                    Some(s.to_string())
                }
            })
            .collect();
        if domains.is_empty() {
            return Err(if is_dns01 {
                "no domains for ACME order".to_string()
            } else {
                "no non-wildcard domains for HTTP-01".to_string()
            });
        }

        let dir_url = if self.use_staging {
            acme_lib::DirectoryUrl::LetsEncryptStaging
        } else {
            acme_lib::DirectoryUrl::LetsEncrypt
        };
        let email_addr = email
            .as_deref()
            .filter(|e| !e.is_empty() && !e.contains("@example.com"))
            .ok_or("ACME requires a valid contact email")?;
        let persist = acme_lib::persist::FilePersist::new(&self.cache_dir);
        let dir = acme_lib::Directory::from_url(persist, dir_url).map_err(|e| e.to_string())?;
        let acc = dir.account(email_addr).map_err(|e| e.to_string())?;

        let (first, rest_slice) = domains.split_first().ok_or("empty domains")?;
        let rest: Vec<&str> = rest_slice.iter().map(String::as_str).collect();
        let mut ord_new = acc.new_order(first, &rest).map_err(|e| e.to_string())?;

        while ord_new.confirm_validations().is_none() {
            let auths = ord_new.authorizations().map_err(|e| e.to_string())?;
            for auth in auths {
                if !auth.need_challenge() {
                    continue;
                }
                if is_http01 {
                    let chall = auth.http_challenge();
                    let token = chall.http_token().to_string();
                    let proof = chall.http_proof();
                    self.http01_store.set_sync(token.clone(), proof);
                    chall.validate(5000).map_err(|e| e.to_string())?;
                    self.http01_store.remove_sync(&token);
                } else if is_dns01 {
                    #[cfg(feature = "dns-challenge")]
                    {
                        let chall = auth.dns_challenge();
                        let fqdn = format!("_acme-challenge.{}", auth.domain_name());
                        let value = chall.dns_proof();
                        let solver = dns_solver.as_ref().ok_or(
                            "DNS-01 requires DNS provider credentials",
                        )?;
                        if let Some(ref handle) = rt_handle {
                            handle.block_on(solver.set_txt(&fqdn, &value))?;
                        } else {
                            return Err("DNS-01 requires a runtime handle".to_string());
                        }
                        std::thread::sleep(std::time::Duration::from_secs(45));
                        chall.validate(5000).map_err(|e| e.to_string())?;
                    }
                    #[cfg(not(feature = "dns-challenge"))]
                    return Err("DNS-01 requires dns-challenge feature".to_string());
                }
            }
            ord_new.refresh().map_err(|e| e.to_string())?;
        }

        let ord_csr = ord_new.confirm_validations().expect("validated");
        let pkey = acme_lib::create_p384_key();
        let ord_cert = ord_csr.finalize_pkey(pkey, 5000).map_err(|e| e.to_string())?;
        let cert = ord_cert.download_and_save_cert().map_err(|e| e.to_string())?;
        let cert_pem = cert.certificate().as_bytes().to_vec();
        let key_pem = cert.private_key().as_bytes().to_vec();
        let hosts = tls_config.hosts.clone();
        info!("ACME: obtained certificate for {:?}", hosts);
        Ok((hosts, cert_pem, key_pem))
    }
}

