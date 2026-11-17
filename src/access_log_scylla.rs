//! Scylla batch writer for access logs (excluded from unit-test coverage; needs a live Session).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::access_log::{hour_bucket, AccessLogSink};
use crate::log::ProxyLogEntry;

const CHANNEL_CAP: usize = 8192;
const BATCH_SIZE: usize = 64;
const FLUSH_MS: u64 = 50;

impl AccessLogSink {
    /// Start (or replace) the Scylla batch writer.
    ///
    /// Must be called from a Tokio runtime context (`tokio::spawn`).
    pub fn start_scylla_writer(&self, session: Arc<::scylla::client::session::Session>) {
        let (tx, rx) = mpsc::channel(CHANNEL_CAP);
        self.install_sender(tx);
        let errors = self.write_errors();
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    scylla_writer_loop(session, rx, errors).await;
                });
            }
            Err(err) => {
                tracing::error!(
                    error = %err,
                    "access_log: cannot start Scylla writer outside a Tokio runtime"
                );
            }
        }
    }
}

async fn scylla_writer_loop(
    session: Arc<::scylla::client::session::Session>,
    mut rx: mpsc::Receiver<ProxyLogEntry>,
    write_errors: Arc<AtomicU64>,
) {
    let mut batch: Vec<ProxyLogEntry> = Vec::with_capacity(BATCH_SIZE);
    let mut flush = tokio::time::interval(Duration::from_millis(FLUSH_MS));
    flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            maybe = rx.recv() => {
                match maybe {
                    Some(entry) => {
                        batch.push(entry);
                        if batch.len() >= BATCH_SIZE {
                            flush_batch(&session, &mut batch, &write_errors).await;
                        }
                    }
                    None => {
                        if !batch.is_empty() {
                            flush_batch(&session, &mut batch, &write_errors).await;
                        }
                        break;
                    }
                }
            }
            _ = flush.tick() => {
                if !batch.is_empty() {
                    flush_batch(&session, &mut batch, &write_errors).await;
                }
            }
        }
    }
}

async fn flush_batch(
    session: &::scylla::client::session::Session,
    batch: &mut Vec<ProxyLogEntry>,
    write_errors: &AtomicU64,
) {
    for entry in batch.drain(..) {
        let id = uuid::Uuid::new_v4();
        let bucket = hour_bucket(entry.timestamp);
        if let Err(err) = crate::db::Database::insert_access_log_row(
            session,
            &bucket,
            entry.timestamp,
            id,
            &entry,
        )
        .await
        {
            write_errors.fetch_add(1, Ordering::Relaxed);
            tracing::debug!(error = %err, "access_log scylla insert failed");
        }
    }
}
