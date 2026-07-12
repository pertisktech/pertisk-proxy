//! Shared reverse-proxy routing used by Pingora (HTTP/1, HTTP/2) and HTTP/3.

use pingora_core::Result;
use pingora_error::ErrorType::HTTPStatus;

use crate::router::{Middleware, RouteTable};

#[derive(Debug, Clone, Default)]
pub struct MiddlewareAction {
    pub strip_prefix: Option<String>,
    pub request_headers: Vec<(String, String)>,
    pub response_headers: Vec<(String, String)>,
    pub forward_client_ip: bool,
}

#[derive(Debug, Clone)]
pub struct ForwardPlan {
    pub peer_host: String,
    pub peer_port: u16,
    pub upstream_url: String,
    pub use_tls: bool,
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

    let mut middleware = apply_middlewares(&route.middlewares);
    middleware.forward_client_ip = route.forward_client_ip;
    let upstream_path = if let Some(ref prefix) = middleware.strip_prefix {
        strip_path_prefix(path_and_query, prefix)
    } else {
        path_and_query.to_string()
    };

    let (peer_host, peer_port) = parse_backend_peer(&route.backend.address, route.backend.port)?;
    let upstream_url = build_upstream_url(&route.backend, &upstream_path);

    Ok(ForwardPlan {
        peer_host,
        peer_port,
        upstream_url,
        use_tls: route.backend.use_tls,
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

pub fn build_upstream_url(backend: &crate::router::Backend, path_and_query: &str) -> String {
    let scheme = if backend.use_tls { "https" } else { "http" };
    format!("{scheme}://{}{path_and_query}", backend.address)
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
                    address: "127.0.0.1:8080".into(),
                    port: 8080,
                    use_tls: false,
                },
                middlewares: vec![Middleware::StripPrefix {
                    prefix: "/api".into(),
                }],
                forward_client_ip: false,
            }],
        )]));

        let plan = resolve_forward(&table, "app.example.com", "/api/health?x=1").unwrap();
        assert_eq!(plan.upstream_url, "http://127.0.0.1:8080/health?x=1");
    }

    #[test]
    fn resolve_forward_without_middleware_keeps_path() {
        let table = RouteTable::from_routes(HashMap::from([(
            "app.example.com".into(),
            vec![Route {
                path: "/api".into(),
                path_type: PathMatchType::Prefix,
                backend: Backend {
                    address: "127.0.0.1:8080".into(),
                    port: 8080,
                    use_tls: false,
                },
                middlewares: vec![],
                forward_client_ip: false,
            }],
        )]));
        let plan = resolve_forward(&table, "app.example.com", "/api/status").unwrap();
        assert_eq!(plan.upstream_url, "http://127.0.0.1:8080/api/status");
    }

    #[test]
    fn apply_middlewares_collects_headers() {
        let mut headers = HashMap::new();
        headers.insert("X-Test".into(), "1".into());
        let action = apply_middlewares(&[
            Middleware::StripPrefix {
                prefix: "/api".into(),
            },
            Middleware::RequestHeaders {
                headers: headers.clone(),
            },
            Middleware::ResponseHeaders { headers },
        ]);
        assert_eq!(action.strip_prefix.as_deref(), Some("/api"));
        assert_eq!(action.request_headers.len(), 1);
        assert_eq!(action.response_headers.len(), 1);
    }

    #[test]
    fn build_upstream_url_adds_scheme() {
        assert_eq!(
            build_upstream_url(
                &Backend {
                    address: "127.0.0.1:8080".into(),
                    port: 8080,
                    use_tls: false,
                },
                "/health",
            ),
            "http://127.0.0.1:8080/health"
        );
        assert_eq!(
            build_upstream_url(
                &Backend {
                    address: "upstream:443".into(),
                    port: 443,
                    use_tls: true,
                },
                "/v1",
            ),
            "https://upstream:443/v1"
        );
    }

    #[test]
    fn parse_backend_peer_splits_host_port() {
        assert_eq!(
            parse_backend_peer("10.0.0.1:9090", 8080).unwrap(),
            ("10.0.0.1".into(), 9090)
        );
        assert_eq!(
            parse_backend_peer("upstream.local", 8080).unwrap(),
            ("upstream.local".into(), 8080)
        );
        assert!(parse_backend_peer("bad:port", 80).is_err());
    }

    #[test]
    fn strip_prefix_empty_becomes_root() {
        assert_eq!(strip_path_prefix("/api", "/api"), "/");
        assert_eq!(strip_path_prefix("/api?x=1", "/api"), "/?x=1");
    }

    #[test]
    fn resolve_forward_https_upstream() {
        let table = RouteTable::from_routes(HashMap::from([(
            "proxmox.example.com".into(),
            vec![Route {
                path: "/".into(),
                path_type: PathMatchType::Prefix,
                backend: Backend {
                    address: "10.1.1.65:8006".into(),
                    port: 8006,
                    use_tls: true,
                },
                middlewares: vec![],
                forward_client_ip: false,
            }],
        )]));
        let plan = resolve_forward(&table, "proxmox.example.com", "/").unwrap();
        assert_eq!(plan.upstream_url, "https://10.1.1.65:8006/");
        assert!(plan.use_tls);
    }

    #[test]
    fn resolve_forward_forwards_client_ip_flag() {
        let table = RouteTable::from_routes(HashMap::from([(
            "git.example.com".into(),
            vec![Route {
                path: "/".into(),
                path_type: PathMatchType::Prefix,
                backend: Backend {
                    address: "127.0.0.1:8080".into(),
                    port: 8080,
                    use_tls: false,
                },
                middlewares: vec![],
                forward_client_ip: true,
            }],
        )]));
        let plan = resolve_forward(&table, "git.example.com", "/").unwrap();
        assert!(plan.middleware.forward_client_ip);
    }

    #[test]
    fn resolve_forward_missing_route_errors() {
        let table = RouteTable::default();
        assert!(resolve_forward(&table, "missing.example", "/").is_err());
    }
}
