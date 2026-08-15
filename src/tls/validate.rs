use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use tracing::warn;
use x509_parser::prelude::{FromDer, GeneralName, X509Certificate};

/// Ensure certificate and key files exist and contain parseable PEM material.
pub fn validate_cert_pair(cert_path: &Path, key_path: &Path) -> Result<()> {
    if !cert_path.is_file() {
        anyhow::bail!("certificate file not found: {}", cert_path.display());
    }
    if !key_path.is_file() {
        anyhow::bail!("private key file not found: {}", key_path.display());
    }

    let cert_pem = fs::read(cert_path)
        .with_context(|| format!("failed to read certificate {}", cert_path.display()))?;
    let key_pem = fs::read(key_path)
        .with_context(|| format!("failed to read private key {}", key_path.display()))?;

    let mut cert_reader = cert_pem.as_slice();
    let certs: Vec<_> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .context("failed to parse certificate PEM")?;
    if certs.is_empty() {
        anyhow::bail!("no certificate found in {}", cert_path.display());
    }

    let mut key_reader = key_pem.as_slice();
    rustls_pemfile::private_key(&mut key_reader)
        .context("failed to parse private key PEM")?
        .context(format!(
            "no private key found in {}",
            key_path.display()
        ))?;

    Ok(())
}

/// Validate in-memory PEM certificate and private key material.
pub fn validate_cert_pair_pem(cert_pem: &[u8], key_pem: &[u8]) -> Result<()> {
    let mut cert_reader = cert_pem;
    let certs: Vec<_> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .context("failed to parse certificate PEM")?;
    if certs.is_empty() {
        anyhow::bail!("no certificate found in PEM data");
    }

    let mut key_reader = key_pem;
    rustls_pemfile::private_key(&mut key_reader)
        .context("failed to parse private key PEM")?
        .context("no private key found in PEM data")?;

    Ok(())
}

pub fn warn_host_cert_mismatch_pem(cert_pem: &[u8], configured_hosts: &[String]) -> Result<()> {
    let cert_names = leaf_cert_names(cert_pem).context("failed to parse certificate PEM")?;
    for host in configured_hosts {
        let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
        if cert_covers_host(&cert_names, &host) {
            continue;
        }
        warn!(
            configured_host = %host,
            cert_names = ?cert_names,
            "TLS hostname is not covered by certificate; clients will reject the handshake (BadCertificate)"
        );
    }
    Ok(())
}

/// Warn when configured TLS hostnames are not covered by the certificate SAN/CN.
pub fn warn_host_cert_mismatch(cert_path: &Path, configured_hosts: &[String]) -> Result<()> {
    let pem = fs::read(cert_path)
        .with_context(|| format!("failed to read certificate {}", cert_path.display()))?;
    let cert_names = leaf_cert_names(&pem)
        .with_context(|| format!("failed to parse certificate {}", cert_path.display()))?;

    for host in configured_hosts {
        let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
        if cert_covers_host(&cert_names, &host) {
            continue;
        }
        warn!(
            configured_host = %host,
            cert_names = ?cert_names,
            cert = %cert_path.display(),
            "TLS hostname is not covered by certificate; clients will reject the handshake (BadCertificate)"
        );
    }

    Ok(())
}

fn leaf_cert_names(pem: &[u8]) -> Result<Vec<String>> {
    let mut reader = pem;
    let der = rustls_pemfile::certs(&mut reader)
        .next()
        .transpose()
        .context("failed to read PEM certificate")?
        .context("no certificate in PEM file")?;

    let (_, cert) = X509Certificate::from_der(der.as_ref()).context("invalid leaf certificate DER")?;
    let mut names = Vec::new();
    if let Some(cn) = cert.subject().iter_common_name().next() {
        if let Ok(cn) = cn.as_str() {
            names.push(cn.to_ascii_lowercase());
        }
    }

    if let Ok(Some(san)) = cert.subject_alternative_name() {
        for name in san.value.general_names.iter() {
            if let GeneralName::DNSName(dns) = name {
                names.push(dns.to_ascii_lowercase());
            }
        }
    }

    Ok(names)
}

fn cert_covers_host(cert_names: &[String], host: &str) -> bool {
    cert_names.iter().any(|name| host_matches_cert_name(host, name))
}

fn host_matches_cert_name(host: &str, cert_name: &str) -> bool {
    if host == cert_name {
        return true;
    }
    if let Some(suffix) = cert_name.strip_prefix("*.") {
        let suffix = format!(".{suffix}");
        return host.ends_with(&suffix) && host.len() > suffix.len();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{CertificateParams, KeyPair, SanType};
    use std::path::Path;

    fn test_cert_pair(hosts: &[&str]) -> (Vec<u8>, Vec<u8>) {
        let key_pair = KeyPair::generate().unwrap();
        let names: Vec<String> = hosts.iter().map(|h| (*h).to_string()).collect();
        let mut params = CertificateParams::new(names).unwrap();
        for host in hosts {
            if host.starts_with("*.") {
                let base = host.strip_prefix("*.").unwrap();
                params
                    .subject_alt_names
                    .push(SanType::DnsName(format!("*.{base}").try_into().unwrap()));
            }
        }
        let cert = params.self_signed(&key_pair).unwrap();
        (cert.pem().into_bytes(), key_pair.serialize_pem().into_bytes())
    }

    #[test]
    fn wildcard_matching() {
        assert!(host_matches_cert_name(
            "admin.amd.pertisk.com",
            "*.amd.pertisk.com"
        ));
        assert!(!host_matches_cert_name(
            "admin.amd.example.com",
            "*.amd.pertisk.com"
        ));
        assert!(host_matches_cert_name("example.com", "example.com"));
    }

    #[test]
    fn validate_cert_pair_rejects_empty_cert_pem() {
        let (_, key) = test_cert_pair(&["example.com"]);
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");
        std::fs::write(&cert_path, b"").unwrap();
        std::fs::write(&key_path, &key).unwrap();
        assert!(validate_cert_pair(&cert_path, &key_path).is_err());
    }

    #[test]
    fn validate_cert_pair_pem_ok() {
        let (cert, key) = test_cert_pair(&["example.com"]);
        validate_cert_pair_pem(&cert, &key).unwrap();
    }

    #[test]
    fn validate_cert_pair_pem_rejects_empty_cert() {
        let (_, key) = test_cert_pair(&["example.com"]);
        assert!(validate_cert_pair_pem(b"not a cert", &key).is_err());
        assert!(validate_cert_pair_pem(b"", &key).is_err());
    }

    #[test]
    fn validate_cert_pair_files() {
        let (cert, key) = test_cert_pair(&["example.com"]);
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");
        std::fs::write(&cert_path, &cert).unwrap();
        std::fs::write(&key_path, &key).unwrap();
        validate_cert_pair(&cert_path, &key_path).unwrap();
        assert!(validate_cert_pair(Path::new("/missing.pem"), &key_path).is_err());
        assert!(validate_cert_pair(&cert_path, Path::new("/missing.key")).is_err());
    }

    #[test]
    fn warn_host_cert_mismatch_pem_logs_mismatch() {
        let (cert, _) = test_cert_pair(&["example.com"]);
        warn_host_cert_mismatch_pem(&cert, &["other.com".into()]).unwrap();
        warn_host_cert_mismatch_pem(&cert, &["example.com".into()]).unwrap();
    }

    #[test]
    fn warn_host_cert_mismatch_file() {
        let (cert, key) = test_cert_pair(&["example.com"]);
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");
        std::fs::write(&cert_path, &cert).unwrap();
        std::fs::write(&key_path, &key).unwrap();
        warn_host_cert_mismatch(&cert_path, &["example.com".into()]).unwrap();
        warn_host_cert_mismatch(&cert_path, &["wrong.example.com".into()]).unwrap();
    }

    #[test]
    fn leaf_cert_names_extracts_san() {
        let (cert, key) = test_cert_pair(&["*.example.com", "example.com"]);
        let _ = key;
        let names = leaf_cert_names(&cert).unwrap();
        assert!(names.iter().any(|n| n.contains("example.com")));
    }
}
