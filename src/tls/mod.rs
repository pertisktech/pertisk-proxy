mod config;
#[cfg(feature = "h3-quinn")]
mod resolver;
mod sni;
mod store;
mod validate;

#[cfg(any(feature = "acme", feature = "dns-challenge"))]
mod dns_01;
mod acme;

pub use config::{TlsConfig, TlsSource};
#[cfg(feature = "h3-quinn")]
pub use resolver::CertStoreResolver;
pub use sni::CertStoreSniCallback;
pub use store::{CertPaths, CertStore};
pub use validate::validate_cert_pair;

/// Select ring as the process-wide rustls crypto provider (required for rustls 0.23+).
pub fn init_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

pub use acme::{AcmeManager, Http01ChallengeStore};
#[cfg(feature = "dns-challenge")]
pub use dns_01::solver_for_provider;
