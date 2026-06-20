//! QUIC / HTTP/3 transport tuning from routes.yaml + env overrides.

use std::time::Duration;

use tokio_quiche::settings::QuicSettings;

use crate::http3_options::Http3Options;
use crate::runtime::{RuntimeConfig, RuntimeMode};

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|n| *n > 0)
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|n| *n > 0)
}

fn env_string(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Parallel QUIC listeners on the same address (SO_REUSEPORT when available).
pub fn listener_count(runtime_cfg: &RuntimeConfig, opts: &Http3Options) -> usize {
    opts.listeners
        .or_else(|| env_usize("PERTISK_HTTP3_LISTENERS"))
        .unwrap_or_else(|| {
            let cpus = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1);
            match runtime_cfg.resolved_mode {
                RuntimeMode::Performance => cpus,
                _ => 1,
            }
        })
}

/// Build QUIC settings from routes.yaml `http3:` block, runtime mode, and env overrides.
pub fn quic_settings(runtime_cfg: &RuntimeConfig, opts: &Http3Options) -> QuicSettings {
    let performance = runtime_cfg.resolved_mode == RuntimeMode::Performance;

    let max_streams = env_u64("PERTISK_HTTP3_MAX_STREAMS")
        .or(opts.max_streams_bidi)
        .unwrap_or(if performance { 1024 } else { 256 });

    let stream_window = env_u64("PERTISK_HTTP3_STREAM_RECEIVE_WINDOW")
        .or(opts.max_stream_data)
        .unwrap_or(if performance {
            8 * 1024 * 1024
        } else {
            1024 * 1024
        });

    let conn_window = env_u64("PERTISK_HTTP3_CONN_RECEIVE_WINDOW")
        .or(opts.max_data)
        .unwrap_or(if performance {
            64 * 1024 * 1024
        } else {
            10 * 1024 * 1024
        });

    let idle_ms = opts.max_idle_timeout_ms.or_else(|| {
        env_u64("PERTISK_HTTP3_IDLE_TIMEOUT_SECS").map(|s| s.saturating_mul(1000))
    });
    let idle_secs = idle_ms
        .map(|ms| ms / 1000)
        .unwrap_or(300);

    let cc = env_string("PERTISK_HTTP3_CC_ALGORITHM")
        .or_else(|| opts.congestion_control.clone())
        .unwrap_or_else(|| {
            if performance {
                "bbr".into()
            } else {
                "cubic".into()
            }
        });

    let enable_0rtt = opts.enable_0rtt.unwrap_or(performance);
    let enable_pacing = opts
        .enable_pacing
        .unwrap_or(performance && cc.eq_ignore_ascii_case("bbr"));

    let mut settings = QuicSettings::default();
    settings.initial_max_streams_bidi = max_streams;
    settings.initial_max_streams_uni = max_streams;
    settings.initial_max_data = conn_window;
    settings.initial_max_stream_data_bidi_local = stream_window;
    settings.initial_max_stream_data_bidi_remote = stream_window;
    settings.initial_max_stream_data_uni = stream_window;
    settings.max_stream_window = stream_window;
    settings.max_idle_timeout = Some(Duration::from_secs(idle_secs.max(1)));
    settings.cc_algorithm = cc;
    settings.enable_early_data = enable_0rtt;
    settings.enable_pacing = enable_pacing;
    if performance {
        settings.initial_congestion_window_packets = 32;
    }

    settings
}
