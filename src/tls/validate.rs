use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use tracing::warn;
use x509_parser::prelude::{FromDer, GeneralName, X509Certificate};

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

    #[test]
    fn wildcard_matching() {
        assert!(host_matches_cert_name(
            "admin.amd.pertisk.com",
            "*.amd.pertisk.com"
        ));
        assert!(!host_matches_cert_name(
            "admin.amd.thaidevops.co",
            "*.amd.pertisk.com"
        ));
    }
}
