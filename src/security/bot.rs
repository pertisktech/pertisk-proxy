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

fn rate_prune_threshold() -> usize {
    #[cfg(test)]
    {
        8
    }
    #[cfg(not(test))]
    {
        10_000
    }
}

fn record_rate(ip: &str, limit: u32) -> u32 {
    let mut map = rate_map().lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    // Opportunistic prune when large.
    if map.len() > rate_prune_threshold() {
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

    if req
        .accept
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_none()
    {
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn req<'a>(
        user_agent: Option<&'a str>,
        accept: Option<&'a str>,
        accept_language: Option<&'a str>,
        client_ip: Option<&'a str>,
    ) -> RequestView<'a> {
        RequestView {
            method: "GET",
            path: "/",
            query: "",
            user_agent,
            accept,
            accept_language,
            cookie: None,
            client_ip,
        }
    }

    fn unique_ip() -> String {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        format!(
            "bot-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )
    }

    /// Serialize tests that mutate the shared rate map.
    static RATE_TEST_LOCK: Mutex<()> = Mutex::new(());

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

    #[test]
    fn policy_helpers_report_default_active_and_normalize() {
        assert!(BotPolicy::default().is_default());
        assert!(!BotPolicy::default().is_active());

        let policy = BotPolicy {
            enabled: true,
            challenge_score: 0,
            block_score: 10,
            rate_limit_per_min: 0,
        }
        .normalized();

        assert!(policy.is_active());
        assert!(!policy.is_default());
        assert_eq!(policy.challenge_score, 40);
        assert_eq!(policy.block_score, 40);
        assert_eq!(policy.rate_limit_per_min, 120);
    }

    #[test]
    fn automation_and_missing_headers_add_scores() {
        let policy = BotPolicy {
            enabled: true,
            challenge_score: 40,
            block_score: 60,
            rate_limit_per_min: 10_000,
        };
        let ip = unique_ip();
        let score = score_bot(&policy, &req(Some("curl/8"), None, None, Some(ip.as_str())));

        assert_eq!(score.verdict, BotVerdict::Block);
        assert!(score.reasons.contains(&"automation-ua".to_string()));
        assert!(score.reasons.contains(&"short-ua".to_string()));
        assert!(score.reasons.contains(&"missing-accept".to_string()));
        assert!(score
            .reasons
            .contains(&"missing-accept-language".to_string()));
    }

    #[test]
    fn benign_browser_request_allows() {
        let policy = BotPolicy {
            enabled: true,
            challenge_score: 40,
            block_score: 80,
            rate_limit_per_min: 10_000,
        };
        let ip = unique_ip();
        let score = score_bot(
            &policy,
            &req(
                Some("Mozilla/5.0 (Macintosh; Intel Mac OS X)"),
                Some("text/html"),
                Some("en-US,en;q=0.9"),
                Some(ip.as_str()),
            ),
        );

        assert_eq!(score.score, 0);
        assert_eq!(score.verdict, BotVerdict::Allow);
        assert!(score.reasons.is_empty());
    }

    #[test]
    fn rate_limit_adds_signal_after_limit() {
        let _guard = RATE_TEST_LOCK.lock().unwrap();
        let policy = BotPolicy {
            enabled: true,
            challenge_score: 25,
            block_score: 80,
            rate_limit_per_min: 1,
        };
        let ip = unique_ip();
        let first = score_bot(
            &policy,
            &req(
                Some("Mozilla/5.0 (Macintosh; Intel Mac OS X)"),
                Some("text/html"),
                Some("en"),
                Some(ip.as_str()),
            ),
        );
        let second = score_bot(
            &policy,
            &req(
                Some("Mozilla/5.0 (Macintosh; Intel Mac OS X)"),
                Some("text/html"),
                Some("en"),
                Some(ip.as_str()),
            ),
        );

        assert_eq!(first.verdict, BotVerdict::Allow);
        assert_eq!(second.verdict, BotVerdict::Challenge);
        assert!(second.reasons.contains(&"rate".to_string()));
        assert!(second.score >= 25);
    }

    #[test]
    fn rate_map_prunes_stale_buckets_and_rolls_window() {
        let _guard = RATE_TEST_LOCK.lock().unwrap();
        {
            let mut map = rate_map().lock().unwrap();
            map.clear();
            for i in 0..10 {
                map.insert(
                    format!("198.51.100.{i}"),
                    RateBucket {
                        window_start: Instant::now() - Duration::from_secs(200),
                        count: 3,
                    },
                );
            }
            map.insert(
                "203.0.113.77".into(),
                RateBucket {
                    window_start: Instant::now() - Duration::from_secs(90),
                    count: 2,
                },
            );
        }

        // Over threshold → prune drops >120s buckets; same-IP call rolls the 90s window.
        assert_eq!(record_rate("203.0.113.77", 100), 0);
        let map = rate_map().lock().unwrap();
        assert!(!map.contains_key("198.51.100.0"));
        assert_eq!(map.get("203.0.113.77").map(|b| b.count), Some(1));
    }
}
