mod config;
mod store;
mod validate;

#[cfg(any(feature = "acme", feature = "dns-challenge"))]
mod dns_01;
#[cfg(feature = "acme")]
mod acme;

pub use config::{TlsConfig, TlsSource};
pub use store::{CertPaths, CertStore};
pub use validate::validate_cert_pair;

#[cfg(feature = "acme")]
pub use acme::{AcmeManager, Http01ChallengeStore};
#[cfg(not(feature = "acme"))]
pub use acme::Http01ChallengeStore;
#[cfg(feature = "dns-challenge")]
pub use dns_01::solver_for_provider;
