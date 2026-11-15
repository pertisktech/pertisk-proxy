//! In-memory proxy and system log for the management UI.
//!
//! Hot-path notes: `push_sync` must stay cheap. The buffer is a ring (`VecDeque`) so
//! eviction is O(1), and the lock is `try_lock` so a busy admin reader never stalls
//! request workers (log line is dropped instead).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use serde::Serialize;

/// Minimum severity stored in the UI log buffer (`PERTISK_LOG_LEVEL`).
static MIN_UI_LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);

#[derive(Debug, Clone, Serialize)]
pub struct ProxyLogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub host: Option<String>,
    pub path: Option<String>,
    pub upstream: Option<String>,
    pub status: Option<u16>,
    pub duration_ms: Option<u64>,
    pub message: String,
    #[serde(rename = "type")]
    pub entry_type: LogEntryType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// Apply `PERTISK_LOG_LEVEL` to the in-memory UI log buffer (HTTP + system tabs).
pub fn set_min_ui_log_level(level: LogLevel) {
    MIN_UI_LOG_LEVEL.store(level as u8, Ordering::Relaxed);
}

pub fn min_ui_log_level() -> LogLevel {
    match MIN_UI_LOG_LEVEL.load(Ordering::Relaxed) {
        x if x == LogLevel::Debug as u8 => LogLevel::Debug,
        x if x == LogLevel::Info as u8 => LogLevel::Info,
        x if x == LogLevel::Warn as u8 => LogLevel::Warn,
        _ => LogLevel::Error,
    }
}

pub fn ui_log_enabled(level: LogLevel) -> bool {
    level >= min_ui_log_level()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogEntryType {
    Request,
    Response,
    HealthCheck,
    ConfigReload,
    Tracing,
    Error,
}

impl ProxyLogEntry {
    pub fn response(
        host: &str,
        path: &str,
        upstream: &str,
        status: u16,
        duration_ms: u64,
        protocol: Option<&str>,
        encoding: Option<&str>,
        method: Option<&str>,
    ) -> Self {
        Self {
            timestamp: Utc::now(),
            level: if status >= 500 {
                LogLevel::Error
            } else if status >= 400 {
                LogLevel::Warn
            } else {
                LogLevel::Info
            },
            host: Some(host.to_string()),
            path: Some(path.to_string()),
            upstream: Some(upstream.to_string()),
            status: Some(status),
            duration_ms: Some(duration_ms),
            message: String::new(),
            entry_type: LogEntryType::Response,
            protocol: protocol.map(String::from),
            encoding: encoding.map(String::from),
            method: method.map(String::from),
        }
    }

    pub fn health_check(upstream: &str, ok: bool) -> Self {
        Self {
            timestamp: Utc::now(),
            level: if ok { LogLevel::Info } else { LogLevel::Warn },
            host: None,
            path: None,
            upstream: Some(upstream.to_string()),
            status: None,
            duration_ms: None,
            message: if ok {
                "healthy".into()
            } else {
                "unhealthy".into()
            },
            entry_type: LogEntryType::HealthCheck,
            protocol: None,
            encoding: None,
            method: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            timestamp: Utc::now(),
            level: LogLevel::Error,
            host: None,
            path: None,
            upstream: None,
            status: None,
            duration_ms: None,
            message: message.into(),
            entry_type: LogEntryType::Error,
            protocol: None,
            encoding: None,
            method: None,
        }
    }

    pub fn config_reload(message: impl Into<String>) -> Self {
        Self {
            timestamp: Utc::now(),
            level: LogLevel::Info,
            host: None,
            path: None,
            upstream: None,
            status: None,
            duration_ms: None,
            message: message.into(),
            entry_type: LogEntryType::ConfigReload,
            protocol: None,
            encoding: None,
            method: None,
        }
    }

    pub fn tracing_event(level: LogLevel, source: &str, message: impl Into<String>) -> Self {
        Self {
            timestamp: Utc::now(),
            level,
            host: None,
            path: None,
            upstream: Some(source.to_string()),
            status: None,
            duration_ms: None,
            message: message.into(),
            entry_type: LogEntryType::Tracing,
            protocol: None,
            encoding: None,
            method: None,
        }
    }

    pub fn error_with_context(
        host: &str,
        path: &str,
        upstream: &str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            timestamp: Utc::now(),
            level: LogLevel::Error,
            host: Some(host.to_string()),
            path: Some(path.to_string()),
            upstream: Some(upstream.to_string()),
            status: None,
            duration_ms: None,
            message: message.into(),
            entry_type: LogEntryType::Error,
            protocol: None,
            encoding: None,
            method: None,
        }
    }

    pub fn is_system(&self) -> bool {
        matches!(
            self.entry_type,
            LogEntryType::HealthCheck | LogEntryType::ConfigReload | LogEntryType::Tracing
        ) || (self.entry_type == LogEntryType::Error && self.host.is_none())
    }

    pub fn has_domain(&self) -> bool {
        self.host.is_some()
    }

    fn is_repeatable_system_entry(&self) -> bool {
        matches!(
            self.entry_type,
            LogEntryType::HealthCheck | LogEntryType::ConfigReload | LogEntryType::Tracing
        )
    }

    fn repeatable_signature(&self) -> Option<(LogEntryType, String, String)> {
        if !self.is_repeatable_system_entry() {
            return None;
        }
        Some((
            self.entry_type,
            self.upstream.clone().unwrap_or_default(),
            self.message.clone(),
        ))
    }
}

pub fn dedupe_consecutive_system_logs(entries: Vec<ProxyLogEntry>) -> Vec<ProxyLogEntry> {
    let mut out: Vec<ProxyLogEntry> = Vec::with_capacity(entries.len());
    for entry in entries {
        let collapse = match (out.last(), entry.repeatable_signature()) {
            (Some(last), Some((et, eu, em))) => last
                .repeatable_signature()
                .is_some_and(|(lt, up, msg)| lt == et && up == eu && msg == em),
            _ => false,
        };
        if collapse {
            if let Some(last) = out.last_mut() {
                last.timestamp = entry.timestamp;
            }
            continue;
        }
        out.push(entry);
    }
    out
}

#[derive(Clone)]
pub struct ProxyLog {
    inner: Arc<Mutex<ProxyLogInner>>,
    max_entries: usize,
}

struct ProxyLogInner {
    entries: VecDeque<ProxyLogEntry>,
}

impl ProxyLog {
    pub fn new(max_entries: usize) -> Self {
        let max_entries = max_entries.max(1);
        Self {
            inner: Arc::new(Mutex::new(ProxyLogInner {
                entries: VecDeque::with_capacity(max_entries.min(4096)),
            })),
            max_entries,
        }
    }

    pub async fn push(&self, entry: ProxyLogEntry) {
        self.push_sync(entry);
    }

    /// Append one entry. Never blocks: if the mutex is busy, the line is dropped.
    pub fn push_sync(&self, entry: ProxyLogEntry) {
        if !ui_log_enabled(entry.level) {
            return;
        }
        let Ok(mut g) = self.inner.try_lock() else {
            return;
        };
        if g.entries.len() >= self.max_entries {
            g.entries.pop_front();
        }
        g.entries.push_back(entry);
    }

    pub async fn recent(&self, limit: usize) -> Vec<ProxyLogEntry> {
        let Ok(g) = self.inner.lock() else {
            return vec![];
        };
        let len = g.entries.len();
        let take = limit.min(len);
        g.entries.iter().skip(len - take).cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().map(|g| g.entries.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// `MIN_UI_LOG_LEVEL` is process-global; serialize tests that mutate it.
    static UI_LOG_LEVEL_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn ui_log_level_filters_by_severity() {
        let _guard = UI_LOG_LEVEL_TEST_LOCK.lock().unwrap();
        set_min_ui_log_level(LogLevel::Warn);
        assert!(!ui_log_enabled(LogLevel::Info));
        assert!(!ui_log_enabled(LogLevel::Debug));
        assert!(ui_log_enabled(LogLevel::Warn));
        assert!(ui_log_enabled(LogLevel::Error));
        set_min_ui_log_level(LogLevel::Info);
    }

    #[test]
    fn push_sync_respects_min_level() {
        let _guard = UI_LOG_LEVEL_TEST_LOCK.lock().unwrap();
        set_min_ui_log_level(LogLevel::Warn);
        let log = ProxyLog::new(10);
        log.push_sync(ProxyLogEntry::config_reload("reload"));
        log.push_sync(ProxyLogEntry::error("boom"));
        assert_eq!(log.len(), 1);
        set_min_ui_log_level(LogLevel::Info);
    }

    #[test]
    fn ring_buffer_evicts_oldest_in_constant_space() {
        let _guard = UI_LOG_LEVEL_TEST_LOCK.lock().unwrap();
        set_min_ui_log_level(LogLevel::Info);
        let log = ProxyLog::new(3);
        for i in 0..5 {
            log.push_sync(ProxyLogEntry::config_reload(format!("n{i}")));
        }
        assert_eq!(log.len(), 3);
        let g = log.inner.lock().unwrap();
        let msgs: Vec<&str> = g.entries.iter().map(|e| e.message.as_str()).collect();
        assert_eq!(msgs, vec!["n2", "n3", "n4"]);
    }
}
