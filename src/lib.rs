#[cfg(feature = "ingress")]
pub mod ingress;
pub mod config;
pub mod deny;
pub mod health;
pub mod http3_options;
pub mod proxy_config;
#[cfg(feature = "admin")]
pub mod api;
#[cfg(feature = "admin")]
pub mod db;
pub mod h3;
pub mod logging;
pub mod log;
pub mod proxy;
pub mod router;
pub mod routes_config;
pub mod runtime;
pub mod server;
pub mod tls;

pub use config::{IngressConfig, ProxyConfig, ServerConfig};
pub use log::{ProxyLog, ProxyLogEntry};
pub use proxy::Gateway;
pub use router::Router;

/// Value for the `X-App-Name` response header (`proxy` or `ingress`).
pub fn app_name() -> &'static str {
    if cfg!(feature = "ingress") {
        "ingress"
    } else {
        "proxy"
    }
}

/// Set `X-App-Name` on an HTTP response header map.
pub fn apply_app_name(headers: &mut http::HeaderMap) {
    if let Ok(value) = http::HeaderValue::from_str(app_name()) {
        headers.insert(http::HeaderName::from_static("x-app-name"), value);
    }
}
