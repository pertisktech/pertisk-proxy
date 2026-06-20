use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::time;
use tracing::{info, warn};

use crate::routes_config;
use crate::tls::CertStore;
use crate::Router;

pub async fn run(
    router: Arc<Router>,
    cert_store: Arc<CertStore>,
    config_path: std::path::PathBuf,
    watch: bool,
) -> Result<()> {
    reload(router.as_ref(), cert_store.as_ref(), &config_path)?;

    if !watch {
        info!(path = %config_path.display(), "routes loaded (static)");
        std::future::pending::<()>().await;
        return Ok(());
    }

    info!(path = %config_path.display(), "routes loaded, watching for changes");
    let mut last_modified = file_modified(&config_path)?;
    let mut interval = time::interval(Duration::from_secs(2));

    loop {
        interval.tick().await;
        match file_modified(&config_path) {
            Ok(modified) if modified > last_modified => {
                last_modified = modified;
                match reload(router.as_ref(), cert_store.as_ref(), &config_path) {
                    Ok(()) => info!("routes and TLS config reloaded"),
                    Err(err) => warn!(error = %err, "route reload failed"),
                }
            }
            Ok(_) => {}
            Err(err) => warn!(error = %err, "failed to stat routes config"),
        }
    }
}

fn reload(router: &Router, cert_store: &CertStore, path: &Path) -> Result<()> {
    let loaded = routes_config::load(path)?;
    info!(
        routes = loaded.table.route_count(),
        tls_entries = loaded.tls.len(),
        path = %path.display(),
        "loaded routes"
    );
    router.replace_all(loaded.table, loaded.http3);
    cert_store.reload_from_configs(&loaded.tls)?;
    Ok(())
}

fn file_modified(path: &Path) -> Result<std::time::SystemTime> {
    Ok(std::fs::metadata(path)
        .with_context(|| format!("failed to stat {}", path.display()))?
        .modified()?)
}
