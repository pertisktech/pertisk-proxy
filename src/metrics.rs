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
    pub geoip_blocked_total: Arc<AtomicU64>,
    pub waf_blocked_total: Arc<AtomicU64>,
    pub waf_logged_total: Arc<AtomicU64>,
    pub bot_challenged_total: Arc<AtomicU64>,
    pub bot_blocked_total: Arc<AtomicU64>,
    pub captcha_passed_total: Arc<AtomicU64>,
    pub captcha_failed_total: Arc<AtomicU64>,
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
            geoip_blocked_total: Arc::new(AtomicU64::new(0)),
            waf_blocked_total: Arc::new(AtomicU64::new(0)),
            waf_logged_total: Arc::new(AtomicU64::new(0)),
            bot_challenged_total: Arc::new(AtomicU64::new(0)),
            bot_blocked_total: Arc::new(AtomicU64::new(0)),
            captcha_passed_total: Arc::new(AtomicU64::new(0)),
            captcha_failed_total: Arc::new(AtomicU64::new(0)),
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

    pub fn inc_geoip_blocked(&self) {
        self.geoip_blocked_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_waf_blocked(&self) {
        self.waf_blocked_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_waf_logged(&self) {
        self.waf_logged_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_bot_challenged(&self) {
        self.bot_challenged_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_bot_blocked(&self) {
        self.bot_blocked_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_captcha_passed(&self) {
        self.captcha_passed_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_captcha_failed(&self) {
        self.captcha_failed_total.fetch_add(1, Ordering::Relaxed);
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

        output.push_str("# HELP pertisk_geoip_blocked_total Total requests blocked by GeoIP policy\n");
        output.push_str("# TYPE pertisk_geoip_blocked_total counter\n");
        output.push_str(&format!(
            "pertisk_geoip_blocked_total {}\n",
            self.geoip_blocked_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP pertisk_waf_blocked_total Total requests blocked by WAF\n");
        output.push_str("# TYPE pertisk_waf_blocked_total counter\n");
        output.push_str(&format!(
            "pertisk_waf_blocked_total {}\n",
            self.waf_blocked_total.load(Ordering::Relaxed)
        ));
        output.push_str("# HELP pertisk_waf_logged_total Total WAF log-only matches\n");
        output.push_str("# TYPE pertisk_waf_logged_total counter\n");
        output.push_str(&format!(
            "pertisk_waf_logged_total {}\n",
            self.waf_logged_total.load(Ordering::Relaxed)
        ));
        output.push_str("# HELP pertisk_bot_challenged_total Total bot challenges issued\n");
        output.push_str("# TYPE pertisk_bot_challenged_total counter\n");
        output.push_str(&format!(
            "pertisk_bot_challenged_total {}\n",
            self.bot_challenged_total.load(Ordering::Relaxed)
        ));
        output.push_str("# HELP pertisk_bot_blocked_total Total requests blocked by bot score\n");
        output.push_str("# TYPE pertisk_bot_blocked_total counter\n");
        output.push_str(&format!(
            "pertisk_bot_blocked_total {}\n",
            self.bot_blocked_total.load(Ordering::Relaxed)
        ));
        output.push_str("# HELP pertisk_captcha_passed_total Total captcha passes\n");
        output.push_str("# TYPE pertisk_captcha_passed_total counter\n");
        output.push_str(&format!(
            "pertisk_captcha_passed_total {}\n",
            self.captcha_passed_total.load(Ordering::Relaxed)
        ));
        output.push_str("# HELP pertisk_captcha_failed_total Total captcha failures\n");
        output.push_str("# TYPE pertisk_captcha_failed_total counter\n");
        output.push_str(&format!(
            "pertisk_captcha_failed_total {}\n",
            self.captcha_failed_total.load(Ordering::Relaxed)
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
    // tarpaulin::skip_start
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
    // tarpaulin::skip_end
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::Version;
    use std::sync::Mutex;

    static METRICS_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn restore_env_var(key: &str, previous: Option<String>) {
        match previous {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn counters_increment_and_export() {
        let m = ProxyMetrics::new();
        m.inc_http_requests();
        m.inc_https_requests();
        m.inc_h2_requests();
        m.inc_h3_requests();
        m.inc_grpc_requests();
        m.inc_upstream_errors();
        m.inc_active_connections();
        m.dec_active_connections();
        m.add_bytes_sent(100);
        m.add_bytes_sent(0);
        m.add_bytes_received(50);
        m.inc_site_protocol_requests("Example.COM", Version::HTTP_2);
        m.inc_site_protocol_requests("example.com", Version::HTTP_3);
        m.inc_site_protocol_requests("", Version::HTTP_2);
        m.inc_site_protocol_requests("other.com", Version::HTTP_11);

        let snap = m.site_protocol_snapshot();
        assert_eq!(snap.get("example.com"), Some(&(1, 1)));

        let text = m.get_metrics_text();
        assert!(text.contains("pertisk_http_requests_total 1"));
        assert!(text.contains("pertisk_site_h2_requests_total{host=\"example.com\"} 1"));
        assert!(text.contains("pertisk_bytes_sent_total 100"));
        assert!(text.contains("pertisk_bytes_received_total 50"));
        assert!(text.contains("pertisk_h3_vs_h2_ratio"));
    }

    #[test]
    fn h3_vs_h2_ratio_infinity_when_only_h3() {
        let m = ProxyMetrics::new();
        m.inc_h3_requests();
        let text = m.get_metrics_text();
        assert!(text.contains("pertisk_h3_vs_h2_ratio +Inf"));
    }

    #[test]
    fn prometheus_float_and_escape() {
        assert_eq!(prometheus_float(f64::INFINITY), "+Inf");
        assert_eq!(prometheus_float(f64::NEG_INFINITY), "-Inf");
        assert!(prometheus_float(f64::NAN).contains("NaN"));
        assert_eq!(prometheus_float(1.5), "1.5");
        assert_eq!(
            prometheus_escape_label_value("host\"with\\newline\n"),
            "host\\\"with\\\\newline\\n"
        );
    }

    #[test]
    fn metrics_env_defaults() {
        let _lock = METRICS_ENV_LOCK.lock().unwrap();
        let saved_enabled = std::env::var("PERTISK_METRICS_ENABLED").ok();
        let saved_addr = std::env::var("PERTISK_METRICS_ADDR").ok();
        std::env::remove_var("PERTISK_METRICS_ENABLED");
        std::env::remove_var("PERTISK_METRICS_ADDR");

        let addr = metrics_addr_from_env();
        assert_eq!(addr.port(), 9090);
        assert!(metrics_enabled_from_env());

        restore_env_var("PERTISK_METRICS_ENABLED", saved_enabled);
        restore_env_var("PERTISK_METRICS_ADDR", saved_addr);
    }

    #[test]
    fn metrics_env_can_disable() {
        let _lock = METRICS_ENV_LOCK.lock().unwrap();
        let saved = std::env::var("PERTISK_METRICS_ENABLED").ok();
        std::env::set_var("PERTISK_METRICS_ENABLED", "off");
        assert!(!metrics_enabled_from_env());
        restore_env_var("PERTISK_METRICS_ENABLED", saved);
    }

    #[test]
    fn site_ratio_zero_when_no_requests() {
        let m = ProxyMetrics::new();
        let text = m.get_metrics_text();
        assert!(text.contains("pertisk_h3_vs_h2_ratio 0"));
    }

    #[test]
    fn default_metrics_impl() {
        let m = ProxyMetrics::default();
        let text = m.get_metrics_text();
        assert!(text.contains("pertisk_http_requests_total"));
        assert!(text.contains("pertisk_active_connections"));
    }
}
