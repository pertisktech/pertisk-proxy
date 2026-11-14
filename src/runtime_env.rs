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
            description:
                "auto, standard, or performance. Enables CPU-scaled defaults when performance.",
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
        #[cfg(test)]
        RuntimeEnvField {
            key: "PERTISK_TEST_TEXT",
            label: "Test text field",
            description: "Test-only text field used to cover text validation.",
            kind: FieldKind::Text,
            options: &[],
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
        if let Some(opt) = field.options.iter().find(|o| o.eq_ignore_ascii_case(value)) {
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

pub fn build_settings(editable: bool, overrides: &HashMap<String, String>) -> RuntimeEnvSettings {
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
                    // Prefer "override" when a stored override exists and matches env.
                    (Some(env.clone()), "override")
                } else {
                    // Process env present (with or without a differing stored override).
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn restore_env_var(key: &str, previous: Option<String>) {
        match previous {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    fn field<'a>(settings: &'a RuntimeEnvSettings, key: &str) -> &'a RuntimeEnvFieldView {
        settings
            .fields
            .iter()
            .find(|field| field.key == key)
            .expect("field exists")
    }

    #[test]
    fn editable_fields_include_known_keys() {
        let fields = editable_fields();

        assert!(!fields.is_empty());
        assert!(fields.iter().any(|field| field.key == "PERTISK_PROXY_MODE"));
        assert!(fields
            .iter()
            .any(|field| field.key == "PERTISK_WORKER_THREADS"));
        assert!(fields
            .iter()
            .any(|field| field.key == "PERTISK_H3_UPSTREAM_POOL_MAX_IDLE"));
    }

    #[test]
    fn editable_key_check_allows_only_known_keys() {
        assert!(is_editable_key("PERTISK_PROXY_MODE"));
        assert!(!is_editable_key("PERTISK_NOT_ALLOWED"));
    }

    #[test]
    fn parse_overrides_json_filters_input() {
        let parsed = parse_overrides_json(
            r#"{
                "PERTISK_PROXY_MODE": " performance ",
                "PERTISK_WORKER_THREADS": "",
                "PERTISK_NOT_ALLOWED": "value"
            }"#,
        )
        .unwrap();

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed.get("PERTISK_PROXY_MODE").unwrap(), "performance");
    }

    #[test]
    fn parse_overrides_json_returns_empty_for_invalid_json() {
        let parsed = parse_overrides_json("not valid json").unwrap();

        assert!(parsed.is_empty());
    }

    #[test]
    fn sanitize_overrides_keeps_valid_values_and_normalizes_selects() {
        let sanitized = sanitize_overrides(HashMap::from([
            (
                "PERTISK_PROXY_MODE".to_string(),
                " PERFORMANCE ".to_string(),
            ),
            ("PERTISK_WORKER_THREADS".to_string(), "4".to_string()),
            ("PERTISK_PINGORA_THREADS".to_string(), "-1".to_string()),
            ("PERTISK_TCP_LISTEN_BACKLOG".to_string(), "abc".to_string()),
            ("PERTISK_NOT_ALLOWED".to_string(), "value".to_string()),
            ("PERTISK_HTTP3_MAX_STREAMS".to_string(), "".to_string()),
        ]));

        assert_eq!(sanitized.get("PERTISK_PROXY_MODE").unwrap(), "performance");
        assert_eq!(sanitized.get("PERTISK_WORKER_THREADS").unwrap(), "4");
        assert!(!sanitized.contains_key("PERTISK_PINGORA_THREADS"));
        assert!(!sanitized.contains_key("PERTISK_TCP_LISTEN_BACKLOG"));
        assert!(!sanitized.contains_key("PERTISK_NOT_ALLOWED"));
        assert!(!sanitized.contains_key("PERTISK_HTTP3_MAX_STREAMS"));
    }

    #[test]
    fn validate_override_covers_field_kinds() {
        assert!(validate_override("PERTISK_PROXY_MODE", "STANDARD").is_ok());
        assert!(validate_override("PERTISK_PROXY_MODE", "turbo").is_err());

        assert!(validate_override("PERTISK_WORKER_THREADS", "0").is_ok());
        assert!(validate_override("PERTISK_WORKER_THREADS", "42").is_ok());
        assert!(validate_override("PERTISK_WORKER_THREADS", "-1").is_err());
        assert!(validate_override("PERTISK_WORKER_THREADS", "many").is_err());

        assert!(validate_override("PERTISK_TEST_TEXT", "").is_ok());
        assert!(validate_override("PERTISK_TEST_TEXT", "anything").is_ok());
        assert!(validate_override("PERTISK_UNKNOWN", "value").is_err());
    }

    #[test]
    fn apply_overrides_applies_only_unset_editable_values() {
        let _guard = ENV_LOCK.lock().unwrap();
        let key = "PERTISK_WORKER_THREADS";
        let previous = std::env::var(key).ok();
        std::env::remove_var(key);

        let applied = apply_overrides(&HashMap::from([
            (key.to_string(), " 7 ".to_string()),
            ("PERTISK_NOT_ALLOWED".to_string(), "1".to_string()),
            ("PERTISK_PROXY_MODE".to_string(), "".to_string()),
        ]));
        assert_eq!(applied, 1);
        assert_eq!(std::env::var(key).unwrap(), "7");

        let applied_again = apply_overrides(&HashMap::from([(key.to_string(), "9".to_string())]));
        assert_eq!(applied_again, 0);
        assert_eq!(std::env::var(key).unwrap(), "7");

        restore_env_var(key, previous);
    }

    #[cfg(feature = "admin")]
    #[test]
    fn load_overrides_from_missing_db_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.sqlite");

        let loaded = load_overrides_from_db_path(&path).unwrap();

        assert!(loaded.is_empty());
    }

    #[cfg(feature = "admin")]
    #[test]
    fn load_overrides_from_db_reads_and_filters_runtime_env_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.sqlite");
        let conn = rusqlite::Connection::open(&path).unwrap();
        crate::db::init_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO proxy_config (key, value, updated_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                RUNTIME_ENV_KEY,
                r#"{"PERTISK_PROXY_MODE":"standard","PERTISK_NOT_ALLOWED":"x"}"#,
                "2026-01-01T00:00:00Z"
            ],
        )
        .unwrap();

        let loaded = load_overrides_from_db_path(&path).unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.get("PERTISK_PROXY_MODE").unwrap(), "standard");
    }

    #[cfg(feature = "admin")]
    #[test]
    fn load_overrides_from_db_without_runtime_env_key_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty-key.sqlite");
        let conn = rusqlite::Connection::open(&path).unwrap();
        crate::db::init_schema(&conn).unwrap();

        let loaded = load_overrides_from_db_path(&path).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn normalize_override_value_returns_input_for_unknown_key() {
        assert_eq!(
            normalize_override_value("PERTISK_NOT_A_FIELD", "raw"),
            "raw"
        );
    }

    #[test]
    fn build_settings_reports_default_env_override_pending_and_env_wins() {
        let _guard = ENV_LOCK.lock().unwrap();
        let keys = [
            "PERTISK_PROXY_MODE",
            "PERTISK_WORKER_THREADS",
            "PERTISK_PINGORA_THREADS",
            "PERTISK_TCP_LISTEN_BACKLOG",
        ];
        let previous: Vec<_> = keys
            .iter()
            .map(|key| (*key, std::env::var(key).ok()))
            .collect();
        for key in keys {
            std::env::remove_var(key);
        }

        let default_settings = build_settings(true, &HashMap::new());
        let default_field = field(&default_settings, "PERTISK_PROXY_MODE");
        assert_eq!(default_field.source, "default");
        assert_eq!(default_field.effective_value, None);
        assert_eq!(default_field.override_value, None);
        assert!(default_settings.editable);
        assert!(default_settings.restart_required);

        std::env::set_var("PERTISK_PROXY_MODE", "standard");
        let env_settings = build_settings(false, &HashMap::new());
        let env_field = field(&env_settings, "PERTISK_PROXY_MODE");
        assert_eq!(env_field.source, "env");
        assert_eq!(env_field.effective_value.as_deref(), Some("standard"));
        assert!(!env_settings.editable);

        std::env::set_var("PERTISK_WORKER_THREADS", "4");
        let override_match = build_settings(
            true,
            &HashMap::from([("PERTISK_WORKER_THREADS".to_string(), "4".to_string())]),
        );
        let override_field = field(&override_match, "PERTISK_WORKER_THREADS");
        assert_eq!(override_field.source, "override");
        assert_eq!(override_field.effective_value.as_deref(), Some("4"));
        assert_eq!(override_field.override_value.as_deref(), Some("4"));

        let pending = build_settings(
            true,
            &HashMap::from([("PERTISK_PINGORA_THREADS".to_string(), "2".to_string())]),
        );
        let pending_field = field(&pending, "PERTISK_PINGORA_THREADS");
        assert_eq!(pending_field.source, "pending");
        assert_eq!(pending_field.effective_value.as_deref(), Some("2"));
        assert_eq!(pending_field.override_value.as_deref(), Some("2"));

        std::env::set_var("PERTISK_TCP_LISTEN_BACKLOG", "4096");
        let env_wins = build_settings(
            true,
            &HashMap::from([("PERTISK_TCP_LISTEN_BACKLOG".to_string(), "2048".to_string())]),
        );
        let env_wins_field = field(&env_wins, "PERTISK_TCP_LISTEN_BACKLOG");
        assert_eq!(env_wins_field.source, "env");
        assert_eq!(env_wins_field.effective_value.as_deref(), Some("4096"));
        assert_eq!(env_wins_field.override_value.as_deref(), Some("2048"));

        let field_metadata = field(&default_settings, "PERTISK_H3_UPSTREAM_POOL_MAX_IDLE");
        assert_eq!(field_metadata.field_type, "number");
        assert!(field_metadata.options.is_empty());

        for (key, value) in previous {
            restore_env_var(key, value);
        }
    }
}
