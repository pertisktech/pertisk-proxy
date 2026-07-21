//! Math captcha challenge with HMAC-signed tokens and pass cookies.

use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub const CAPTCHA_PATH: &str = "/.pertisk/captcha";
pub const CAPTCHA_VERIFY_PATH: &str = "/.pertisk/captcha/verify";
const COOKIE_NAME: &str = "pertisk_captcha";
const CHALLENGE_TTL_SECS: u64 = 600;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptchaPolicy {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub enabled: bool,
    #[serde(default = "default_cookie_ttl")]
    pub cookie_ttl_secs: u64,
}

fn default_cookie_ttl() -> u64 {
    86_400
}

impl Default for CaptchaPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            cookie_ttl_secs: default_cookie_ttl(),
        }
    }
}

impl CaptchaPolicy {
    pub fn is_default(&self) -> bool {
        !self.enabled && self.cookie_ttl_secs == default_cookie_ttl()
    }

    pub fn normalized(mut self) -> Self {
        if self.cookie_ttl_secs == 0 {
            self.cookie_ttl_secs = default_cookie_ttl();
        }
        self
    }
}

static SECRET: LazyLock<Vec<u8>> = LazyLock::new(|| {
    if let Ok(raw) = std::env::var("PERTISK_CAPTCHA_SECRET") {
        let t = raw.trim();
        if !t.is_empty() {
            return t.as_bytes().to_vec();
        }
    }
    if let Ok(raw) = std::env::var("PERTISK_AUTH_SIGNING_SECRET") {
        let t = raw.trim();
        if !t.is_empty() {
            return t.as_bytes().to_vec();
        }
    }
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill(&mut bytes);
    tracing::warn!(
        "PERTISK_CAPTCHA_SECRET unset; using ephemeral captcha secret (cookies reset on restart)"
    );
    bytes.to_vec()
});

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sign(payload: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(&SECRET).expect("HMAC key length is valid");
    mac.update(payload.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn verify_sig(payload: &str, sig: &str) -> bool {
    let expected = sign(payload);
    expected.eq_ignore_ascii_case(sig.trim())
}

pub fn is_captcha_path(path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    path == CAPTCHA_PATH || path == CAPTCHA_VERIFY_PATH
}

fn parse_cookie_value(cookie_header: &str, name: &str) -> Option<String> {
    for part in cookie_header.split(';') {
        let part = part.trim();
        if let Some((k, v)) = part.split_once('=') {
            if k.trim() == name {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// Pass cookie format: `v1.<exp>.<ip_hash_or_dash>.<sig>`
pub fn has_valid_pass_cookie(cookie_header: Option<&str>, client_ip: Option<&str>) -> bool {
    let Some(header) = cookie_header else {
        return false;
    };
    let Some(raw) = parse_cookie_value(header, COOKIE_NAME) else {
        return false;
    };
    let parts: Vec<&str> = raw.split('.').collect();
    if parts.len() != 4 || parts[0] != "v1" {
        return false;
    }
    let exp: u64 = match parts[1].parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    if now_secs() > exp {
        return false;
    }
    let payload = format!("v1.{}.{}", parts[1], parts[2]);
    if !verify_sig(&payload, parts[3]) {
        return false;
    }
    // Optional IP bind: if cookie has ip hash and client IP present, require match.
    if parts[2] != "-" {
        if let Some(ip) = client_ip {
            let ip_tag = short_ip_tag(ip);
            if parts[2] != ip_tag {
                return false;
            }
        }
    }
    true
}

fn short_ip_tag(ip: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(&SECRET).expect("HMAC key length is valid");
    mac.update(ip.as_bytes());
    let bytes = mac.finalize().into_bytes();
    hex::encode(&bytes[..8])
}

pub fn mint_pass_cookie(client_ip: Option<&str>, ttl_secs: u64) -> String {
    let exp = now_secs().saturating_add(ttl_secs.max(60));
    let ip_tag = client_ip
        .map(short_ip_tag)
        .unwrap_or_else(|| "-".to_string());
    let payload = format!("v1.{exp}.{ip_tag}");
    let sig = sign(&payload);
    let value = format!("{payload}.{sig}");
    format!(
        "{COOKIE_NAME}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={ttl_secs}"
    )
}

struct Challenge {
    a: u32,
    b: u32,
    token: String,
}

fn mint_challenge() -> Challenge {
    let mut rng = rand::thread_rng();
    let a = rng.gen_range(1..20);
    let b = rng.gen_range(1..20);
    let exp = now_secs().saturating_add(CHALLENGE_TTL_SECS);
    let payload = format!("c1.{a}.{b}.{exp}");
    let sig = sign(&payload);
    Challenge {
        a,
        b,
        token: format!("{payload}.{sig}"),
    }
}

fn verify_challenge_token(token: &str, answer: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 5 || parts[0] != "c1" {
        return false;
    }
    let a: u32 = match parts[1].parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    let b: u32 = match parts[2].parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    let exp: u64 = match parts[3].parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    if now_secs() > exp {
        return false;
    }
    let payload = format!("c1.{a}.{b}.{exp}");
    if !verify_sig(&payload, parts[4]) {
        return false;
    }
    let Ok(ans) = answer.trim().parse::<u32>() else {
        return false;
    };
    ans == a.saturating_add(b)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn challenge_page(next_path: &str, reason: &str) -> (String, &'static str) {
    let challenge = mint_challenge();
    let next = html_escape(if next_path.is_empty() { "/" } else { next_path });
    let reason = html_escape(reason);
    let body = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>Security check</title>
<style>
body{{font-family:ui-sans-serif,system-ui,sans-serif;background:#0f1115;color:#e8eaed;display:flex;min-height:100vh;align-items:center;justify-content:center;margin:0}}
.card{{background:#1a1d24;border:1px solid #2a2f3a;border-radius:12px;padding:2rem;max-width:420px;width:90%}}
h1{{font-size:1.25rem;margin:0 0 .5rem}}
p{{color:#9aa0a6;font-size:.9rem;margin:0 0 1.25rem}}
label{{display:block;font-size:.85rem;margin-bottom:.35rem}}
input{{width:100%;box-sizing:border-box;padding:.65rem .75rem;border-radius:8px;border:1px solid #2a2f3a;background:#0f1115;color:#e8eaed}}
button{{margin-top:1rem;width:100%;padding:.7rem;border:0;border-radius:8px;background:#7c59f0;color:white;font-weight:600;cursor:pointer}}
.q{{font-size:1.4rem;font-weight:700;margin:1rem 0}}
</style>
</head>
<body>
<div class="card">
<h1>Security check</h1>
<p>Confirm you are human to continue ({reason}).</p>
<div class="q">{a} + {b} = ?</div>
<form method="get" action="{verify}">
<input type="hidden" name="token" value="{token}"/>
<input type="hidden" name="next" value="{next}"/>
<label for="answer">Answer</label>
<input id="answer" name="answer" inputmode="numeric" autocomplete="off" required autofocus/>
<button type="submit">Continue</button>
</form>
</div>
</body>
</html>"#,
        reason = reason,
        a = challenge.a,
        b = challenge.b,
        verify = CAPTCHA_VERIFY_PATH,
        token = html_escape(&challenge.token),
        next = next,
    );
    (body, "text/html; charset=utf-8")
}

/// Returns (set_cookie, redirect_location) on success, or error message.
pub fn verify_and_pass_cookie(
    token: Option<&str>,
    answer: Option<&str>,
    next: Option<&str>,
    client_ip: Option<&str>,
    ttl_secs: u64,
) -> Result<(String, String), &'static str> {
    let token = token.ok_or("missing token")?;
    let answer = answer.ok_or("missing answer")?;
    if !verify_challenge_token(token, answer) {
        return Err("incorrect answer");
    }
    let next = next.unwrap_or("/");
    let next = if next.starts_with('/') && !next.starts_with("//") {
        next
    } else {
        "/"
    };
    Ok((mint_pass_cookie(client_ip, ttl_secs), next.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_roundtrip() {
        let c = mint_challenge();
        let ans = (c.a + c.b).to_string();
        assert!(verify_challenge_token(&c.token, &ans));
        assert!(!verify_challenge_token(&c.token, "0"));
    }

    #[test]
    fn pass_cookie_valid() {
        let cookie_line = mint_pass_cookie(Some("1.2.3.4"), 3600);
        let value = cookie_line
            .split(';')
            .next()
            .unwrap()
            .trim_start_matches("pertisk_captcha=");
        let header = format!("pertisk_captcha={value}");
        assert!(has_valid_pass_cookie(Some(&header), Some("1.2.3.4")));
        assert!(!has_valid_pass_cookie(Some(&header), Some("8.8.8.8")));
    }
}
