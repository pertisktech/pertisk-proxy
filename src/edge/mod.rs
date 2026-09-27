//! Edge features shared by the Pingora and HTTP/3 paths:
//! response cache, route auth, rate limits, Consul discovery, static files,
//! TLS cipher grades, and automatic upstream TLS.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TlsGrade {
    High,
    #[default]
    Medium,
    Unsafe,
}

impl TlsGrade {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Unsafe => "unsafe",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsulConfig {
    /// Consul HTTP API, e.g. `http://127.0.0.1:8500`.
    pub address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datacenter: Option<String>,
}

/// Global cache / Consul / JWT settings copied onto the route table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeGlobals {
    #[serde(default)]
    pub tls_grade: TlsGrade,
    /// `0` disables the response cache.
    #[serde(default)]
    pub cache_size_mb: u64,
    #[serde(default = "default_cache_ttl")]
    pub cache_ttl_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consul: Option<ConsulConfig>,
    /// Master secret accepted by `POST /api/jwt` in addition to per-route secrets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jwt_secret: Option<String>,
}

fn default_cache_ttl() -> u64 {
    60
}

impl Default for EdgeGlobals {
    fn default() -> Self {
        Self {
            tls_grade: TlsGrade::Medium,
            cache_size_mb: 0,
            cache_ttl_secs: default_cache_ttl(),
            consul: None,
            jwt_secret: None,
        }
    }
}

/// Per-route authentication. Only one method is active.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RouteAuth {
    Basic { username: String, password: String },
    ApiKey { key: String },
    /// HS256. Accepts `Authorization: Bearer` or `?pertisk_token=` / `?araleztoken=`.
    Jwt { secret: String },
    /// Subrequest. 2xx from `url` allows the request.
    Forward { url: String },
}

/// Per-route edge policy. Missing fields stay off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EdgePolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<RouteAuth>,
    /// Requests per second for this client IP + host. `0` disables.
    #[serde(default)]
    pub rate_limit_rps: u32,
    /// Further requests are rejected after this many 4xx responses per second. `0` disables.
    #[serde(default)]
    pub x4xx_limit_rps: u32,
    /// `None` follows the global cache switch. `Some(false)` opts out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache: Option<bool>,
}

#[derive(Clone)]
pub struct CachedResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: bytes::Bytes,
}

struct CacheEntry {
    response: CachedResponse,
    expires: Instant,
    bytes: usize,
}

struct CacheInner {
    entries: HashMap<String, CacheEntry>,
    bytes: usize,
}

fn cache_store() -> &'static Mutex<CacheInner> {
    static CACHE: OnceLock<Mutex<CacheInner>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(CacheInner {
            entries: HashMap::new(),
            bytes: 0,
        })
    })
}

pub fn cache_get(key: &str) -> Option<CachedResponse> {
    let mut guard = cache_store().lock().ok()?;
    let expired = guard
        .entries
        .get(key)
        .map(|e| e.expires <= Instant::now())
        .unwrap_or(false);
    if expired {
        if let Some(old) = guard.entries.remove(key) {
            guard.bytes = guard.bytes.saturating_sub(old.bytes);
        }
        return None;
    }
    guard.entries.get(key).map(|e| e.response.clone())
}

pub fn cache_put(key: String, response: CachedResponse, ttl: Duration, max_bytes: usize) {
    if max_bytes == 0 || ttl.is_zero() {
        return;
    }
    let size = response.body.len()
        + response
            .headers
            .iter()
            .map(|(k, v)| k.len() + v.len())
            .sum::<usize>()
        + key.len();
    if size > max_bytes || size > 1024 * 1024 {
        return;
    }
    let Ok(mut guard) = cache_store().lock() else {
        return;
    };
    if let Some(old) = guard.entries.remove(&key) {
        guard.bytes = guard.bytes.saturating_sub(old.bytes);
    }
    while guard.bytes + size > max_bytes && !guard.entries.is_empty() {
        if let Some(drop_key) = guard.entries.keys().next().cloned() {
            if let Some(old) = guard.entries.remove(&drop_key) {
                guard.bytes = guard.bytes.saturating_sub(old.bytes);
            }
        } else {
            break;
        }
    }
    guard.bytes = guard.bytes.saturating_add(size);
    guard.entries.insert(
        key,
        CacheEntry {
            response,
            expires: Instant::now() + ttl,
            bytes: size,
        },
    );
}

pub fn cache_enabled(globals: &EdgeGlobals, policy: &EdgePolicy) -> bool {
    if globals.cache_size_mb == 0 {
        return false;
    }
    policy.cache.unwrap_or(true)
}

struct Window {
    second: u64,
    count: u32,
}

fn windows() -> &'static Mutex<HashMap<String, Window>> {
    static WINDOWS: OnceLock<Mutex<HashMap<String, Window>>> = OnceLock::new();
    WINDOWS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_second() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Returns true when the caller should be rejected with 429.
pub fn rate_limited(key: &str, limit: u32) -> bool {
    if limit == 0 {
        return false;
    }
    let Ok(mut guard) = windows().lock() else {
        return false;
    };
    let sec = now_second();
    let entry = guard.entry(key.to_string()).or_insert(Window {
        second: sec,
        count: 0,
    });
    if entry.second != sec {
        entry.second = sec;
        entry.count = 0;
    }
    if entry.count >= limit {
        return true;
    }
    entry.count = entry.count.saturating_add(1);
    false
}

pub fn note_4xx(client_ip: &str, host: &str) {
    let key = format!("4xx|{client_ip}|{host}");
    let _ = rate_limited(&key, u32::MAX);
}

pub fn x4xx_blocked(client_ip: &str, host: &str, limit: u32) -> bool {
    if limit == 0 {
        return false;
    }
    let key = format!("4xx|{client_ip}|{host}");
    let Ok(guard) = windows().lock() else {
        return false;
    };
    let sec = now_second();
    match guard.get(&key) {
        Some(entry) if entry.second == sec && entry.count >= limit => true,
        _ => false,
    }
}

pub fn b64url_encode(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | data[i + 2] as u32;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(T[((n >> 6) & 63) as usize] as char);
        out.push(T[(n & 63) as usize] as char);
        i += 3;
    }
    if i < data.len() {
        let rem = data.len() - i;
        let mut n = (data[i] as u32) << 16;
        if rem == 2 {
            n |= (data[i + 1] as u32) << 8;
        }
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        if rem == 2 {
            out.push(T[((n >> 6) & 63) as usize] as char);
        }
    }
    out
}

pub fn b64url_decode(data: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'-' | b'+' => Some(62),
            b'_' | b'/' => Some(63),
            _ => None,
        }
    }
    let bytes = data.as_bytes();
    let mut out = Vec::new();
    let mut buf: u32 = 0;
    let mut bits = 0;
    for &c in bytes {
        if c == b'=' {
            break;
        }
        let v = val(c)?;
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

pub fn issue_jwt(secret: &str, owner: &str, valid_minutes: u64) -> String {
    let header = b64url_encode(br#"{"alg":"HS256","typ":"JWT"}"#);
    let exp = now_second().saturating_add(valid_minutes.saturating_mul(60).max(60));
    let payload = format!(r#"{{"sub":"{owner}","exp":{exp}}}"#);
    let payload = b64url_encode(payload.as_bytes());
    let signing = format!("{header}.{payload}");
    let sig = sign_hs256(secret, signing.as_bytes());
    format!("{signing}.{sig}")
}

pub fn verify_jwt(secret: &str, token: &str) -> bool {
    let mut parts = token.split('.');
    let (Some(header), Some(payload), Some(sig)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    let signing = format!("{header}.{payload}");
    if !hmac_eq(secret, signing.as_bytes(), sig) {
        return false;
    }
    let Ok(raw) = String::from_utf8(b64url_decode(payload).unwrap_or_default()) else {
        return false;
    };
    let Some(exp) = raw
        .split("\"exp\":")
        .nth(1)
        .and_then(|s| s.trim_start().split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|s| s.parse::<u64>().ok())
    else {
        return false;
    };
    exp >= now_second()
}

fn sign_hs256(secret: &str, msg: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(msg);
    b64url_encode(&mac.finalize().into_bytes())
}

fn hmac_eq(secret: &str, msg: &[u8], sig: &str) -> bool {
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(msg);
    let Some(bytes) = b64url_decode(sig) else {
        return false;
    };
    mac.verify_slice(&bytes).is_ok()
}

pub fn basic_credentials(header: Option<&str>) -> Option<(String, String)> {
    let header = header?.trim();
    let encoded = header.strip_prefix("Basic ")?.trim();
    let raw = b64url_decode_std(encoded)?;
    let text = String::from_utf8(raw).ok()?;
    let (user, pass) = text.split_once(':')?;
    Some((user.to_string(), pass.to_string()))
}

fn b64url_decode_std(data: &str) -> Option<Vec<u8>> {
    // Standard base64 (basic auth), not URL-safe.
    let data = data.trim();
    let mut padded = data.replace('+', "-").replace('/', "_");
    while padded.len() % 4 != 0 {
        padded.push('=');
    }
    // Decoder ignores '=' and accepts both alphabets.
    b64url_decode(&padded)
}

pub fn bearer_token(header: Option<&str>) -> Option<&str> {
    header?.trim().strip_prefix("Bearer ").map(str::trim)
}

pub fn query_token(query: &str) -> Option<String> {
    for key in ["pertisk_token", "araleztoken"] {
        if let Some(v) = query_param(query, key) {
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

fn query_param(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        if parts.next() == Some(key) {
            return Some(parts.next().unwrap_or("").to_string());
        }
    }
    None
}

pub fn auth_header_ok(auth: &RouteAuth, headers: &http::HeaderMap, query: &str) -> bool {
    match auth {
        RouteAuth::Basic { username, password } => {
            let Some((user, pass)) = basic_credentials(
                headers
                    .get(http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok()),
            ) else {
                return false;
            };
            user == *username && pass == *password
        }
        RouteAuth::ApiKey { key } => headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
            == Some(key.as_str()),
        RouteAuth::Jwt { secret } => {
            if let Some(token) = bearer_token(
                headers
                    .get(http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok()),
            ) {
                if verify_jwt(secret, token) {
                    return true;
                }
            }
            query_token(query)
                .map(|token| verify_jwt(secret, &token))
                .unwrap_or(false)
        }
        RouteAuth::Forward { .. } => false,
    }
}

pub async fn forward_auth_ok(url: &str, host: &str, path: &str, headers: &http::HeaderMap) -> bool {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let mut req = client.get(url);
    req = req.header("X-Forwarded-Host", host);
    req = req.header("X-Forwarded-Uri", path);
    if let Some(method) = headers
        .get(http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        req = req.header(http::header::AUTHORIZATION, method);
    }
    match req.send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

pub fn safe_file(root: &str, url_path: &str) -> Option<PathBuf> {
    let root = Path::new(root);
    let rel = url_path.split('?').next().unwrap_or(url_path);
    let rel = rel.trim_start_matches('/');
    if rel.contains('\0') {
        return None;
    }
    let mut path = root.to_path_buf();
    for part in Path::new(rel).components() {
        match part {
            Component::Normal(s) => path.push(s),
            Component::CurDir => {}
            _ => return None,
        }
    }
    if path == root {
        path.push("index.html");
    }
    Some(path)
}

pub fn guess_mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "txt" | "md" => "text/plain; charset=utf-8",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

struct ConsulState {
    endpoints: HashMap<String, Vec<(String, u16)>>,
    rr: AtomicUsize,
}

fn consul_state() -> &'static Mutex<ConsulState> {
    static STATE: OnceLock<Mutex<ConsulState>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(ConsulState {
            endpoints: HashMap::new(),
            rr: AtomicUsize::new(0),
        })
    })
}

pub fn consul_pick(service: &str) -> Option<(String, u16)> {
    let guard = consul_state().lock().ok()?;
    let list = guard.endpoints.get(service)?;
    if list.is_empty() {
        return None;
    }
    let idx = guard.rr.fetch_add(1, Ordering::Relaxed) % list.len();
    Some(list[idx].clone())
}

pub fn consul_set(service: &str, endpoints: Vec<(String, u16)>) {
    if let Ok(mut guard) = consul_state().lock() {
        guard.endpoints.insert(service.to_string(), endpoints);
    }
}

pub async fn consul_refresh(config: &ConsulConfig, services: &[String]) {
    if services.is_empty() {
        return;
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    for service in services {
        let mut url = format!(
            "{}/v1/health/service/{}?passing=true",
            config.address.trim_end_matches('/'),
            service
        );
        if let Some(dc) = config.datacenter.as_deref().filter(|s| !s.is_empty()) {
            url.push_str("&dc=");
            url.push_str(dc);
        }
        let mut req = client.get(&url);
        if let Some(token) = config.token.as_deref().filter(|s| !s.is_empty()) {
            req = req.header("X-Consul-Token", token);
        }
        let Ok(resp) = req.send().await else {
            continue;
        };
        let Ok(body) = resp.json::<serde_json::Value>().await else {
            continue;
        };
        let Some(items) = body.as_array() else {
            continue;
        };
        let mut endpoints = Vec::new();
        for item in items {
            let port = item
                .pointer("/Service/Port")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u16;
            if port == 0 {
                continue;
            }
            let addr = item
                .pointer("/Service/Address")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    item.pointer("/Node/Address")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                });
            if let Some(addr) = addr {
                endpoints.push((addr.to_string(), port));
            }
        }
        if !endpoints.is_empty() {
            consul_set(service, endpoints);
        }
    }
}

struct TlsProbe {
    use_tls: bool,
    at: Instant,
}

fn probes() -> &'static Mutex<HashMap<String, TlsProbe>> {
    static PROBES: OnceLock<Mutex<HashMap<String, TlsProbe>>> = OnceLock::new();
    PROBES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Remember whether `host:port` speaks TLS. A short ClientHello distinguishes
/// TLS listeners from plain HTTP without trusting the certificate.
pub fn probe_upstream_tls(host: &str, port: u16) -> bool {
    let key = format!("{host}:{port}");
    if let Ok(guard) = probes().lock() {
        if let Some(hit) = guard.get(&key) {
            if hit.at.elapsed() < Duration::from_secs(300) {
                return hit.use_tls;
            }
        }
    }
    let use_tls = tcp_looks_like_tls(host, port);
    if let Ok(mut guard) = probes().lock() {
        guard.insert(
            key,
            TlsProbe {
                use_tls,
                at: Instant::now(),
            },
        );
    }
    use_tls
}

fn tcp_looks_like_tls(host: &str, port: u16) -> bool {
    use std::io::{Read, Write};
    use std::net::{TcpStream, ToSocketAddrs};
    let addr = match (host, port).to_socket_addrs() {
        Ok(mut addrs) => addrs.next(),
        Err(_) => None,
    };
    let Some(addr) = addr else {
        return port == 443;
    };
    let Ok(mut sock) = TcpStream::connect_timeout(&addr, Duration::from_millis(400)) else {
        return port == 443;
    };
    let _ = sock.set_read_timeout(Some(Duration::from_millis(400)));
    let _ = sock.set_write_timeout(Some(Duration::from_millis(400)));
    // TLS 1.0 ClientHello (no extensions) — enough for a server to answer 0x16.
    const HELLO: &[u8] = &[
        0x16, 0x03, 0x01, 0x00, 0x2d, 0x01, 0x00, 0x00, 0x29, 0x03, 0x03, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x02, 0x00, 0x2f, 0x01, 0x00,
    ];
    if sock.write_all(HELLO).is_err() {
        return false;
    }
    let mut buf = [0u8; 1];
    match sock.read(&mut buf) {
        Ok(1) => buf[0] == 0x16,
        _ => false,
    }
}

/// Apply a cipher grade to a Pingora OpenSSL acceptor.
/// `medium` keeps Pingora's Mozilla intermediate profile.
pub fn apply_tls_grade(settings: &mut pingora_core::listeners::tls::TlsSettings, grade: TlsGrade) {
    match grade {
        TlsGrade::Medium => {}
        TlsGrade::High => {
            let _ = settings.set_cipher_list(
                "ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256:\
                 ECDHE-ECDSA-AES256-GCM-SHA384:ECDHE-RSA-AES256-GCM-SHA384:\
                 ECDHE-ECDSA-CHACHA20-POLY1305:ECDHE-RSA-CHACHA20-POLY1305",
            );
            let _ = settings.set_ciphersuites(
                "TLS_AES_128_GCM_SHA256:TLS_AES_256_GCM_SHA384:TLS_CHACHA20_POLY1305_SHA256",
            );
        }
        TlsGrade::Unsafe => {
            let _ = settings.set_cipher_list("ALL:@SECLEVEL=0");
        }
    }
}

pub fn spawn_consul_poller(router: std::sync::Arc<crate::router::Router>) {
    std::thread::Builder::new()
        .name("pertisk-consul".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    tracing::warn!(error = %err, "consul poller runtime failed");
                    return;
                }
            };
            rt.block_on(async move {
                loop {
                    let snap = router.snapshot();
                    if let Some(cfg) = snap.globals().consul.clone() {
                        let services = snap.consul_services();
                        consul_refresh(&cfg, &services).await;
                    }
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            });
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jwt_roundtrip() {
        let token = issue_jwt("secret", "valod", 10);
        assert!(verify_jwt("secret", &token));
        assert!(!verify_jwt("other", &token));
    }

    #[test]
    fn rate_limit_trips() {
        let key = format!("test-{}", now_second());
        assert!(!rate_limited(&key, 2));
        assert!(!rate_limited(&key, 2));
        assert!(rate_limited(&key, 2));
    }

    #[test]
    fn file_rejects_traversal() {
        assert!(safe_file("/var/www", "/../etc/passwd").is_none());
        let path = safe_file("/var/www", "/css/app.css").unwrap();
        assert_eq!(path, PathBuf::from("/var/www/css/app.css"));
    }

    #[test]
    fn api_key_header() {
        let mut headers = http::HeaderMap::new();
        headers.insert("x-api-key", http::HeaderValue::from_static("abc"));
        let auth = RouteAuth::ApiKey { key: "abc".into() };
        assert!(auth_header_ok(&auth, &headers, ""));
        headers.insert("x-api-key", http::HeaderValue::from_static("nope"));
        assert!(!auth_header_ok(&auth, &headers, ""));
    }
}
