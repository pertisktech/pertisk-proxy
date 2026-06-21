use std::sync::Arc;

use async_trait::async_trait;
use pingora_core::listeners::TlsAccept;
use pingora_core::protocols::tls::TlsRef;
use pingora_core::tls::ext::{ssl_use_certificate, ssl_use_private_key};
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
            Ok((cert, key)) => {
                if let Err(err) = ssl_use_certificate(ssl, &cert) {
                    warn!(error = %err, sni = ?sni, "failed to set TLS certificate");
                    return;
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
) -> Result<(x509::X509, pkey::PKey<Private>), String> {
    let cert_bytes = std::fs::read(&paths.cert)
        .map_err(|e| format!("read cert {}: {e}", paths.cert.display()))?;
    let key_bytes = std::fs::read(&paths.key)
        .map_err(|e| format!("read key {}: {e}", paths.key.display()))?;
    let cert = x509::X509::from_pem(&cert_bytes).map_err(|e| format!("parse cert PEM: {e}"))?;
    let key =
        pkey::PKey::private_key_from_pem(&key_bytes).map_err(|e| format!("parse key PEM: {e}"))?;
    Ok((cert, key))
}
