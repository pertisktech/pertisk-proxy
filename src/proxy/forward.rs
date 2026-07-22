//! Shared reverse-proxy routing used by Pingora (HTTP/1, HTTP/2) and HTTP/3.

use std::net::IpAddr;

use pingora_core::Result;
use pingora_error::ErrorType::HTTPStatus;

use crate::router::{Middleware, RouteTable};

#[derive(Debug, Clone)]
pub struct MiddlewareAction {
    pub strip_prefix: Option<String>,
    pub request_headers: Vec<(String, String)>,
    pub response_headers: Vec<(String, String)>,
    pub forward_client_ip: bool,
    pub geoip: crate::geoip::GeoIpPolicy,
    pub security: crate::security::SecurityPolicy,
}

impl Default for MiddlewareAction {
    fn default() -> Self {
        Self {
            strip_prefix: None,
            request_headers: Vec::new(),
            response_headers: Vec::new(),
            forward_client_ip: true,
            geoip: Default::default(),
            security: Default::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ForwardPlan {
    pub peer_host: String,
    pub peer_port: u16,
    pub upstream_url: String,
    pub use_tls: bool,
    pub middleware: MiddlewareAction,
}

/// Header pairs for upstream apps that need the client address.
pub fn forwarded_client_ip_header_pairs(
    client_ip: &str,
    existing_xff: Option<&str>,
) -> Vec<(&'static str, String)> {
    let xff = existing_xff
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|existing| format!("{existing}, {client_ip}"))
        .unwrap_or_else(|| client_ip.to_string());
    vec![
        ("X-Real-IP", client_ip.to_string()),
        ("X-Forwarded-For", xff),
    ]
}

fn first_hop_from_xff(xff: Option<&str>) -> Option<String> {
    let ip = xff?
        .split(',')
        .next()?
        .trim()
        .trim_matches('"');
    if ip.is_empty() {
        None
    } else {
        Some(ip.to_string())
    }
}

/// Resolve the best-known client IP from the downstream socket and proxy headers.
pub fn resolve_client_ip(
    socket_ip: Option<&str>,
    xff: Option<&str>,
    x_real_ip: Option<&str>,
) -> Option<String> {
    // Prefer the TCP peer when it looks like a real client. Behind kube-proxy with
    // externalTrafficPolicy=Cluster the peer is often a private SNAT address — skip it
    // so GeoIP / forwarding can use X-Real-IP / X-Forwarded-For instead.
    if let Some(ip) = socket_ip.map(str::trim).filter(|value| !value.is_empty()) {
        if is_public_routable_ip(ip) {
            return Some(ip.to_string());
        }
    }
    if let Some(ip) = x_real_ip.map(str::trim).filter(|value| !value.is_empty()) {
        if is_public_routable_ip(ip) || socket_ip.is_none() {
            return Some(ip.to_string());
        }
        // private X-Real-IP only if we have nothing better from XFF
    }
    if let Some(ip) = first_hop_from_xff(xff) {
        return Some(ip);
    }
    if let Some(ip) = x_real_ip.map(str::trim).filter(|value| !value.is_empty()) {
        return Some(ip.to_string());
    }
    // Last resort: private socket (Cluster ET policy, no forwarded headers).
    socket_ip
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// True for addresses that are not private/loopback/link-local (usable as client IP).
pub fn is_public_routable_ip(ip: &str) -> bool {
    match ip.trim().parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => {
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast())
        }
        Ok(IpAddr::V6(v6)) => {
            !(v6.is_loopback()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || v6.is_unspecified())
        }
        Err(_) => false,
    }
}

pub fn client_ip_from_http_headers(headers: &http::HeaderMap) -> Option<String> {
    for name in [
        "cf-connecting-ip",
        "true-client-ip",
        "x-client-ip",
        "x-real-ip",
        "x-forwarded-for",
    ] {
        if let Some(value) = headers.get(name).and_then(|v| v.to_str().ok()) {
            if name == "x-forwarded-for" {
                if let Some(ip) = first_hop_from_xff(Some(value)) {
                    return Some(ip);
                }
            } else if !value.trim().is_empty() {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

/// Append `X-Real-IP` / `X-Forwarded-For` to a header map (HTTP/3 upstream hops).
pub fn apply_forwarded_client_ip_headers(headers: &mut http::HeaderMap, client_ip: &str) {
    let existing = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok());
    for (name, value) in forwarded_client_ip_header_pairs(client_ip, existing) {
        if let Ok(header_value) = http::HeaderValue::from_str(&value) {
            headers.insert(name, header_value);
        }
    }
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
    middleware.geoip = route.geoip.clone();
    middleware.security = route.security.clone();
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
                geoip: Default::default(),
                security: Default::default(),
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
                geoip: Default::default(),
                security: Default::default(),
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
    fn resolve_client_ip_prefers_public_socket() {
        let ip = resolve_client_ip(Some("203.0.113.9"), Some("198.51.100.1"), None);
        assert_eq!(ip.as_deref(), Some("203.0.113.9"));
    }

    #[test]
    fn resolve_client_ip_skips_private_socket_for_xff() {
        let ip = resolve_client_ip(Some("10.244.1.5"), Some("203.0.113.9, 10.0.0.1"), None);
        assert_eq!(ip.as_deref(), Some("203.0.113.9"));
    }

    #[test]
    fn resolve_client_ip_falls_back_to_xff() {
        let ip = resolve_client_ip(None, Some("203.0.113.9, 10.0.0.1"), None);
        assert_eq!(ip.as_deref(), Some("203.0.113.9"));
    }

    #[test]
    fn forwarded_client_ip_header_pairs_appends_existing_xff() {
        let pairs = forwarded_client_ip_header_pairs(
            "203.0.113.5",
            Some("198.51.100.1"),
        );
        assert_eq!(pairs[0], ("X-Real-IP", "203.0.113.5".to_string()));
        assert_eq!(
            pairs[1],
            ("X-Forwarded-For", "198.51.100.1, 203.0.113.5".to_string())
        );
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
                geoip: Default::default(),
                security: Default::default(),
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
                geoip: Default::default(),
                security: Default::default(),
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
