//! MaxMind GeoIP lookups and per-site allow/deny policy (country + ASN).

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use maxminddb::geoip2;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

/// Per-site GeoIP policy. Empty allow lists mean “no allowlist” (only deny applies).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeoIpPolicy {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub enabled: bool,
    /// ISO 3166-1 alpha-2 codes (e.g. `TH`, `US`). Non-empty = allowlist.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_countries: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny_countries: Vec<String>,
    /// Autonomous System Numbers. Non-empty = allowlist.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_asns: Vec<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny_asns: Vec<u32>,
}

impl GeoIpPolicy {
    pub fn is_active(&self) -> bool {
        self.enabled
            && (!self.allow_countries.is_empty()
                || !self.deny_countries.is_empty()
                || !self.allow_asns.is_empty()
                || !self.deny_asns.is_empty())
    }

    pub fn is_default(&self) -> bool {
        !self.enabled
            && self.allow_countries.is_empty()
            && self.deny_countries.is_empty()
            && self.allow_asns.is_empty()
            && self.deny_asns.is_empty()
    }

    pub fn normalized(mut self) -> Self {
        self.allow_countries = normalize_countries(self.allow_countries);
        self.deny_countries = normalize_countries(self.deny_countries);
        self.allow_asns.sort_unstable();
        self.allow_asns.dedup();
        self.deny_asns.sort_unstable();
        self.deny_asns.dedup();
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GeoInfo {
    pub country: Option<String>,
    pub asn: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    BlockCountry,
    BlockAsn,
}

pub struct GeoIpEngine {
    country: Option<maxminddb::Reader<Vec<u8>>>,
    asn: Option<maxminddb::Reader<Vec<u8>>>,
    country_path: Option<PathBuf>,
    asn_path: Option<PathBuf>,
}

impl GeoIpEngine {
    pub fn from_env() -> Self {
        let country_path = env_path("PERTISK_GEOIP_COUNTRY_DB").or_else(default_country_path);
        let asn_path = env_path("PERTISK_GEOIP_ASN_DB").or_else(default_asn_path);
        Self::load(country_path.as_deref(), asn_path.as_deref())
    }

    pub fn load(country_path: Option<&Path>, asn_path: Option<&Path>) -> Self {
        let country = country_path.and_then(|path| match maxminddb::Reader::open_readfile(path) {
            Ok(reader) => {
                info!(path = %path.display(), "GeoIP country database loaded");
                Some(reader)
            }
            Err(err) => {
                warn!(path = %path.display(), error = %err, "GeoIP country database unavailable");
                None
            }
        });
        let asn = asn_path.and_then(|path| match maxminddb::Reader::open_readfile(path) {
            Ok(reader) => {
                info!(path = %path.display(), "GeoIP ASN database loaded");
                Some(reader)
            }
            Err(err) => {
                warn!(path = %path.display(), error = %err, "GeoIP ASN database unavailable");
                None
            }
        });
        Self {
            country,
            asn,
            country_path: country_path.map(Path::to_path_buf),
            asn_path: asn_path.map(Path::to_path_buf),
        }
    }

    pub fn country_loaded(&self) -> bool {
        self.country.is_some()
    }

    pub fn asn_loaded(&self) -> bool {
        self.asn.is_some()
    }

    pub fn status(&self) -> GeoIpStatus {
        GeoIpStatus {
            country_db_loaded: self.country_loaded(),
            asn_db_loaded: self.asn_loaded(),
            country_db_path: self
                .country_path
                .as_ref()
                .map(|p| p.display().to_string()),
            asn_db_path: self.asn_path.as_ref().map(|p| p.display().to_string()),
        }
    }

    pub fn lookup(&self, ip: &str) -> Option<GeoInfo> {
        let addr: IpAddr = ip.trim().parse().ok()?;
        let mut info = GeoInfo::default();
        if let Some(reader) = &self.country {
            match reader.lookup::<geoip2::Country>(addr) {
                Ok(Some(record)) => {
                    info.country = record
                        .country
                        .and_then(|c| c.iso_code)
                        .map(|code| code.to_ascii_uppercase());
                }
                Ok(None) => {}
                Err(err) => debug!(ip = %ip, error = %err, "GeoIP country lookup failed"),
            }
        }
        if let Some(reader) = &self.asn {
            match reader.lookup::<geoip2::Asn>(addr) {
                Ok(Some(record)) => {
                    info.asn = record.autonomous_system_number;
                }
                Ok(None) => {}
                Err(err) => debug!(ip = %ip, error = %err, "GeoIP ASN lookup failed"),
            }
        }
        if info.country.is_none() && info.asn.is_none() {
            None
        } else {
            Some(info)
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GeoIpStatus {
    pub country_db_loaded: bool,
    pub asn_db_loaded: bool,
    pub country_db_path: Option<String>,
    pub asn_db_path: Option<String>,
}

static ENGINE: LazyLock<GeoIpEngine> = LazyLock::new(GeoIpEngine::from_env);

pub fn engine() -> &'static GeoIpEngine {
    &ENGINE
}

pub fn status() -> GeoIpStatus {
    ENGINE.status()
}

pub fn lookup(ip: &str) -> Option<GeoInfo> {
    ENGINE.lookup(ip)
}

/// Evaluate policy against a lookup result. Fail-open when the relevant DB is missing.
pub fn evaluate(policy: &GeoIpPolicy, info: Option<&GeoInfo>) -> Decision {
    if !policy.is_active() {
        return Decision::Allow;
    }

    let country = info.and_then(|i| i.country.as_deref());
    let asn = info.and_then(|i| i.asn);

    if !policy.deny_countries.is_empty() {
        if let Some(code) = country {
            if policy.deny_countries.iter().any(|c| c == code) {
                return Decision::BlockCountry;
            }
        }
    }
    if !policy.allow_countries.is_empty() {
        match country {
            Some(code) if policy.allow_countries.iter().any(|c| c == code) => {}
            Some(_) => return Decision::BlockCountry,
            None if ENGINE.country_loaded() => return Decision::BlockCountry,
            None => {} // fail-open without country DB / private IP
        }
    }

    if !policy.deny_asns.is_empty() {
        if let Some(num) = asn {
            if policy.deny_asns.contains(&num) {
                return Decision::BlockAsn;
            }
        }
    }
    if !policy.allow_asns.is_empty() {
        match asn {
            Some(num) if policy.allow_asns.contains(&num) => {}
            Some(_) => return Decision::BlockAsn,
            None if ENGINE.asn_loaded() => return Decision::BlockAsn,
            None => {}
        }
    }

    Decision::Allow
}

pub fn evaluate_ip(policy: &GeoIpPolicy, ip: Option<&str>) -> Decision {
    if !policy.is_active() {
        return Decision::Allow;
    }
    let info = ip.and_then(lookup);
    evaluate(policy, info.as_ref())
}

fn normalize_countries(list: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = list
        .into_iter()
        .map(|c| c.trim().to_ascii_uppercase())
        .filter(|c| !c.is_empty())
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).and_then(|v| {
        let path = PathBuf::from(v);
        if path.as_os_str().is_empty() {
            None
        } else {
            Some(path)
        }
    })
}

fn default_country_path() -> Option<PathBuf> {
    for candidate in [
        "/var/lib/pertisk-proxy/geoip/GeoLite2-Country.mmdb",
        "/usr/share/GeoIP/GeoLite2-Country.mmdb",
        "/usr/share/GeoIP/GeoIP2-Country.mmdb",
    ] {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn default_asn_path() -> Option<PathBuf> {
    for candidate in [
        "/var/lib/pertisk-proxy/geoip/GeoLite2-ASN.mmdb",
        "/usr/share/GeoIP/GeoLite2-ASN.mmdb",
        "/usr/share/GeoIP/GeoIP2-ASN.mmdb",
    ] {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// Build a policy from Kubernetes Ingress / HTTPRoute annotations.
///
/// Supported keys:
/// - `proxy.pertisk.tech/geoip-enabled`: `true`/`false`
/// - `proxy.pertisk.tech/geoip-allow-countries`: `TH,US`
/// - `proxy.pertisk.tech/geoip-deny-countries`: `CN`
/// - `proxy.pertisk.tech/geoip-allow-asns`: `13335,AS15169`
/// - `proxy.pertisk.tech/geoip-deny-asns`: `12345`
pub fn policy_from_annotations(
    annotations: Option<&std::collections::BTreeMap<String, String>>,
) -> GeoIpPolicy {
    let Some(map) = annotations else {
        return GeoIpPolicy::default();
    };
    let enabled = map
        .get("proxy.pertisk.tech/geoip-enabled")
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            matches!(v.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false);
    let allow_countries = map
        .get("proxy.pertisk.tech/geoip-allow-countries")
        .map(|v| parse_countries(v))
        .unwrap_or_default();
    let deny_countries = map
        .get("proxy.pertisk.tech/geoip-deny-countries")
        .map(|v| parse_countries(v))
        .unwrap_or_default();
    let allow_asns = map
        .get("proxy.pertisk.tech/geoip-allow-asns")
        .map(|v| parse_asns(v))
        .unwrap_or_default();
    let deny_asns = map
        .get("proxy.pertisk.tech/geoip-deny-asns")
        .map(|v| parse_asns(v))
        .unwrap_or_default();
    GeoIpPolicy {
        enabled: enabled
            || !allow_countries.is_empty()
            || !deny_countries.is_empty()
            || !allow_asns.is_empty()
            || !deny_asns.is_empty(),
        allow_countries,
        deny_countries,
        allow_asns,
        deny_asns,
    }
    .normalized()
}

/// Parse comma/space-separated country codes from an annotation or form field.
pub fn parse_countries(raw: &str) -> Vec<String> {
    normalize_countries(
        raw.split([',', ' ', ';'])
            .map(|s| s.to_string())
            .collect(),
    )
}

/// Parse comma/space-separated ASNs (`AS13335` or `13335`).
pub fn parse_asns(raw: &str) -> Vec<u32> {
    let mut out: Vec<u32> = raw
        .split([',', ' ', ';'])
        .filter_map(|part| {
            let part = part.trim().trim_start_matches(['A', 'S', 'a', 's']);
            part.parse().ok()
        })
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_country_blocks() {
        let policy = GeoIpPolicy {
            enabled: true,
            deny_countries: vec!["CN".into()],
            ..Default::default()
        };
        let info = GeoInfo {
            country: Some("CN".into()),
            asn: None,
        };
        assert_eq!(evaluate(&policy, Some(&info)), Decision::BlockCountry);
        let ok = GeoInfo {
            country: Some("TH".into()),
            asn: None,
        };
        assert_eq!(evaluate(&policy, Some(&ok)), Decision::Allow);
    }

    #[test]
    fn allowlist_country() {
        let policy = GeoIpPolicy {
            enabled: true,
            allow_countries: vec!["TH".into(), "US".into()],
            ..Default::default()
        };
        assert_eq!(
            evaluate(
                &policy,
                Some(&GeoInfo {
                    country: Some("US".into()),
                    asn: None
                })
            ),
            Decision::Allow
        );
        assert_eq!(
            evaluate(
                &policy,
                Some(&GeoInfo {
                    country: Some("RU".into()),
                    asn: None
                })
            ),
            Decision::BlockCountry
        );
    }

    #[test]
    fn deny_asn_blocks() {
        let policy = GeoIpPolicy {
            enabled: true,
            deny_asns: vec![13335],
            ..Default::default()
        };
        assert_eq!(
            evaluate(
                &policy,
                Some(&GeoInfo {
                    country: None,
                    asn: Some(13335)
                })
            ),
            Decision::BlockAsn
        );
    }

    #[test]
    fn inactive_without_lists() {
        let policy = GeoIpPolicy {
            enabled: true,
            ..Default::default()
        };
        assert!(!policy.is_active());
        assert_eq!(evaluate(&policy, None), Decision::Allow);
    }

    #[test]
    fn parse_helpers() {
        assert_eq!(parse_countries("th, US;vn"), vec!["TH", "US", "VN"]);
        assert_eq!(parse_asns("AS13335, 15169"), vec![13335, 15169]);
    }
}
