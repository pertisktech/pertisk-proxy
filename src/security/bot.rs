//! Lightweight bot scoring from request signals + per-IP rate.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::RequestView;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotPolicy {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub enabled: bool,
    /// Score at or above this triggers captcha challenge (if captcha enabled).
    #[serde(default = "default_challenge_score")]
    pub challenge_score: u32,
    /// Score at or above this hard-blocks.
    #[serde(default = "default_block_score")]
    pub block_score: u32,
    /// Soft rate signal: requests/minute from one IP above this adds score.
    #[serde(default = "default_rate_limit")]
    pub rate_limit_per_min: u32,
}

fn default_challenge_score() -> u32 {
    40
}
fn default_block_score() -> u32 {
    80
}
fn default_rate_limit() -> u32 {
    120
}

impl Default for BotPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            challenge_score: default_challenge_score(),
            block_score: default_block_score(),
            rate_limit_per_min: default_rate_limit(),
        }
    }
}

impl BotPolicy {
    pub fn is_active(&self) -> bool {
        self.enabled
    }

    pub fn is_default(&self) -> bool {
        !self.enabled
            && self.challenge_score == default_challenge_score()
            && self.block_score == default_block_score()
            && self.rate_limit_per_min == default_rate_limit()
    }

    pub fn normalized(mut self) -> Self {
        if self.challenge_score == 0 {
            self.challenge_score = default_challenge_score();
        }
        if self.block_score == 0 {
            self.block_score = default_block_score();
        }
        if self.block_score < self.challenge_score {
            self.block_score = self.challenge_score;
        }
        if self.rate_limit_per_min == 0 {
            self.rate_limit_per_min = default_rate_limit();
        }
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotVerdict {
    Allow,
    Challenge,
    Block,
}

#[derive(Debug, Clone)]
pub struct BotScore {
    pub score: u32,
    pub verdict: BotVerdict,
    pub reasons: Vec<String>,
}

const AUTOMATION_UA: &[&str] = &[
    "curl/",
    "wget/",
    "python-requests",
    "python-urllib",
    "go-http-client",
    "scrapy",
    "httpclient",
    "libwww-perl",
    "java/",
    "okhttp",
    "headlesschrome",
    "phantomjs",
];

struct RateBucket {
    window_start: Instant,
    count: u32,
}

fn rate_map() -> &'static Mutex<HashMap<String, RateBucket>> {
    static MAP: OnceLock<Mutex<HashMap<String, RateBucket>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

fn record_rate(ip: &str, limit: u32) -> u32 {
    let mut map = rate_map().lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    // Opportunistic prune when large.
    if map.len() > 10_000 {
        map.retain(|_, b| now.duration_since(b.window_start) < Duration::from_secs(120));
    }
    let entry = map.entry(ip.to_string()).or_insert(RateBucket {
        window_start: now,
        count: 0,
    });
    if now.duration_since(entry.window_start) >= Duration::from_secs(60) {
        entry.window_start = now;
        entry.count = 0;
    }
    entry.count = entry.count.saturating_add(1);
    if entry.count > limit {
        entry.count.saturating_sub(limit)
    } else {
        0
    }
}

pub fn score_bot(policy: &BotPolicy, req: &RequestView<'_>) -> BotScore {
    let mut score = 0u32;
    let mut reasons = Vec::new();

    match req.user_agent.map(str::trim).filter(|s| !s.is_empty()) {
        None => {
            score += 45;
            reasons.push("missing-ua".into());
        }
        Some(ua) => {
            let ua_l = ua.to_ascii_lowercase();
            if AUTOMATION_UA.iter().any(|p| ua_l.contains(p)) {
                score += 30;
                reasons.push("automation-ua".into());
            }
            if ua_l.len() < 12 {
                score += 15;
                reasons.push("short-ua".into());
            }
        }
    }

    if req.accept.map(str::trim).filter(|s| !s.is_empty()).is_none() {
        score += 15;
        reasons.push("missing-accept".into());
    }
    if req
        .accept_language
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_none()
    {
        score += 10;
        reasons.push("missing-accept-language".into());
    }

    if let Some(ip) = req.client_ip.map(str::trim).filter(|s| !s.is_empty()) {
        let over = record_rate(ip, policy.rate_limit_per_min);
        if over > 0 {
            let add = 25u32.saturating_add((over / 10).min(40));
            score += add;
            reasons.push("rate".into());
        }
    }

    let verdict = if score >= policy.block_score {
        BotVerdict::Block
    } else if score >= policy.challenge_score {
        BotVerdict::Challenge
    } else {
        BotVerdict::Allow
    };

    BotScore {
        score,
        verdict,
        reasons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::RequestView;

    #[test]
    fn missing_ua_challenges() {
        let policy = BotPolicy {
            enabled: true,
            challenge_score: 40,
            block_score: 80,
            rate_limit_per_min: 10_000,
        };
        let req = RequestView {
            method: "GET",
            path: "/",
            query: "",
            user_agent: None,
            accept: Some("*/*"),
            accept_language: Some("en"),
            cookie: None,
            client_ip: Some("9.9.9.9"),
        };
        let s = score_bot(&policy, &req);
        assert!(s.score >= 40);
        assert_eq!(s.verdict, BotVerdict::Challenge);
    }
}
