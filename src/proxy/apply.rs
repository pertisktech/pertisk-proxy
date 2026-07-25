//! Apply SQLite-backed proxy config to the in-memory router.

use std::collections::HashMap;

use anyhow::{Context, Result};
use tracing::warn;

use crate::proxy_config::{
    Backend, Config, PathMatchType as ConfigPathMatchType, PathRewrite, Site,
};
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
        let default_target = parse_upstream(&upstream.addr).with_context(|| {
            format!(
                "invalid upstream '{}' for backend '{}'",
                upstream.addr, site.backend
            )
        })?;

        let geoip = config.resolve_site_geoip(site);
        let security = config.resolve_site_security(site);

        let routes = if site.routes.is_empty() {
            vec![site_route(
                "/",
                ConfigPathMatchType::Prefix,
                None,
                &default_target,
                site.forward_client_ip,
                geoip.clone(),
                security.clone(),
            )]
        } else {
            let mut built = Vec::with_capacity(site.routes.len());
            for r in &site.routes {
                let target = resolve_route_target(r, &default_target, &host)?;
                built.push(site_route(
                    &r.path,
                    r.path_type,
                    path_only_rewrite(r.rewrite.as_deref()),
                    &target,
                    site.forward_client_ip,
                    geoip.clone(),
                    security.clone(),
                ));
            }
            built
        };

        by_host.entry(host).or_default().extend(routes);
    }

    Ok(RouteTable::from_routes(by_host))
}

/// Per-route upstream, with a compat path for URLs mistakenly stored in `rewrite`.
fn resolve_route_target(
    route: &PathRewrite,
    default_target: &crate::router::Backend,
    host: &str,
) -> Result<crate::router::Backend> {
    if let Some(raw) = route
        .upstream
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return parse_upstream(raw).with_context(|| {
            format!("invalid per-route upstream '{raw}' for site {host} path {}", route.path)
        });
    }

    if let Some(raw) = route
        .rewrite
        .as_deref()
        .map(str::trim)
        .filter(|s| looks_like_upstream_url(s))
    {
        warn!(
            host,
            path = %route.path,
            rewrite = %raw,
            "treating route rewrite as upstream URL; set routes[].upstream instead"
        );
        return parse_upstream(raw).with_context(|| {
            format!("invalid rewrite-as-upstream '{raw}' for site {host} path {}", route.path)
        });
    }

    Ok(default_target.clone())
}

fn looks_like_upstream_url(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("localhost:")
        || lower.starts_with("127.0.0.1:")
        || lower.starts_with("[::1]:")
}

fn path_only_rewrite(rewrite: Option<&str>) -> Option<&str> {
    rewrite.filter(|s| !looks_like_upstream_url(s.trim()))
}

fn site_route(
    path: &str,
    path_type: ConfigPathMatchType,
    rewrite: Option<&str>,
    target: &crate::router::Backend,
    forward_client_ip: bool,
    geoip: crate::geoip::GeoIpPolicy,
    security: crate::security::SecurityPolicy,
) -> Route {
    let mut middlewares = Vec::<Middleware>::new();
    // Path rewrite to a shorter prefix is modeled as StripPrefix of the match path
    // when rewrite is "/" or empty-equivalent; otherwise leave path unchanged for now.
    if let Some(rw) = rewrite.map(str::trim).filter(|s| !s.is_empty()) {
        if rw == "/" && path != "/" {
            middlewares.push(Middleware::StripPrefix {
                prefix: path.trim_end_matches('/').to_string(),
            });
        }
    }

    Route {
        path: path.to_string(),
        path_type: match path_type {
            ConfigPathMatchType::Exact => PathMatchType::Exact,
            ConfigPathMatchType::Prefix => PathMatchType::Prefix,
            ConfigPathMatchType::ImplementationSpecific => PathMatchType::ImplementationSpecific,
        },
        backend: target.clone(),
        middlewares,
        forward_client_ip,
        geoip: geoip.normalized(),
        security: security.normalized(),
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
                upstream: None,
            }],
            backend: backend_name,
            security_headers: None,
            ingress_namespace: None,
            ingress_name: None,
            k8s_resource_kind: None,
            http3_alt_svc_enabled: true,
            forward_client_ip: false,
            access_list_id: None,
            waf_policy_id: None,
            geoip: Default::default(),
            security: Default::default(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy_config::{PathMatchType as ConfigPathMatchType, Upstream};

    fn sample_config() -> Config {
        Config {
            backends: vec![
                Backend {
                    name: "web".into(),
                    upstreams: vec![Upstream {
                        addr: "http://127.0.0.1:6681".into(),
                        weight: 1,
                    }],
                    algorithm: Default::default(),
                    health_path: None,
                    health_interval_secs: 0,
                },
                Backend {
                    name: "api".into(),
                    upstreams: vec![Upstream {
                        addr: "http://127.0.0.1:7780".into(),
                        weight: 1,
                    }],
                    algorithm: Default::default(),
                    health_path: None,
                    health_interval_secs: 0,
                },
            ],
            sites: vec![Site {
                host: "driver.erp.astrosystem.co".into(),
                backend: "web".into(),
                routes: vec![
                    PathRewrite {
                        path_type: ConfigPathMatchType::Prefix,
                        path: "/".into(),
                        rewrite: Some("/".into()),
                        upstream: None,
                    },
                    PathRewrite {
                        path_type: ConfigPathMatchType::Prefix,
                        path: "/api".into(),
                        rewrite: None,
                        upstream: Some("http://127.0.0.1:7780".into()),
                    },
                ],
                security_headers: None,
                ingress_namespace: None,
                ingress_name: None,
                k8s_resource_kind: None,
                http3_alt_svc_enabled: true,
                forward_client_ip: true,
                access_list_id: None,
                waf_policy_id: None,
                geoip: Default::default(),
                security: Default::default(),
            }],
            ..Config::default()
        }
    }

    #[test]
    fn per_route_upstream_overrides_site_backend() {
        let table = config_to_route_table(&sample_config()).expect("table");
        let api = table
            .match_route("driver.erp.astrosystem.co", "/api/v1/x")
            .expect("api route");
        assert_eq!(api.address, "127.0.0.1:7780");
        assert_eq!(api.port, 7780);

        let web = table
            .match_route("driver.erp.astrosystem.co", "/app")
            .expect("web route");
        assert_eq!(web.address, "127.0.0.1:6681");
        assert_eq!(web.port, 6681);
    }

    #[test]
    fn rewrite_url_compat_used_as_upstream() {
        let mut cfg = sample_config();
        cfg.sites[0].routes[1].upstream = None;
        cfg.sites[0].routes[1].rewrite = Some("http://localhost:7780".into());
        let table = config_to_route_table(&cfg).expect("table");
        let api = table
            .match_route("driver.erp.astrosystem.co", "/api/health")
            .expect("api");
        assert_eq!(api.port, 7780);
    }

    #[test]
    fn two_sites_same_host_merge_routes() {
        let cfg = Config {
            backends: vec![
                Backend {
                    name: "web".into(),
                    upstreams: vec![Upstream {
                        addr: "http://127.0.0.1:6681".into(),
                        weight: 1,
                    }],
                    algorithm: Default::default(),
                    health_path: None,
                    health_interval_secs: 0,
                },
                Backend {
                    name: "api".into(),
                    upstreams: vec![Upstream {
                        addr: "http://127.0.0.1:7780".into(),
                        weight: 1,
                    }],
                    algorithm: Default::default(),
                    health_path: None,
                    health_interval_secs: 0,
                },
            ],
            sites: vec![
                Site {
                    host: "driver.erp.astrosystem.co".into(),
                    backend: "web".into(),
                    routes: vec![PathRewrite {
                        path_type: ConfigPathMatchType::Prefix,
                        path: "/".into(),
                        rewrite: None,
                        upstream: None,
                    }],
                    security_headers: None,
                    ingress_namespace: None,
                    ingress_name: None,
                    k8s_resource_kind: None,
                    http3_alt_svc_enabled: true,
                    forward_client_ip: true,
                    access_list_id: None,
                    waf_policy_id: None,
                    geoip: Default::default(),
                    security: Default::default(),
                },
                Site {
                    host: "driver.erp.astrosystem.co".into(),
                    backend: "api".into(),
                    routes: vec![PathRewrite {
                        path_type: ConfigPathMatchType::Prefix,
                        path: "/api".into(),
                        rewrite: None,
                        upstream: None,
                    }],
                    security_headers: None,
                    ingress_namespace: None,
                    ingress_name: None,
                    k8s_resource_kind: None,
                    http3_alt_svc_enabled: true,
                    forward_client_ip: true,
                    access_list_id: None,
                    waf_policy_id: None,
                    geoip: Default::default(),
                    security: Default::default(),
                },
            ],
            ..Config::default()
        };
        let table = config_to_route_table(&cfg).expect("table");
        assert_eq!(
            table
                .match_route("driver.erp.astrosystem.co", "/api/x")
                .unwrap()
                .port,
            7780
        );
        assert_eq!(
            table
                .match_route("driver.erp.astrosystem.co", "/")
                .unwrap()
                .port,
            6681
        );
    }
}
