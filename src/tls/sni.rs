use std::sync::Arc;

use async_trait::async_trait;
use pingora_core::listeners::TlsAccept;
use pingora_core::protocols::tls::TlsRef;
use pingora_core::tls::ext::{ssl_add_chain_cert, ssl_use_certificate, ssl_use_private_key};
use pingora_core::tls::ssl::NameType;
use pingora_core::tls::{pkey, pkey::Private, x509};
use tracing::{debug, warn};

use super::store::{CertStore, StoredCert};

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
        let cert = sni
            .as_deref()
            .and_then(|host| self.store.lookup_sni(host))
            .or_else(|| {
                if sni.is_none() {
                    self.store.default_cert()
                } else {
                    None
                }
            });

        let Some(cert) = cert else {
            // Scanners / wrong DNS often hit :443 with unrelated SNI. Only warn when the
            // hostname is a configured site/TLS entry that is missing a certificate.
            let configured = sni
                .as_deref()
                .map(|h| self.store.expects_host(h))
                .unwrap_or(false);
            if configured {
                warn!(sni = ?sni, "TLS handshake: no certificate for configured host");
            } else {
                debug!(sni = ?sni, "TLS handshake: no certificate for unknown SNI");
            }
            return;
        };

        match load_openssl_key_pair(&cert) {
            Ok((leaf, chain, key)) => {
                if let Err(err) = ssl_use_certificate(ssl, &leaf) {
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
                warn!(error = %err, sni = ?sni, "failed to load TLS certificate for SNI");
            }
        }
    }
}

fn load_openssl_key_pair(
    cert: &StoredCert,
) -> Result<(x509::X509, Vec<x509::X509>, pkey::PKey<Private>), String> {
    let (cert_bytes, key_bytes) = cert
        .read_pem()
        .map_err(|e| format!("read TLS PEM: {e}"))?;
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
