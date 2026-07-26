//! Background ACME certificate obtain / renew (like pertisk-rproxy).

use std::collections::HashSet;
use std::sync::Arc;

use dashmap::DashSet;
use once_cell::sync::Lazy;

use crate::db::{CertificateRow, Database};
use crate::proxy_config::{Config, TlsSource};
use crate::tls::{AcmeManager, CertStore};

use super::load_db_certs_into_store;

static ACME_INFLIGHT: Lazy<DashSet<String>> = Lazy::new(DashSet::new);

fn cert_expires_within_days(expires_at: Option<&String>, days: i64) -> bool {
    let Some(s) = expires_at else {
        return false;
    };
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    let expiry = chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .or_else(|_| chrono::DateTime::parse_from_rfc2822(s).map(|dt| dt.with_timezone(&chrono::Utc)));
    let Ok(expiry) = expiry else {
        return false;
    };
    (expiry - chrono::Utc::now()).num_days() <= days
}

fn hosts_set(hosts: &[String]) -> HashSet<String> {
    hosts
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn cert_row_covers_hosts(row: &CertificateRow, hosts: &[String]) -> bool {
    cert_row_covers_tls_hosts(&row.hosts, hosts)
}

/// True when a DB certificate row satisfies a TLS config host list (including wildcard + apex).
pub fn cert_row_covers_tls_hosts(cert_hosts: &[String], tls_hosts: &[String]) -> bool {
    let want = hosts_set(tls_hosts);
    if want.is_empty() {
        return false;
    }
    let have = hosts_set(cert_hosts);
    if have == want {
        return true;
    }
    // Cert with only wildcard(s) covers config that lists wildcard + apex names.
    let wildcard_only: HashSet<String> = want
        .iter()
        .filter(|s| s.starts_with('*'))
        .cloned()
        .collect();
    !wildcard_only.is_empty() && have == wildcard_only
}

/// Every non-wildcard TLS hostname must appear in the cert SAN or be covered by a wildcard SAN.
pub fn cert_row_matches_tls_config(row: &CertificateRow, tls_hosts: &[String]) -> bool {
    if !cert_row_covers_tls_hosts(&row.hosts, tls_hosts) {
        return false;
    }
    let have = hosts_set(&row.hosts);
    for h in tls_hosts {
        let h = h.trim();
        if h.is_empty() || h.starts_with('*') {
            continue;
        }
        if have.contains(h) {
            continue;
        }
        if have
            .iter()
            .any(|w| crate::proxy_config::wildcard_covers_host(w, h))
        {
            continue;
        }
        return false;
    }
    true
}

fn acme_cert_is_valid(row: &CertificateRow, tls_hosts: &[String]) -> bool {
    if !row.source_type.eq_ignore_ascii_case("acme") {
        return false;
    }
    if row
        .expires_at
        .as_ref()
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
    {
        return false;
    }
    if cert_expires_within_days(row.expires_at.as_ref(), 0) {
        return false;
    }
    cert_row_matches_tls_config(row, tls_hosts)
}

fn find_acme_cert_for_hosts<'a>(
    cert_rows: &'a [CertificateRow],
    hosts: &[String],
) -> Option<&'a CertificateRow> {
    cert_rows.iter().find(|row| {
        row.source_type.eq_ignore_ascii_case("acme") && acme_cert_is_valid(row, hosts)
    })
}

fn hosts_missing_from_store(cert_store: &CertStore, hosts: &[String]) -> Vec<String> {
    hosts
        .iter()
        .filter(|h| {
            let h = h.trim();
            !h.is_empty() && !h.starts_with('*') && !cert_store.has_cert_for_host(h)
        })
        .cloned()
        .collect()
}

async fn reload_acme_cert_into_store(
    db: &Database,
    cert_store: &CertStore,
    certs_dir: &std::path::Path,
    row: &CertificateRow,
    hosts: &[String],
) -> Result<(), String> {
    let rows = db
        .get_all_certificates_for_store()
        .await
        .map_err(|e| e.to_string())?;
    let Some((id, _, cert_pem, key_pem)) = rows.into_iter().find(|(id, _, _, _)| id == &row.id) else {
        return Err(format!("certificate {} not found in database", row.id));
    };
    cert_store
        .insert_pem_for_hosts(hosts, &cert_pem, &key_pem, certs_dir, &id)
        .map_err(|e| e.to_string())
}

fn acme_hosts_for_order(hosts: &[String], challenge: &str) -> Vec<String> {
    let is_dns01 = challenge.eq_ignore_ascii_case("dns01") || challenge.eq_ignore_ascii_case("dns-01");
    if is_dns01 {
        let wildcards: Vec<String> = hosts
            .iter()
            .filter(|h| h.starts_with('*'))
            .cloned()
            .collect();
        if !wildcards.is_empty() {
            return wildcards;
        }
        return hosts.to_vec();
    }
    hosts
        .iter()
        .filter(|h| !h.starts_with('*'))
        .cloned()
        .collect()
}

fn acme_tls_count(config: &Config) -> usize {
    config
        .tls
        .iter()
        .filter(|t| matches!(t.source, TlsSource::Acme { .. }))
        .count()
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

    let acme_tls = acme_tls_count(config);
    let acme_certs = cert_rows
        .iter()
        .filter(|r| r.source_type.eq_ignore_ascii_case("acme"))
        .count();
    tracing::info!(
        "Auto-SSL sweep: {} TLS config(s) ({} ACME), {} cert(s) in DB ({} ACME)",
        config.tls.len(),
        acme_tls,
        cert_rows.len(),
        acme_certs
    );

    if acme_tls == 0 {
        tracing::debug!("Auto-SSL: no ACME TLS entries in config; nothing to do");
        return;
    }

    const RENEW_DAYS: i64 = 30;
    for row in cert_rows.iter() {
        if !row.source_type.eq_ignore_ascii_case("acme") {
            continue;
        }
        if !cert_expires_within_days(row.expires_at.as_ref(), RENEW_DAYS) {
            continue;
        }
        let want = hosts_set(&row.hosts);
        let tls_matches = config.tls.iter().any(|tls| {
            matches!(&tls.source, TlsSource::Acme { .. })
                && hosts_set(&tls.hosts).intersection(&want).next().is_some()
        });
        if !tls_matches {
            continue;
        }
        if let Ok(true) = db.delete_certificate(&row.id).await {
            cert_store.remove_for_hosts(&row.hosts);
            tracing::info!(
                "Auto-SSL renew: removed expiring cert for {} (<= {} days)",
                row.hosts.join(", "),
                RENEW_DAYS
            );
        }
    }

    cert_rows = match db.list_certificates().await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("Auto-SSL: could not re-list certificates: {}", e);
            return;
        }
    };

    let mut spawned = 0u32;
    for tls in &config.tls {
        let TlsSource::Acme { challenge, .. } = &tls.source else {
            continue;
        };
        let hosts: Vec<String> = tls
            .hosts
            .iter()
            .map(|h| h.trim().to_string())
            .filter(|h| !h.is_empty())
            .collect();
        if hosts.is_empty() {
            tracing::warn!("Auto-SSL: ACME TLS entry has no hosts; skipping");
            continue;
        }

        if let Some(uploaded) = cert_rows.iter().find(|row| {
            row.source_type.eq_ignore_ascii_case("uploaded") && cert_row_covers_hosts(row, &hosts)
        }) {
            tracing::info!(
                "Auto-SSL: removing uploaded certificate {} for {} — TLS config requests ACME",
                uploaded.id,
                hosts.join(", ")
            );
            if db.delete_certificate(&uploaded.id).await.ok() == Some(true) {
                cert_store.remove_for_hosts(&uploaded.hosts);
            }
        }

        if let Some(row) = find_acme_cert_for_hosts(&cert_rows, &hosts) {
            let missing = hosts_missing_from_store(&cert_store, &hosts);
            if missing.is_empty() {
                tracing::info!(
                    "Auto-SSL: valid ACME certificate already present for {} (id={})",
                    hosts.join(", "),
                    row.id
                );
                continue;
            }
            tracing::info!(
                "Auto-SSL: ACME certificate {} valid in DB but not loaded for {:?}; reloading",
                row.id,
                missing
            );
            match reload_acme_cert_into_store(
                db.as_ref(),
                cert_store.as_ref(),
                &certs_dir,
                row,
                &hosts,
            )
            .await
            {
                Ok(()) => {
                    tracing::info!(
                        "Auto-SSL: reloaded ACME certificate for {} — restart pertisk-proxy if HTTPS is not listening yet",
                        hosts.join(", ")
                    );
                    continue;
                }
                Err(e) => tracing::warn!(
                    "Auto-SSL: failed to reload ACME certificate for {}: {}",
                    hosts.join(", "),
                    e
                ),
            }
        }

        // Drop any other stale ACME rows for these hosts before obtain.
        for row in cert_rows.iter() {
            if !row.source_type.eq_ignore_ascii_case("acme") || !cert_row_covers_hosts(row, &hosts) {
                continue;
            }
            if acme_cert_is_valid(row, &hosts) {
                continue;
            }
            if db.delete_certificate(&row.id).await.ok() == Some(true) {
                cert_store.remove_for_hosts(&row.hosts);
                tracing::info!(
                    "Auto-SSL: removed stale ACME cert {} for {}",
                    row.id,
                    row.hosts.join(", ")
                );
            }
        }

        cert_rows = db.list_certificates().await.unwrap_or(cert_rows);

        let hosts_for_acme = acme_hosts_for_order(&hosts, challenge);
        if hosts_for_acme.is_empty() {
            tracing::warn!(
                "Auto-SSL: no eligible domains for {} challenge on {}; \
                 HTTP-01 requires a non-wildcard hostname, DNS-01 requires DNS provider credentials",
                challenge,
                hosts.join(", ")
            );
            continue;
        }

        let mut lock_key = hosts_for_acme.clone();
        lock_key.sort();
        let lock_key = lock_key.join(",");
        if !ACME_INFLIGHT.insert(lock_key.clone()) {
            tracing::info!(
                "Auto-SSL: obtain already in progress for {}, skipping duplicate",
                hosts_for_acme.join(", ")
            );
            continue;
        }

        let mut source = tls.source.clone();
        if let TlsSource::Acme { email, .. } = &mut source {
            let empty = email.as_ref().map(|e| e.trim().is_empty()).unwrap_or(true);
            if empty {
                if let Some(default) = config
                    .acme_email
                    .as_deref()
                    .map(str::trim)
                    .filter(|e| !e.is_empty())
                {
                    *email = Some(default.to_string());
                }
            }
        }
        let tls_config = crate::proxy_config::TlsConfig {
            hosts: hosts_for_acme.clone(),
            source,
            expires_at: None,
        };
        let acme_clone = acme.clone();
        let db_clone = db.clone();
        let store_clone = cert_store.clone();
        let certs_dir = certs_dir.clone();
        let host_label = hosts_for_acme.join(", ");
        spawned += 1;

        tracing::info!("Auto-SSL (background): obtaining certificate for {}", host_label);

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
                        match crate::tls::solver_for_provider(pty.as_deref().unwrap_or(""), &creds) {
                            Ok(s) => Some(s),
                            Err(e) => {
                                tracing::warn!(
                                    "Auto-SSL: DNS provider solver unavailable for {}: {}",
                                    host_label,
                                    e
                                );
                                None
                            }
                        }
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
                                tracing::error!(
                                    "Auto-SSL: cert saved to DB but load failed for {}: {}",
                                    host_label,
                                    e
                                );
                            } else {
                                tracing::info!(
                                    "Auto-SSL: certificate obtained for {}",
                                    host_label
                                );
                            }
                        }
                        Err(e) => tracing::error!("Auto-SSL: failed to save cert for {}: {}", host_label, e),
                    }
                }
                Ok(Err(e)) => tracing::warn!("Auto-SSL failed for {}: {}", host_label, e),
                Err(e) => tracing::warn!("Auto-SSL task failed for {}: {}", host_label, e),
            }
            ACME_INFLIGHT.remove(&lock_key);
        });
    }

    if spawned == 0 {
        tracing::info!("Auto-SSL sweep: no new certificate tasks started");
    } else {
        tracing::info!("Auto-SSL sweep: started {} background obtain task(s)", spawned);
    }

    if let Err(e) = load_db_certs_into_store(db.as_ref(), cert_store.as_ref(), &certs_dir).await {
        tracing::warn!("Auto-SSL: failed to sync certificates into memory store: {}", e);
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
