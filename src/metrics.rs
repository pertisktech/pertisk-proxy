//! Prometheus metrics: proxy request counters and a scrape endpoint on `PERTISK_METRICS_ADDR`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tracing::info;

#[derive(Default)]
struct SiteProtocolCounters {
    h2_requests_total: AtomicU64,
    h3_requests_total: AtomicU64,
}

/// Metrics counters and gauges for the proxy.
#[derive(Clone)]
pub struct ProxyMetrics {
    pub http_requests_total: Arc<AtomicU64>,
    pub https_requests_total: Arc<AtomicU64>,
    pub h3_requests_total: Arc<AtomicU64>,
    pub h2_requests_total: Arc<AtomicU64>,
    pub upstream_errors_total: Arc<AtomicU64>,
    pub grpc_requests_total: Arc<AtomicU64>,
    pub active_connections: Arc<AtomicU64>,
    pub bytes_sent_total: Arc<AtomicU64>,
    pub bytes_received_total: Arc<AtomicU64>,
    site_protocol_requests: Arc<Mutex<HashMap<String, SiteProtocolCounters>>>,
}

impl ProxyMetrics {
    pub fn new() -> Self {
        Self {
            http_requests_total: Arc::new(AtomicU64::new(0)),
            https_requests_total: Arc::new(AtomicU64::new(0)),
            h3_requests_total: Arc::new(AtomicU64::new(0)),
            h2_requests_total: Arc::new(AtomicU64::new(0)),
            upstream_errors_total: Arc::new(AtomicU64::new(0)),
            grpc_requests_total: Arc::new(AtomicU64::new(0)),
            active_connections: Arc::new(AtomicU64::new(0)),
            bytes_sent_total: Arc::new(AtomicU64::new(0)),
            bytes_received_total: Arc::new(AtomicU64::new(0)),
            site_protocol_requests: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn inc_http_requests(&self) {
        self.http_requests_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_https_requests(&self) {
        self.https_requests_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_h3_requests(&self) {
        self.h3_requests_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_h2_requests(&self) {
        self.h2_requests_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_site_protocol_requests(&self, host: &str, version: http::Version) {
        let host = host.trim().to_lowercase();
        if host.is_empty() {
            return;
        }
        let mut map = self.site_protocol_requests.lock().unwrap_or_else(|e| e.into_inner());
        let entry = map.entry(host).or_default();
        match version {
            http::Version::HTTP_2 => {
                entry.h2_requests_total.fetch_add(1, Ordering::Relaxed);
            }
            http::Version::HTTP_3 => {
                entry.h3_requests_total.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    pub fn site_protocol_snapshot(&self) -> HashMap<String, (u64, u64)> {
        self.site_protocol_requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(host, counters)| {
                (
                    host.clone(),
                    (
                        counters.h2_requests_total.load(Ordering::Relaxed),
                        counters.h3_requests_total.load(Ordering::Relaxed),
                    ),
                )
            })
            .collect()
    }

    pub fn inc_upstream_errors(&self) {
        self.upstream_errors_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_grpc_requests(&self) {
        self.grpc_requests_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_active_connections(&self) {
        self.active_connections.fetch_add(1, Ordering::Relaxed);
    }

    pub fn dec_active_connections(&self) {
        self.active_connections.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn add_bytes_sent(&self, bytes: u64) {
        if bytes > 0 {
            self.bytes_sent_total.fetch_add(bytes, Ordering::Relaxed);
        }
    }

    pub fn add_bytes_received(&self, bytes: u64) {
        if bytes > 0 {
            self.bytes_received_total.fetch_add(bytes, Ordering::Relaxed);
        }
    }

    pub fn get_metrics_text(&self) -> String {
        let mut output = String::new();

        output.push_str("# HELP pertisk_http_requests_total Total number of HTTP requests processed\n");
        output.push_str("# TYPE pertisk_http_requests_total counter\n");
        output.push_str(&format!(
            "pertisk_http_requests_total {}\n",
            self.http_requests_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP pertisk_https_requests_total Total number of HTTPS requests processed\n");
        output.push_str("# TYPE pertisk_https_requests_total counter\n");
        output.push_str(&format!(
            "pertisk_https_requests_total {}\n",
            self.https_requests_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP pertisk_h3_requests_total Total number of HTTP/3 (QUIC) requests processed\n");
        output.push_str("# TYPE pertisk_h3_requests_total counter\n");
        output.push_str(&format!(
            "pertisk_h3_requests_total {}\n",
            self.h3_requests_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP pertisk_h2_requests_total Total number of HTTP/2 requests processed\n");
        output.push_str("# TYPE pertisk_h2_requests_total counter\n");
        output.push_str(&format!(
            "pertisk_h2_requests_total {}\n",
            self.h2_requests_total.load(Ordering::Relaxed)
        ));

        let h3_total = self.h3_requests_total.load(Ordering::Relaxed) as f64;
        let h2_total = self.h2_requests_total.load(Ordering::Relaxed) as f64;
        let h3_vs_h2_ratio = if h2_total > 0.0 {
            h3_total / h2_total
        } else if h3_total > 0.0 {
            f64::INFINITY
        } else {
            0.0
        };
        output.push_str("# HELP pertisk_h3_vs_h2_ratio Ratio of HTTP/3 requests to HTTP/2 requests\n");
        output.push_str("# TYPE pertisk_h3_vs_h2_ratio gauge\n");
        output.push_str(&format!(
            "pertisk_h3_vs_h2_ratio {}\n",
            prometheus_float(h3_vs_h2_ratio)
        ));

        output.push_str("# HELP pertisk_site_h2_requests_total Total number of HTTP/2 requests per host\n");
        output.push_str("# TYPE pertisk_site_h2_requests_total counter\n");
        output.push_str("# HELP pertisk_site_h3_requests_total Total number of HTTP/3 requests per host\n");
        output.push_str("# TYPE pertisk_site_h3_requests_total counter\n");
        output.push_str("# HELP pertisk_site_h3_vs_h2_ratio Ratio of HTTP/3 requests to HTTP/2 requests per host\n");
        output.push_str("# TYPE pertisk_site_h3_vs_h2_ratio gauge\n");

        let mut site_rows = self
            .site_protocol_requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(host, counters)| {
                (
                    host.clone(),
                    counters.h2_requests_total.load(Ordering::Relaxed),
                    counters.h3_requests_total.load(Ordering::Relaxed),
                )
            })
            .collect::<Vec<_>>();
        site_rows.sort_by(|a, b| a.0.cmp(&b.0));
        for (host, h2, h3) in site_rows {
            let host_escaped = prometheus_escape_label_value(&host);
            output.push_str(&format!(
                "pertisk_site_h2_requests_total{{host=\"{}\"}} {}\n",
                host_escaped, h2
            ));
            output.push_str(&format!(
                "pertisk_site_h3_requests_total{{host=\"{}\"}} {}\n",
                host_escaped, h3
            ));
            let ratio = if h2 > 0 {
                h3 as f64 / h2 as f64
            } else if h3 > 0 {
                f64::INFINITY
            } else {
                0.0
            };
            output.push_str(&format!(
                "pertisk_site_h3_vs_h2_ratio{{host=\"{}\"}} {}\n",
                host_escaped,
                prometheus_float(ratio)
            ));
        }

        output.push_str("# HELP pertisk_grpc_requests_total Total number of gRPC requests processed\n");
        output.push_str("# TYPE pertisk_grpc_requests_total counter\n");
        output.push_str(&format!(
            "pertisk_grpc_requests_total {}\n",
            self.grpc_requests_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP pertisk_upstream_errors_total Total number of upstream errors\n");
        output.push_str("# TYPE pertisk_upstream_errors_total counter\n");
        output.push_str(&format!(
            "pertisk_upstream_errors_total {}\n",
            self.upstream_errors_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP pertisk_active_connections Current number of active connections\n");
        output.push_str("# TYPE pertisk_active_connections gauge\n");
        output.push_str(&format!(
            "pertisk_active_connections {}\n",
            self.active_connections.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP pertisk_bytes_sent_total Total number of bytes sent to clients\n");
        output.push_str("# TYPE pertisk_bytes_sent_total counter\n");
        output.push_str(&format!(
            "pertisk_bytes_sent_total {}\n",
            self.bytes_sent_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP pertisk_bytes_received_total Total number of bytes received from clients\n");
        output.push_str("# TYPE pertisk_bytes_received_total counter\n");
        output.push_str(&format!(
            "pertisk_bytes_received_total {}\n",
            self.bytes_received_total.load(Ordering::Relaxed)
        ));

        output
    }
}

impl Default for ProxyMetrics {
    fn default() -> Self {
        Self::new()
    }
}

fn prometheus_escape_label_value(v: &str) -> String {
    v.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn prometheus_float(v: f64) -> String {
    if v.is_infinite() {
        if v.is_sign_negative() {
            "-Inf".to_string()
        } else {
            "+Inf".to_string()
        }
    } else if v.is_nan() {
        "NaN".to_string()
    } else {
        v.to_string()
    }
}

/// Listen address for the Prometheus scrape server (default `0.0.0.0:9090`).
pub fn metrics_addr_from_env() -> SocketAddr {
    std::env::var("PERTISK_METRICS_ADDR")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| "0.0.0.0:9090".parse().expect("valid default metrics addr"))
}

/// Whether the dedicated Prometheus HTTP server should start.
pub fn metrics_enabled_from_env() -> bool {
    std::env::var("PERTISK_METRICS_ENABLED")
        .ok()
        .map(|v| {
            !matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        })
        .unwrap_or(true)
}

#[cfg(feature = "admin")]
pub async fn start_metrics_server(addr: SocketAddr, metrics: ProxyMetrics) -> anyhow::Result<()> {
    use axum::{
        extract::Extension,
        response::IntoResponse,
        routing::get,
        Router,
    };

    async fn metrics_handler(Extension(metrics): Extension<ProxyMetrics>) -> impl IntoResponse {
        (
            axum::http::StatusCode::OK,
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; version=0.0.4; charset=utf-8",
            )],
            metrics.get_metrics_text(),
        )
    }

    async fn health_handler() -> impl IntoResponse {
        (axum::http::StatusCode::OK, "OK")
    }

    let app = Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/health", get(health_handler))
        .layer(Extension(metrics));

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "metrics port {} unavailable: {} (set PERTISK_METRICS_ADDR or free the port)",
                addr,
                e
            )
        })?;
    info!("Prometheus metrics server listening on http://{addr}/metrics");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    info!("Prometheus metrics server stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::Version;

    #[test]
    fn counters_increment_and_export() {
        let m = ProxyMetrics::new();
        m.inc_http_requests();
        m.inc_https_requests();
        m.inc_h2_requests();
        m.inc_h3_requests();
        m.inc_grpc_requests();
        m.inc_upstream_errors();
        m.inc_site_protocol_requests("Example.COM", Version::HTTP_2);
        m.inc_site_protocol_requests("example.com", Version::HTTP_3);

        let snap = m.site_protocol_snapshot();
        assert_eq!(snap.get("example.com"), Some(&(1, 1)));

        let text = m.get_metrics_text();
        assert!(text.contains("pertisk_http_requests_total 1"));
        assert!(text.contains("pertisk_site_h2_requests_total{host=\"example.com\"} 1"));
    }
}
