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
    /// Connect to upstream over TLS when true (upstream URL used `https://`).
    pub use_tls: bool,
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
        let routes = self.routes_for_host(host)?;
        find_route(routes, path)
    }

    /// True when the host has at least one route (exact, wildcard, or `*` catch-all).
    pub fn has_host(&self, host: &str) -> bool {
        let host = normalize_host(host);
        !host.is_empty() && self.routes_for_host(&host).is_some()
    }

    fn routes_for_host(&self, host: &str) -> Option<&Vec<Route>> {
        let host = normalize_host(host);
        if host.is_empty() {
            return None;
        }

        if let Some(routes) = self.routes.get(&host) {
            return Some(routes);
        }

        let mut best: Option<(usize, &Vec<Route>)> = None;
        for (key, routes) in &self.routes {
            if key.starts_with("*.") && crate::proxy_config::wildcard_covers_host(key, &host) {
                let suffix = key.strip_prefix('*').unwrap_or(key);
                let score = suffix.len();
                if best.as_ref().map(|(s, _)| *s).unwrap_or(0) < score {
                    best = Some((score, routes));
                }
            }
        }
        if let Some((_, routes)) = best {
            return Some(routes);
        }

        self.routes.get("*")
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
            let host = normalize_upstream_host(url.host_str()?);
            let use_tls = url.scheme() == "https";
            let default_port = if use_tls { 443 } else { 80 };
            let port = url.port_or_known_default().unwrap_or(default_port);
            return Some(Backend {
                address: format!("{host}:{port}"),
                port,
                use_tls,
            });
        }
    }

    parse_host_port(trimmed)
}

fn parse_host_port(raw: &str) -> Option<Backend> {
    let without_path = raw.split('/').next().unwrap_or(raw);
    if let Some((host, port)) = without_path.rsplit_once(':') {
        let port = port.parse().ok()?;
        let host = normalize_upstream_host(host);
        return Some(Backend {
            address: format!("{host}:{port}"),
            port,
            use_tls: false,
        });
    }

    let host = normalize_upstream_host(without_path);
    Some(Backend {
        address: format!("{host}:80"),
        port: 80,
        use_tls: false,
    })
}

/// macOS often resolves `localhost` to `[::1]` while local services bind `127.0.0.1` only.
fn normalize_upstream_host(host: &str) -> String {
    if host.eq_ignore_ascii_case("localhost") {
        "127.0.0.1".to_string()
    } else {
        host.to_string()
    }
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

    pub fn route_count(&self) -> usize {
        self.snapshot().route_count()
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
        use_tls: false,
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
    fn parse_upstream_localhost_uses_ipv4() {
        let backend = parse_upstream("http://localhost:9080").expect("parse");
        assert_eq!(backend.address, "127.0.0.1:9080");
        assert_eq!(backend.port, 9080);
    }

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
                        use_tls: false,
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
                            use_tls: false,
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
                            use_tls: false,
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

    #[test]
    fn exact_path_matching() {
        let table = RouteTable::from_routes(HashMap::from([(
            "app.example.com".into(),
            vec![Route {
                path: "/api".into(),
                path_type: PathMatchType::Exact,
                backend: Backend {
                    address: "api:8080".into(),
                    port: 8080,
                    use_tls: false,
                },
                middlewares: vec![],
            }],
        )]));
        assert!(table.match_route("app.example.com", "/api").is_some());
        assert!(table.match_route("app.example.com", "/api/v1").is_none());
    }

    #[test]
    fn catch_all_wildcard_host() {
        let table = RouteTable::from_routes(HashMap::from([(
            "*".into(),
            vec![Route {
                path: "/".into(),
                path_type: PathMatchType::Prefix,
                backend: Backend {
                    address: "catch:80".into(),
                    port: 80,
                    use_tls: false,
                },
                middlewares: vec![],
            }],
        )]));
        assert!(table.match_route("unknown.host", "/any").is_some());
    }

    #[test]
    fn subdomain_wildcard_host_routes() {
        let table = RouteTable::from_routes(HashMap::from([(
            "*.orion.thaidevops.co".into(),
            vec![Route {
                path: "/".into(),
                path_type: PathMatchType::Prefix,
                backend: Backend {
                    address: "10.1.1.183:443".into(),
                    port: 443,
                    use_tls: true,
                },
                middlewares: vec![],
            }],
        )]));
        assert!(table.match_route("app.orion.thaidevops.co", "/").is_some());
        assert!(table.has_host("app.orion.thaidevops.co"));
        assert!(!table.match_route("orion.thaidevops.co", "/").is_some());
        assert!(!table.match_route("a.b.orion.thaidevops.co", "/").is_some());
    }

    #[test]
    fn exact_host_beats_wildcard() {
        let table = RouteTable::from_routes(HashMap::from([
            (
                "*.example.com".into(),
                vec![Route {
                    path: "/".into(),
                    path_type: PathMatchType::Prefix,
                    backend: Backend {
                        address: "wildcard:80".into(),
                        port: 80,
                        use_tls: false,
                    },
                    middlewares: vec![],
                }],
            ),
            (
                "app.example.com".into(),
                vec![Route {
                    path: "/".into(),
                    path_type: PathMatchType::Prefix,
                    backend: Backend {
                        address: "exact:80".into(),
                        port: 80,
                        use_tls: false,
                    },
                    middlewares: vec![],
                }],
            ),
        ]));
        assert_eq!(
            table.match_route("app.example.com", "/").unwrap().address,
            "exact:80"
        );
    }

    #[test]
    fn longest_prefix_wins() {
        let table = RouteTable::from_routes(HashMap::from([(
            "app.example.com".into(),
            vec![
                Route {
                    path: "/api".into(),
                    path_type: PathMatchType::Prefix,
                    backend: Backend {
                        address: "short:8080".into(),
                        port: 8080,
                        use_tls: false,
                    },
                    middlewares: vec![],
                },
                Route {
                    path: "/api/v2".into(),
                    path_type: PathMatchType::Prefix,
                    backend: Backend {
                        address: "long:8080".into(),
                        port: 8080,
                        use_tls: false,
                    },
                    middlewares: vec![],
                },
            ],
        )]));
        assert_eq!(
            table.match_route("app.example.com", "/api/v2/x").unwrap().address,
            "long:8080"
        );
    }

    #[test]
    fn parse_upstream_variants() {
        assert!(parse_upstream("").is_none());
        assert!(parse_upstream("  ").is_none());
        let b = parse_upstream("backend:9090;").unwrap();
        assert_eq!(b.port, 9090);
        let b = parse_upstream("https://example.com").unwrap();
        assert_eq!(b.port, 443);
        assert!(b.use_tls);
        let b = parse_upstream("https://10.1.1.65:8006").unwrap();
        assert_eq!(b.address, "10.1.1.65:8006");
        assert!(b.use_tls);
        let b = parse_upstream("http://backend:8080").unwrap();
        assert!(!b.use_tls);
        let b = parse_upstream("example.com").unwrap();
        assert_eq!(b.address, "example.com:80");
        let b = parse_upstream("10.0.0.1:8080").unwrap();
        assert_eq!(b.port, 8080);
    }

    #[test]
    fn route_table_counts_and_iter() {
        let table = RouteTable::from_routes(HashMap::from([(
            "a.example.com".into(),
            vec![Route {
                path: "/".into(),
                path_type: PathMatchType::Prefix,
                backend: Backend {
                    address: "a:80".into(),
                    port: 80,
                    use_tls: false,
                },
                middlewares: vec![],
            }],
        )]));
        assert_eq!(table.route_count(), 1);
        assert_eq!(table.all_routes().count(), 1);
    }

    #[test]
    fn router_replace_and_snapshot() {
        let router = Router::new();
        assert_eq!(router.route_count(), 0);
        let table = RouteTable::from_routes(HashMap::from([(
            "x.example.com".into(),
            vec![Route {
                path: "/".into(),
                path_type: PathMatchType::Prefix,
                backend: Backend {
                    address: "x:80".into(),
                    port: 80,
                    use_tls: false,
                },
                middlewares: vec![],
            }],
        )]));
        router.replace(table);
        assert_eq!(router.route_count(), 1);
        assert!(router.snapshot().has_host("x.example.com"));
        router.replace_http3(crate::http3_options::Http3Options {
            max_streams_bidi: Some(100),
            ..Default::default()
        });
        assert_eq!(router.http3_options().max_streams_bidi, Some(100));
    }

    #[test]
    fn ingress_matches_class_filter() {
        use k8s_openapi::api::networking::v1::{Ingress, IngressSpec};
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

        let ingress = Ingress {
            metadata: ObjectMeta::default(),
            spec: Some(IngressSpec {
                ingress_class_name: Some("nginx".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(ingress_matches_class(&ingress, None));
        assert!(ingress_matches_class(&ingress, Some("nginx")));
        assert!(!ingress_matches_class(&ingress, Some("traefik")));
    }

    #[test]
    fn build_route_table_from_ingress() {
        use k8s_openapi::api::networking::v1::{
            HTTPIngressPath, HTTPIngressRuleValue, Ingress, IngressBackend, IngressRule,
            IngressServiceBackend, IngressSpec, ServiceBackendPort,
        };
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

        let ingress = Ingress {
            metadata: ObjectMeta {
                name: Some("api".into()),
                namespace: Some("prod".into()),
                ..Default::default()
            },
            spec: Some(IngressSpec {
                rules: Some(vec![IngressRule {
                    host: Some("app.example.com".into()),
                    http: Some(HTTPIngressRuleValue {
                        paths: vec![HTTPIngressPath {
                            path: Some("/api".into()),
                            path_type: "Prefix".into(),
                            backend: IngressBackend {
                                service: Some(IngressServiceBackend {
                                    name: "api".into(),
                                    port: Some(ServiceBackendPort {
                                        number: Some(8080),
                                        ..Default::default()
                                    }),
                                }),
                                ..Default::default()
                            },
                            ..Default::default()
                        }],
                    }),
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let table = build_route_table_from_ingresses([ingress]);
        let backend = table.match_route("app.example.com", "/api/v1").unwrap();
        assert_eq!(backend.address, "api.prod.svc.cluster.local:8080");
        assert_eq!(backend.port, 8080);
    }

    #[test]
    fn prefix_with_trailing_slash() {
        let table = RouteTable::from_routes(HashMap::from([(
            "app.example.com".into(),
            vec![Route {
                path: "/api/".into(),
                path_type: PathMatchType::Prefix,
                backend: Backend {
                    address: "api:8080".into(),
                    port: 8080,
                    use_tls: false,
                },
                middlewares: vec![],
            }],
        )]));
        assert!(table.match_route("app.example.com", "/api/v1").is_some());
    }

    #[test]
    fn router_default_and_replace_all() {
        let router = Router::default();
        assert_eq!(router.route_count(), 0);
        let table = RouteTable::from_routes(HashMap::from([(
            "all.example.com".into(),
            vec![Route {
                path: "/".into(),
                path_type: PathMatchType::Prefix,
                backend: Backend {
                    address: "all:80".into(),
                    port: 80,
                    use_tls: false,
                },
                middlewares: vec![],
            }],
        )]));
        router.replace_all(
            table,
            crate::http3_options::Http3Options {
                enable_0rtt: Some(true),
                ..Default::default()
            },
        );
        assert_eq!(router.route_count(), 1);
        assert_eq!(router.http3_options().enable_0rtt, Some(true));
    }

    #[test]
    fn ingress_default_backend_catchall() {
        use k8s_openapi::api::networking::v1::{
            Ingress, IngressBackend, IngressServiceBackend, IngressSpec, ServiceBackendPort,
        };
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

        let ingress = Ingress {
            metadata: ObjectMeta {
                namespace: Some("ns".into()),
                ..Default::default()
            },
            spec: Some(IngressSpec {
                default_backend: Some(IngressBackend {
                    service: Some(IngressServiceBackend {
                        name: "default".into(),
                        port: Some(ServiceBackendPort {
                            number: Some(80),
                            ..Default::default()
                        }),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let table = build_route_table_from_ingresses([ingress]);
        assert!(table.match_route("any.host", "/").is_some());
    }

    #[test]
    fn ingress_without_http_paths_skipped() {
        use k8s_openapi::api::networking::v1::{Ingress, IngressRule, IngressSpec};
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

        let ingress = Ingress {
            metadata: ObjectMeta::default(),
            spec: Some(IngressSpec {
                rules: Some(vec![IngressRule {
                    host: Some("bare.example.com".into()),
                    http: None,
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let table = build_route_table_from_ingresses([ingress]);
        assert_eq!(table.route_count(), 0);
    }

    #[test]
    fn implementation_specific_path_type() {
        let table = RouteTable::from_routes(HashMap::from([(
            "app.example.com".into(),
            vec![Route {
                path: "/custom".into(),
                path_type: PathMatchType::ImplementationSpecific,
                backend: Backend {
                    address: "custom:80".into(),
                    port: 80,
                    use_tls: false,
                },
                middlewares: vec![],
            }],
        )]));
        assert!(table.match_route("app.example.com", "/custom/path").is_some());
    }

    #[test]
    fn ingress_exact_path_type() {
        use k8s_openapi::api::networking::v1::{
            HTTPIngressPath, HTTPIngressRuleValue, Ingress, IngressBackend, IngressRule,
            IngressServiceBackend, IngressSpec, ServiceBackendPort,
        };
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

        let ingress = Ingress {
            metadata: ObjectMeta {
                namespace: Some("dev".into()),
                ..Default::default()
            },
            spec: Some(IngressSpec {
                rules: Some(vec![IngressRule {
                    host: Some("exact.example.com".into()),
                    http: Some(HTTPIngressRuleValue {
                        paths: vec![HTTPIngressPath {
                            path: Some("/exact".into()),
                            path_type: "Exact".into(),
                            backend: IngressBackend {
                                service: Some(IngressServiceBackend {
                                    name: "svc".into(),
                                    port: Some(ServiceBackendPort {
                                        number: Some(9000),
                                        ..Default::default()
                                    }),
                                }),
                                ..Default::default()
                            },
                            ..Default::default()
                        }],
                    }),
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let table = build_route_table_from_ingresses([ingress]);
        assert!(table.match_route("exact.example.com", "/exact").is_some());
        assert!(table.match_route("exact.example.com", "/exact/more").is_none());
    }

    #[test]
    fn ingress_without_spec_is_noop() {
        use k8s_openapi::api::networking::v1::Ingress;
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

        let ingress = Ingress {
            metadata: ObjectMeta::default(),
            spec: None,
            ..Default::default()
        };
        let table = build_route_table_from_ingresses([ingress]);
        assert_eq!(table.route_count(), 0);
    }

    #[test]
    fn ingress_skips_paths_without_service_backend() {
        use k8s_openapi::api::networking::v1::{
            HTTPIngressPath, HTTPIngressRuleValue, Ingress, IngressBackend, IngressRule,
            IngressSpec,
        };
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

        let ingress = Ingress {
            metadata: ObjectMeta::default(),
            spec: Some(IngressSpec {
                rules: Some(vec![IngressRule {
                    host: Some("skip.example.com".into()),
                    http: Some(HTTPIngressRuleValue {
                        paths: vec![HTTPIngressPath {
                            path: Some("/".into()),
                            path_type: "Prefix".into(),
                            backend: IngressBackend {
                                service: None,
                                ..Default::default()
                            },
                            ..Default::default()
                        }],
                    }),
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let table = build_route_table_from_ingresses([ingress]);
        assert_eq!(table.route_count(), 0);
    }

    #[test]
    fn ingress_implementation_specific_path_type() {
        use k8s_openapi::api::networking::v1::{
            HTTPIngressPath, HTTPIngressRuleValue, Ingress, IngressBackend, IngressRule,
            IngressServiceBackend, IngressSpec, ServiceBackendPort,
        };
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

        let ingress = Ingress {
            metadata: ObjectMeta {
                namespace: Some("dev".into()),
                ..Default::default()
            },
            spec: Some(IngressSpec {
                rules: Some(vec![IngressRule {
                    host: Some("impl.example.com".into()),
                    http: Some(HTTPIngressRuleValue {
                        paths: vec![HTTPIngressPath {
                            path: None,
                            path_type: "ImplementationSpecific".into(),
                            backend: IngressBackend {
                                service: Some(IngressServiceBackend {
                                    name: "svc".into(),
                                    port: Some(ServiceBackendPort {
                                        number: Some(80),
                                        ..Default::default()
                                    }),
                                }),
                                ..Default::default()
                            },
                            ..Default::default()
                        }],
                    }),
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let table = build_route_table_from_ingresses([ingress]);
        assert!(table.match_route("impl.example.com", "/anything").is_some());
    }
}
