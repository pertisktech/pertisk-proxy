//! HTTP/3 server via Quinn + rustls (compatible with ACME/OpenSSL in the same binary).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use bytes::{Buf, Bytes, BytesMut};
use http::header::HOST;
use quinn::Endpoint;
use reqwest::Client;
use rustls::pki_types::CertificateDer;
use tracing::{info, warn};

use crate::deny;
use crate::health::{is_health_path, is_json_health_path, API_HEALTH_BODY};
use crate::h3::bind::h3_bind_candidates;
use crate::h3::config::H3Config;
use crate::metrics::ProxyMetrics;
use crate::proxy::forward::resolve_forward;
use crate::proxy::grpc;
use crate::router::Router;
use crate::runtime::RuntimeConfig;
use crate::tls::{CertStore, CertStoreResolver};

const ALPN_H3: &[u8] = b"h3";

fn is_benign_h3_disconnect(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_ascii_lowercase();
    msg.contains("aborted by peer")
        || msg.contains("closed abruptly")
        || msg.contains("connection reset")
        || msg.contains("timed out")
        // Normal client shutdown (curl, k6, browsers closing QUIC cleanly).
        || msg.contains("h3_no_error")
        || msg.contains("h3_cancel")
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default)
}

fn build_rustls_config(cert_store: Arc<CertStore>) -> Result<rustls::ServerConfig> {
    if cert_store.host_count() > 0 {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let resolver = CertStoreResolver::new_arc(Arc::clone(&cert_store), provider.clone());
        let mut config = rustls::ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS12, &rustls::version::TLS13])
            .context("TLS protocol versions")?
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(resolver));
        config.alpn_protocols = vec![ALPN_H3.to_vec()];
        return Ok(config);
    }

    let paths = cert_store
        .default_paths()
        .ok_or_else(|| anyhow::anyhow!("no TLS certificates available for HTTP/3"))?;
    build_rustls_config_from_paths(
        &paths.cert.to_string_lossy(),
        &paths.key.to_string_lossy(),
    )
}

fn build_rustls_config_from_paths(cert_path: &str, key_path: &str) -> Result<rustls::ServerConfig> {
    let cert_pem = std::fs::read(cert_path).with_context(|| format!("read cert {cert_path}"))?;
    let key_pem = std::fs::read(key_path).with_context(|| format!("read key {key_path}"))?;
    let mut cert_reader = std::io::Cursor::new(cert_pem);
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .context("parse certificate PEM")?;
    let mut key_reader = std::io::Cursor::new(key_pem);
    let key = rustls_pemfile::private_key(&mut key_reader)
        .context("parse private key PEM")?
        .ok_or_else(|| anyhow::anyhow!("no private key in {key_path}"))?;
    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("invalid TLS certificate pair for HTTP/3")?;
    config.alpn_protocols = vec![ALPN_H3.to_vec()];
    Ok(config)
}

fn build_quinn_server_config(cert_store: Arc<CertStore>) -> Result<quinn::ServerConfig> {
    let rustls_config = build_rustls_config(cert_store)?;
    let quic_crypto =
        quinn::crypto::rustls::QuicServerConfig::try_from(Arc::new(rustls_config))
            .context("QuicServerConfig from rustls")?;
    let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_crypto));

    let idle_secs = env_u64("PERTISK_HTTP3_IDLE_TIMEOUT_SECS", 300);
    let keepalive_secs = env_u64("PERTISK_HTTP3_KEEPALIVE_SECS", 30);
    let max_streams = env_u64("PERTISK_HTTP3_MAX_STREAMS", 1024) as u32;
    let stream_window = env_u64("PERTISK_HTTP3_STREAM_RECEIVE_WINDOW", 8 * 1024 * 1024) as u32;
    let conn_window = env_u64("PERTISK_HTTP3_CONN_RECEIVE_WINDOW", 64 * 1024 * 1024) as u32;

    let mut transport = quinn::TransportConfig::default();
    transport.max_concurrent_bidi_streams(max_streams.into());
    transport.stream_receive_window(stream_window.into());
    transport.receive_window(conn_window.into());
    if idle_secs > 0 {
        if let Ok(v) = Duration::from_secs(idle_secs).try_into() {
            transport.max_idle_timeout(Some(v));
        }
    }
    if keepalive_secs > 0 {
        transport.keep_alive_interval(Some(Duration::from_secs(keepalive_secs)));
    }
    server_config.transport_config(Arc::new(transport));
    Ok(server_config)
}

fn bind_quinn_endpoint(
    cert_store: Arc<CertStore>,
    listen: &str,
) -> Result<(Endpoint, std::net::SocketAddr)> {
    let candidates = h3_bind_candidates(listen);
    if candidates.is_empty() {
        anyhow::bail!("invalid HTTP/3 UDP listen address {:?}", listen);
    }

    let mut last_err: Option<anyhow::Error> = None;
    for addr in candidates {
        let server_config = build_quinn_server_config(Arc::clone(&cert_store))?;
        match Endpoint::server(server_config, addr) {
            Ok(endpoint) => return Ok((endpoint, addr)),
            Err(e) => {
                let addr_in_use = matches!(e.raw_os_error(), Some(48 | 98));
                let is_unspecified_v4 = addr.ip().is_unspecified() && addr.is_ipv4();
                if addr_in_use && is_unspecified_v4 {
                    tracing::info!(
                        %addr,
                        error = %e,
                        "HTTP/3 skipped redundant UDP bind (already covered by dual-stack listener)"
                    );
                    continue;
                }
                warn!(
                    %addr,
                    error = %e,
                    os_error = ?e.raw_os_error(),
                    "HTTP/3 UDP bind failed, trying next address"
                );
                last_err = Some(e.into());
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no HTTP/3 UDP bind candidates for {listen}")))
        .with_context(|| format!("bind HTTP/3 UDP for {listen}"))
}

pub async fn run(
    router: Arc<Router>,
    config: H3Config,
    cert_store: Arc<CertStore>,
    _runtime_cfg: &RuntimeConfig,
    metrics: ProxyMetrics,
) -> Result<()> {
    while !cert_store.has_any_cert() {
        tracing::info!(
            udp = %config.udp_listen,
            "HTTP/3 waiting for TLS certificates from ingress reconcile"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    let (endpoint, bound_addr) = bind_quinn_endpoint(Arc::clone(&cert_store), &config.udp_listen)?;
    info!(%bound_addr, "HTTP/3 (QUIC/Quinn) listening");

    let client = Client::builder()
        .pool_max_idle_per_host(64)
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_keepalive(Duration::from_secs(60))
        .build()?;

    loop {
        match endpoint.accept().await {
            Some(connecting) => {
                let router = Arc::clone(&router);
                let client = client.clone();
                let metrics = metrics.clone();
                tokio::spawn(async move {
                    match connecting.await {
                        Ok(connection) => {
                            let h3_conn = h3_quinn::Connection::new(connection);
                            if let Err(err) =
                                serve_h3_connection(router, client, h3_conn, metrics).await
                            {
                                if is_benign_h3_disconnect(&err) {
                                    tracing::debug!(error = %err, "HTTP/3 client closed connection");
                                } else {
                                    warn!(error = %err, "HTTP/3 connection error");
                                }
                            }
                        }
                        Err(err) => warn!(error = %err, "HTTP/3 handshake failed"),
                    }
                });
            }
            None => break,
        }
    }
    Ok(())
}

async fn serve_h3_connection(
    router: Arc<Router>,
    client: Client,
    conn: h3_quinn::Connection,
    metrics: ProxyMetrics,
) -> Result<()> {
    let mut h3 = h3::server::Connection::new(conn)
        .await
        .context("h3 server connection")?;
    loop {
        match h3.accept().await {
            Ok(Some(resolver)) => {
                let router = Arc::clone(&router);
                let client = client.clone();
                let metrics = metrics.clone();
                tokio::spawn(async move {
                    match resolver.resolve_request().await {
                        Ok((req, stream)) => {
                            if let Err(err) =
                                handle_request(router, client, req, stream, metrics).await
                            {
                                warn!(error = %err, "HTTP/3 request failed");
                            }
                        }
                        Err(err) => warn!(error = %err, "HTTP/3 resolve_request failed"),
                    }
                });
            }
            Ok(None) => break,
            Err(err) => return Err(err.into()),
        }
    }
    Ok(())
}

async fn handle_request(
    router: Arc<Router>,
    client: Client,
    req: http::Request<()>,
    mut stream: h3::server::RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>,
    metrics: ProxyMetrics,
) -> Result<()> {
    metrics.inc_active_connections();
    let result = handle_request_inner(router, client, req, &mut stream, &metrics).await;
    metrics.dec_active_connections();
    result
}

async fn handle_request_inner(
    router: Arc<Router>,
    client: Client,
    req: http::Request<()>,
    stream: &mut h3::server::RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>,
    metrics: &ProxyMetrics,
) -> Result<()> {
    if try_serve_health(&req, stream).await? {
        return Ok(());
    }

    if grpc::is_h3_incompatible_request(req.headers(), req.method(), req.uri().path()) {
        send_h3_response(
            stream,
            plain_response(
                http::StatusCode::MISDIRECTED_REQUEST,
                b"Omni /api/ RPC requires HTTP/2 (Alt-Svc: clear)",
            ),
            Bytes::from_static(b"Omni /api/ RPC requires HTTP/2 (Alt-Svc: clear)"),
        )
        .await?;
        return Ok(());
    }

    let host = request_host(&req);
    let is_grpc = grpc::is_grpc_request(req.headers());
    let bytes_received = req
        .headers()
        .get(http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    if deny::enabled() && !host.is_empty() && !router.snapshot().has_host(&host) {
        send_h3_response(
            stream,
            plain_response(http::StatusCode::NOT_FOUND, b"unknown host"),
            Bytes::from_static(b"unknown host"),
        )
        .await?;
        return Ok(());
    }

    let path_q = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or(req.uri().path());

    let plan = match resolve_forward(router.snapshot().as_ref(), &host, path_q) {
        Ok(plan) => plan,
        Err(_) => {
            send_h3_response(
                stream,
                plain_response(http::StatusCode::NOT_FOUND, b"no route"),
                Bytes::from_static(b"no route"),
            )
            .await?;
            return Ok(());
        }
    };

    let expects_body = req
        .headers()
        .get(http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .map(|n| n > 0)
        .unwrap_or(false)
        || matches!(
            req.method(),
            &http::Method::POST | &http::Method::PUT | &http::Method::PATCH
        );
    let body = read_h3_request_body(stream, expects_body).await?;
    let body_len = body.len() as u64;
    let bytes_received = bytes_received.saturating_add(body_len);

    let mut upstream_req = client.request(req.method().clone(), plan.upstream_url);
    for (name, value) in req.headers().iter() {
        if name == HOST {
            continue;
        }
        upstream_req = upstream_req.header(name, value);
    }
    for (name, value) in &plan.middleware.request_headers {
        upstream_req = upstream_req.header(name.as_str(), value.as_str());
    }
    upstream_req = upstream_req.header(HOST, &host);
    upstream_req = upstream_req.body(body);

    let upstream_res = match upstream_req.send().await {
        Ok(res) => res,
        Err(err) => {
            let msg = err.to_string();
            send_h3_response(
                stream,
                plain_response(http::StatusCode::BAD_GATEWAY, msg.as_bytes()),
                Bytes::from(msg),
            )
            .await?;
            record_h3_request(metrics, &host, bytes_received, 0, true, is_grpc);
            return Ok(());
        }
    };

    let status = upstream_res.status();
    let mut headers = upstream_res.headers().clone();
    headers.remove(http::header::SERVER);
    headers.remove(http::header::TRANSFER_ENCODING);
    let body = upstream_res.bytes().await.unwrap_or_default();
    if let Ok(v) = http::HeaderValue::from_str(&body.len().to_string()) {
        headers.insert(http::header::CONTENT_LENGTH, v);
    }
    headers.insert(http::header::SERVER, http::HeaderValue::from_static("pertisk-proxy/h3"));
    crate::apply_app_name(&mut headers);
    for (name, value) in &plan.middleware.response_headers {
        if let (Ok(n), Ok(v)) = (
            http::HeaderName::from_bytes(name.as_bytes()),
            http::HeaderValue::from_str(value),
        ) {
            headers.insert(n, v);
        }
    }

    let h3_resp = http::Response::builder()
        .status(status)
        .body(())
        .unwrap();
    let mut h3_resp = h3_resp;
    *h3_resp.headers_mut() = headers;
    send_h3_response(stream, h3_resp, body.clone()).await?;
    record_h3_request(
        metrics,
        &host,
        bytes_received,
        body.len() as u64,
        false,
        is_grpc,
    );
    Ok(())
}

fn record_h3_request(
    metrics: &ProxyMetrics,
    host: &str,
    bytes_received: u64,
    bytes_sent: u64,
    upstream_error: bool,
    is_grpc: bool,
) {
    if host.is_empty() {
        return;
    }
    metrics.inc_h3_requests();
    metrics.inc_https_requests();
    metrics
        .inc_site_protocol_requests(host, http::Version::HTTP_3);
    if is_grpc {
        metrics.inc_grpc_requests();
    }
    if upstream_error {
        metrics.inc_upstream_errors();
    }
    metrics.add_bytes_received(bytes_received);
    metrics.add_bytes_sent(bytes_sent);
}

async fn try_serve_health(
    req: &http::Request<()>,
    stream: &mut h3::server::RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>,
) -> Result<bool> {
    if !matches!(req.method(), &http::Method::GET | &http::Method::HEAD) {
        return Ok(false);
    }
    if !is_health_path(req.uri().path()) {
        return Ok(false);
    }
    let (content_type, body) = if is_json_health_path(req.uri().path()) {
        ("application/json", Bytes::from_static(API_HEALTH_BODY))
    } else {
        ("text/plain", Bytes::from_static(b"ok"))
    };
    let body = if *req.method() == http::Method::HEAD {
        Bytes::new()
    } else {
        body
    };
    let mut h3_resp = http::Response::builder()
        .status(http::StatusCode::OK)
        .header(http::header::CONTENT_TYPE, content_type)
        .header(http::header::SERVER, "pertisk-proxy/h3")
        .header("x-app-name", crate::app_name())
        .body(())
        .unwrap();
    if !body.is_empty() {
        if let Ok(v) = http::HeaderValue::from_str(&body.len().to_string()) {
            h3_resp.headers_mut().insert(http::header::CONTENT_LENGTH, v);
        }
    }
    send_h3_response(stream, h3_resp, body).await?;
    Ok(true)
}

fn plain_response(status: http::StatusCode, body: &[u8]) -> http::Response<()> {
    let mut resp = http::Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, "text/plain")
        .header(http::header::SERVER, "pertisk-proxy/h3")
        .header("x-app-name", crate::app_name())
        .body(())
        .unwrap();
    if let Ok(v) = http::HeaderValue::from_str(&body.len().to_string()) {
        resp.headers_mut().insert(http::header::CONTENT_LENGTH, v);
    }
    resp
}

async fn send_h3_response(
    stream: &mut h3::server::RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>,
    h3_resp: http::Response<()>,
    body: Bytes,
) -> Result<()> {
    stream.send_response(h3_resp).await.context("send h3 response headers")?;
    if !body.is_empty() {
        stream.send_data(body).await.context("send h3 response body")?;
    }
    stream.finish().await.context("finish h3 response")?;
    Ok(())
}

async fn read_h3_request_body(
    stream: &mut h3::server::RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>,
    store_body: bool,
) -> Result<Bytes> {
    if !store_body {
        return Ok(Bytes::new());
    }
    let mut body = BytesMut::new();
    loop {
        match stream.recv_data().await.context("recv h3 request body")? {
            Some(mut chunk) => body.extend_from_slice(&chunk.copy_to_bytes(chunk.remaining())),
            None => break,
        }
    }
    Ok(body.freeze())
}

fn request_host(req: &http::Request<()>) -> String {
    if let Some(host) = req.headers().get(HOST).and_then(|v| v.to_str().ok()) {
        let host = host.trim();
        if !host.is_empty() {
            return host.split(':').next().unwrap_or(host).to_string();
        }
    }
    if let Some(authority) = req.uri().authority() {
        return authority.host().to_string();
    }
    String::new()
}
