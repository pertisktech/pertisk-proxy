use std::sync::Arc;

use anyhow::{Context, Result};
use pingora_core::listeners::tls::TlsSettings;
use pingora_core::listeners::TcpSocketOptions;
use pingora_core::listeners::TlsAcceptCallbacks;
use pingora_core::server::configuration::Opt;
use pingora_core::server::Server;
#[cfg(feature = "prometheus")]
use pingora_core::services::listening::Service as ListeningService;
use pingora_proxy::http_proxy_service;
use tracing::info;

use crate::config::ServerConfig;
use crate::h3::{tcp_bind_addrs, H3Config};
use crate::log::ProxyLog;
use crate::metrics::ProxyMetrics;
use crate::proxy::Gateway;
use crate::runtime::RuntimeConfig;
use crate::tls::{validate_cert_pair, CertStore, CertStoreSniCallback};
use crate::Router;

/// HTTP/3 listeners to start after Pingora TCP/TLS bind (avoids concurrent TLS stack init crashes).
pub struct PendingH3 {
    pub router: Arc<Router>,
    pub cert_store: Arc<CertStore>,
    pub configs: Vec<H3Config>,
    pub runtime_cfg: RuntimeConfig,
    pub metrics: ProxyMetrics,
}

/// Start the Pingora reverse proxy. HTTP/3 is started on the Tokio runtime after TCP/TLS bind.
pub fn run(
    server_config: &ServerConfig,
    router: Arc<Router>,
    cert_store: Arc<CertStore>,
    auto_https: bool,
    runtime_cfg: &RuntimeConfig,
    http01_store: Option<Arc<crate::tls::Http01ChallengeStore>>,
    pending_h3: Option<PendingH3>,
    log: Arc<ProxyLog>,
    proxy_log_enabled: Arc<std::sync::atomic::AtomicBool>,
    metrics: ProxyMetrics,
) -> Result<()> {
    let https_enabled = https_should_listen(server_config, &cert_store);
    let sni_enabled = cert_store.host_count() > 0;

    info!(
        http = %server_config.http_listen,
        http_listeners = ?tcp_bind_addrs(&server_config.http_listen),
        https = %server_config.https_listen,
        https_listeners = ?tcp_bind_addrs(&server_config.https_listen),
        h3_udp = %server_config.h3_udp_listen,
        h3_listeners = ?crate::h3::h3_bind_addrs(&server_config.h3_udp_listen),
        https_enabled,
        h3_enabled = server_config.enable_h3,
        h3_advertised_port = server_config.http3_advertised_port(),
        http2_enabled = https_enabled && server_config.enable_h2,
        tls_hosts = cert_store.host_count(),
        tls_sni = sni_enabled,
        auto_https,
        "starting data plane"
    );

    let pingora_conf = crate::runtime::pingora_server_conf(runtime_cfg);
    info!(
        pingora_threads = pingora_conf.threads,
        pingora_listener_tasks = pingora_conf.listener_tasks_per_fd,
        pingora_upstream_keepalive = pingora_conf.upstream_keepalive_pool_size,
        grace_period_seconds = pingora_conf.grace_period_seconds,
        graceful_shutdown_timeout_seconds = pingora_conf.graceful_shutdown_timeout_seconds,
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
        server_config.http3_advertised_port(),
        http01_store,
        log,
        proxy_log_enabled,
        metrics.clone(),
    );
    let mut proxy = http_proxy_service(&server.configuration, gateway);
    for addr in tcp_bind_addrs(&server_config.http_listen) {
        if let Some(opts) = dual_stack_tcp_options(&addr) {
            proxy.add_tcp_with_settings(&addr, opts);
        } else {
            proxy.add_tcp(&addr);
        }
        info!(addr = %addr, "HTTP listener started");
    }

    if https_enabled {
        if let Some(paths) = cert_store.default_paths() {
            validate_cert_pair(&paths.cert, &paths.key)
                .with_context(|| format!("invalid default TLS cert={}", paths.cert.display()))?;
        } else if !sni_enabled {
            let cert = server_config.tls_cert_path()?;
            let key = server_config.tls_key_path()?;
            validate_cert_pair(std::path::Path::new(cert), std::path::Path::new(key))
                .with_context(|| format!("HTTPS listener cannot load cert={cert} key={key}"))?;
        }

        for addr in tcp_bind_addrs(&server_config.https_listen) {
            let tls_settings = build_tls_settings(
                server_config,
                Arc::clone(&cert_store),
                server_config.enable_h2,
            )?;
            proxy.add_tls_with_settings(&addr, dual_stack_tcp_options(&addr), tls_settings);
            info!(
                addr = %addr,
                sni = sni_enabled,
                http2 = server_config.enable_h2,
                "HTTPS (HTTP/1 + HTTP/2) listener started"
            );
        }
    }

    server.add_service(proxy);

    #[cfg(feature = "prometheus")]
    if pingora_prometheus_enabled() {
        if let Some(addr) = pingora_prometheus_listen_addr() {
            let mut prom = ListeningService::prometheus_http_service();
            prom.add_tcp(&addr);
            server.add_service(prom);
            info!(addr = %addr, "Pingora Prometheus metrics listener started");
        }
    }

    if let Some(h3) = pending_h3 {
        for config in h3.configs {
            let router = Arc::clone(&h3.router);
            let cert_store = Arc::clone(&h3.cert_store);
            let runtime_cfg = h3.runtime_cfg.clone();
            let metrics = h3.metrics.clone();
            let udp = config.udp_listen.clone();
            let udp_err = udp.clone();
            // Dedicated thread + runtime: sharing the main Tokio runtime with Pingora
            // can segfault when Quiche/BoringSSL initializes alongside rustls on macOS.
            if let Err(err) = std::thread::Builder::new()
                .name(format!("pertisk-h3-{udp}"))
                .spawn(move || {
                    let rt = match tokio::runtime::Builder::new_multi_thread()
                        .enable_all()
                        .worker_threads(2)
                        .thread_name("h3-worker")
                        .build()
                    {
                        Ok(rt) => rt,
                        Err(err) => {
                            tracing::error!(error = %err, udp = %udp, "failed to build HTTP/3 runtime");
                            return;
                        }
                    };
                    rt.block_on(async move {
                        if let Err(err) =
                            crate::h3::run(router, config, cert_store, &runtime_cfg, metrics).await
                        {
                            tracing::error!(error = %err, udp = %udp, "HTTP/3 listener stopped");
                        }
                    });
                })
            {
                tracing::error!(error = %err, udp = %udp_err, "failed to spawn HTTP/3 thread");
            }
        }
    }

    server.run_forever();
}

/// Dual-stack TCP on `[::]:port` (IPV6_V6ONLY=0), matching UDP QUIC and pertisk-rproxy.
fn dual_stack_tcp_options(addr: &str) -> Option<TcpSocketOptions> {
    addr
        .parse::<std::net::SocketAddr>()
        .ok()
        .filter(|a| a.is_ipv6())
        .map(|_| {
            let mut opts = TcpSocketOptions::default();
            opts.ipv6_only = Some(false);
            opts
        })
}

fn https_should_listen(server_config: &ServerConfig, cert_store: &CertStore) -> bool {
    cert_store.host_count() > 0
        || cert_store.default_paths().is_some()
        || (server_config.tls_cert_path().is_ok() && server_config.tls_key_path().is_ok())
}

fn build_tls_settings(
    server_config: &ServerConfig,
    cert_store: Arc<CertStore>,
    enable_h2: bool,
) -> Result<TlsSettings> {
    let mut tls_settings = if cert_store.host_count() > 0 {
        let callbacks: TlsAcceptCallbacks = Box::new(CertStoreSniCallback {
            store: cert_store,
        });
        TlsSettings::with_callbacks(callbacks)?
    } else if let Some(paths) = cert_store.default_paths() {
        TlsSettings::intermediate(
            &paths.cert.to_string_lossy(),
            &paths.key.to_string_lossy(),
        )?
    } else {
        let cert = server_config.tls_cert_path()?;
        let key = server_config.tls_key_path()?;
        TlsSettings::intermediate(cert, key)?
    };

    if enable_h2 {
        tls_settings.enable_h2();
    }
    Ok(tls_settings)
}

#[cfg(feature = "prometheus")]
fn pingora_prometheus_enabled() -> bool {
    std::env::var("PERTISK_PINGORA_PROMETHEUS")
        .ok()
        .map(|v| {
            !matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        })
        .unwrap_or(false)
}

#[cfg(feature = "prometheus")]
fn pingora_prometheus_listen_addr() -> Option<String> {
    std::env::var("PERTISK_PINGORA_METRICS_ADDR")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| Some("127.0.0.1:9091".to_string()))
}
