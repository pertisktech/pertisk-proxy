//! pertisk-proxy — standalone reverse proxy (proxy mode).
//!
//! Loads routes from `ROUTES_CONFIG` and serves HTTP/1, HTTP/2, and HTTP/3.
//! Per-site TLS is configured in the routes file (`tls:` section); ACME follows later.
//! For Kubernetes Ingress control, use the `pertisk-proxy-ingress` binary.

use std::sync::Arc;

use anyhow::Result;
use tracing::info;

use pertisk_proxy::config::ProxyConfig;
use pertisk_proxy::h3;
use pertisk_proxy::h3::H3Config;
use pertisk_proxy::logging;
use pertisk_proxy::proxy::routes;
use pertisk_proxy::routes_config;
use pertisk_proxy::runtime;
use pertisk_proxy::server;
use pertisk_proxy::tls::CertStore;
use pertisk_proxy::Router;

fn main() -> Result<()> {
    logging::init();
    let runtime_cfg = runtime::runtime_config_from_env(&runtime::proxy_runtime_env())?;
    let tokio_runtime = runtime::build_runtime(&runtime_cfg, "pertisk-proxy-worker")?;

    let config = ProxyConfig::from_env()?;

    info!(
        binary = "pertisk-proxy",
        mode = "proxy",
        requested_runtime = runtime_cfg.requested_mode.as_str(),
        resolved_runtime = runtime_cfg.resolved_mode.as_str(),
        worker_threads = runtime_cfg.worker_threads,
        max_blocking_threads = runtime_cfg.max_blocking_threads,
        pingora_threads = runtime::pingora_service_threads(&runtime_cfg),
        tcp_listen_backlog = runtime::tcp_listen_backlog(&runtime_cfg),
        routes = %config.routes_config.display(),
        routes_watch = config.routes_watch,
        "starting pertisk-proxy"
    );

    let router = Router::new();
    let cert_store = Arc::new(CertStore::new());

    if let (Some(cert), Some(key)) = (
        config.server.tls_cert_path.as_ref(),
        config.server.tls_key_path.as_ref(),
    ) {
        cert_store.set_global_fallback(cert.clone(), key.clone())?;
    }

    let initial = routes_config::load(&config.routes_config)?;
    router.replace_all(initial.table, initial.http3);
    cert_store.reload_from_configs(&initial.tls)?;

    let routes_path = config.routes_config.clone();
    let watch = config.routes_watch;
    tokio_runtime.spawn(routes::run(
        Arc::clone(&router),
        Arc::clone(&cert_store),
        routes_path,
        watch,
    ));

    if config.server.enable_h3 {
        let paths = cert_store.default_paths().or_else(|| {
            config
                .server
                .tls_cert_path
                .as_ref()
                .zip(config.server.tls_key_path.as_ref())
                .map(|(cert, key)| pertisk_proxy::tls::CertPaths {
                    cert: cert.clone(),
                    key: key.clone(),
                })
        });

        if let Some(paths) = paths {
            let runtime_for_h3 = runtime_cfg.clone();
            for udp_addr in h3::h3_bind_addrs(&config.server.h3_udp_listen) {
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
        } else {
            tracing::warn!("ENABLE_H3 is set but no TLS certificates are available; HTTP/3 disabled");
        }
    } else {
        info!("HTTP/3 disabled; serving HTTP/1.1 and HTTP/2 over TLS");
    }

    // Pingora owns the blocking server loop and creates its own runtime; do not call
    // `run_forever` from inside `block_on`.
    server::run(
        &config.server,
        router,
        cert_store,
        config.auto_https,
        &runtime_cfg,
    )
}
