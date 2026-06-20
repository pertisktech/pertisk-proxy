use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use k8s_openapi::api::networking::v1::{
    HTTPIngressRuleValue, Ingress, IngressBackend, IngressServiceBackend,
};
use serde::Deserialize;
use tracing::debug;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathMatchType {
    Exact,
    Prefix,
    ImplementationSpecific,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backend {
    pub address: String,
    pub port: u16,
}

/// Traefik-style middleware applied to a route (ignored in ingress/nginx/caddy modes except traefik).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Middleware {
    StripPrefix { prefix: String },
    RequestHeaders { headers: HashMap<String, String> },
    ResponseHeaders { headers: HashMap<String, String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub path: String,
    pub path_type: PathMatchType,
    pub backend: Backend,
    pub middlewares: Vec<Middleware>,
}

#[derive(Debug, Default)]
pub struct RouteTable {
    /// Host (lowercase) -> ordered routes (longest prefix first).
    routes: HashMap<String, Vec<Route>>,
}

impl RouteTable {
    pub fn match_route(&self, host: &str, path: &str) -> Option<&Backend> {
        self.match_route_entry(host, path).map(|r| &r.backend)
    }

    pub fn match_route_entry(&self, host: &str, path: &str) -> Option<&Route> {
        let host = normalize_host(host);

        if let Some(routes) = self.routes.get(&host) {
            if let Some(route) = find_route(routes, path) {
                return Some(route);
            }
        }

        if let Some(routes) = self.routes.get("*") {
            return find_route(routes, path);
        }

        None
    }

    /// True when the host has at least one route or a `*` catch-all is configured.
    pub fn has_host(&self, host: &str) -> bool {
        let host = normalize_host(host);
        !host.is_empty() && (self.routes.contains_key(&host) || self.routes.contains_key("*"))
    }

    pub fn all_routes(&self) -> impl Iterator<Item = (&String, &Route)> {
        self.routes
            .iter()
            .flat_map(|(host, routes)| routes.iter().map(move |r| (host, r)))
    }

    pub fn route_count(&self) -> usize {
        self.routes.values().map(|routes| routes.len()).sum()
    }

    pub fn from_routes(mut by_host: HashMap<String, Vec<Route>>) -> Self {
        for routes in by_host.values_mut() {
            routes.sort_by(|a, b| b.path.len().cmp(&a.path.len()));
        }
        Self { routes: by_host }
    }
}

/// Parse an upstream URL or host:port into a backend address.
pub fn parse_upstream(raw: &str) -> Option<Backend> {
    let trimmed = raw.trim().trim_end_matches(';').trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.contains("://") {
        if let Ok(url) = url::Url::parse(trimmed) {
            let host = url.host_str()?;
            let port = url.port_or_known_default().unwrap_or(80);
            return Some(Backend {
                address: format!("{host}:{port}"),
                port,
            });
        }
    }

    parse_host_port(trimmed)
}

fn parse_host_port(raw: &str) -> Option<Backend> {
    let without_path = raw.split('/').next().unwrap_or(raw);
    if let Some((host, port)) = without_path.rsplit_once(':') {
        let port = port.parse().ok()?;
        return Some(Backend {
            address: format!("{host}:{port}"),
            port,
        });
    }

    Some(Backend {
        address: format!("{without_path}:80"),
        port: 80,
    })
}

fn find_route<'a>(routes: &'a [Route], path: &str) -> Option<&'a Route> {
    for route in routes {
        if path_matches(&route.path_type, &route.path, path) {
            return Some(route);
        }
    }
    None
}

fn path_matches(path_type: &PathMatchType, rule_path: &str, request_path: &str) -> bool {
    match path_type {
        PathMatchType::Exact => rule_path == request_path,
        PathMatchType::Prefix => {
            request_path == rule_path
                || (rule_path.ends_with('/')
                    && request_path.starts_with(rule_path))
                || (!rule_path.ends_with('/')
                    && (request_path.starts_with(&format!("{rule_path}/"))
                        || request_path == rule_path))
        }
        PathMatchType::ImplementationSpecific => request_path.starts_with(rule_path),
    }
}

fn normalize_host(host: &str) -> String {
    host.split(':').next().unwrap_or(host).to_ascii_lowercase()
}

fn service_dns(name: &str, namespace: &str, port: u16) -> String {
    format!("{name}.{namespace}.svc.cluster.local:{port}")
}

use crate::http3_options::Http3Options;

pub struct Router {
    table: ArcSwap<RouteTable>,
    http3: ArcSwap<Http3Options>,
}

impl Router {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            table: ArcSwap::from_pointee(RouteTable::default()),
            http3: ArcSwap::from_pointee(Http3Options::default()),
        })
    }

    pub fn snapshot(&self) -> Arc<RouteTable> {
        self.table.load_full()
    }

    pub fn http3_options(&self) -> Arc<Http3Options> {
        self.http3.load_full()
    }

    pub fn replace(&self, table: RouteTable) {
        debug!(routes = table.route_count(), "updated routing table");
        self.table.store(Arc::new(table));
    }

    pub fn replace_http3(&self, http3: Http3Options) {
        debug!(
            max_streams_bidi = ?http3.max_streams_bidi,
            enable_0rtt = ?http3.enable_0rtt,
            "updated HTTP/3 options"
        );
        self.http3.store(Arc::new(http3));
    }

    pub fn replace_all(&self, table: RouteTable, http3: Http3Options) {
        self.replace(table);
        self.replace_http3(http3);
    }
}

impl Default for Router {
    fn default() -> Self {
        Self {
            table: ArcSwap::from_pointee(RouteTable::default()),
            http3: ArcSwap::from_pointee(Http3Options::default()),
        }
    }
}

pub fn build_route_table_from_ingresses(
    ingresses: impl IntoIterator<Item = Ingress>,
) -> RouteTable {
    let mut by_host: HashMap<String, Vec<Route>> = HashMap::new();

    for ingress in ingresses {
        add_ingress_to_table(&mut by_host, &ingress);
    }

    for routes in by_host.values_mut() {
        routes.sort_by(|a, b| b.path.len().cmp(&a.path.len()));
    }

    RouteTable { routes: by_host }
}

fn add_ingress_to_table(by_host: &mut HashMap<String, Vec<Route>>, ingress: &Ingress) {
    let namespace = ingress
        .metadata
        .namespace
        .as_deref()
        .unwrap_or("default");

    let Some(spec) = ingress.spec.as_ref() else {
        return;
    };

    if let Some(default_backend) = spec.default_backend.as_ref() {
        if let Some(backend) = resolve_backend(default_backend, namespace) {
            by_host.entry("*".into()).or_default().push(Route {
                path: "/".into(),
                path_type: PathMatchType::Prefix,
                backend,
                middlewares: vec![],
            });
        }
    }

    for rule in spec.rules.iter().flatten() {
        let host = rule
            .host
            .as_deref()
            .map(normalize_host)
            .unwrap_or_else(|| "*".into());

        let Some(http) = rule.http.as_ref() else {
            continue;
        };

        collect_http_paths(&host, http, namespace, by_host);
    }
}

fn collect_http_paths(
    host: &str,
    http: &HTTPIngressRuleValue,
    namespace: &str,
    by_host: &mut HashMap<String, Vec<Route>>,
) {
    for path in http.paths.iter() {
        let Some(backend) = resolve_backend(&path.backend, namespace) else {
            continue;
        };

        let (path_value, path_type) = match path.path_type.as_str() {
            "Exact" => (
                path.path.clone().unwrap_or_else(|| "/".into()),
                PathMatchType::Exact,
            ),
            "ImplementationSpecific" => (
                path.path.clone().unwrap_or_else(|| "/".into()),
                PathMatchType::ImplementationSpecific,
            ),
            _ => (
                path.path.clone().unwrap_or_else(|| "/".into()),
                PathMatchType::Prefix,
            ),
        };

        by_host.entry(host.to_string()).or_default().push(Route {
            path: path_value,
            path_type,
            backend,
            middlewares: vec![],
        });
    }
}

fn resolve_backend(backend: &IngressBackend, namespace: &str) -> Option<Backend> {
    let service = backend.service.as_ref()?;
    resolve_service_backend(service, namespace)
}

fn resolve_service_backend(service: &IngressServiceBackend, namespace: &str) -> Option<Backend> {
    let name = service.name.as_str();
    let port = service.port.as_ref()?;
    let port_number = port.number?;

    Some(Backend {
        address: service_dns(name, namespace, port_number as u16),
        port: port_number as u16,
    })
}

pub fn ingress_matches_class(ingress: &Ingress, class: Option<&str>) -> bool {
    let Some(expected) = class else {
        return true;
    };

    ingress
        .spec
        .as_ref()
        .and_then(|spec| spec.ingress_class_name.as_deref())
        == Some(expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_matching() {
        let table = RouteTable {
            routes: HashMap::from([(
                "app.example.com".into(),
                vec![Route {
                    path: "/api".into(),
                    path_type: PathMatchType::Prefix,
                    backend: Backend {
                        address: "api.default.svc.cluster.local:8080".into(),
                        port: 8080,
                    },
                    middlewares: vec![],
                }],
            )]),
        };

        assert!(table.match_route("app.example.com", "/api/v1").is_some());
        assert!(table.match_route("app.example.com", "/health").is_none());
    }

    #[test]
    fn has_host_matches_configured_and_wildcard() {
        let table = RouteTable {
            routes: HashMap::from([
                (
                    "app.example.com".into(),
                    vec![Route {
                        path: "/".into(),
                        path_type: PathMatchType::Prefix,
                        backend: Backend {
                            address: "upstream:8080".into(),
                            port: 8080,
                        },
                        middlewares: vec![],
                    }],
                ),
                (
                    "*".into(),
                    vec![Route {
                        path: "/".into(),
                        path_type: PathMatchType::Prefix,
                        backend: Backend {
                            address: "catchall:8080".into(),
                            port: 8080,
                        },
                        middlewares: vec![],
                    }],
                ),
            ]),
        };

        assert!(table.has_host("app.example.com"));
        assert!(table.has_host("APP.EXAMPLE.COM:443"));
        assert!(table.has_host("other.example.com"));
        assert!(!table.has_host(""));
    }
}
