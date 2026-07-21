//! Simple WAF rule matching (built-in + per-site custom rules).

use serde::{Deserialize, Serialize};

use super::RequestView;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WafAction {
    #[default]
    Block,
    Log,
    Challenge,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WafRule {
    pub id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub action: WafAction,
    /// Empty = any method.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub methods: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_contains: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_contains: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ua_contains: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WafPolicy {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub enabled: bool,
    /// Enable built-in SQLi / XSS / path-traversal signatures.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub use_builtin_rules: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<WafRule>,
}

impl WafPolicy {
    pub fn is_active(&self) -> bool {
        self.enabled && (self.use_builtin_rules || self.rules.iter().any(|r| r.enabled))
    }

    pub fn is_default(&self) -> bool {
        !self.enabled && !self.use_builtin_rules && self.rules.is_empty()
    }

    pub fn normalized(mut self) -> Self {
        for rule in &mut self.rules {
            rule.id = rule.id.trim().to_string();
            rule.methods = rule
                .methods
                .iter()
                .map(|m| m.trim().to_ascii_uppercase())
                .filter(|m| !m.is_empty())
                .collect();
            if let Some(v) = rule.path_contains.as_mut() {
                *v = v.trim().to_string();
            }
            if let Some(v) = rule.query_contains.as_mut() {
                *v = v.trim().to_string();
            }
            if let Some(v) = rule.ua_contains.as_mut() {
                *v = v.trim().to_string();
            }
        }
        self.rules.retain(|r| !r.id.is_empty());
        self
    }
}

#[derive(Debug, Clone)]
pub struct WafHit {
    pub rule_id: String,
    pub action: WafAction,
}

pub fn builtin_rules() -> Vec<WafRule> {
    vec![
        WafRule {
            id: "builtin-path-traversal".into(),
            enabled: true,
            action: WafAction::Block,
            methods: vec![],
            path_contains: Some("../".into()),
            query_contains: None,
            ua_contains: None,
        },
        WafRule {
            id: "builtin-path-traversal-encoded".into(),
            enabled: true,
            action: WafAction::Block,
            methods: vec![],
            path_contains: Some("..%2f".into()),
            query_contains: None,
            ua_contains: None,
        },
        WafRule {
            id: "builtin-null-byte-path".into(),
            enabled: true,
            action: WafAction::Block,
            methods: vec![],
            path_contains: Some("%00".into()),
            query_contains: None,
            ua_contains: None,
        },
        WafRule {
            id: "builtin-null-byte-query".into(),
            enabled: true,
            action: WafAction::Block,
            methods: vec![],
            path_contains: None,
            query_contains: Some("%00".into()),
            ua_contains: None,
        },
        WafRule {
            id: "builtin-sqli-union".into(),
            enabled: true,
            action: WafAction::Block,
            methods: vec![],
            path_contains: None,
            query_contains: Some("union select".into()),
            ua_contains: None,
        },
        WafRule {
            id: "builtin-sqli-or".into(),
            enabled: true,
            action: WafAction::Block,
            methods: vec![],
            path_contains: None,
            query_contains: Some("' or ".into()),
            ua_contains: None,
        },
        WafRule {
            id: "builtin-xss-script".into(),
            enabled: true,
            action: WafAction::Block,
            methods: vec![],
            path_contains: None,
            query_contains: Some("<script".into()),
            ua_contains: None,
        },
    ]
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
    !needle.is_empty() && haystack.to_ascii_lowercase().contains(&needle.to_ascii_lowercase())
}

fn rule_matches(rule: &WafRule, req: &RequestView<'_>) -> bool {
    let method = req.method.to_ascii_uppercase();
    if !rule.methods.is_empty() && !rule.methods.iter().any(|m| m == &method) {
        return false;
    }

    let mut any_constraint = false;
    let mut matched = false;

    if let Some(needle) = rule.path_contains.as_deref().filter(|s| !s.is_empty()) {
        any_constraint = true;
        if contains_ci(req.path, needle) {
            matched = true;
        }
    }
    if let Some(needle) = rule.query_contains.as_deref().filter(|s| !s.is_empty()) {
        any_constraint = true;
        if contains_ci(req.query, needle) {
            matched = true;
        }
    }
    if let Some(needle) = rule.ua_contains.as_deref().filter(|s| !s.is_empty()) {
        any_constraint = true;
        if contains_ci(req.user_agent.unwrap_or(""), needle) {
            matched = true;
        }
    }

    any_constraint && matched
}

pub fn evaluate_waf(policy: &WafPolicy, req: &RequestView<'_>) -> Option<WafHit> {
    if !policy.is_active() {
        return None;
    }

    for rule in policy.rules.iter().filter(|r| r.enabled) {
        if rule_matches(rule, req) {
            return Some(WafHit {
                rule_id: rule.id.clone(),
                action: rule.action,
            });
        }
    }

    if policy.use_builtin_rules {
        for rule in builtin_rules() {
            if rule.enabled && rule_matches(&rule, req) {
                return Some(WafHit {
                    rule_id: rule.id,
                    action: rule.action,
                });
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::RequestView;

    #[test]
    fn detects_sqli_in_query() {
        let policy = WafPolicy {
            enabled: true,
            use_builtin_rules: true,
            rules: vec![],
        };
        let req = RequestView {
            method: "GET",
            path: "/search",
            query: "q=1' or 1=1--",
            user_agent: Some("Mozilla"),
            accept: None,
            accept_language: None,
            cookie: None,
            client_ip: None,
        };
        let hit = evaluate_waf(&policy, &req).expect("hit");
        assert_eq!(hit.rule_id, "builtin-sqli-or");
    }

    #[test]
    fn custom_ua_rule() {
        let policy = WafPolicy {
            enabled: true,
            use_builtin_rules: false,
            rules: vec![WafRule {
                id: "block-curl".into(),
                enabled: true,
                action: WafAction::Block,
                methods: vec![],
                path_contains: None,
                query_contains: None,
                ua_contains: Some("curl".into()),
            }],
        };
        let req = RequestView {
            method: "GET",
            path: "/",
            query: "",
            user_agent: Some("curl/8.0"),
            accept: None,
            accept_language: None,
            cookie: None,
            client_ip: None,
        };
        assert!(evaluate_waf(&policy, &req).is_some());
    }
}
