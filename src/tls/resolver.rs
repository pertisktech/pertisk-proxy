use std::sync::Arc;

use rustls::pki_types::CertificateDer;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;

use super::store::{CertStore, StoredCert};

/// Resolves server certificates from [`CertStore`] by SNI (HTTPS via rustls / HTTP/3).
#[derive(Clone)]
pub struct CertStoreResolver {
    store: Arc<CertStore>,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl CertStoreResolver {
    pub fn new_arc(store: Arc<CertStore>, provider: Arc<rustls::crypto::CryptoProvider>) -> Self {
        Self { store, provider }
    }
}

impl std::fmt::Debug for CertStoreResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CertStoreResolver").finish_non_exhaustive()
    }
}

impl ResolvesServerCert for CertStoreResolver {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let cert = if let Some(name) = client_hello.server_name() {
            self.store.lookup_sni(name)?
        } else {
            self.store.default_cert()?
        };
        stored_to_certified_key(&cert, self.provider.as_ref())
            .ok()
            .map(Arc::new)
    }
}

fn stored_to_certified_key(
    cert: &StoredCert,
    provider: &rustls::crypto::CryptoProvider,
) -> Result<CertifiedKey, String> {
    let (cert_pem, key_pem) = cert
        .read_pem()
        .map_err(|e| format!("read TLS PEM: {e}"))?;
    let cert_chain: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_pem.as_slice())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("parse cert PEM: {e}"))?
        .into_iter()
        .map(CertificateDer::from)
        .collect();
    if cert_chain.is_empty() {
        return Err("no certificates in PEM".to_string());
    }
    let key_der = rustls_pemfile::private_key(&mut key_pem.as_slice())
        .map_err(|e| format!("parse key PEM: {e}"))?
        .ok_or_else(|| "no private key in PEM".to_string())?;
    CertifiedKey::from_der(cert_chain, key_der, provider).map_err(|e| e.to_string())
}
