//! Configuration types for the reverse proxy: path matching, backends, routes, and TLS.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;

/// Path match type for routing (Kubernetes Ingress-style).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
pub enum PathMatchType {
    /// Exact path match.
    Exact,
    /// Prefix match.
    Prefix,
    /// Implementation-specific (e.g. regex or longest prefix).
    #[default]
    ImplementationSpecific,
}

/// Load balancing algorithm for upstreams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum LoadBalancerAlgorithm {
    #[default]
    RoundRobin,
    LeastConnections,
    IpHash,
}

/// A single upstream server (host:port).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Upstream {
    pub addr: String,
    #[serde(default)]
    pub weight: u32,
}

/// Reusable backend: multiple upstreams + load balancing + optional health check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backend {
    pub name: String,
    pub upstreams: Vec<Upstream>,
    #[serde(default)]
    pub algorithm: LoadBalancerAlgorithm,
    #[serde(default)]
    pub health_path: Option<String>,
    #[serde(default)]
    pub health_interval_secs: u64,
}

/// Path rewrite: path pattern -> upstream path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathRewrite {
    /// Path match type for this route.
    #[serde(default)]
    pub path_type: PathMatchType,
    /// Path pattern (e.g. "/api" or "/v1/users").
    pub path: String,
    /// Rewrite target path when forwarding to upstream (e.g. "/" or "/users").
    #[serde(default)]
    pub rewrite: Option<String>,
}

/// Site/route: host + paths + backend reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Site {
    pub host: String,
    /// Path-based routes (order can matter for prefix vs exact).
    pub routes: Vec<PathRewrite>,
    /// Backend name (must exist in [backends]).
    pub backend: String,
    /// Optional per-site security headers override (empty map disables global headers for this site).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security_headers: Option<HashMap<String, String>>,
    /// When set (ingress mode), the Kubernetes resource this site came from; used for edit/delete in Admin UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingress_namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingress_name: Option<String>,
    /// Source resource kind: "ingress" (default) or "httproute" (Gateway API HTTPRoute).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub k8s_resource_kind: Option<String>,
    /// Per-site HTTP/3 Alt-Svc advertisement toggle.
    /// When false, responses include `Alt-Svc: clear` for this site.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub http3_alt_svc_enabled: bool,
}

fn is_true(value: &bool) -> bool {
    *value
}

/// TLS certificate source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TlsSource {
    /// Let's Encrypt / ACME (HTTP-01 or DNS-01).
    Acme {
        #[serde(default)]
        email: Option<String>,
        /// "http01" or "dns01"
        #[serde(default = "default_challenge")]
        challenge: String,
        /// For DNS-01: provider name (e.g. my-cloudflare).
        #[serde(default)]
        dns_provider: Option<String>,
        /// For DNS-01: provider type for solver (e.g. cloudflare, digitalocean).
        #[serde(default)]
        dns_provider_type: Option<String>,
        #[serde(default)]
        dns_credentials: Option<HashMap<String, String>>,
    },
    /// Paths to cert and key files.
    File {
        cert: PathBuf,
        key: PathBuf,
    },
    /// TLS from Kubernetes Secret (Ingress/CRD spec.tls.secretName).
    Kubernetes,
}

fn default_challenge() -> String {
    "http01".to_string()
}

fn tls_hosts_set(hosts: &[String]) -> HashSet<String> {
    hosts
        .iter()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

fn acme_wildcard_host(hosts: &[String]) -> Option<String> {
    hosts
        .iter()
        .find(|h| h.trim().starts_with("*."))
        .map(|h| h.trim().to_ascii_lowercase())
}

fn acme_sources_equivalent(a: &TlsSource, b: &TlsSource) -> bool {
    match (a, b) {
        (
            TlsSource::Acme {
                email: e1,
                challenge: c1,
                dns_provider: d1,
                dns_provider_type: t1,
                ..
            },
            TlsSource::Acme {
                email: e2,
                challenge: c2,
                dns_provider: d2,
                dns_provider_type: t2,
                ..
            },
        ) => e1 == e2 && c1 == c2 && d1 == d2 && t1 == t2,
        (TlsSource::File { cert: c1, key: k1 }, TlsSource::File { cert: c2, key: k2 }) => {
            c1 == c2 && k1 == k2
        }
        _ => false,
    }
}

fn tls_entries_should_merge(a: &TlsConfig, b: &TlsConfig) -> bool {
    if !acme_sources_equivalent(&a.source, &b.source) {
        return false;
    }
    match (&a.source, &b.source) {
        (TlsSource::Acme { .. }, TlsSource::Acme { .. }) => {
            if acme_wildcard_host(&a.hosts).is_some()
                && acme_wildcard_host(&a.hosts) == acme_wildcard_host(&b.hosts)
            {
                return true;
            }
            !tls_hosts_set(&a.hosts).is_disjoint(&tls_hosts_set(&b.hosts))
        }
        (TlsSource::File { .. }, TlsSource::File { .. }) => true,
        _ => false,
    }
}

/// Merge duplicate TLS rows (e.g. repeated ACME wildcard entries from site saves).
pub fn normalize_tls_config(tls: &mut Vec<TlsConfig>) {
    let mut merged: Vec<TlsConfig> = Vec::new();
    for entry in tls.drain(..) {
        if let Some(idx) = merged.iter().position(|e| tls_entries_should_merge(e, &entry)) {
            let existing = &mut merged[idx];
            for h in entry.hosts {
                let h = h.trim().to_string();
                if h.is_empty() {
                    continue;
                }
                if !existing
                    .hosts
                    .iter()
                    .any(|x| x.eq_ignore_ascii_case(&h))
                {
                    existing.hosts.push(h);
                }
            }
            existing.expires_at = None;
        } else {
            merged.push(entry);
        }
    }
    *tls = merged;
}

/// True when `*.example.com` covers `app.example.com` (single label only).
pub fn wildcard_covers_host(wildcard: &str, host: &str) -> bool {
    let w = wildcard.trim();
    let h = host.trim().to_ascii_lowercase();
    if w.is_empty() || h.is_empty() {
        return false;
    }
    if !w.starts_with('*') {
        return w.eq_ignore_ascii_case(&h);
    }
    let suffix = w.strip_prefix('*').unwrap_or(w);
    if !h.ends_with(suffix) || h.len() <= suffix.len() {
        return false;
    }
    let prefix = &h[..h.len() - suffix.len()];
    !prefix.is_empty() && !prefix.contains('.')
}

/// TLS block per host or shared (wildcard).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    /// Hostnames this cert applies to (for sharing wildcards).
    pub hosts: Vec<String>,
    pub source: TlsSource,
    /// Certificate expiry (not stored in DB; filled by API from certificates table).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// True when any TLS entry uses ACME (certificates may arrive after process start).
pub fn tls_has_acme_config(tls: &[TlsConfig]) -> bool {
    tls.iter()
        .any(|t| matches!(t.source, TlsSource::Acme { .. }))
}

/// Top-level proxy configuration (file or hot-reloaded).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Bind address for HTTP (e.g. "0.0.0.0:80").
    #[serde(default = "default_http_addr")]
    pub http_addr: SocketAddr,
    /// Bind address for HTTPS (e.g. "0.0.0.0:443").
    #[serde(default = "default_https_addr")]
    pub https_addr: SocketAddr,
    /// Optional HTTP/3 (QUIC) bind address (e.g. "0.0.0.0:443" with same port as HTTPS via ALPN).
    #[serde(default)]
    pub http3_addr: Option<SocketAddr>,
    /// Management API and UI (e.g. "0.0.0.0:9080" for LAN access).
    #[serde(default = "default_management_addr")]
    pub management_addr: SocketAddr,
    /// Reusable backends.
    #[serde(default)]
    pub backends: Vec<Backend>,
    /// Sites (host + routes + backend).
    pub sites: Vec<Site>,
    /// TLS configs (ACME or file).
    #[serde(default)]
    pub tls: Vec<TlsConfig>,
    /// Global security headers applied to responses (site override can replace or clear).
    #[serde(default = "default_security_headers")]
    pub security_headers: HashMap<String, String>,
    /// Enable access / proxy log.
    #[serde(default = "default_true")]
    pub proxy_log: bool,
    /// HTTP/3 / QUIC tuning (optional).
    #[serde(default)]
    pub http3: crate::http3_options::Http3Options,
}

fn default_http_addr() -> SocketAddr {
    // [::] binds to all interfaces (IPv4 + IPv6 dual-stack on most systems)
    "[::]:80".parse().unwrap()
}
fn default_https_addr() -> SocketAddr {
    "[::]:443".parse().unwrap()
}
fn default_management_addr() -> SocketAddr {
    "0.0.0.0:9080".parse().unwrap()
}
fn default_true() -> bool {
    true
}

fn default_security_headers() -> HashMap<String, String> {
    let mut headers = HashMap::new();
    headers.insert(
        "Strict-Transport-Security".to_string(),
        "max-age=63072000; includeSubDomains; preload".to_string(),
    );
    headers.insert("X-Content-Type-Options".to_string(), "nosniff".to_string());
    headers.insert("X-Frame-Options".to_string(), "SAMEORIGIN".to_string());
    headers.insert(
        "Referrer-Policy".to_string(),
        "strict-origin-when-cross-origin".to_string(),
    );
    headers.insert(
        "Permissions-Policy".to_string(),
        "geolocation=(), microphone=(), camera=()".to_string(),
    );
    headers.insert(
        "Content-Security-Policy".to_string(),
        "default-src 'self' https: data: blob:; script-src 'self' 'unsafe-inline' 'unsafe-eval' https:; style-src 'self' 'unsafe-inline' https:; font-src 'self' https: data:; img-src 'self' data: https: blob:; connect-src 'self' https: wss:; worker-src 'self' blob:; base-uri 'self'; object-src 'none'; upgrade-insecure-requests"
            .to_string(),
    );
    headers
}

impl Default for Config {
    fn default() -> Self {
        Self {
            http_addr: default_http_addr(),
            https_addr: default_https_addr(),
            http3_addr: None,
            management_addr: default_management_addr(),
            backends: Vec::new(),
            sites: Vec::new(),
            tls: Vec::new(),
            security_headers: default_security_headers(),
            proxy_log: true,
            http3: crate::http3_options::Http3Options::default(),
        }
    }
}

impl Config {
    /// Load config from YAML (e.g. for Kubernetes-style config).
    pub fn from_yaml(s: &str) -> Result<Self, anyhow::Error> {
        Ok(serde_yaml::from_str(s)?)
    }

    /// Proxy listen ports and configured upstream targets (for management UI).
    pub fn upstream_ports_summary(&self) -> UpstreamPortsSummary {
        upstream_ports_summary(self)
    }
}

/// Parsed upstream target (scheme, host, port).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ParsedUpstreamEndpoint {
    pub scheme: String,
    pub host: String,
    pub port: u16,
}

/// Proxy bind port entry.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ListenPortEntry {
    pub name: String,
    pub addr: String,
    pub port: u16,
}

/// One upstream server from config with sites that route to its backend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct UpstreamPortEntry {
    pub backend: String,
    pub addr: String,
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub weight: u32,
    pub sites: Vec<String>,
}

/// Listen ports + all upstream ports referenced by backends/sites.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct UpstreamPortsSummary {
    pub listen_ports: Vec<ListenPortEntry>,
    pub upstreams: Vec<UpstreamPortEntry>,
}

/// Parse `host:port`, `https://host:port`, or `[ipv6]:port` upstream addresses.
pub fn parse_upstream_endpoint(raw: &str) -> ParsedUpstreamEndpoint {
    let trimmed = raw.trim();
    let (scheme, rest) = if let Some(r) = trimmed.strip_prefix("https://") {
        ("https", r)
    } else if let Some(r) = trimmed.strip_prefix("http://") {
        ("http", r)
    } else {
        ("http", trimmed)
    };
    let default_port = if scheme == "https" { 443 } else { 80 };
    let (host, port) = parse_host_port(rest, default_port);
    ParsedUpstreamEndpoint {
        scheme: scheme.to_string(),
        host,
        port,
    }
}

fn parse_host_port(rest: &str, default_port: u16) -> (String, u16) {
    let rest = rest.trim();
    if rest.is_empty() {
        return (String::new(), default_port);
    }
    if rest.starts_with('[') {
        if let Some(end) = rest.find(']') {
            let host = rest[..=end].to_string();
            let port = rest[end + 1..]
                .strip_prefix(':')
                .and_then(|p| p.parse::<u16>().ok())
                .unwrap_or(default_port);
            return (host, port);
        }
    }
    if let Some((host, port_str)) = rest.rsplit_once(':') {
        if let Ok(port) = port_str.parse::<u16>() {
            if !host.is_empty() && !host.contains(':') {
                return (host.to_string(), port);
            }
        }
    }
    (rest.to_string(), default_port)
}

fn listen_port(name: &str, addr: SocketAddr) -> ListenPortEntry {
    ListenPortEntry {
        name: name.to_string(),
        addr: addr.to_string(),
        port: addr.port(),
    }
}

/// Build listen + upstream port list from the active proxy config.
pub fn upstream_ports_summary(config: &Config) -> UpstreamPortsSummary {
    let mut listen_ports = vec![
        listen_port("http", config.http_addr),
        listen_port("https", config.https_addr),
        listen_port("management", config.management_addr),
    ];
    if let Some(addr) = config.http3_addr {
        listen_ports.push(listen_port("http3", addr));
    }

    let mut sites_by_backend: HashMap<String, Vec<String>> = HashMap::new();
    for site in &config.sites {
        sites_by_backend
            .entry(site.backend.clone())
            .or_default()
            .push(site.host.clone());
    }

    let mut upstreams = Vec::new();
    for backend in &config.backends {
        let sites = sites_by_backend
            .get(&backend.name)
            .cloned()
            .unwrap_or_default();
        for u in &backend.upstreams {
            let parsed = parse_upstream_endpoint(&u.addr);
            upstreams.push(UpstreamPortEntry {
                backend: backend.name.clone(),
                addr: u.addr.clone(),
                scheme: parsed.scheme,
                host: parsed.host,
                port: parsed.port,
                weight: u.weight.max(1),
                sites: sites.clone(),
            });
        }
    }

    upstreams.sort_by(|a, b| {
        a.backend
            .cmp(&b.backend)
            .then(a.port.cmp(&b.port))
            .then(a.host.cmp(&b.host))
    });

    UpstreamPortsSummary {
        listen_ports,
        upstreams,
    }
}

#[cfg(test)]
mod upstream_parse_tests {
    use super::*;

    #[test]
    fn parse_host_port_ipv4() {
        let p = parse_upstream_endpoint("10.1.1.187:443");
        assert_eq!(p.scheme, "http");
        assert_eq!(p.host, "10.1.1.187");
        assert_eq!(p.port, 443);
    }

    #[test]
    fn parse_https_url() {
        let p = parse_upstream_endpoint("https://10.1.1.99:443");
        assert_eq!(p.scheme, "https");
        assert_eq!(p.host, "10.1.1.99");
        assert_eq!(p.port, 443);
    }

    #[test]
    fn summary_includes_backends() {
        let mut config = Config::default();
        config.backends.push(Backend {
            name: "api".into(),
            upstreams: vec![Upstream {
                addr: "127.0.0.1:8080".into(),
                weight: 1,
            }],
            algorithm: LoadBalancerAlgorithm::default(),
            health_path: None,
            health_interval_secs: 0,
        });
        config.sites.push(Site {
            host: "example.com".into(),
            backend: "api".into(),
            routes: vec![],
            security_headers: None,
            http3_alt_svc_enabled: true,
            ingress_namespace: None,
            ingress_name: None,
            k8s_resource_kind: None,
        });
        let s = upstream_ports_summary(&config);
        assert_eq!(s.upstreams.len(), 1);
        assert_eq!(s.upstreams[0].port, 8080);
        assert_eq!(s.upstreams[0].sites, vec!["example.com"]);
    }

    #[test]
    fn parse_ipv6_bracket_host() {
        let p = parse_upstream_endpoint("https://[2001:db8::1]:8443");
        assert_eq!(p.scheme, "https");
        assert_eq!(p.host, "[2001:db8::1]");
        assert_eq!(p.port, 8443);
    }

    #[test]
    fn parse_host_without_port_uses_defaults() {
        let p = parse_upstream_endpoint("backend.local");
        assert_eq!(p.port, 80);
        let p = parse_upstream_endpoint("https://secure.local");
        assert_eq!(p.port, 443);
    }

    #[test]
    fn config_from_yaml_roundtrip() {
        let yaml = r#"
sites:
  - host: test.example
    backend: api
    routes: []
backends:
  - name: api
    upstreams:
      - addr: "127.0.0.1:8080"
"#;
        let cfg = Config::from_yaml(yaml).unwrap();
        assert_eq!(cfg.sites[0].host, "test.example");
        assert_eq!(cfg.backends[0].name, "api");
    }

    #[test]
    fn default_config_has_security_headers() {
        let cfg = Config::default();
        assert!(cfg.security_headers.contains_key("Strict-Transport-Security"));
        assert!(cfg.proxy_log);
    }

    #[test]
    fn listen_ports_include_http3_when_set() {
        let mut cfg = Config::default();
        cfg.http3_addr = Some("[::]:443".parse().unwrap());
        let s = upstream_ports_summary(&cfg);
        assert!(s.listen_ports.iter().any(|p| p.name == "http3"));
    }

    #[test]
    fn config_upstream_ports_summary_method() {
        let cfg = Config::default();
        let s = cfg.upstream_ports_summary();
        assert!(s.listen_ports.iter().any(|p| p.name == "http"));
    }

    #[test]
    fn parse_http_scheme_url() {
        let p = parse_upstream_endpoint("http://backend:8080");
        assert_eq!(p.scheme, "http");
        assert_eq!(p.host, "backend");
        assert_eq!(p.port, 8080);
    }

    #[test]
    fn parse_empty_host_uses_default_port() {
        let p = parse_upstream_endpoint("https://");
        assert_eq!(p.port, 443);
        assert!(p.host.is_empty());
    }

    #[test]
    fn upstream_entries_sorted_by_backend_port_host() {
        let mut config = Config::default();
        config.backends.push(Backend {
            name: "z-backend".into(),
            upstreams: vec![Upstream {
                addr: "10.0.0.2:80".into(),
                weight: 0,
            }],
            algorithm: LoadBalancerAlgorithm::default(),
            health_path: None,
            health_interval_secs: 0,
        });
        config.backends.push(Backend {
            name: "a-backend".into(),
            upstreams: vec![Upstream {
                addr: "10.0.0.1:443".into(),
                weight: 1,
            }],
            algorithm: LoadBalancerAlgorithm::default(),
            health_path: None,
            health_interval_secs: 0,
        });
        let s = upstream_ports_summary(&config);
        assert_eq!(s.upstreams.len(), 2);
        assert_eq!(s.upstreams[0].backend, "a-backend");
        assert_eq!(s.upstreams[1].backend, "z-backend");
        assert_eq!(s.upstreams[0].weight, 1);
    }

    #[test]
    fn acme_default_challenge_is_http01() {
        let yaml = r#"
sites: []
tls:
  - hosts: ["example.com"]
    source:
      type: acme
"#;
        let cfg = Config::from_yaml(yaml).unwrap();
        match &cfg.tls[0].source {
            TlsSource::Acme { challenge, .. } => assert_eq!(challenge, "http01"),
            _ => panic!("expected acme"),
        }
    }

    #[test]
    fn serde_defaults_use_default_true() {
        let yaml = r#"
sites:
  - host: test.example
    backend: api
    routes: []
    http3_alt_svc_enabled: false
backends:
  - name: api
    upstreams:
      - addr: "127.0.0.1:8080"
"#;
        let cfg = Config::from_yaml(yaml).unwrap();
        assert!(cfg.proxy_log);
        assert!(!cfg.sites[0].http3_alt_svc_enabled);
    }

    #[test]
    fn config_default_trait_impl() {
        let cfg: Config = Default::default();
        assert!(cfg.sites.is_empty());
        assert!(cfg.proxy_log);
    }

    #[test]
    fn upstream_summary_orphan_backend_has_no_sites() {
        let mut config = Config::default();
        config.backends.push(Backend {
            name: "orphan".into(),
            upstreams: vec![Upstream {
                addr: "127.0.0.1:9999".into(),
                weight: 1,
            }],
            algorithm: LoadBalancerAlgorithm::default(),
            health_path: None,
            health_interval_secs: 0,
        });
        let s = upstream_ports_summary(&config);
        assert_eq!(s.upstreams.len(), 1);
        assert!(s.upstreams[0].sites.is_empty());
    }
}

#[cfg(test)]
mod tls_normalize_tests {
    use super::*;

    fn acme_source() -> TlsSource {
        TlsSource::Acme {
            email: Some("admin@example.com".into()),
            challenge: "http01".into(),
            dns_provider: None,
            dns_provider_type: None,
            dns_credentials: None,
        }
    }

    #[test]
    fn wildcard_covers_single_label() {
        assert!(wildcard_covers_host("*.example.com", "app.example.com"));
        assert!(!wildcard_covers_host("*.example.com", "a.b.example.com"));
        assert!(!wildcard_covers_host("*.example.com", "example.com"));
        assert!(wildcard_covers_host("example.com", "example.com"));
        assert!(!wildcard_covers_host("", "example.com"));
    }

    #[test]
    fn normalize_tls_merges_overlapping_acme_hosts() {
        let mut tls = vec![
            TlsConfig {
                hosts: vec!["a.example.com".into()],
                source: acme_source(),
                expires_at: Some("2026-01-01".into()),
            },
            TlsConfig {
                hosts: vec!["a.example.com".into()],
                source: acme_source(),
                expires_at: None,
            },
        ];
        normalize_tls_config(&mut tls);
        assert_eq!(tls.len(), 1);
        assert_eq!(tls[0].hosts.len(), 1);
        assert!(tls[0].expires_at.is_none());
    }

    #[test]
    fn normalize_tls_merges_wildcard_acme_entries() {
        let mut tls = vec![
            TlsConfig {
                hosts: vec!["*.example.com".into()],
                source: acme_source(),
                expires_at: None,
            },
            TlsConfig {
                hosts: vec!["*.example.com".into()],
                source: acme_source(),
                expires_at: None,
            },
        ];
        normalize_tls_config(&mut tls);
        assert_eq!(tls.len(), 1);
    }

    #[test]
    fn normalize_tls_dedupes_hosts_case_insensitively() {
        let mut tls = vec![
            TlsConfig {
                hosts: vec!["A.example.com".into()],
                source: acme_source(),
                expires_at: None,
            },
            TlsConfig {
                hosts: vec!["a.example.com".into()],
                source: acme_source(),
                expires_at: None,
            },
        ];
        normalize_tls_config(&mut tls);
        assert_eq!(tls.len(), 1);
        assert_eq!(tls[0].hosts.len(), 1);
    }

    #[test]
    fn normalize_tls_merges_file_entries_with_different_paths() {
        let mut tls = vec![
            TlsConfig {
                hosts: vec!["a.example.com".into()],
                source: TlsSource::File {
                    cert: "/a.pem".into(),
                    key: "/a.key".into(),
                },
                expires_at: None,
            },
            TlsConfig {
                hosts: vec!["b.example.com".into()],
                source: TlsSource::File {
                    cert: "/b.pem".into(),
                    key: "/b.key".into(),
                },
                expires_at: None,
            },
        ];
        normalize_tls_config(&mut tls);
        assert_eq!(tls.len(), 2);
    }

    #[test]
    fn normalize_tls_merges_same_file_cert_hosts() {
        let mut tls = vec![
            TlsConfig {
                hosts: vec!["a.example.com".into()],
                source: TlsSource::File {
                    cert: "/shared.pem".into(),
                    key: "/shared.key".into(),
                },
                expires_at: None,
            },
            TlsConfig {
                hosts: vec!["b.example.com".into()],
                source: TlsSource::File {
                    cert: "/shared.pem".into(),
                    key: "/shared.key".into(),
                },
                expires_at: None,
            },
        ];
        normalize_tls_config(&mut tls);
        assert_eq!(tls.len(), 1);
        assert_eq!(tls[0].hosts.len(), 2);
    }

    #[test]
    fn normalize_tls_skips_disjoint_acme() {
        let http = acme_source();
        let mut dns = acme_source();
        if let TlsSource::Acme { challenge, .. } = &mut dns {
            *challenge = "dns01".into();
        }
        let mut tls = vec![
            TlsConfig {
                hosts: vec!["a.example.com".into()],
                source: http,
                expires_at: None,
            },
            TlsConfig {
                hosts: vec!["a.example.com".into()],
                source: dns,
                expires_at: None,
            },
        ];
        normalize_tls_config(&mut tls);
        assert_eq!(tls.len(), 2);
    }

    #[test]
    fn normalize_tls_skips_empty_host_names() {
        let mut tls = vec![
            TlsConfig {
                hosts: vec!["ok.example.com".into()],
                source: acme_source(),
                expires_at: None,
            },
            TlsConfig {
                hosts: vec!["".into(), "   ".into(), "ok.example.com".into()],
                source: acme_source(),
                expires_at: None,
            },
        ];
        normalize_tls_config(&mut tls);
        assert_eq!(tls.len(), 1);
        assert_eq!(tls[0].hosts, vec!["ok.example.com"]);
    }

    #[test]
    fn tls_has_acme_config_detects_acme_entries() {
        assert!(!tls_has_acme_config(&[]));
        assert!(tls_has_acme_config(&[TlsConfig {
            hosts: vec!["*.example.com".into()],
            source: acme_source(),
            expires_at: None,
        }]));
        assert!(!tls_has_acme_config(&[TlsConfig {
            hosts: vec!["example.com".into()],
            source: TlsSource::File {
                cert: "/a.pem".into(),
                key: "/a.key".into(),
            },
            expires_at: None,
        }]));
    }

    #[test]
    fn acme_sources_equivalent_rejects_mixed_types() {
        let file = TlsSource::File {
            cert: "/a.pem".into(),
            key: "/a.key".into(),
        };
        let acme = acme_source();
        let mut tls = vec![
            TlsConfig {
                hosts: vec!["x.example.com".into()],
                source: file,
                expires_at: None,
            },
            TlsConfig {
                hosts: vec!["x.example.com".into()],
                source: acme,
                expires_at: None,
            },
        ];
        normalize_tls_config(&mut tls);
        assert_eq!(tls.len(), 2);
    }
}
