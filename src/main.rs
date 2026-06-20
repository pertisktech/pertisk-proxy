use std::sync::Arc;

use anyhow::{Context, Result};
use pingora_core::listeners::tls::TlsSettings;
use pingora_core::server::configuration::Opt;
use pingora_core::server::Server;
use pingora_proxy::http_proxy_service;
use tracing::{info, Level};
use tracing_subscriber::EnvFilter;

use pertisk_proxy::config::Config;
use pertisk_proxy::controller;
use pertisk_proxy::h3;
use pertisk_proxy::mode::OperatingMode;
use pertisk_proxy::proxy::Gateway;
use pertisk_proxy::proxy::kinds;
use pertisk_proxy::Router;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive(Level::INFO.into()),
        )
        .init();

    let config = Config::from_env()?;
    let router = Router::new();

    let runtime = tokio::runtime::Runtime::new()?;
    spawn_route_loader(&runtime, Arc::clone(&router), &config)?;

    if config.enable_h3 {
        let router_for_h3 = Arc::clone(&router);
        let h3_config = config.clone();
        runtime.spawn(async move {
            if let Err(err) = h3::run(router_for_h3, h3_config).await {
                tracing::error!(error = %err, "HTTP/3 server stopped");
            }
        });
    }

    info!(
        mode = %config.mode,
        http = %config.http_listen,
        https = %config.https_listen,
        h3_udp = %config.h3_udp_listen,
        h3_enabled = config.enable_h3,
        auto_https = config.auto_https,
        "starting pertisk-proxy"
    );

    let opt = Opt::parse_args();
    let mut server = Server::new(Some(opt))?;
    server.bootstrap();

    let gateway = Gateway::new(
        Arc::clone(&router),
        config.mode.clone(),
        config.auto_https,
        config.https_port(),
    );
    let mut proxy = http_proxy_service(&server.configuration, gateway);
    proxy.add_tcp(&config.http_listen);

    if let (Ok(cert), Ok(key)) = (config.tls_cert_path(), config.tls_key_path()) {
        let mut tls_settings = TlsSettings::intermediate(cert, key)?;
        tls_settings.enable_h2();
        proxy.add_tls_with_settings(&config.https_listen, None, tls_settings);
        info!(addr = %config.https_listen, "HTTPS (HTTP/1 + HTTP/2) listener started");
    }

    server.add_service(proxy);
    server.run_forever();
}

fn spawn_route_loader(
    runtime: &tokio::runtime::Runtime,
    router: Arc<Router>,
    config: &Config,
) -> Result<()> {
    match &config.mode {
        OperatingMode::Ingress => {
            let controller_config = config.clone();
            runtime.spawn(async move {
                if let Err(err) = controller::run(router, controller_config).await {
                    tracing::error!(error = %err, "ingress controller stopped");
                }
            });
        }
        OperatingMode::Proxy(kind) => {
            let config_path = config
                .routes_config
                .clone()
                .context("routes config path missing")?;
            let kind = *kind;
            let watch = config.routes_watch;
            runtime.spawn(async move {
                if let Err(err) = kinds::run(router, kind, config_path, watch).await {
                    tracing::error!(error = %err, "proxy route loader stopped");
                }
            });
        }
    }
    Ok(())
}
