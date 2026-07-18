mod bind;
mod config;
mod upstream_client;

#[cfg(feature = "h3-quiche")]
mod headers;
#[cfg(feature = "h3-quiche")]
mod health;
#[cfg(feature = "h3-quiche")]
mod server;
#[cfg(feature = "h3-quiche")]
mod settings;

#[cfg(feature = "h3-quinn")]
mod quinn_server;

pub use bind::effective_listen_display;
pub use bind::effective_udp_listen_display;
pub use bind::h3_bind_addrs;
pub use bind::tcp_bind_addrs;
pub use config::H3Config;
pub(crate) use upstream_client::upstream_pool_config;

#[cfg(feature = "h3-quinn")]
pub(crate) use quinn_server::effective_transport_config;
#[cfg(feature = "h3-quinn")]
pub use quinn_server::run;

#[cfg(all(feature = "h3-quiche", not(feature = "h3-quinn")))]
pub use server::run;

#[cfg(not(any(feature = "h3-quinn", feature = "h3-quiche")))]
pub async fn run(
    _router: std::sync::Arc<crate::Router>,
    _config: H3Config,
    _cert_store: std::sync::Arc<crate::tls::CertStore>,
    _runtime_cfg: &crate::runtime::RuntimeConfig,
    _metrics: crate::metrics::ProxyMetrics,
) -> anyhow::Result<()> {
    anyhow::bail!("HTTP/3 support not compiled in (enable h3-quinn or h3-quiche feature)")
}
