use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::Client;

/// HTTP client for H3 → upstream hops. Does not follow redirects so 302/307 responses
/// (e.g. OIDC login to Auth0) reach the browser unchanged, matching Pingora/H2 behavior.
pub fn build_upstream_client() -> Result<Client> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .pool_max_idle_per_host(64)
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_keepalive(Duration::from_secs(60))
        .danger_accept_invalid_certs(true)
        .build()
        .context("build H3 upstream client")
}
