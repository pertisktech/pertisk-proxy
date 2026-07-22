//! Edge security: WAF rules, bot scoring, and math captcha challenges.
//!
//! Pipeline (after GeoIP): captcha endpoints → WAF → bot score → challenge/block.

mod bot;
mod captcha;
mod waf;

pub use bot::{score_bot, BotPolicy, BotVerdict};
pub use captcha::{
    challenge_page, has_valid_pass_cookie, is_captcha_path, verify_and_pass_cookie, CaptchaPolicy,
    CAPTCHA_PATH, CAPTCHA_VERIFY_PATH,
};
pub use waf::{builtin_rules, evaluate_waf, WafAction, WafPolicy, WafRule};

use bot::{score_bot as score_bot_fn, BotScore};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityPolicy {
    #[serde(default, skip_serializing_if = "WafPolicy::is_default")]
    pub waf: WafPolicy,
    #[serde(default, skip_serializing_if = "BotPolicy::is_default")]
    pub bot: BotPolicy,
    #[serde(default, skip_serializing_if = "CaptchaPolicy::is_default")]
    pub captcha: CaptchaPolicy,
}

impl SecurityPolicy {
    pub fn is_active(&self) -> bool {
        self.waf.is_active() || self.bot.is_active() || self.captcha.enabled
    }

    pub fn is_default(&self) -> bool {
        self.waf.is_default() && self.bot.is_default() && self.captcha.is_default()
    }

    pub fn normalized(mut self) -> Self {
        self.waf = self.waf.normalized();
        self.bot = self.bot.normalized();
        self.captcha = self.captcha.normalized();
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityAction {
    Allow,
    Log,
    Challenge,
    Block,
}

#[derive(Debug, Clone)]
pub struct SecurityDecision {
    pub action: SecurityAction,
    pub reason: &'static str,
    pub detail: String,
    pub bot_score: Option<u32>,
    pub waf_rule: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RequestView<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub query: &'a str,
    pub user_agent: Option<&'a str>,
    pub accept: Option<&'a str>,
    pub accept_language: Option<&'a str>,
    pub cookie: Option<&'a str>,
    pub client_ip: Option<&'a str>,
}

fn resolve_challenge(policy: &SecurityPolicy, req: &RequestView<'_>) -> SecurityAction {
    if policy.captcha.enabled && has_valid_pass_cookie(req.cookie, req.client_ip) {
        SecurityAction::Allow
    } else if policy.captcha.enabled {
        SecurityAction::Challenge
    } else {
        SecurityAction::Block
    }
}

fn from_bot(
    policy: &SecurityPolicy,
    req: &RequestView<'_>,
    score: BotScore,
    waf_rule: Option<String>,
    prefer_log: bool,
) -> SecurityDecision {
    let action = match score.verdict {
        BotVerdict::Allow => {
            if prefer_log {
                SecurityAction::Log
            } else {
                SecurityAction::Allow
            }
        }
        BotVerdict::Challenge => resolve_challenge(policy, req),
        BotVerdict::Block => SecurityAction::Block,
    };
    SecurityDecision {
        action,
        reason: match score.verdict {
            BotVerdict::Allow if prefer_log => "waf-log",
            BotVerdict::Allow => "bot-allow",
            BotVerdict::Challenge => "bot-challenge",
            BotVerdict::Block => "bot-block",
        },
        detail: if prefer_log {
            waf_rule
                .clone()
                .unwrap_or_else(|| score.reasons.join(","))
        } else {
            score.reasons.join(",")
        },
        bot_score: Some(score.score),
        waf_rule,
    }
}

/// Evaluate WAF then bot. Captcha pass cookie short-circuits challenge → allow.
pub fn evaluate(policy: &SecurityPolicy, req: &RequestView<'_>) -> SecurityDecision {
    if !policy.is_active() {
        return SecurityDecision {
            action: SecurityAction::Allow,
            reason: "off",
            detail: String::new(),
            bot_score: None,
            waf_rule: None,
        };
    }

    if let Some(hit) = evaluate_waf(&policy.waf, req) {
        match hit.action {
            WafAction::Log => {
                if policy.bot.is_active() {
                    let score = score_bot_fn(&policy.bot, req);
                    return from_bot(policy, req, score, Some(hit.rule_id), true);
                }
                return SecurityDecision {
                    action: SecurityAction::Log,
                    reason: "waf-log",
                    detail: hit.rule_id.clone(),
                    bot_score: None,
                    waf_rule: Some(hit.rule_id),
                };
            }
            WafAction::Challenge => {
                return SecurityDecision {
                    action: resolve_challenge(policy, req),
                    reason: "waf-challenge",
                    detail: hit.rule_id.clone(),
                    bot_score: None,
                    waf_rule: Some(hit.rule_id),
                };
            }
            WafAction::Block => {
                return SecurityDecision {
                    action: SecurityAction::Block,
                    reason: "waf-block",
                    detail: hit.rule_id.clone(),
                    bot_score: None,
                    waf_rule: Some(hit.rule_id),
                };
            }
        }
    }

    if policy.bot.is_active() {
        let score = score_bot_fn(&policy.bot, req);
        return from_bot(policy, req, score, None, false);
    }

    SecurityDecision {
        action: SecurityAction::Allow,
        reason: "allow",
        detail: String::new(),
        bot_score: None,
        waf_rule: None,
    }
}

/// Annotation keys written for WAF / bot / captcha on Ingress / HTTPRoute.
pub const ANNOTATION_KEYS: &[&str] = &[
    "proxy.pertisk.tech/waf-enabled",
    "proxy.pertisk.tech/waf-builtin",
    "proxy.pertisk.tech/bot-enabled",
    "proxy.pertisk.tech/bot-challenge-score",
    "proxy.pertisk.tech/bot-block-score",
    "proxy.pertisk.tech/bot-rate-limit",
    "proxy.pertisk.tech/captcha-enabled",
    "proxy.pertisk.tech/captcha-ttl-secs",
];

/// Parse security policy from Ingress / HTTPRoute annotations.
pub fn policy_from_annotations(
    annotations: Option<&std::collections::BTreeMap<String, String>>,
) -> SecurityPolicy {
    let Some(map) = annotations else {
        return SecurityPolicy::default();
    };
    let flag = |key: &str| {
        map.get(key)
            .map(|v| {
                let v = v.trim().to_ascii_lowercase();
                matches!(v.as_str(), "1" | "true" | "yes" | "on")
            })
            .unwrap_or(false)
    };
    if flag("proxy.pertisk.tech/security-exempt") {
        return SecurityPolicy::default();
    }
    let parse_u32 = |key: &str, default: u32| {
        map.get(key)
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(default)
    };

    let waf_enabled = flag("proxy.pertisk.tech/waf-enabled");
    let bot_enabled = flag("proxy.pertisk.tech/bot-enabled");
    let captcha_enabled = flag("proxy.pertisk.tech/captcha-enabled");

    SecurityPolicy {
        waf: WafPolicy {
            enabled: waf_enabled,
            use_builtin_rules: waf_enabled || flag("proxy.pertisk.tech/waf-builtin"),
            rules: Vec::new(),
        },
        bot: BotPolicy {
            enabled: bot_enabled,
            challenge_score: parse_u32("proxy.pertisk.tech/bot-challenge-score", 40),
            block_score: parse_u32("proxy.pertisk.tech/bot-block-score", 80),
            rate_limit_per_min: parse_u32("proxy.pertisk.tech/bot-rate-limit", 120),
        },
        captcha: CaptchaPolicy {
            enabled: captcha_enabled,
            cookie_ttl_secs: u64::from(parse_u32("proxy.pertisk.tech/captcha-ttl-secs", 86_400)),
        },
    }
    .normalized()
}

/// Replace WAF / bot / captcha annotation keys on a metadata map from a policy.
pub fn apply_annotations(
    annotations: &mut std::collections::BTreeMap<String, String>,
    policy: &SecurityPolicy,
) {
    for key in ANNOTATION_KEYS {
        annotations.remove(*key);
    }
    let policy = policy.clone().normalized();
    if policy.is_default() {
        return;
    }
    if policy.waf.enabled {
        annotations.insert(
            "proxy.pertisk.tech/waf-enabled".to_string(),
            "true".to_string(),
        );
        annotations.insert(
            "proxy.pertisk.tech/waf-builtin".to_string(),
            if policy.waf.use_builtin_rules {
                "true".to_string()
            } else {
                "false".to_string()
            },
        );
    }
    if policy.bot.enabled {
        annotations.insert(
            "proxy.pertisk.tech/bot-enabled".to_string(),
            "true".to_string(),
        );
        annotations.insert(
            "proxy.pertisk.tech/bot-challenge-score".to_string(),
            policy.bot.challenge_score.to_string(),
        );
        annotations.insert(
            "proxy.pertisk.tech/bot-block-score".to_string(),
            policy.bot.block_score.to_string(),
        );
        annotations.insert(
            "proxy.pertisk.tech/bot-rate-limit".to_string(),
            policy.bot.rate_limit_per_min.to_string(),
        );
    }
    if policy.captcha.enabled {
        annotations.insert(
            "proxy.pertisk.tech/captcha-enabled".to_string(),
            "true".to_string(),
        );
        annotations.insert(
            "proxy.pertisk.tech/captcha-ttl-secs".to_string(),
            policy.captcha.cookie_ttl_secs.to_string(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view<'a>(
        method: &'a str,
        path: &'a str,
        query: &'a str,
        ua: Option<&'a str>,
    ) -> RequestView<'a> {
        RequestView {
            method,
            path,
            query,
            user_agent: ua,
            accept: Some("text/html"),
            accept_language: Some("en"),
            cookie: None,
            client_ip: Some("1.2.3.4"),
        }
    }

    #[test]
    fn builtin_waf_blocks_traversal() {
        let policy = SecurityPolicy {
            waf: WafPolicy {
                enabled: true,
                use_builtin_rules: true,
                rules: vec![],
            },
            ..Default::default()
        };
        let d = evaluate(
            &policy,
            &view("GET", "/static/../../../etc/passwd", "", Some("Mozilla/5.0")),
        );
        assert_eq!(d.action, SecurityAction::Block);
        assert_eq!(d.reason, "waf-block");
    }

    #[test]
    fn bot_challenges_empty_ua_when_captcha_on() {
        let policy = SecurityPolicy {
            bot: BotPolicy {
                enabled: true,
                challenge_score: 30,
                block_score: 90,
                rate_limit_per_min: 10_000,
            },
            captcha: CaptchaPolicy {
                enabled: true,
                cookie_ttl_secs: 3600,
            },
            ..Default::default()
        };
        let d = evaluate(&policy, &view("GET", "/", "", None));
        assert_eq!(d.action, SecurityAction::Challenge);
    }
}
