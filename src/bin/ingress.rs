//! pertisk-proxy-ingress — Kubernetes Ingress controller (ingress mode).

#[cfg(feature = "ingress")]
use std::sync::Arc;

#[cfg(feature = "ingress")]
use anyhow::Result;
#[cfg(feature = "ingress")]
use tracing::info;

#[cfg(feature = "ingress")]
use pertisk_proxy::config::IngressConfig;
#[cfg(feature = "ingress")]
use pertisk_proxy::controller;
#[cfg(feature = "ingress")]
use pertisk_proxy::h3;
#[cfg(feature = "ingress")]
use pertisk_proxy::h3::H3Config;
#[cfg(feature = "ingress")]
use pertisk_proxy::logging;
#[cfg(feature = "ingress")]
use pertisk_proxy::runtime;
#[cfg(feature = "ingress")]
use pertisk_proxy::server;
#[cfg(feature = "ingress")]
use pertisk_proxy::tls::CertStore;
#[cfg(feature = "ingress")]
use pertisk_proxy::Router;

#[cfg(feature = "ingress")]
fn main() -> Result<()> {
    logging::init();
    let runtime_cfg = runtime::runtime_config_from_env(&runtime::ingress_runtime_env())?;
    let tokio_runtime = runtime::build_runtime(&runtime_cfg, "pertisk-proxy-ingress-worker")?;

    let config = IngressConfig::from_env()?;

    info!(
        binary = "pertisk-proxy-ingress",
        mode = "ingress",
        requested_runtime = runtime_cfg.requested_mode.as_str(),
        resolved_runtime = runtime_cfg.resolved_mode.as_str(),
        worker_threads = runtime_cfg.worker_threads,
        ingress_class = ?config.ingress_class,
        "starting pertisk-proxy-ingress"
    );

    let router = Router::new();
    let controller_config = config.clone();
    let router_for_controller = Arc::clone(&router);
    tokio_runtime.spawn(async move {
        if let Err(err) = controller::run(router_for_controller, controller_config).await {
            tracing::error!(error = %err, "ingress controller stopped");
        }
    });

    let h3_config = if config.server.enable_h3 {
        Some(H3Config::from_server(&config.server)?)
    } else {
        None
    };

    if let Some(h3_config) = h3_config {
        let runtime_for_h3 = runtime_cfg.clone();
        let router_for_h3 = Arc::clone(&router);
        tokio_runtime.spawn(async move {
            if let Err(err) = h3::run(router_for_h3, h3_config, &runtime_for_h3).await {
                tracing::error!(error = %err, "HTTP/3 listener stopped");
            }
        });
    }

    server::run(
        &config.server,
        router,
        Arc::new(CertStore::new()),
        false,
        &runtime_cfg,
    )
}

#[cfg(not(feature = "ingress"))]
fn main() {
    eprintln!("Build with --features ingress to run pertisk-proxy-ingress");
    std::process::exit(1);
}
