pub mod kinds;

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use pingora_core::upstreams::peer::HttpPeer;
use pingora_core::Result;
use pingora_error::ErrorType::HTTPStatus;
use pingora_http::{RequestHeader, ResponseHeader};
use pingora_proxy::{ProxyHttp, Session};
use serde_json::json;

use crate::mode::{OperatingMode, ProxyKind};
use crate::router::{Middleware, RouteTable};
use crate::Router;

/// Per-request state for middleware and routing.
#[derive(Default)]
pub struct RequestCtx {
    pub strip_prefix: Option<String>,
    pub request_headers: Vec<(String, String)>,
    pub response_headers: Vec<(String, String)>,
}

pub struct Gateway {
    router: Arc<Router>,
    mode: OperatingMode,
    auto_https: bool,
    https_port: u16,
}

impl Gateway {
    pub fn new(router: Arc<Router>, mode: OperatingMode, auto_https: bool, https_port: u16) -> Self {
        Self {
            router,
            mode,
            auto_https,
            https_port,
        }
    }

    fn proxy_kind(&self) -> Option<ProxyKind> {
        match &self.mode {
            OperatingMode::Proxy(kind) => Some(*kind),
            OperatingMode::Ingress => None,
        }
    }

    fn server_header(&self) -> &'static str {
        match self.proxy_kind() {
            Some(ProxyKind::Nginx) => "pertisk-proxy/nginx",
            Some(ProxyKind::Caddy) => "Caddy",
            Some(ProxyKind::Traefik) => "Traefik",
            None => "pertisk-proxy",
        }
    }
}

#[async_trait]
impl ProxyHttp for Gateway {
    type CTX = RequestCtx;

    fn new_ctx(&self) -> Self::CTX {
        RequestCtx::default()
    }

    async fn request_filter(&self, session: &mut Session, ctx: &mut Self::CTX) -> Result<bool> {
        let path = session.req_header().uri.path().to_string();

        if path == "/healthz" || path == "/readyz" {
            session
                .respond_error_with_body(200, Bytes::from_static(b"ok"))
                .await?;
            return Ok(true);
        }

        if self.proxy_kind() == Some(ProxyKind::Traefik) && path.starts_with("/api/") {
            return self.serve_traefik_api(session, &path).await;
        }

        if self.auto_https && is_plain_http(session) {
            let req = session.req_header();
            let host = req
                .headers
                .get("host")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("localhost");
            let host = host.split(':').next().unwrap_or(host);
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

        let host = session
            .req_header()
            .headers
            .get("host")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();

        if let Some(route) = self.router.snapshot().match_route_entry(&host, &path) {
            if self.proxy_kind() == Some(ProxyKind::Traefik) {
                apply_middleware_ctx(ctx, &route.middlewares);
            }
        }

        Ok(false)
    }

    async fn upstream_peer(
        &self,
        session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        let req = session.req_header();
        let host = req
            .headers
            .get("host")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        let path = req.uri.path();

        let table = self.router.snapshot();
        let route = table.match_route_entry(host, path).ok_or_else(|| {
            pingora_error::Error::explain(
                HTTPStatus(404),
                format!("no route for host={host} path={path}"),
            )
        })?;

        if self.proxy_kind() == Some(ProxyKind::Traefik) && ctx.strip_prefix.is_none() {
            apply_middleware_ctx(ctx, &route.middlewares);
        }

        let (address, port) = parse_address(&route.backend.address, route.backend.port)?;
        let peer = Box::new(HttpPeer::new((address.as_str(), port), false, host.to_string()));
        Ok(peer)
    }

    async fn upstream_request_filter(
        &self,
        session: &mut Session,
        upstream_request: &mut RequestHeader,
        ctx: &mut Self::CTX,
    ) -> Result<()> {
        if let Some(host) = session
            .req_header()
            .headers
            .get("host")
            .and_then(|v| v.to_str().ok())
        {
            upstream_request.insert_header("Host", host).ok();
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
        _session: &mut Session,
        upstream_response: &mut ResponseHeader,
        ctx: &mut Self::CTX,
    ) -> Result<()> {
        upstream_response
            .insert_header("Server", self.server_header())
            .ok();

        for (name, value) in ctx.response_headers.clone() {
            upstream_response.insert_header(name, value).ok();
        }

        Ok(())
    }
}

impl Gateway {
    async fn serve_traefik_api(&self, session: &mut Session, path: &str) -> Result<bool> {
        let body = match path {
            "/api/http/routers" => traefik_routers_json(&self.router.snapshot()),
            "/api/overview" => json!({
                "mode": "proxy/traefik",
                "provider": "file",
            })
            .to_string(),
            _ => return Ok(false),
        };

        session
            .respond_error_with_body(200, Bytes::from(body))
            .await?;
        Ok(true)
    }
}

fn traefik_routers_json(table: &RouteTable) -> String {
    let routers: Vec<_> = table
        .all_routes()
        .map(|(host, route)| {
            json!({
                "host": host,
                "path": route.path,
                "upstream": route.backend.address,
            })
        })
        .collect();
    json!({ "routers": routers }).to_string()
}

fn apply_middleware_ctx(ctx: &mut RequestCtx, middlewares: &[Middleware]) {
    for mw in middlewares {
        match mw {
            Middleware::StripPrefix { prefix } => ctx.strip_prefix = Some(prefix.clone()),
            Middleware::RequestHeaders { headers } => {
                for (k, v) in headers {
                    ctx.request_headers.push((k.clone(), v.clone()));
                }
            }
            Middleware::ResponseHeaders { headers } => {
                for (k, v) in headers {
                    ctx.response_headers.push((k.clone(), v.clone()));
                }
            }
        }
    }
}

fn parse_address(address: &str, fallback_port: u16) -> Result<(String, u16)> {
    if let Some((host, port)) = address.rsplit_once(':') {
        let port = port.parse::<u16>().map_err(|_| {
            pingora_error::Error::explain(
                pingora_error::ErrorType::InternalError,
                format!("invalid port in backend address: {address}"),
            )
        })?;
        return Ok((host.to_string(), port));
    }

    Ok((address.to_string(), fallback_port))
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
