//! DB-backed runtime environment overrides (admin UI).
//!
//! Process environment always wins over stored overrides. Overrides are applied
//! at process start (before the Tokio runtime is built) and require a restart
//! to take effect after saving from the admin UI.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// `proxy_config` table key for the JSON overrides map.
pub const RUNTIME_ENV_KEY: &str = "runtime_env";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Number,
    Select,
}

#[derive(Debug, Clone)]
pub struct RuntimeEnvField {
    pub key: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub kind: FieldKind,
    pub options: &'static [&'static str],
}

/// Keys the admin UI may persist. Keep in sync with Environment panel docs.
pub fn editable_fields() -> &'static [RuntimeEnvField] {
    &[
        RuntimeEnvField {
            key: "PERTISK_PROXY_MODE",
            label: "Proxy mode",
            description: "auto, standard, or performance. Enables CPU-scaled defaults when performance.",
            kind: FieldKind::Select,
            options: &["auto", "standard", "performance"],
        },
        RuntimeEnvField {
            key: "PERTISK_WORKER_THREADS",
            label: "Tokio worker threads",
            description: "Override Tokio worker count; leave empty to use available CPUs.",
            kind: FieldKind::Number,
            options: &[],
        },
        RuntimeEnvField {
            key: "PERTISK_PINGORA_THREADS",
            label: "Pingora service threads",
            description: "HTTP/1 and HTTP/2 Pingora service threads.",
            kind: FieldKind::Number,
            options: &[],
        },
        RuntimeEnvField {
            key: "PERTISK_PINGORA_LISTENER_TASKS",
            label: "Listener accept tasks",
            description: "Parallel accept tasks per listener; performance default is 4.",
            kind: FieldKind::Number,
            options: &[],
        },
        RuntimeEnvField {
            key: "PERTISK_TCP_LISTEN_BACKLOG",
            label: "TCP listen backlog",
            description: "Application listen backlog; performance default is 8192.",
            kind: FieldKind::Number,
            options: &[],
        },
        RuntimeEnvField {
            key: "PERTISK_TCP_KEEPALIVE",
            label: "TCP keepalive",
            description: "Client-facing TCP keepalive (1/0 or on/off). Default on.",
            kind: FieldKind::Select,
            options: &["1", "0"],
        },
        RuntimeEnvField {
            key: "PERTISK_TCP_KEEPALIVE_IDLE_SECS",
            label: "TCP keepalive idle (secs)",
            description: "Idle time before keepalive probes; default 60.",
            kind: FieldKind::Number,
            options: &[],
        },
        RuntimeEnvField {
            key: "PERTISK_TCP_KEEPALIVE_INTERVAL_SECS",
            label: "TCP keepalive interval (secs)",
            description: "Seconds between keepalive probes; default 10.",
            kind: FieldKind::Number,
            options: &[],
        },
        RuntimeEnvField {
            key: "PERTISK_TCP_KEEPALIVE_COUNT",
            label: "TCP keepalive count",
            description: "Failed probes before closing; default 5.",
            kind: FieldKind::Number,
            options: &[],
        },
        RuntimeEnvField {
            key: "PERTISK_H3_UPSTREAM_POOL_MAX_IDLE",
            label: "H3 upstream pool max idle",
            description: "Idle H3-to-upstream connections per host; performance default is 256.",
            kind: FieldKind::Number,
            options: &[],
        },
        RuntimeEnvField {
            key: "PERTISK_H3_UPSTREAM_POOL_IDLE_TIMEOUT_SECS",
            label: "H3 pool idle timeout (secs)",
            description: "How long idle upstream connections remain pooled.",
            kind: FieldKind::Number,
            options: &[],
        },
        RuntimeEnvField {
            key: "PERTISK_H3_UPSTREAM_TCP_KEEPALIVE_SECS",
            label: "H3 upstream TCP keepalive (secs)",
            description: "TCP keepalive interval for H3-to-upstream connections.",
            kind: FieldKind::Number,
            options: &[],
        },
        RuntimeEnvField {
            key: "PERTISK_HTTP3_MAX_STREAMS",
            label: "HTTP/3 max streams",
            description: "Maximum concurrent QUIC streams.",
            kind: FieldKind::Number,
            options: &[],
        },
        RuntimeEnvField {
            key: "PERTISK_HTTP3_STREAM_RECEIVE_WINDOW",
            label: "HTTP/3 stream receive window",
            description: "QUIC per-stream receive window in bytes.",
            kind: FieldKind::Number,
            options: &[],
        },
        RuntimeEnvField {
            key: "PERTISK_HTTP3_CONN_RECEIVE_WINDOW",
            label: "HTTP/3 connection receive window",
            description: "QUIC connection receive window in bytes.",
            kind: FieldKind::Number,
            options: &[],
        },
        RuntimeEnvField {
            key: "PERTISK_HTTP3_CC_ALGORITHM",
            label: "HTTP/3 congestion control",
            description: "QUIC congestion control (tokio-quiche): bbr, cubic, or reno.",
            kind: FieldKind::Select,
            options: &["bbr", "cubic", "reno"],
        },
    ]
}

pub fn is_editable_key(key: &str) -> bool {
    editable_fields().iter().any(|f| f.key == key)
}

/// Apply allowlisted overrides into the process environment.
/// Existing process env values are never overwritten (ops / Helm win).
pub fn apply_overrides(overrides: &HashMap<String, String>) -> usize {
    let mut applied = 0usize;
    for (key, value) in overrides {
        if !is_editable_key(key) {
            continue;
        }
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if std::env::var_os(key).is_some() {
            continue;
        }
        // Called from main before the Tokio runtime / worker threads start.
        std::env::set_var(key, value);
        applied += 1;
    }
    applied
}

/// Load overrides from SQLite if the DB file already exists.
/// Returns an empty map when the DB is missing or the key is unset.
#[cfg(feature = "admin")]
pub fn load_overrides_from_db_path(path: &Path) -> anyhow::Result<HashMap<String, String>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let conn = rusqlite::Connection::open(path)?;
    crate::db::init_schema(&conn)?;
    let mut stmt = conn.prepare_cached("SELECT value FROM proxy_config WHERE key = ?1")?;
    let mut rows = stmt.query(rusqlite::params![RUNTIME_ENV_KEY])?;
    let Some(row) = rows.next()? else {
        return Ok(HashMap::new());
    };
    let json: String = row.get(0)?;
    parse_overrides_json(&json)
}

pub fn parse_overrides_json(json: &str) -> anyhow::Result<HashMap<String, String>> {
    let raw: HashMap<String, String> = serde_json::from_str(json).unwrap_or_default();
    let mut out = HashMap::new();
    for (k, v) in raw {
        if !is_editable_key(&k) {
            continue;
        }
        let v = v.trim().to_string();
        if v.is_empty() {
            continue;
        }
        out.insert(k, v);
    }
    Ok(out)
}

pub fn sanitize_overrides(input: HashMap<String, String>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for (k, v) in input {
        if !is_editable_key(&k) {
            continue;
        }
        let v = v.trim().to_string();
        if v.is_empty() {
            continue;
        }
        if let Err(msg) = validate_override(&k, &v) {
            tracing::warn!(key = %k, error = %msg, "ignoring invalid runtime env override");
            continue;
        }
        let normalized = normalize_override_value(&k, &v);
        out.insert(k, normalized);
    }
    out
}

fn normalize_override_value(key: &str, value: &str) -> String {
    let Some(field) = editable_fields().iter().find(|f| f.key == key) else {
        return value.to_string();
    };
    if field.kind == FieldKind::Select {
        if let Some(opt) = field
            .options
            .iter()
            .find(|o| o.eq_ignore_ascii_case(value))
        {
            return (*opt).to_string();
        }
    }
    value.to_string()
}

pub fn validate_override(key: &str, value: &str) -> Result<(), String> {
    let field = editable_fields()
        .iter()
        .find(|f| f.key == key)
        .ok_or_else(|| format!("unknown key: {key}"))?;
    match field.kind {
        FieldKind::Select => {
            if field.options.iter().any(|o| o.eq_ignore_ascii_case(value)) {
                Ok(())
            } else {
                Err(format!(
                    "{key} must be one of: {}",
                    field.options.join(", ")
                ))
            }
        }
        FieldKind::Number => {
            if value.parse::<u64>().is_ok() {
                Ok(())
            } else {
                Err(format!("{key} must be a non-negative integer"))
            }
        }
        FieldKind::Text => Ok(()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeEnvFieldView {
    pub key: String,
    pub label: String,
    pub description: String,
    pub field_type: String,
    pub options: Vec<String>,
    pub override_value: Option<String>,
    pub effective_value: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeEnvSettings {
    pub editable: bool,
    pub restart_required: bool,
    pub overrides: HashMap<String, String>,
    pub fields: Vec<RuntimeEnvFieldView>,
}

pub fn build_settings(
    editable: bool,
    overrides: &HashMap<String, String>,
) -> RuntimeEnvSettings {
    let fields = editable_fields()
        .iter()
        .map(|f| {
            let override_value = overrides.get(f.key).cloned();
            let env_value = std::env::var(f.key).ok().filter(|s| !s.trim().is_empty());
            // After apply_overrides, env already includes DB values that were unset.
            // Distinguish: if process had env before override... we can't know after apply.
            // Use: if override matches current env and we'd have applied it, source=override
            // when override is set AND (env equals override). If env set but no override, source=env.
            // If neither, source=default (null effective shown as unset).
            let (effective_value, source) = if let Some(ref env) = env_value {
                if override_value.as_ref() == Some(env) {
                    // Could be override applied into env, or ops set same value.
                    // Prefer "override" when a stored override exists and matches.
                    if override_value.is_some() {
                        (Some(env.clone()), "override")
                    } else {
                        (Some(env.clone()), "env")
                    }
                } else if override_value.is_some() {
                    // Process env differs from stored override — ops/Helm wins.
                    (Some(env.clone()), "env")
                } else {
                    (Some(env.clone()), "env")
                }
            } else if let Some(ref ov) = override_value {
                // Stored but not yet applied (pending restart).
                (Some(ov.clone()), "pending")
            } else {
                (None, "default")
            };
            RuntimeEnvFieldView {
                key: f.key.to_string(),
                label: f.label.to_string(),
                description: f.description.to_string(),
                field_type: match f.kind {
                    FieldKind::Text => "text",
                    FieldKind::Number => "number",
                    FieldKind::Select => "select",
                }
                .to_string(),
                options: f.options.iter().map(|s| (*s).to_string()).collect(),
                override_value,
                effective_value,
                source: source.to_string(),
            }
        })
        .collect();

    RuntimeEnvSettings {
        editable,
        restart_required: true,
        overrides: overrides.clone(),
        fields,
    }
}
