//! pertisk-proxy — standalone reverse proxy (proxy mode).
//!
//! Configuration and certificates are stored in SQLite (`PERTISK_DB_PATH`).
//! Admin UI is the primary way to add sites, import certificates, and configure DNS providers.
//! For Kubernetes Ingress control, use the `pertisk-proxy-ingress` binary.

use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::info;

use pertisk_proxy::config::ProxyConfig;
use pertisk_proxy::db::Database;
use pertisk_proxy::h3;
use pertisk_proxy::h3::H3Config;
use pertisk_proxy::logging;
use pertisk_proxy::proxy::apply;
use pertisk_proxy::proxy_config::Config;
use pertisk_proxy::runtime;
use pertisk_proxy::server;
use pertisk_proxy::tls::{CertStore, Http01ChallengeStore};
use pertisk_proxy::Router;

#[cfg(feature = "acme")]
use pertisk_proxy::tls::AcmeManager;

fn main() -> Result<()> {
    logging::init();
    let runtime_cfg = runtime::runtime_config_from_env(&runtime::proxy_runtime_env())?;
    let tokio_runtime = runtime::build_runtime(&runtime_cfg, "pertisk-proxy-worker")?;

    let proxy_env = ProxyConfig::from_env()?;

    info!(
        binary = "pertisk-proxy",
        mode = "proxy",
        db = %proxy_env.db_path.display(),
        requested_runtime = runtime_cfg.requested_mode.as_str(),
        resolved_runtime = runtime_cfg.resolved_mode.as_str(),
        "starting pertisk-proxy"
    );

    let db = Arc::new(Database::open(proxy_env.db_path.clone())?);
    let certs_dir = pertisk_proxy::api::certs_dir_for_db(db.path());

    let mut runtime_config = tokio_runtime.block_on(async {
        load_or_migrate_config(&db, proxy_env.migrate_routes_path.as_deref()).await
    })?;

    apply_listen_addrs_from_env(&mut runtime_config, &proxy_env);

    let router = Router::new();
    let cert_store = Arc::new(CertStore::new());
    let http01_store = Arc::new(Http01ChallengeStore::new());

    apply::apply_config(&router, &runtime_config)?;
    cert_store.reload_from_configs(&runtime_config.tls)?;

    tokio_runtime.block_on(async {
        if let Err(err) = pertisk_proxy::api::load_db_certs_into_store(
            db.as_ref(),
            cert_store.as_ref(),
            &certs_dir,
        )
        .await
        {
            tracing::warn!(error = %err, "failed to load certificates from database");
        }
    });

    if cert_store.default_paths().is_none() {
        if let (Some(cert), Some(key)) = (
            proxy_env.server.tls_cert_path.as_ref(),
            proxy_env.server.tls_key_path.as_ref(),
        ) {
            cert_store.set_global_fallback(cert.clone(), key.clone())?;
        }
    }

    #[cfg(feature = "acme")]
    let acme_manager = {
        let cache_dir = db
            .path()
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("acme");
        std::fs::create_dir_all(&cache_dir).ok();
        let staging = std::env::var("PERTISK_ACME_STAGING")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        Arc::new(AcmeManager::new(
            cache_dir,
            staging,
            (*http01_store).clone(),
        ))
    };

    #[cfg(feature = "acme")]
    {
        let cfg = runtime_config.clone();
        let db_c = Arc::clone(&db);
        let acme_c = Arc::clone(&acme_manager);
        let store_c = Arc::clone(&cert_store);
        let dir = certs_dir.clone();
        tokio_runtime.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            pertisk_proxy::api::acme::spawn_auto_ssl_for_config(
                &cfg, db_c, acme_c, store_c, dir,
            )
            .await;
        });
    }

    #[cfg(feature = "admin")]
    {
        let admin_state = pertisk_proxy::api::build_state(
            Arc::clone(&router),
            Arc::clone(&cert_store),
            proxy_env.clone(),
            runtime_cfg.clone(),
            Some(Arc::clone(&db)),
            Arc::clone(&http01_store),
            #[cfg(feature = "acme")]
            Some(acme_manager.clone()),
            runtime_config.clone(),
        );
        let admin_addr = pertisk_proxy::api::management_addr();
        tokio_runtime.spawn(async move {
            if let Err(err) = pertisk_proxy::api::serve(admin_state, admin_addr).await {
                tracing::error!(error = %err, "management API stopped");
            }
        });
    }

    if proxy_env.server.enable_h3 {
        let paths = cert_store.default_paths().or_else(|| {
            proxy_env
                .server
                .tls_cert_path
                .as_ref()
                .zip(proxy_env.server.tls_key_path.as_ref())
                .map(|(cert, key)| pertisk_proxy::tls::CertPaths {
                    cert: cert.clone(),
                    key: key.clone(),
                })
        });

        if let Some(paths) = paths {
            let runtime_for_h3 = runtime_cfg.clone();
            for udp_addr in h3::h3_bind_addrs(&proxy_env.server.h3_udp_listen) {
                let h3_config = H3Config::from_tls_paths(
                    paths.cert.to_string_lossy(),
                    paths.key.to_string_lossy(),
                    udp_addr.clone(),
                );
                info!(udp = %udp_addr, "starting HTTP/3 listener");
                let router_for_h3 = Arc::clone(&router);
                let runtime_for_h3 = runtime_for_h3.clone();
                tokio_runtime.spawn(async move {
                    if let Err(err) = h3::run(router_for_h3, h3_config, &runtime_for_h3).await {
                        tracing::error!(error = %err, udp = %udp_addr, "HTTP/3 listener stopped");
                    }
                });
            }
        } else if !runtime_config.tls.iter().any(|t| t.source.is_acme()) {
            tracing::warn!("ENABLE_H3 is set but no TLS certificates are available; HTTP/3 disabled");
        }
    }

    server::run(
        &proxy_env.server,
        router,
        cert_store,
        proxy_env.auto_https,
        &runtime_cfg,
        Some(http01_store),
    )
}

async fn load_or_migrate_config(
    db: &Database,
    migrate_path: Option<&std::path::Path>,
) -> Result<Config> {
    if let Some(mut cfg) = db.get_proxy_config().await? {
        info!("loaded proxy config from database");
        return Ok(cfg);
    }

    if let Some(path) = migrate_path {
        let yaml = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read migration routes file {}", path.display()))?;
        let cfg = apply::migrate_from_routes_yaml(&yaml)?;
        db.save_proxy_config(&cfg).await?;
        info!(path = %path.display(), "migrated legacy routes.yaml into database");
        return Ok(cfg);
    }

    info!("no proxy config in database; starting with empty config (add sites via admin UI)");
    Ok(Config::default())
}

fn apply_listen_addrs_from_env(cfg: &mut Config, proxy_env: &ProxyConfig) {
    if let Ok(addr) = std::env::var("LISTEN_HTTP") {
        if let Ok(parsed) = addr.parse() {
            cfg.http_addr = parsed;
        }
    }
    if let Ok(addr) = std::env::var("LISTEN_HTTPS") {
        if let Ok(parsed) = addr.parse() {
            cfg.https_addr = parsed;
        }
    }
    if let Ok(addr) = std::env::var("PERTISK_MANAGEMENT_ADDR") {
        if let Ok(parsed) = addr.parse() {
            cfg.management_addr = parsed;
        }
    }
    if proxy_env.server.enable_h3 {
        if let Ok(addr) = std::env::var("LISTEN_H3_UDP") {
            if let Ok(parsed) = addr.parse() {
                cfg.http3_addr = Some(parsed);
            }
        }
    }
}
