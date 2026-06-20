use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use k8s_openapi::api::networking::v1::{
    HTTPIngressRuleValue, Ingress, IngressBackend, IngressServiceBackend,
};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub path: String,
    pub path_type: PathMatchType,
    pub backend: Backend,
}

#[derive(Debug, Default)]
pub struct RouteTable {
    /// Host (lowercase) -> ordered routes (longest prefix first).
    routes: HashMap<String, Vec<Route>>,
}

impl RouteTable {
    pub fn match_route(&self, host: &str, path: &str) -> Option<&Backend> {
        let host = normalize_host(host);

        if let Some(routes) = self.routes.get(&host) {
            if let Some(backend) = match_routes(routes, path) {
                return Some(backend);
            }
        }

        if let Some(routes) = self.routes.get("*") {
            return match_routes(routes, path);
        }

        None
    }

    pub fn route_count(&self) -> usize {
        self.routes.values().map(|routes| routes.len()).sum()
    }
}

fn match_routes<'a>(routes: &'a [Route], path: &str) -> Option<&'a Backend> {
    for route in routes {
        if path_matches(&route.path_type, &route.path, path) {
            return Some(&route.backend);
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

pub struct Router {
    table: ArcSwap<RouteTable>,
}

impl Router {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            table: ArcSwap::from_pointee(RouteTable::default()),
        })
    }

    pub fn snapshot(&self) -> Arc<RouteTable> {
        self.table.load_full()
    }

    pub fn replace(&self, table: RouteTable) {
        debug!(routes = table.route_count(), "updated routing table");
        self.table.store(Arc::new(table));
    }
}

impl Default for Router {
    fn default() -> Self {
        Self {
            table: ArcSwap::from_pointee(RouteTable::default()),
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
                }],
            )]),
        };

        assert!(table.match_route("app.example.com", "/api/v1").is_some());
        assert!(table.match_route("app.example.com", "/health").is_none());
    }
}
