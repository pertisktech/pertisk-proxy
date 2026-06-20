//! Shared reverse-proxy routing used by Pingora (HTTP/1, HTTP/2) and HTTP/3.

use pingora_core::Result;
use pingora_error::ErrorType::HTTPStatus;

use crate::router::{Middleware, RouteTable};

#[derive(Debug, Clone, Default)]
pub struct MiddlewareAction {
    pub strip_prefix: Option<String>,
    pub request_headers: Vec<(String, String)>,
    pub response_headers: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct ForwardPlan {
    pub peer_host: String,
    pub peer_port: u16,
    pub upstream_url: String,
    pub middleware: MiddlewareAction,
}

pub fn apply_middlewares(middlewares: &[Middleware]) -> MiddlewareAction {
    let mut action = MiddlewareAction::default();
    for mw in middlewares {
        match mw {
            Middleware::StripPrefix { prefix } => action.strip_prefix = Some(prefix.clone()),
            Middleware::RequestHeaders { headers } => {
                for (k, v) in headers {
                    action.request_headers.push((k.clone(), v.clone()));
                }
            }
            Middleware::ResponseHeaders { headers } => {
                for (k, v) in headers {
                    action.response_headers.push((k.clone(), v.clone()));
                }
            }
        }
    }
    action
}

pub fn resolve_forward(
    table: &RouteTable,
    host: &str,
    path_and_query: &str,
) -> Result<ForwardPlan> {
    let path = path_and_query.split('?').next().unwrap_or(path_and_query);
    let route = table.match_route_entry(host, path).ok_or_else(|| {
        pingora_error::Error::explain(
            HTTPStatus(404),
            format!("no route for host={host} path={path}"),
        )
    })?;

    let middleware = apply_middlewares(&route.middlewares);
    let upstream_path = if let Some(ref prefix) = middleware.strip_prefix {
        strip_path_prefix(path_and_query, prefix)
    } else {
        path_and_query.to_string()
    };

    let (peer_host, peer_port) = parse_backend_peer(&route.backend.address, route.backend.port)?;
    let upstream_url = build_upstream_url(&route.backend.address, &upstream_path);

    Ok(ForwardPlan {
        peer_host,
        peer_port,
        upstream_url,
        middleware,
    })
}

fn strip_path_prefix(path_and_query: &str, prefix: &str) -> String {
    let (path, query) = match path_and_query.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path_and_query, None),
    };

    let stripped = path.strip_prefix(prefix).unwrap_or(path);
    let new_path = if stripped.is_empty() { "/" } else { stripped };

    match query {
        Some(q) => format!("{new_path}?{q}"),
        None => new_path.to_string(),
    }
}

pub fn build_upstream_url(backend_address: &str, path_and_query: &str) -> String {
    if backend_address.contains("://") {
        format!("{backend_address}{path_and_query}")
    } else {
        format!("http://{backend_address}{path_and_query}")
    }
}

pub fn parse_backend_peer(address: &str, fallback_port: u16) -> Result<(String, u16)> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::{Backend, PathMatchType, Route};
    use std::collections::HashMap;

    #[test]
    fn strip_prefix_keeps_query() {
        assert_eq!(
            strip_path_prefix("/api/v1/health?x=1", "/api"),
            "/v1/health?x=1"
        );
    }

    #[test]
    fn resolve_forward_applies_strip_prefix() {
        let table = RouteTable::from_routes(HashMap::from([(
            "app.example.com".into(),
            vec![Route {
                path: "/api".into(),
                path_type: PathMatchType::Prefix,
                backend: Backend {
                    address: "http://127.0.0.1:8080".into(),
                    port: 8080,
                },
                middlewares: vec![Middleware::StripPrefix {
                    prefix: "/api".into(),
                }],
            }],
        )]));

        let plan = resolve_forward(&table, "app.example.com", "/api/health").unwrap();
        assert_eq!(plan.upstream_url, "http://127.0.0.1:8080/health");
    }
}
