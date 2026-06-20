//! HTTP/3 / QUIC options from `routes.yaml` (prox-style `http3_config`).

use serde::{Deserialize, Serialize};

/// QUIC transport options loaded from the routes file `http3:` section.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Http3Options {
    /// Max connection flow-control window (bytes). Maps to `initial_max_data`.
    pub max_data: Option<u64>,
    /// Per-stream flow-control window (bytes).
    pub max_stream_data: Option<u64>,
    /// Max concurrent bidirectional streams per connection.
    pub max_streams_bidi: Option<u64>,
    /// Connection idle timeout in milliseconds.
    pub max_idle_timeout_ms: Option<u64>,
    /// Congestion control: `cubic`, `reno`, or `bbr`.
    pub congestion_control: Option<String>,
    /// Enable TLS 0-RTT / QUIC early data for returning clients.
    pub enable_0rtt: Option<bool>,
    /// SO_REUSEPORT listener count (defaults to CPU count in performance mode).
    pub listeners: Option<usize>,
    /// Enable QUIC pacing (recommended with BBR).
    pub enable_pacing: Option<bool>,
}
