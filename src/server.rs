use std::sync::Arc;

use anyhow::{Context, Result};
use pingora_core::listeners::tls::TlsSettings;
use pingora_core::server::configuration::Opt;
use pingora_core::server::Server;
use pingora_proxy::http_proxy_service;
use tracing::{info, warn};

use crate::config::ServerConfig;
use crate::h3::tcp_bind_addrs;
use crate::proxy::Gateway;
use crate::runtime::RuntimeConfig;
use crate::tls::{validate_cert_pair, CertStore};
use crate::Router;

/// Start the Pingora reverse proxy. HTTP/3 is started separately on the Tokio runtime.
pub fn run(
    server_config: &ServerConfig,
    router: Arc<Router>,
    cert_store: Arc<CertStore>,
    auto_https: bool,
    runtime_cfg: &RuntimeConfig,
) -> Result<()> {
    let tls_paths = resolve_tls_paths(server_config, &cert_store);

    info!(
        http = %server_config.http_listen,
        http_listeners = ?tcp_bind_addrs(&server_config.http_listen),
        https = %server_config.https_listen,
        https_listeners = ?tcp_bind_addrs(&server_config.https_listen),
        h3_udp = %server_config.h3_udp_listen,
        h3_listeners = ?crate::h3::h3_bind_addrs(&server_config.h3_udp_listen),
        https_enabled = tls_paths.is_some(),
        h3_enabled = server_config.enable_h3,
        http2_enabled = tls_paths.is_some(),
        tls_hosts = cert_store.host_count(),
        auto_https,
        "starting data plane"
    );

    if cert_store.host_count() > 1 {
        warn!(
            "multiple site TLS certificates are configured; HTTPS listener uses the default cert until SNI selection is added"
        );
    }

    let pingora_conf = crate::runtime::pingora_server_conf(runtime_cfg);
    info!(
        pingora_threads = pingora_conf.threads,
        pingora_listener_tasks = pingora_conf.listener_tasks_per_fd,
        pingora_upstream_keepalive = pingora_conf.upstream_keepalive_pool_size,
        tcp_listen_backlog = crate::runtime::tcp_listen_backlog(runtime_cfg),
        runtime_mode = runtime_cfg.resolved_mode.as_str(),
        "pingora runtime tuning"
    );

    let opt = Opt::parse_args();
    let mut server = Server::new_with_opt_and_conf(Some(opt), pingora_conf);
    server.bootstrap();

    let gateway = Gateway::new(
        Arc::clone(&router),
        Arc::clone(&cert_store),
        auto_https,
        server_config.https_port(),
        server_config.enable_h3,
        server_config.h3_port(),
    );
    let mut proxy = http_proxy_service(&server.configuration, gateway);
    for addr in tcp_bind_addrs(&server_config.http_listen) {
        proxy.add_tcp(&addr);
        info!(addr = %addr, "HTTP listener started");
    }

    if let Some((cert, key)) = tls_paths.as_ref() {
        validate_cert_pair(std::path::Path::new(cert), std::path::Path::new(key))
            .with_context(|| format!("HTTPS listener cannot load cert={cert} key={key}"))?;
    }

    if let Some((cert, key)) = tls_paths {
        for addr in tcp_bind_addrs(&server_config.https_listen) {
            let mut tls_settings = TlsSettings::intermediate(&cert, &key)
                .with_context(|| format!("HTTPS listener cannot load cert={cert} key={key}"))?;
            tls_settings.enable_h2();
            proxy.add_tls_with_settings(&addr, None, tls_settings);
            info!(addr = %addr, cert = %cert, "HTTPS (HTTP/1 + HTTP/2) listener started");
        }
    }

    server.add_service(proxy);
    server.run_forever();
}

fn resolve_tls_paths(
    server_config: &ServerConfig,
    cert_store: &CertStore,
) -> Option<(String, String)> {
    if let (Ok(cert), Ok(key)) = (server_config.tls_cert_path(), server_config.tls_key_path()) {
        return Some((cert.to_string(), key.to_string()));
    }

    cert_store.default_paths().map(|paths| {
        (
            paths.cert.to_string_lossy().into_owned(),
            paths.key.to_string_lossy().into_owned(),
        )
    })
}
