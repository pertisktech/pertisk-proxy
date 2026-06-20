//! Background ACME certificate obtain / renew (like pertisk-rproxy).

use std::collections::HashSet;
use std::sync::Arc;

use dashmap::DashSet;
use once_cell::sync::Lazy;

use crate::db::Database;
use crate::proxy_config::{Config, TlsSource};
use crate::tls::{AcmeManager, CertStore};

static ACME_INFLIGHT: Lazy<DashSet<String>> = Lazy::new(DashSet::new);

fn cert_expires_within_days(expires_at: Option<&String>, days: i64) -> bool {
    let Some(s) = expires_at else { return false };
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    let expiry = chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .or_else(|_| chrono::DateTime::parse_from_rfc2822(s).map(|dt| dt.with_timezone(&chrono::Utc)));
    let Ok(expiry) = expiry else { return false };
    (expiry - chrono::Utc::now()).num_days() <= days
}

fn cert_exists_for_hosts(
    cert_rows: &[crate::db::CertificateRow],
    hosts: &[String],
) -> bool {
    let want: HashSet<String> = hosts
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if want.is_empty() {
        return true;
    }
    let wildcard_only: HashSet<String> = want
        .iter()
        .filter(|s| s.starts_with('*'))
        .cloned()
        .collect();
    for row in cert_rows {
        let have: HashSet<String> = row
            .hosts
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if have == want {
            return true;
        }
        if !wildcard_only.is_empty() && have == wildcard_only {
            return true;
        }
    }
    false
}

#[cfg(feature = "acme")]
pub async fn spawn_auto_ssl_for_config(
    config: &Config,
    db: Arc<Database>,
    acme: Arc<AcmeManager>,
    cert_store: Arc<CertStore>,
    certs_dir: std::path::PathBuf,
) {
    let mut cert_rows = match db.list_certificates().await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("Auto-SSL: could not list certificates: {}", e);
            return;
        }
    };

    const RENEW_DAYS: i64 = 30;
    for row in cert_rows.iter() {
        if !row.source_type.eq_ignore_ascii_case("acme") {
            continue;
        }
        if !cert_expires_within_days(row.expires_at.as_ref(), RENEW_DAYS) {
            continue;
        }
        let want: HashSet<String> = row
            .hosts
            .iter()
            .map(|h| h.trim().to_string())
            .filter(|h| !h.is_empty())
            .collect();
        let tls_matches = config.tls.iter().any(|tls| {
            matches!(&tls.source, TlsSource::Acme { .. })
                && tls
                    .hosts
                    .iter()
                    .map(|h| h.trim().to_string())
                    .filter(|h| !h.is_empty())
                    .collect::<HashSet<_>>()
                    .intersection(&want)
                    .next()
                    .is_some()
        });
        if !tls_matches {
            continue;
        }
        if let Ok(true) = db.delete_certificate(&row.id).await {
            cert_store.remove_for_hosts(&row.hosts);
            tracing::info!("Auto-SSL renew: removed expiring cert for {}", row.hosts.join(", "));
        }
    }

    cert_rows = match db.list_certificates().await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("Auto-SSL: could not re-list certificates: {}", e);
            return;
        }
    };

    for tls in &config.tls {
        let TlsSource::Acme { .. } = &tls.source else { continue };
        let hosts: Vec<String> = tls
            .hosts
            .iter()
            .map(|h| h.trim().to_string())
            .filter(|h| !h.is_empty())
            .collect();
        if hosts.is_empty() || cert_exists_for_hosts(&cert_rows, &hosts) {
            continue;
        }
        let hosts_for_acme: Vec<String> = if hosts.iter().any(|h| h.starts_with('*')) {
            hosts.iter().filter(|h| h.starts_with('*')).cloned().collect()
        } else {
            hosts.clone()
        };
        let mut lock_key = hosts_for_acme.clone();
        lock_key.sort();
        let lock_key = lock_key.join(",");
        if !ACME_INFLIGHT.insert(lock_key.clone()) {
            continue;
        }

        let tls_config = crate::proxy_config::TlsConfig {
            hosts: hosts_for_acme.clone(),
            source: tls.source.clone(),
            expires_at: None,
        };
        let acme_clone = acme.clone();
        let db_clone = db.clone();
        let store_clone = cert_store.clone();
        let certs_dir = certs_dir.clone();
        let host_label = hosts_for_acme.join(", ");

        tokio::spawn(async move {
            #[cfg(feature = "dns-challenge")]
            let dns_solver = match &tls_config.source {
                TlsSource::Acme {
                    challenge,
                    dns_provider,
                    dns_provider_type,
                    dns_credentials,
                    ..
                } => {
                    let is_dns01 = challenge.eq_ignore_ascii_case("dns01")
                        || challenge.eq_ignore_ascii_case("dns-01");
                    if !is_dns01 {
                        None
                    } else {
                        let mut creds = dns_credentials.clone().unwrap_or_default();
                        let mut pty = dns_provider_type
                            .as_deref()
                            .filter(|s| !s.is_empty())
                            .map(str::to_string);
                        if creds.is_empty() {
                            if let Some(id) = dns_provider.as_ref().filter(|s| !s.is_empty()) {
                                if let Ok(Some(row)) = db_clone.get_dns_provider(id).await {
                                    if let Some(c) = row.credentials {
                                        creds = c;
                                    }
                                    pty = Some(row.provider_type);
                                }
                            }
                        }
                        crate::tls::solver_for_provider(pty.as_deref().unwrap_or(""), &creds).ok()
                    }
                }
                _ => None,
            };
            #[cfg(feature = "dns-challenge")]
            let handle = tokio::runtime::Handle::current();

            let result = tokio::task::spawn_blocking(move || {
                #[cfg(feature = "dns-challenge")]
                {
                    acme_clone.obtain_certs_sync(&tls_config, dns_solver, Some(handle))
                }
                #[cfg(not(feature = "dns-challenge"))]
                {
                    acme_clone.obtain_certs_sync(&tls_config)
                }
            })
            .await;

            match result {
                Ok(Ok((hosts, cert_pem, key_pem))) => {
                    match db_clone
                        .add_certificate(hosts.clone(), cert_pem.clone(), key_pem.clone(), "acme")
                        .await
                    {
                        Ok(id) => {
                            if let Err(e) = store_clone.insert_pem_for_hosts(
                                &hosts,
                                &cert_pem,
                                &key_pem,
                                &certs_dir,
                                &id,
                            ) {
                                tracing::error!("Auto-SSL: cert saved to DB but load failed: {}", e);
                            } else {
                                tracing::info!("Auto-SSL: certificate obtained for {}", host_label);
                            }
                        }
                        Err(e) => tracing::error!("Auto-SSL: failed to save cert: {}", e),
                    }
                }
                Ok(Err(e)) => tracing::warn!("Auto-SSL failed for {}: {}", host_label, e),
                Err(e) => tracing::warn!("Auto-SSL task failed for {}: {}", host_label, e),
            }
            ACME_INFLIGHT.remove(&lock_key);
        });
    }
}

#[cfg(not(feature = "acme"))]
pub async fn spawn_auto_ssl_for_config(
    _config: &Config,
    _db: Arc<Database>,
    _cert_store: Arc<CertStore>,
    _certs_dir: std::path::PathBuf,
) {
}
