use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::router::{parse_upstream, Middleware, PathMatchType, Route, RouteTable};

#[derive(Debug, Deserialize)]
struct RoutesFile {
    #[serde(default)]
    routes: Vec<RouteSpec>,
}

#[derive(Debug, Deserialize)]
struct RouteSpec {
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    hosts: Vec<String>,
    path: String,
    #[serde(default = "default_prefix")]
    path_type: String,
    upstream: String,
    #[serde(default)]
    middlewares: Vec<Middleware>,
}

fn default_prefix() -> String {
    "prefix".into()
}

/// Load pertisk-native route definitions from YAML or JSON.
pub fn load(path: &Path) -> Result<RouteTable> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read routes config {}", path.display()))?;

    let spec: RoutesFile = if content.trim_start().starts_with('{') {
        serde_json::from_str(&content).context("invalid routes JSON")?
    } else {
        serde_yaml::from_str(&content).context("invalid routes YAML")?
    };

    build_table(spec.routes)
}

fn build_table(specs: Vec<RouteSpec>) -> Result<RouteTable> {
    let mut by_host: HashMap<String, Vec<Route>> = HashMap::new();

    for spec in specs {
        let backend = parse_upstream(&spec.upstream)
            .with_context(|| format!("invalid upstream {}", spec.upstream))?;

        let path_type = match spec.path_type.to_ascii_lowercase().as_str() {
            "exact" => PathMatchType::Exact,
            "implementation_specific" | "implementationspecific" => {
                PathMatchType::ImplementationSpecific
            }
            _ => PathMatchType::Prefix,
        };

        let mut hosts = spec.hosts;
        if let Some(host) = spec.host {
            hosts.push(host);
        }
        if hosts.is_empty() {
            hosts.push("*".into());
        }

        for host in hosts {
            by_host.entry(host.to_ascii_lowercase()).or_default().push(Route {
                path: spec.path.clone(),
                path_type: path_type.clone(),
                backend: backend.clone(),
                middlewares: spec.middlewares.clone(),
            });
        }
    }

    Ok(RouteTable::from_routes(by_host))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_yaml_routes() {
        let yaml = r#"
routes:
  - host: app.example.com
    path: /api
    path_type: prefix
    upstream: http://backend:8080
"#;
        let table = build_table(serde_yaml::from_str::<RoutesFile>(yaml).unwrap().routes).unwrap();
        assert!(table.match_route("app.example.com", "/api/v1").is_some());
    }
}
