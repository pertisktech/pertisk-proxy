pub mod routes;
pub mod forward;
pub mod apply;
pub mod grpc;
pub mod registry;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use pingora_core::listeners::ALPN;
use pingora_core::upstreams::peer::HttpPeer;
use pingora_core::Result;
use pingora_error::{Error, ErrorSource, ErrorType};
use pingora_http::{RequestHeader, ResponseHeader};
use pingora_proxy::{FailToProxy, ProxyHttp, Session};

use crate::deny;
use crate::health;
use crate::log::{ProxyLog, ProxyLogEntry};
use crate::metrics::ProxyMetrics;
use crate::proxy::forward::resolve_forward;
use crate::router::Router;
use crate::tls::CertStore;

#[derive(Default)]
pub struct RequestCtx {
    pub strip_prefix: Option<String>,
    pub request_headers: Vec<(String, String)>,
    pub response_headers: Vec<(String, String)>,
    pub forward_client_ip: bool,
    pub is_grpc: bool,
    pub is_grpc_web: bool,
    pub is_long_lived_stream: bool,
    pub is_registry: bool,
    pub log_started: Option<Instant>,
    pub log_upstream: Option<String>,
    pub metrics_counted: bool,
}

impl From<crate::proxy::forward::MiddlewareAction> for RequestCtx {
    fn from(mw: crate::proxy::forward::MiddlewareAction) -> Self {
        Self {
            strip_prefix: mw.strip_prefix,
            request_headers: mw.request_headers,
            response_headers: mw.response_headers,
            forward_client_ip: mw.forward_client_ip,
            is_grpc: false,
            is_grpc_web: false,
            is_long_lived_stream: false,
            is_registry: false,
            log_started: None,
            log_upstream: None,
            metrics_counted: false,
        }
    }
}

pub struct Gateway {
    router: Arc<Router>,
    cert_store: Arc<CertStore>,
    auto_https: bool,
    https_port: u16,
    enable_h3: bool,
    h3_port: u16,
    http01_store: Option<Arc<crate::tls::Http01ChallengeStore>>,
    log: Arc<ProxyLog>,
    proxy_log_enabled: Arc<AtomicBool>,
    metrics: ProxyMetrics,
}

impl Gateway {
    pub fn new(
        router: Arc<Router>,
        cert_store: Arc<CertStore>,
        auto_https: bool,
        https_port: u16,
        enable_h3: bool,
        h3_port: u16,
        http01_store: Option<Arc<crate::tls::Http01ChallengeStore>>,
        log: Arc<ProxyLog>,
        proxy_log_enabled: Arc<AtomicBool>,
        metrics: ProxyMetrics,
    ) -> Self {
        Self {
            router,
            cert_store,
            auto_https,
            https_port,
            enable_h3,
            h3_port,
            http01_store,
            log,
            proxy_log_enabled,
            metrics,
        }
    }

    fn record_request_metrics(
        &self,
        session: &Session,
        ctx: &RequestCtx,
        host: &str,
        upstream_error: bool,
    ) {
        if host.is_empty() || health::is_health_path(session.req_header().uri.path()) {
            return;
        }

        let tls = is_downstream_tls(session);
        let h2 = session.as_downstream().is_http2();
        if tls {
            self.metrics.inc_https_requests();
            if h2 {
                self.metrics.inc_h2_requests();
                self.metrics
                    .inc_site_protocol_requests(host, http::Version::HTTP_2);
            }
        } else {
            self.metrics.inc_http_requests();
        }

        if ctx.is_grpc {
            self.metrics.inc_grpc_requests();
        }

        if upstream_error {
            self.metrics.inc_upstream_errors();
        }

        if let Some(len) = session
            .req_header()
            .headers
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
        {
            self.metrics.add_bytes_received(len);
        }

        if let Some(len) = session
            .response_written()
            .and_then(|r| r.headers.get("content-length"))
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
        {
            self.metrics.add_bytes_sent(len);
        }
    }
}

fn request_host(req: &RequestHeader) -> String {
    if let Some(host) = req.headers.get("host").and_then(|v| v.to_str().ok()) {
        let host = host.trim();
        if !host.is_empty() {
            return host.split(':').next().unwrap_or(host).to_string();
        }
    }

    if let Some(authority) = req.uri.authority() {
        return authority.host().to_string();
    }

    String::new()
}

#[async_trait]
impl ProxyHttp for Gateway {
    type CTX = RequestCtx;

    fn new_ctx(&self) -> Self::CTX {
        RequestCtx::default()
    }

    async fn early_request_filter(&self, session: &mut Session, ctx: &mut Self::CTX) -> Result<()> {
        ctx.log_started = Some(Instant::now());
        ctx.metrics_counted = true;
        self.metrics.inc_active_connections();
        let req = session.req_header();
        let host = request_host(req);
        let (is_grpc, is_grpc_web) = grpc::classify_grpc_request(
            &req.headers,
            &req.method,
            req.uri.path(),
            &host,
        );
        ctx.is_long_lived_stream =
            grpc::is_long_lived_api_stream(&req.method, req.uri.path(), &req.headers);
        ctx.is_registry = registry::is_oci_registry_path(req.uri.path());
        if ctx.is_registry {
            registry::prepare_registry_downstream_session(session);
        }
        if is_grpc {
            ctx.is_grpc = true;
            ctx.is_grpc_web = is_grpc_web;
        }
        Ok(())
    }

    async fn request_filter(&self, session: &mut Session, ctx: &mut Self::CTX) -> Result<bool> {
        let method = session.req_header().method.clone();
        let path = session.req_header().uri.path().to_string();
        let protocol = downstream_protocol_label(session);

        if health::try_respond_health(
            session,
            &method,
            &path,
            &format!("pertisk-proxy/{protocol}"),
        )
        .await?
        {
            return Ok(true);
        }

        if let Some(store) = &self.http01_store {
            if let Some(token) = path.strip_prefix("/.well-known/acme-challenge/") {
                let token = token.trim_end_matches('/');
                if !token.is_empty() {
                    if let Some(body) = store.get(token) {
                        let mut resp = ResponseHeader::build(http::StatusCode::OK, Some(2))?;
                        resp.insert_header("Content-Type", "text/plain")?;
                        resp.insert_header("X-App-Name", crate::app_name())?;
                        session.write_response_header(Box::new(resp), false).await?;
                        session
                            .write_response_body(Some(bytes::Bytes::from(body)), true)
                            .await?;
                        return Ok(true);
                    }
                }
            }
        }

        if self.auto_https && is_plain_http(session) {
            let req = session.req_header();
            let host = request_host(req);
            let host = if host.is_empty() { "localhost" } else { host.as_str() };

            if deny::enabled()
                && !host.is_empty()
                && host != "localhost"
                && !self.router.snapshot().has_host(host)
            {
                deny::respond_pingora(
                    session,
                    false,
                    &format!("pertisk-proxy/{protocol}"),
                )
                .await?;
                return Ok(true);
            }

            if !self.cert_store.has_cert_for_host(host) {
                return Ok(false);
            }

            let port_suffix = if self.https_port == 443 {
                String::new()
            } else {
                format!(":{}", self.https_port)
            };
            let path_q = req
                .uri
                .path_and_query()
                .map(|pq| pq.as_str())
                .unwrap_or(&path);
            let target = format!("https://{host}{port_suffix}{path_q}");

            let mut resp =
                ResponseHeader::build(http::StatusCode::PERMANENT_REDIRECT, Some(3))?;
            resp.insert_header("Location", target)?;
            resp.insert_header("Content-Length", "0")?;
            resp.insert_header("X-App-Name", crate::app_name())?;
            session.write_response_header(Box::new(resp), true).await?;
            return Ok(true);
        }

        let host = request_host(session.req_header());

        if deny::enabled() && !host.is_empty() && !self.router.snapshot().has_host(&host) {
            deny::respond_pingora(
                session,
                is_downstream_tls(session),
                &format!("pertisk-proxy/{protocol}"),
            )
            .await?;
            return Ok(true);
        }

        let path_q = session
            .req_header()
            .uri
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or(&path)
            .to_string();

        if let Ok(plan) = resolve_forward(self.router.snapshot().as_ref(), &host, &path_q) {
            let is_grpc = ctx.is_grpc;
            let is_grpc_web = ctx.is_grpc_web;
            let is_long_lived_stream = ctx.is_long_lived_stream;
            let is_registry = ctx.is_registry;
            *ctx = plan.middleware.into();
            ctx.is_grpc = is_grpc;
            ctx.is_grpc_web = is_grpc_web;
            ctx.is_long_lived_stream = is_long_lived_stream;
            ctx.is_registry = is_registry;
        }

        if ctx.is_grpc {
            let req = session.req_header();
            if let Err(msg) = grpc::validate_downstream(req, session, ctx.is_grpc_web) {
                tracing::warn!(host = %host, path = %path_q, error = msg, "invalid gRPC request");
                grpc::respond_error(session, 2, msg, ctx.is_grpc_web).await?;
                return Ok(true);
            }
            tracing::debug!(host = %host, path = %path_q, grpc_web = ctx.is_grpc_web, "gRPC request");
        }

        Ok(false)
    }

    async fn upstream_peer(
        &self,
        session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        let req = session.req_header();
        let host = request_host(req);
        let path_q = req
            .uri
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or(req.uri.path());

        let plan = resolve_forward(self.router.snapshot().as_ref(), &host, path_q)?;

        if ctx.strip_prefix.is_none()
            && ctx.request_headers.is_empty()
            && ctx.response_headers.is_empty()
        {
            let is_grpc = ctx.is_grpc;
            let is_grpc_web = ctx.is_grpc_web;
            let is_long_lived_stream = ctx.is_long_lived_stream;
            let is_registry = ctx.is_registry;
            *ctx = plan.middleware.into();
            ctx.is_grpc = is_grpc;
            ctx.is_grpc_web = is_grpc_web;
            ctx.is_long_lived_stream = is_long_lived_stream;
            ctx.is_registry = is_registry;
        }

        let peer = Box::new(configure_upstream_peer(
            &plan.peer_host,
            plan.peer_port,
            &host,
            plan.use_tls,
            ctx,
        ));
        ctx.log_upstream = Some(format!("{}:{}", plan.peer_host, plan.peer_port));
        Ok(peer)
    }

    async fn upstream_request_filter(
        &self,
        session: &mut Session,
        upstream_request: &mut RequestHeader,
        ctx: &mut Self::CTX,
    ) -> Result<()> {
        let host = request_host(session.req_header());
        if !host.is_empty() {
            upstream_request.insert_header("Host", host.as_str()).ok();
        }

        if !ctx.is_registry && is_downstream_tls(session) {
            upstream_request.insert_header("X-Forwarded-Proto", "https").ok();
            upstream_request.insert_header("X-Forwarded-Host", host.as_str()).ok();
        }

        if !ctx.is_registry {
            if let Some(ip) = session
                .client_addr()
                .and_then(|a| a.as_inet().map(|addr| addr.ip().to_string()))
            {
                inject_forwarded_client_ip(upstream_request, &ip);
            }
        }

        if let Some(prefix) = &ctx.strip_prefix {
            let skip_strip = ctx.is_grpc_web;
            if !skip_strip {
                let path = upstream_request.uri.path();
                if let Some(stripped) = path.strip_prefix(prefix) {
                    let new_path = if stripped.is_empty() { "/" } else { stripped };
                    let mut parts = upstream_request.uri.clone().into_parts();
                    parts.path_and_query = Some(new_path.parse().map_err(|_| {
                        pingora_error::Error::explain(
                            pingora_error::ErrorType::InternalError,
                            "invalid stripped path",
                        )
                    })?);
                    upstream_request.set_uri(http::Uri::from_parts(parts).map_err(|_| {
                        pingora_error::Error::explain(
                            pingora_error::ErrorType::InternalError,
                            "failed to rebuild uri",
                        )
                    })?);
                }
            }
        }

        for (name, value) in ctx.request_headers.clone() {
            upstream_request.insert_header(name, value).ok();
        }

        grpc::merge_cookie_headers(upstream_request);

        if registry::is_oci_registry_path(upstream_request.uri.path()) {
            registry::normalize_registry_token_request(upstream_request);
            let method = upstream_request.method.clone();
            let path = upstream_request.uri.path().to_string();
            let client_ip = session
                .client_addr()
                .and_then(|a| a.as_inet().map(|addr| addr.ip().to_string()));
            let tls = is_downstream_tls(session);
            registry::prepare_registry_upstream_request(
                upstream_request,
                &method,
                &path,
                &host,
                client_ip.as_deref(),
                tls,
            );
        }

        if ctx.is_grpc || ctx.is_long_lived_stream {
            grpc::prepare_upstream_streaming_request(upstream_request);
        }

        if ctx.is_grpc {
            if !ctx.is_grpc_web {
                grpc::rewrite_upstream_grpc_path(upstream_request)?;
            }
            grpc::prepare_upstream_request(upstream_request, ctx.is_grpc_web);
        }

        Ok(())
    }

    async fn response_filter(
        &self,
        session: &mut Session,
        upstream_response: &mut ResponseHeader,
        ctx: &mut Self::CTX,
    ) -> Result<()> {
        let path = session.req_header().uri.path();
        let streaming = ctx.is_long_lived_stream
            || (ctx.is_grpc && grpc::is_grpc_server_streaming(path));
        if streaming && upstream_response.status.is_success() {
            grpc::prepare_streaming_response_headers(upstream_response);
        } else if ctx.is_grpc || ctx.is_long_lived_stream || session.as_downstream().is_http2() {
            grpc::strip_hop_by_hop_response_headers(upstream_response);
        }
        Ok(())
    }

    async fn upstream_response_filter(
        &self,
        session: &mut Session,
        upstream_response: &mut ResponseHeader,
        ctx: &mut Self::CTX,
    ) -> Result<()> {
        let host = request_host(session.req_header());
        let protocol = downstream_protocol_label(session);
        upstream_response
            .insert_header("Server", format!("pertisk-proxy/{protocol}"))
            .ok();
        upstream_response
            .insert_header("X-App-Name", crate::app_name())
            .ok();

        let path = session.req_header().uri.path();
        if registry::is_oci_registry_path(path) {
            registry::prepare_registry_response_headers(
                upstream_response,
                is_downstream_tls(session),
            );
        } else if let Some(alt_svc) = alt_svc_header_value(self.enable_h3, self.h3_port, &host, ctx) {
            upstream_response.insert_header("Alt-Svc", alt_svc.as_str()).ok();
        } else if is_downstream_tls(session) {
            upstream_response.insert_header("Alt-Svc", "clear").ok();
        }

        for (name, value) in ctx.response_headers.clone() {
            upstream_response.insert_header(name, value).ok();
        }

        Ok(())
    }

    fn suppress_error_log(&self, session: &Session, ctx: &Self::CTX, error: &Error) -> bool {
        if !ctx.is_grpc && !ctx.is_long_lived_stream {
            return false;
        }
        if !grpc::is_benign_downstream_disconnect(error) {
            return false;
        }
        tracing::debug!(
            path = session.req_header().uri.path(),
            "client closed gRPC/Connect stream"
        );
        true
    }

    async fn logging(&self, session: &mut Session, e: Option<&Error>, ctx: &mut Self::CTX) {
        let host = request_host(session.req_header());
        let upstream_error = e.is_some_and(|err| *err.esource() == ErrorSource::Upstream);
        self.record_request_metrics(session, ctx, &host, upstream_error);
        if ctx.metrics_counted {
            self.metrics.dec_active_connections();
            ctx.metrics_counted = false;
        }

        if !self.proxy_log_enabled.load(Ordering::Relaxed) {
            return;
        }

        let req = session.req_header();
        if host.is_empty() {
            return;
        }
        let path = req
            .uri
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or(req.uri.path())
            .to_string();
        let method = req.method.as_str().to_string();
        let protocol = protocol_short(downstream_protocol_label(session));
        let upstream = ctx
            .log_upstream
            .clone()
            .unwrap_or_else(|| "-".to_string());
        let duration_ms = ctx
            .log_started
            .map(|s| s.elapsed().as_millis() as u64)
            .unwrap_or(0);

        if let Some(err) = e {
            if self.suppress_error_log(session, ctx, err) {
                return;
            }
            let _ = self
                .log
                .push(ProxyLogEntry::error_with_context(
                    &host,
                    &path,
                    &upstream,
                    err.to_string(),
                ))
                .await;
            return;
        }

        let status = session
            .response_written()
            .map(|r| r.status.as_u16())
            .unwrap_or(0);
        let encoding = session
            .response_written()
            .and_then(|r| r.headers.get("content-encoding"))
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let _ = self
            .log
            .push(ProxyLogEntry::response(
                &host,
                &path,
                &upstream,
                status,
                duration_ms,
                Some(protocol),
                encoding.as_deref(),
                Some(&method),
            ))
            .await;
    }

    async fn fail_to_proxy(
        &self,
        session: &mut Session,
        e: &Error,
        ctx: &mut Self::CTX,
    ) -> FailToProxy {
        if (ctx.is_grpc || ctx.is_long_lived_stream) && grpc::is_benign_downstream_disconnect(e) {
            return FailToProxy {
                error_code: 0,
                can_reuse_downstream: false,
            };
        }

        let code = match e.etype() {
            ErrorType::HTTPStatus(code) => *code,
            _ => match e.esource() {
                ErrorSource::Upstream => 502,
                ErrorSource::Downstream => match e.etype() {
                    ErrorType::WriteError | ErrorType::ReadError | ErrorType::ConnectionClosed => 0,
                    _ => 400,
                },
                ErrorSource::Internal | ErrorSource::Unset => 500,
            },
        };
        if code > 0 {
            if let Err(err) = session.respond_error(code).await {
                tracing::debug!(error = %err, code, "failed to send error response to downstream");
            }
        }

        FailToProxy {
            error_code: code,
            can_reuse_downstream: false,
        }
    }
}

fn configure_upstream_peer(
    peer_host: &str,
    peer_port: u16,
    sni: &str,
    use_tls: bool,
    ctx: &RequestCtx,
) -> HttpPeer {
    let mut peer = HttpPeer::new((peer_host, peer_port), use_tls, sni.to_string());
    // Internal upstreams (Proxmox, etc.) often use self-signed certs.
    if use_tls {
        peer.options.verify_cert = false;
    }
    if grpc::is_h2c_only_upstream(sni, peer_port)
        || grpc::uses_h2c_upstream(ctx.is_grpc, ctx.is_grpc_web)
    {
        peer.options.alpn = ALPN::H2;
        peer.options.max_h2_streams = 128;
        peer.options.h2_ping_interval = Some(grpc::grpc_h2_ping_interval());
    }
    if ctx.is_grpc || ctx.is_long_lived_stream {
        let timeout = grpc::grpc_upstream_timeout();
        if timeout != std::time::Duration::MAX {
            peer.options.read_timeout = Some(timeout);
            peer.options.write_timeout = Some(timeout);
            peer.options.idle_timeout = Some(timeout);
        }
    } else if ctx.is_registry {
        let timeout = registry::registry_upstream_timeout();
        if timeout != std::time::Duration::MAX {
            peer.options.read_timeout = Some(timeout);
            peer.options.write_timeout = Some(timeout);
            peer.options.idle_timeout = Some(timeout);
        }
    }
    peer
}

fn downstream_protocol_label(session: &Session) -> &'static str {
    if session.as_downstream().is_http2() {
        "h2"
    } else {
        "http/1.1"
    }
}

fn protocol_short(label: &str) -> &str {
    match label {
        "h2" => "2",
        "http/1.1" => "1.1",
        other => other,
    }
}

/// Set `X-Real-IP` and `X-Forwarded-For` for upstream apps that need the client address.
fn inject_forwarded_client_ip(upstream_request: &mut RequestHeader, client_ip: &str) {
    let existing = upstream_request
        .headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok());
    for (name, value) in crate::proxy::forward::forwarded_client_ip_header_pairs(client_ip, existing) {
        upstream_request.insert_header(name, value).ok();
    }
}

fn is_downstream_tls(session: &Session) -> bool {
    if session
        .as_downstream()
        .digest()
        .and_then(|d| d.ssl_digest.as_ref())
        .is_some()
    {
        return true;
    }

    if session.as_downstream().is_http2() {
        return true;
    }

    matches!(
        session.req_header().uri.scheme_str(),
        Some("https") | Some("wss")
    ) || session
        .req_header()
        .headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        == Some("https")
}

fn is_plain_http(session: &Session) -> bool {
    if session
        .req_header()
        .headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        == Some("https")
    {
        return false;
    }

    !matches!(
        session.req_header().uri.scheme_str(),
        Some("https") | Some("wss")
    )
}

/// Alt-Svc for HTTP/3 discovery (RFC 7838). Advertised on both HTTP and HTTPS responses.
fn alt_svc_header_value(
    enable_h3: bool,
    h3_port: u16,
    host: &str,
    ctx: &RequestCtx,
) -> Option<String> {
    if ctx.is_grpc || ctx.is_long_lived_stream || grpc::is_machine_api_host(host) {
        return Some("clear".into());
    }
    if enable_h3 {
        return Some(format!(
            "h3=\":{h3_port}\"; ma=86400; persist=1, h3-29=\":{h3_port}\"; ma=86400; persist=1"
        ));
    }
    None
}

#[cfg(test)]
mod alt_svc_tests {
    use super::*;

    #[test]
    fn alt_svc_advertises_h3_when_enabled() {
        let ctx = RequestCtx::default();
        let v = alt_svc_header_value(true, 443, "example.com", &ctx).unwrap();
        assert!(v.contains("h3=\":443\""));
        assert!(v.contains("h3-29=\":443\""));
    }

    #[test]
    fn alt_svc_clear_for_grpc() {
        let mut ctx = RequestCtx::default();
        ctx.is_grpc = true;
        assert_eq!(
            alt_svc_header_value(true, 443, "example.com", &ctx).as_deref(),
            Some("clear")
        );
    }

    #[test]
    fn alt_svc_omitted_when_h3_disabled() {
        let ctx = RequestCtx::default();
        assert!(alt_svc_header_value(false, 443, "example.com", &ctx).is_none());
    }
}
