use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use pingora_core::upstreams::peer::HttpPeer;
use pingora_core::Result;
use pingora_error::ErrorType::HTTPStatus;
use pingora_http::RequestHeader;
use pingora_proxy::{ProxyHttp, Session};

use crate::router::Router;

pub struct K8sGateway {
    router: Arc<Router>,
}

impl K8sGateway {
    pub fn new(router: Arc<Router>) -> Self {
        Self { router }
    }
}

#[async_trait]
impl ProxyHttp for K8sGateway {
    type CTX = ();

    fn new_ctx(&self) -> Self::CTX {}

    async fn request_filter(&self, session: &mut Session, _ctx: &mut Self::CTX) -> Result<bool> {
        let path = session.req_header().uri.path();
        if path == "/healthz" || path == "/readyz" {
            session
                .respond_error_with_body(200, Bytes::from_static(b"ok"))
                .await?;
            return Ok(true);
        }
        Ok(false)
    }

    async fn upstream_peer(
        &self,
        session: &mut Session,
        _ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        let req = session.req_header();
        let host = req
            .headers
            .get("host")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        let path = req.uri.path();

        let table = self.router.snapshot();
        let backend = table.match_route(host, path).ok_or_else(|| {
            pingora_error::Error::explain(
                HTTPStatus(404),
                format!("no route for host={host} path={path}"),
            )
        })?;

        let (address, port) = parse_address(&backend.address, backend.port)?;
        let peer = Box::new(HttpPeer::new((address.as_str(), port), false, host.to_string()));
        Ok(peer)
    }

    async fn upstream_request_filter(
        &self,
        session: &mut Session,
        upstream_request: &mut RequestHeader,
        _ctx: &mut Self::CTX,
    ) -> Result<()> {
        if let Some(host) = session
            .req_header()
            .headers
            .get("host")
            .and_then(|v| v.to_str().ok())
        {
            upstream_request.insert_header("Host", host).ok();
        }
        Ok(())
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
