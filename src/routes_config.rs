use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::router::{parse_upstream, Middleware, PathMatchType, Route, RouteTable};
use crate::tls::TlsConfig;

#[derive(Debug)]
pub struct LoadedRoutes {
    pub table: RouteTable,
    pub tls: Vec<TlsConfig>,
    pub http3: crate::http3_options::Http3Options,
}

#[derive(Debug, Deserialize)]
struct RoutesFile {
    #[serde(default)]
    routes: Vec<RouteSpec>,
    #[serde(default)]
    tls: Vec<TlsConfig>,
    #[serde(default)]
    http3: crate::http3_options::Http3Options,
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

/// Load from YAML/JSON string (migration helper).
pub fn load_from_str(content: &str) -> Result<LoadedRoutes> {
    let spec: RoutesFile = if content.trim_start().starts_with('{') {
        serde_json::from_str(content).context("invalid routes JSON")?
    } else {
        serde_yaml::from_str(content).context("invalid routes YAML")?
    };
    Ok(LoadedRoutes {
        table: build_table(spec.routes)?,
        tls: spec.tls,
        http3: spec.http3,
    })
}

/// Load pertisk-native route and TLS definitions from YAML or JSON.
pub fn load(path: &Path) -> Result<LoadedRoutes> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read routes config {}", path.display()))?;
    load_from_str(&content)
}

/// Parse YAML without writing to disk (admin API validation).
pub fn validate_yaml(content: &str) -> Result<()> {
    let spec: RoutesFile = serde_yaml::from_str(content).context("invalid routes YAML")?;
    build_table(spec.routes)?;
    Ok(())
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
        let spec: RoutesFile = serde_yaml::from_str(yaml).unwrap();
        let loaded = LoadedRoutes {
            table: build_table(spec.routes).unwrap(),
            tls: spec.tls,
            http3: spec.http3,
        };
        assert!(loaded
            .table
            .match_route("app.example.com", "/api/v1")
            .is_some());
    }

    #[test]
    fn parses_tls_section() {
        let yaml = r#"
routes:
  - host: app.example.com
    path: /
    upstream: http://backend:8080
tls:
  - hosts: [app.example.com]
    source:
      type: file
      cert: /tmp/cert.pem
      key: /tmp/key.pem
"#;
        let spec: RoutesFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(spec.tls.len(), 1);
        assert_eq!(spec.tls[0].hosts, vec!["app.example.com"]);
    }

    #[test]
    fn parses_http3_section() {
        let yaml = r#"
routes:
  - host: app.example.com
    path: /
    upstream: http://backend:8080
http3:
  max_streams_bidi: 512
  enable_0rtt: true
  congestion_control: bbr
"#;
        let spec: RoutesFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(spec.http3.max_streams_bidi, Some(512));
        assert_eq!(spec.http3.enable_0rtt, Some(true));
        assert_eq!(spec.http3.congestion_control.as_deref(), Some("bbr"));
    }

    #[test]
    fn load_from_str_json() {
        let json = r#"{"routes":[{"host":"app.example.com","path":"/","upstream":"http://backend:8080"}]}"#;
        let loaded = load_from_str(json).unwrap();
        assert!(loaded.table.match_route("app.example.com", "/").is_some());
    }

    #[test]
    fn validate_yaml_rejects_bad_upstream() {
        let yaml = r#"
routes:
  - host: app.example.com
    path: /
    upstream: ":"
"#;
        assert!(validate_yaml(yaml).is_err());
    }

    #[test]
    fn validate_yaml_accepts_good_routes() {
        let yaml = r#"
routes:
  - host: app.example.com
    path: /
    upstream: http://backend:8080
"#;
        validate_yaml(yaml).unwrap();
    }

    #[test]
    fn multiple_hosts_and_wildcard_default() {
        let yaml = r#"
routes:
  - hosts: [a.example.com, b.example.com]
    path: /v1
    path_type: exact
    upstream: backend:9000
  - path: /
    upstream: http://catchall:80
"#;
        let loaded = load_from_str(yaml).unwrap();
        assert!(loaded.table.match_route("a.example.com", "/v1").is_some());
        assert!(loaded.table.match_route("b.example.com", "/v1").is_some());
        assert!(loaded.table.match_route("other.example.com", "/").is_some());
    }

    #[test]
    fn implementation_specific_path_type() {
        let yaml = r#"
routes:
  - host: app.example.com
    path: /api
    path_type: implementation_specific
    upstream: http://backend:8080
"#;
        let loaded = load_from_str(yaml).unwrap();
        assert!(loaded.table.match_route("app.example.com", "/api/extra").is_some());
    }

    #[test]
    fn load_from_file_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routes.yaml");
        std::fs::write(
            &path,
            r#"
routes:
  - host: file.example.com
    path: /
    upstream: http://backend:8080
"#,
        )
        .unwrap();
        let loaded = load(&path).unwrap();
        assert!(loaded.table.match_route("file.example.com", "/").is_some());
    }
}
