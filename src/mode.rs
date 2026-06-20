use std::env;
use std::fmt;
use std::str::FromStr;

use anyhow::{bail, Result};

/// Top-level operating mode for pertisk-proxy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperatingMode {
    /// Kubernetes Ingress controller — watches Ingress resources.
    Ingress,
    /// Standalone reverse proxy with nginx-, caddy-, or traefik-like behavior.
    Proxy(ProxyKind),
}

/// Which reverse-proxy personality to emulate in proxy mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyKind {
    /// Static high-performance reverse proxy (nginx-style).
    Nginx,
    /// Simple declarative proxy with automatic HTTPS (caddy-style).
    Caddy,
    /// Dynamic routing with hot reload and middleware (traefik-style).
    Traefik,
}

impl ProxyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Nginx => "nginx",
            Self::Caddy => "caddy",
            Self::Traefik => "traefik",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Nginx => "static reverse proxy with manual TLS",
            Self::Caddy => "simple reverse proxy with automatic HTTPS",
            Self::Traefik => "dynamic reverse proxy with hot reload and middleware",
        }
    }

    pub fn default_routes_watch(self) -> bool {
        match self {
            Self::Nginx => false,
            Self::Caddy | Self::Traefik => true,
        }
    }
}

impl FromStr for ProxyKind {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "nginx" => Ok(Self::Nginx),
            "caddy" => Ok(Self::Caddy),
            "traefik" => Ok(Self::Traefik),
            other => bail!("unknown proxy kind {other:?}, expected nginx, caddy, or traefik"),
        }
    }
}

impl fmt::Display for ProxyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for OperatingMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ingress => write!(f, "ingress"),
            Self::Proxy(kind) => write!(f, "proxy/{kind}"),
        }
    }
}

pub fn parse_operating_mode() -> Result<OperatingMode> {
    let raw = env::var("MODE")
        .or_else(|_| env::var("OPERATING_MODE"))
        .unwrap_or_else(|_| "ingress".into());

    match raw.to_ascii_lowercase().as_str() {
        "ingress" | "k8s" | "kubernetes" => Ok(OperatingMode::Ingress),
        "nginx" => Ok(OperatingMode::Proxy(ProxyKind::Nginx)),
        "caddy" => Ok(OperatingMode::Proxy(ProxyKind::Caddy)),
        "traefik" => Ok(OperatingMode::Proxy(ProxyKind::Traefik)),
        "proxy" => {
            let kind = env::var("PROXY_KIND")
                .or_else(|_| env::var("PROXY_TYPE"))
                .unwrap_or_else(|_| "nginx".into())
                .parse()?;
            Ok(OperatingMode::Proxy(kind))
        }
        other => bail!(
            "unknown MODE {other:?}; use ingress, nginx, caddy, traefik, or proxy with PROXY_KIND"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_kinds() {
        assert_eq!("nginx".parse::<ProxyKind>().unwrap(), ProxyKind::Nginx);
        assert_eq!("caddy".parse::<ProxyKind>().unwrap(), ProxyKind::Caddy);
        assert_eq!("traefik".parse::<ProxyKind>().unwrap(), ProxyKind::Traefik);
    }
}
