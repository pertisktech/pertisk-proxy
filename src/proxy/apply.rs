//! Apply SQLite-backed proxy config to the in-memory router.

use std::collections::HashMap;

use anyhow::{Context, Result};

use crate::proxy_config::{Backend, Config, PathMatchType as ConfigPathMatchType, Site};
use crate::router::{parse_upstream, Middleware, PathMatchType, Route, RouteTable};
use crate::Router;

const PROXY_CONFIG_KEY: &str = "current";

pub fn config_key() -> &'static str {
    PROXY_CONFIG_KEY
}

/// Build a route table from sites + named backends.
pub fn config_to_route_table(config: &Config) -> Result<RouteTable> {
    let backends: HashMap<String, &Backend> = config
        .backends
        .iter()
        .map(|b| (b.name.clone(), b))
        .collect();

    let mut by_host: HashMap<String, Vec<Route>> = HashMap::new();

    for site in &config.sites {
        let host = site.host.trim().to_lowercase();
        if host.is_empty() {
            continue;
        }
        let backend = backends
            .get(&site.backend)
            .with_context(|| format!("site {host} references unknown backend '{}'", site.backend))?;
        let upstream = backend
            .upstreams
            .first()
            .with_context(|| format!("backend '{}' has no upstreams", site.backend))?;
        let target = parse_upstream(&upstream.addr).with_context(|| {
            format!(
                "invalid upstream '{}' for backend '{}'",
                upstream.addr, site.backend
            )
        })?;

        let routes = if site.routes.is_empty() {
            vec![site_route("/", ConfigPathMatchType::Prefix, None, &target)]
        } else {
            site
                .routes
                .iter()
                .map(|r| {
                    site_route(
                        &r.path,
                        r.path_type,
                        r.rewrite.as_deref(),
                        &target,
                    )
                })
                .collect()
        };

        by_host.entry(host).or_default().extend(routes);
    }

    Ok(RouteTable::from_routes(by_host))
}

fn site_route(
    path: &str,
    path_type: ConfigPathMatchType,
    _rewrite: Option<&str>,
    target: &crate::router::Backend,
) -> Route {
    Route {
        path: path.to_string(),
        path_type: match path_type {
            ConfigPathMatchType::Exact => PathMatchType::Exact,
            ConfigPathMatchType::Prefix => PathMatchType::Prefix,
            ConfigPathMatchType::ImplementationSpecific => PathMatchType::ImplementationSpecific,
        },
        backend: target.clone(),
        middlewares: Vec::<Middleware>::new(),
    }
}

pub fn apply_config(router: &Router, config: &Config) -> Result<()> {
    let table = config_to_route_table(config)?;
    router.replace_all(table, config.http3.clone());
    Ok(())
}

/// Convert legacy flat routes.yaml into proxy Config (one-off migration).
pub fn migrate_from_routes_yaml(yaml: &str) -> Result<Config> {
    use crate::routes_config;

    let loaded = routes_config::load_from_str(yaml)?;
    let mut backends: HashMap<String, String> = HashMap::new();
    let mut sites = Vec::new();

    for (host, route) in loaded.table.all_routes() {
        let upstream = format!("http://{}:{}", route.backend.address, route.backend.port);
        let backend_name = format!("backend-{}", route.backend.address.replace(':', "-"));
        backends.entry(backend_name.clone()).or_insert(upstream);
        sites.push(Site {
            host: host.clone(),
            routes: vec![crate::proxy_config::PathRewrite {
                path_type: match route.path_type {
                    PathMatchType::Exact => ConfigPathMatchType::Exact,
                    PathMatchType::Prefix => ConfigPathMatchType::Prefix,
                    PathMatchType::ImplementationSpecific => ConfigPathMatchType::ImplementationSpecific,
                },
                path: route.path.clone(),
                rewrite: None,
            }],
            backend: backend_name,
            security_headers: None,
            ingress_namespace: None,
            ingress_name: None,
            k8s_resource_kind: None,
            http3_alt_svc_enabled: true,
        });
    }

    let backend_list = backends
        .into_iter()
        .map(|(name, addr)| Backend {
            name,
            upstreams: vec![crate::proxy_config::Upstream {
                addr,
                weight: 1,
            }],
            algorithm: Default::default(),
            health_path: None,
            health_interval_secs: 0,
        })
        .collect();

    Ok(Config {
        sites,
        backends: backend_list,
        tls: loaded
            .tls
            .into_iter()
            .map(|t| crate::proxy_config::TlsConfig {
                hosts: t.hosts,
                source: match t.source {
                    crate::tls::TlsSource::File { cert, key } => {
                        crate::proxy_config::TlsSource::File { cert, key }
                    }
                    crate::tls::TlsSource::Acme {
                        email,
                        challenge,
                        dns_provider,
                        dns_provider_type,
                        dns_credentials,
                    } => crate::proxy_config::TlsSource::Acme {
                        email,
                        challenge,
                        dns_provider,
                        dns_provider_type,
                        dns_credentials,
                    },
                    crate::tls::TlsSource::Kubernetes => crate::proxy_config::TlsSource::Kubernetes,
                },
                expires_at: t.expires_at,
            })
            .collect(),
        http3: loaded.http3,
        ..Config::default()
    })
}
