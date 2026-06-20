pub mod routes;
pub mod forward;

use std::sync::Arc;

use async_trait::async_trait;
use pingora_core::upstreams::peer::HttpPeer;
use pingora_core::Result;
use pingora_http::{RequestHeader, ResponseHeader};
use pingora_proxy::{ProxyHttp, Session};

use crate::deny;
use crate::health;
use crate::proxy::forward::resolve_forward;
use crate::router::Router;
use crate::tls::CertStore;

#[derive(Default)]
pub struct RequestCtx {
    pub strip_prefix: Option<String>,
    pub request_headers: Vec<(String, String)>,
    pub response_headers: Vec<(String, String)>,
}

impl From<crate::proxy::forward::MiddlewareAction> for RequestCtx {
    fn from(mw: crate::proxy::forward::MiddlewareAction) -> Self {
        Self {
            strip_prefix: mw.strip_prefix,
            request_headers: mw.request_headers,
            response_headers: mw.response_headers,
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
}

impl Gateway {
    pub fn new(
        router: Arc<Router>,
        cert_store: Arc<CertStore>,
        auto_https: bool,
        https_port: u16,
        enable_h3: bool,
        h3_port: u16,
    ) -> Self {
        Self {
            router,
            cert_store,
            auto_https,
            https_port,
            enable_h3,
            h3_port,
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
                ResponseHeader::build(http::StatusCode::PERMANENT_REDIRECT, Some(2))?;
            resp.insert_header("Location", target)?;
            resp.insert_header("Content-Length", "0")?;
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
            .unwrap_or(&path);

        if let Ok(plan) = resolve_forward(self.router.snapshot().as_ref(), &host, path_q) {
            *ctx = plan.middleware.into();
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

        if ctx.strip_prefix.is_none() && ctx.request_headers.is_empty() && ctx.response_headers.is_empty() {
            *ctx = plan.middleware.into();
        }

        let peer = Box::new(HttpPeer::new(
            (plan.peer_host.as_str(), plan.peer_port),
            false,
            host,
        ));
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

        if let Some(prefix) = &ctx.strip_prefix {
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

        for (name, value) in ctx.request_headers.clone() {
            upstream_request.insert_header(name, value).ok();
        }

        Ok(())
    }

    async fn upstream_response_filter(
        &self,
        session: &mut Session,
        upstream_response: &mut ResponseHeader,
        ctx: &mut Self::CTX,
    ) -> Result<()> {
        let protocol = downstream_protocol_label(session);
        upstream_response
            .insert_header("Server", format!("pertisk-proxy/{protocol}"))
            .ok();

        if is_downstream_tls(session) {
            if self.enable_h3 {
                let alt_svc = format!(
                    "h3=\":{}\"; ma=86400; persist=1, h3-29=\":{}\"; ma=86400; persist=1",
                    self.h3_port, self.h3_port
                );
                upstream_response.insert_header("Alt-Svc", alt_svc.as_str()).ok();
            } else {
                upstream_response.insert_header("Alt-Svc", "clear").ok();
            }
        }

        for (name, value) in ctx.response_headers.clone() {
            upstream_response.insert_header(name, value).ok();
        }

        Ok(())
    }
}

fn downstream_protocol_label(session: &Session) -> &'static str {
    if session.as_downstream().is_http2() {
        "h2"
    } else {
        "http/1.1"
    }
}

fn is_downstream_tls(session: &Session) -> bool {
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
