use std::sync::Arc;

use async_trait::async_trait;
use pingora_core::listeners::TlsAccept;
use pingora_core::protocols::tls::TlsRef;
use pingora_core::tls::ext::{ssl_add_chain_cert, ssl_use_certificate, ssl_use_private_key};
use pingora_core::tls::ssl::NameType;
use pingora_core::tls::{pkey, pkey::Private, x509};
use tracing::warn;

use super::store::{CertPaths, CertStore};

/// Pingora OpenSSL/BoringSSL callback: pick certificate from [`CertStore`] using client SNI.
pub struct CertStoreSniCallback {
    pub store: Arc<CertStore>,
}

#[async_trait]
impl TlsAccept for CertStoreSniCallback {
    async fn certificate_callback(&self, ssl: &mut TlsRef) -> () {
        let sni = ssl
            .servername(NameType::HOST_NAME)
            .map(str::to_owned);
        let paths = sni
            .as_deref()
            .and_then(|host| self.store.lookup_sni(host))
            .or_else(|| {
                if sni.is_none() {
                    self.store.default_paths()
                } else {
                    None
                }
            });

        let Some(paths) = paths else {
            warn!(sni = ?sni, "TLS handshake: no certificate for SNI");
            return;
        };

        match load_openssl_key_pair(&paths) {
            Ok((cert, chain, key)) => {
                if let Err(err) = ssl_use_certificate(ssl, &cert) {
                    warn!(error = %err, sni = ?sni, "failed to set TLS certificate");
                    return;
                }
                for intermediate in &chain {
                    if let Err(err) = ssl_add_chain_cert(ssl, intermediate) {
                        warn!(error = %err, sni = ?sni, "failed to add TLS intermediate certificate");
                        return;
                    }
                }
                if let Err(err) = ssl_use_private_key(ssl, &key) {
                    warn!(error = %err, sni = ?sni, "failed to set TLS private key");
                }
            }
            Err(err) => {
                warn!(
                    error = %err,
                    sni = ?sni,
                    cert = %paths.cert.display(),
                    "failed to load TLS certificate for SNI"
                );
            }
        }
    }
}

fn load_openssl_key_pair(
    paths: &CertPaths,
) -> Result<(x509::X509, Vec<x509::X509>, pkey::PKey<Private>), String> {
    let cert_bytes = std::fs::read(&paths.cert)
        .map_err(|e| format!("read cert {}: {e}", paths.cert.display()))?;
    let key_bytes = std::fs::read(&paths.key)
        .map_err(|e| format!("read key {}: {e}", paths.key.display()))?;
    let stack = x509::X509::stack_from_pem(&cert_bytes)
        .map_err(|e| format!("parse cert PEM: {e}"))?;
    if stack.is_empty() {
        return Err("no certificates in PEM".to_string());
    }
    let mut certs: Vec<x509::X509> = stack.into_iter().collect();
    let leaf = certs.remove(0);
    let key =
        pkey::PKey::private_key_from_pem(&key_bytes).map_err(|e| format!("parse key PEM: {e}"))?;
    Ok((leaf, certs, key))
}
