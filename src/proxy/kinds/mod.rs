use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::time;
use tracing::{info, warn};

use crate::mode::ProxyKind;
use crate::routes_config;
use crate::Router;

pub async fn run(
    router: Arc<Router>,
    kind: ProxyKind,
    config_path: PathBuf,
    watch: bool,
) -> Result<()> {
    info!(kind = %kind, description = kind.description(), "starting proxy mode");
    reload(router.as_ref(), &config_path)?;

    if !watch {
        info!(path = %config_path.display(), "routes loaded (static, no watch)");
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
                match reload(router.as_ref(), &config_path) {
                    Ok(()) => info!(kind = %kind, "routes reloaded"),
                    Err(err) => warn!(error = %err, "route reload failed"),
                }
            }
            Ok(_) => {}
            Err(err) => warn!(error = %err, "failed to stat routes config"),
        }
    }
}

fn reload(router: &Router, path: &Path) -> Result<()> {
    let table = routes_config::load(path)?;
    info!(routes = table.route_count(), path = %path.display(), "loaded routes");
    router.replace(table);
    Ok(())
}

fn file_modified(path: &Path) -> Result<std::time::SystemTime> {
    Ok(std::fs::metadata(path)
        .with_context(|| format!("failed to stat {}", path.display()))?
        .modified()?)
}
