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
    asn: Option<AsnDb>,
    country_path: Option<PathBuf>,
    asn_path: Option<PathBuf>,
}

enum AsnDb {
    Mmdb(maxminddb::Reader<Vec<u8>>),
    /// ip2asn-combined.tsv style: start_ip\tend_ip\tasn\tcountry\torg
    Tsv(AsnTsvDb),
}

struct AsnTsvDb {
    v4: Vec<(u32, u32, u32)>,
    v6: Vec<(u128, u128, u32)>,
}

impl AsnTsvDb {
    fn load(path: &Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut v4 = Vec::new();
        let mut v6 = Vec::new();
        for (line_no, line) in raw.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut cols = line.split('\t');
            let start_s = cols.next().ok_or_else(|| format!("line {}: missing start", line_no + 1))?;
            let end_s = cols.next().ok_or_else(|| format!("line {}: missing end", line_no + 1))?;
            let asn_s = cols.next().unwrap_or("0");
            let asn: u32 = asn_s.parse().unwrap_or(0);
            if asn == 0 {
                continue;
            }
            let start: IpAddr = start_s
                .parse()
                .map_err(|e| format!("line {}: bad start IP: {e}", line_no + 1))?;
            let end: IpAddr = end_s
                .parse()
                .map_err(|e| format!("line {}: bad end IP: {e}", line_no + 1))?;
            match (start, end) {
                (IpAddr::V4(a), IpAddr::V4(b)) => {
                    v4.push((u32::from(a), u32::from(b), asn));
                }
                (IpAddr::V6(a), IpAddr::V6(b)) => {
                    v6.push((u128::from(a), u128::from(b), asn));
                }
                _ => {}
            }
        }
        v4.sort_unstable_by_key(|(start, _, _)| *start);
        v6.sort_unstable_by_key(|(start, _, _)| *start);
        Ok(Self { v4, v6 })
    }

    fn lookup(&self, addr: IpAddr) -> Option<u32> {
        match addr {
            IpAddr::V4(ip) => {
                let key = u32::from(ip);
                let idx = self.v4.partition_point(|(start, _, _)| *start <= key);
                if idx == 0 {
                    return None;
                }
                let (start, end, asn) = self.v4[idx - 1];
                if key >= start && key <= end {
                    Some(asn)
                } else {
                    None
                }
            }
            IpAddr::V6(ip) => {
                let key = u128::from(ip);
                let idx = self.v6.partition_point(|(start, _, _)| *start <= key);
                if idx == 0 {
                    return None;
                }
                let (start, end, asn) = self.v6[idx - 1];
                if key >= start && key <= end {
                    Some(asn)
                } else {
                    None
                }
            }
        }
    }
}

impl AsnDb {
    fn lookup(&self, addr: IpAddr) -> Option<u32> {
        match self {
            AsnDb::Mmdb(reader) => match reader.lookup::<geoip2::Asn>(addr) {
                Ok(Some(record)) => record.autonomous_system_number,
                Ok(None) => None,
                Err(err) => {
                    debug!(error = %err, "GeoIP ASN MMDB lookup failed");
                    None
                }
            },
            AsnDb::Tsv(db) => db.lookup(addr),
        }
    }
}

fn load_asn_db(path: &Path) -> Option<AsnDb> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_tsv = name.ends_with(".tsv")
        || name.ends_with(".tsv.gz")
        || name.contains("ip2asn")
        || name.ends_with(".csv");

    if is_tsv {
        // .tsv.gz not supported yet — only plain .tsv
        if name.ends_with(".gz") {
            warn!(path = %path.display(), "compressed ASN TSV is not supported; use uncompressed .tsv");
            return None;
        }
        match AsnTsvDb::load(path) {
            Ok(db) => {
                info!(
                    path = %path.display(),
                    v4_ranges = db.v4.len(),
                    v6_ranges = db.v6.len(),
                    "GeoIP ASN TSV database loaded"
                );
                Some(AsnDb::Tsv(db))
            }
            Err(err) => {
                warn!(path = %path.display(), error = %err, "GeoIP ASN TSV unavailable");
                None
            }
        }
    } else {
        match maxminddb::Reader::open_readfile(path) {
            Ok(reader) => {
                info!(path = %path.display(), "GeoIP ASN MMDB database loaded");
                Some(AsnDb::Mmdb(reader))
            }
            Err(err) => {
                warn!(path = %path.display(), error = %err, "GeoIP ASN MMDB unavailable");
                None
            }
        }
    }
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
        let asn = asn_path.and_then(load_asn_db);
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
        if let Some(asn_db) = &self.asn {
            info.asn = asn_db.lookup(addr);
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
    let ip = crate::proxy::forward::normalize_ip_str(ip)?;
    ENGINE.lookup(&ip)
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
    let normalized = ip.and_then(crate::proxy::forward::normalize_ip_str);
    // Private / SNAT peers (e.g. kube-proxy Cluster ET) have no MaxMind country — fail-open
    // so allowlists do not lock out the whole cluster until client IP is preserved.
    if let Some(ref ip) = normalized {
        if !crate::proxy::forward::is_public_routable_ip(ip) {
            return Decision::Allow;
        }
    }
    let info = normalized.as_deref().and_then(lookup);
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
        "/var/lib/pertisk-proxy/geoip/ip2asn-combined.tsv",
        "/var/lib/pertisk-proxy/geoip/ip2asn-v4.tsv",
        "/usr/share/GeoIP/GeoLite2-ASN.mmdb",
        "/usr/share/GeoIP/GeoIP2-ASN.mmdb",
        "/usr/share/GeoIP/ip2asn-combined.tsv",
    ] {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// Annotation keys written for GeoIP policy on Ingress / HTTPRoute.
pub const ANNOTATION_KEYS: &[&str] = &[
    "proxy.pertisk.tech/geoip-enabled",
    "proxy.pertisk.tech/geoip-allow-countries",
    "proxy.pertisk.tech/geoip-deny-countries",
    "proxy.pertisk.tech/geoip-allow-asns",
    "proxy.pertisk.tech/geoip-deny-asns",
];

fn flag_true(map: &std::collections::BTreeMap<String, String>, key: &str) -> bool {
    map.get(key)
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            matches!(v.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
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
    if flag_true(map, "proxy.pertisk.tech/security-exempt")
        || flag_true(map, "proxy.pertisk.tech/geoip-exempt")
    {
        return GeoIpPolicy::default();
    }
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

/// Replace GeoIP annotation keys on a metadata map from a policy.
pub fn apply_annotations(
    annotations: &mut std::collections::BTreeMap<String, String>,
    policy: &GeoIpPolicy,
) {
    for key in ANNOTATION_KEYS {
        annotations.remove(*key);
    }
    let policy = policy.clone().normalized();
    if policy.is_default() {
        return;
    }
    if policy.enabled {
        annotations.insert(
            "proxy.pertisk.tech/geoip-enabled".to_string(),
            "true".to_string(),
        );
    }
    if !policy.allow_countries.is_empty() {
        annotations.insert(
            "proxy.pertisk.tech/geoip-allow-countries".to_string(),
            policy.allow_countries.join(","),
        );
    }
    if !policy.deny_countries.is_empty() {
        annotations.insert(
            "proxy.pertisk.tech/geoip-deny-countries".to_string(),
            policy.deny_countries.join(","),
        );
    }
    if !policy.allow_asns.is_empty() {
        annotations.insert(
            "proxy.pertisk.tech/geoip-allow-asns".to_string(),
            policy
                .allow_asns
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    if !policy.deny_asns.is_empty() {
        annotations.insert(
            "proxy.pertisk.tech/geoip-deny-asns".to_string(),
            policy
                .deny_asns
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(","),
        );
    }
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

    #[test]
    fn tsv_asn_lookup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ip2asn-combined.tsv");
        std::fs::write(
            &path,
            "1.0.0.0\t1.0.0.255\t13335\tUS\tCLOUDFLARENET\n\
             8.8.8.0\t8.8.8.255\t15169\tUS\tGOOGLE\n",
        )
        .unwrap();
        let db = AsnTsvDb::load(&path).unwrap();
        assert_eq!(db.lookup("1.0.0.10".parse().unwrap()), Some(13335));
        assert_eq!(db.lookup("8.8.8.8".parse().unwrap()), Some(15169));
        assert_eq!(db.lookup("9.9.9.9".parse().unwrap()), None);

        let engine = GeoIpEngine::load(None, Some(path.as_path()));
        assert!(engine.asn_loaded());
        assert_eq!(engine.lookup("1.0.0.1").and_then(|i| i.asn), Some(13335));
    }
}
