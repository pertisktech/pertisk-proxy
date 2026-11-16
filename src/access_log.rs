//! Access-log sink: memory ring and/or Scylla warehouse (async, non-blocking).

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Timelike, Utc};
use tokio::sync::mpsc;

use crate::db::LogStorageBackend;
use crate::log::{ProxyLog, ProxyLogEntry};

const STORAGE_MEMORY: u8 = 0;
const STORAGE_SCYLLA: u8 = 1;

/// Hour bucket `YYYYMMDDHH` (UTC) for Scylla partition key.
pub fn hour_bucket(ts: DateTime<Utc>) -> String {
    ts.format("%Y%m%d%H").to_string()
}

/// Most recent `hours` hour buckets ending at `now` (inclusive of current hour).
pub fn recent_buckets(now: DateTime<Utc>, hours: i64) -> Vec<String> {
    let hours = hours.clamp(1, 168);
    let mut out = Vec::with_capacity(hours as usize);
    let base = now
        .with_minute(0)
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(now);
    for i in 0..hours {
        out.push(hour_bucket(base - chrono::Duration::hours(i)));
    }
    out
}

#[derive(Clone)]
pub struct AccessLogSink {
    inner: Arc<AccessLogInner>,
}

struct AccessLogInner {
    memory: Arc<ProxyLog>,
    enabled: AtomicBool,
    storage: AtomicU8,
    tx: std::sync::Mutex<Option<mpsc::Sender<ProxyLogEntry>>>,
    enqueued: AtomicU64,
    dropped: AtomicU64,
    write_errors: Arc<AtomicU64>,
}

impl AccessLogSink {
    pub fn new(memory: Arc<ProxyLog>) -> Self {
        Self {
            inner: Arc::new(AccessLogInner {
                memory,
                enabled: AtomicBool::new(false),
                storage: AtomicU8::new(STORAGE_MEMORY),
                tx: std::sync::Mutex::new(None),
                enqueued: AtomicU64::new(0),
                dropped: AtomicU64::new(0),
                write_errors: Arc::new(AtomicU64::new(0)),
            }),
        }
    }

    pub fn memory(&self) -> Arc<ProxyLog> {
        Arc::clone(&self.inner.memory)
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.inner.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn enabled(&self) -> bool {
        self.inner.enabled.load(Ordering::Relaxed)
    }

    pub fn set_storage(&self, storage: LogStorageBackend) {
        let v = match storage {
            LogStorageBackend::Memory => STORAGE_MEMORY,
            LogStorageBackend::Scylla => STORAGE_SCYLLA,
        };
        self.inner.storage.store(v, Ordering::Relaxed);
    }

    pub fn storage(&self) -> LogStorageBackend {
        if self.inner.storage.load(Ordering::Relaxed) == STORAGE_SCYLLA {
            LogStorageBackend::Scylla
        } else {
            LogStorageBackend::Memory
        }
    }

    pub fn stats(&self) -> (u64, u64, u64) {
        (
            self.inner.enqueued.load(Ordering::Relaxed),
            self.inner.dropped.load(Ordering::Relaxed),
            self.inner.write_errors.load(Ordering::Relaxed),
        )
    }

    pub(crate) fn install_sender(&self, tx: mpsc::Sender<ProxyLogEntry>) {
        if let Ok(mut slot) = self.inner.tx.lock() {
            *slot = Some(tx);
        }
    }

    pub(crate) fn write_errors(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.inner.write_errors)
    }

    #[cfg(test)]
    fn install_test_channel(&self, capacity: usize) -> mpsc::Receiver<ProxyLogEntry> {
        let (tx, rx) = mpsc::channel(capacity);
        self.install_sender(tx);
        rx
    }

    /// Always push system/tracing-style entries to the memory ring (for Logs UI system tab).
    pub fn emit_system(&self, entry: ProxyLogEntry) {
        self.inner.memory.push_sync(entry);
    }

    /// Hot-path emit for access logs (request/response/errors with host).
    pub fn emit(&self, entry: ProxyLogEntry) {
        if !self.inner.enabled.load(Ordering::Relaxed) {
            return;
        }
        match self.storage() {
            LogStorageBackend::Memory => {
                self.inner.memory.push_sync(entry);
            }
            LogStorageBackend::Scylla => {
                // Keep a short memory mirror for live WS while also shipping to Scylla.
                self.inner.memory.push_sync(entry.clone());
                let tx = {
                    let g = self.inner.tx.lock().unwrap_or_else(|e| e.into_inner());
                    g.clone()
                };
                let Some(tx) = tx else {
                    self.inner.dropped.fetch_add(1, Ordering::Relaxed);
                    return;
                };
                match tx.try_send(entry) {
                    Ok(()) => {
                        self.inner.enqueued.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        self.inner.dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
    }

    pub async fn emit_async(&self, entry: ProxyLogEntry) {
        self.emit(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::ProxyLogEntry;

    #[test]
    fn hour_bucket_format() {
        let ts = DateTime::parse_from_rfc3339("2026-11-16T04:27:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(hour_bucket(ts), "2026111604");
    }

    #[test]
    fn recent_buckets_count() {
        let now = Utc::now();
        let b = recent_buckets(now, 3);
        assert_eq!(b.len(), 3);
        assert_eq!(b[0], hour_bucket(now));
    }

    #[test]
    fn emit_memory_stores() {
        let mem = Arc::new(ProxyLog::new(100));
        let sink = AccessLogSink::new(Arc::clone(&mem));
        sink.set_enabled(true);
        sink.set_storage(LogStorageBackend::Memory);
        sink.emit(sample_entry());
        assert_eq!(mem.len(), 1);
    }

    #[test]
    fn emit_disabled_is_noop() {
        let mem = Arc::new(ProxyLog::new(100));
        let sink = AccessLogSink::new(Arc::clone(&mem));
        sink.set_enabled(false);
        sink.emit(sample_entry());
        assert_eq!(mem.len(), 0);
    }

    #[test]
    fn scylla_without_writer_drops() {
        let mem = Arc::new(ProxyLog::new(100));
        let sink = AccessLogSink::new(Arc::clone(&mem));
        sink.set_enabled(true);
        sink.set_storage(LogStorageBackend::Scylla);
        sink.emit(sample_entry());
        let (_e, dropped, _) = sink.stats();
        assert!(dropped >= 1);
        // still mirrored to memory
        assert_eq!(mem.len(), 1);
    }

    #[test]
    fn recent_buckets_are_descending_hours() {
        let now = DateTime::parse_from_rfc3339("2026-11-16T04:27:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let b = recent_buckets(now, 3);
        assert_eq!(b, vec!["2026111604", "2026111603", "2026111602"]);
    }

    fn sample_entry() -> ProxyLogEntry {
        ProxyLogEntry::response(
            "a.example",
            "/",
            "127.0.0.1:80",
            200,
            1,
            Some("h2"),
            None,
            Some("GET"),
        )
    }

    #[test]
    fn memory_and_enabled_accessors() {
        let mem = Arc::new(ProxyLog::new(100));
        let sink = AccessLogSink::new(Arc::clone(&mem));
        assert_eq!(sink.memory().len(), 0);
        assert!(!sink.enabled());
        sink.set_enabled(true);
        assert!(sink.enabled());
        assert_eq!(sink.storage(), LogStorageBackend::Memory);
        sink.set_storage(LogStorageBackend::Scylla);
        assert_eq!(sink.storage(), LogStorageBackend::Scylla);
        assert_eq!(sink.stats(), (0, 0, 0));
    }

    #[test]
    fn emit_system_always_writes_memory() {
        let mem = Arc::new(ProxyLog::new(100));
        let sink = AccessLogSink::new(Arc::clone(&mem));
        sink.set_enabled(false);
        sink.emit_system(ProxyLogEntry::config_reload("reload"));
        assert_eq!(mem.len(), 1);
    }

    #[tokio::test]
    async fn emit_async_memory_path() {
        let mem = Arc::new(ProxyLog::new(100));
        let sink = AccessLogSink::new(Arc::clone(&mem));
        sink.set_enabled(true);
        sink.emit_async(sample_entry()).await;
        assert_eq!(mem.len(), 1);
    }

    #[test]
    fn scylla_enqueue_increments_enqueued() {
        let mem = Arc::new(ProxyLog::new(100));
        let sink = AccessLogSink::new(Arc::clone(&mem));
        sink.set_enabled(true);
        sink.set_storage(LogStorageBackend::Scylla);
        let mut rx = sink.install_test_channel(8);
        sink.emit(sample_entry());
        let (enqueued, dropped, _) = sink.stats();
        assert_eq!(enqueued, 1);
        assert_eq!(dropped, 0);
        assert!(rx.try_recv().is_ok());
        assert_eq!(mem.len(), 1);
    }

    #[test]
    fn scylla_channel_full_increments_dropped() {
        let mem = Arc::new(ProxyLog::new(100));
        let sink = AccessLogSink::new(Arc::clone(&mem));
        sink.set_enabled(true);
        sink.set_storage(LogStorageBackend::Scylla);
        let _rx = sink.install_test_channel(1);
        sink.emit(sample_entry());
        sink.emit(sample_entry());
        let (enqueued, dropped, _) = sink.stats();
        assert_eq!(enqueued, 1);
        assert_eq!(dropped, 1);
        assert_eq!(mem.len(), 2);
    }

    #[test]
    fn install_sender_replaces_channel() {
        let mem = Arc::new(ProxyLog::new(100));
        let sink = AccessLogSink::new(Arc::clone(&mem));
        let (tx, _rx) = mpsc::channel(1);
        sink.install_sender(tx);
        assert_eq!(sink.write_errors().load(Ordering::Relaxed), 0);
    }
}
