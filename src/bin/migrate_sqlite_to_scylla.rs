//! Migrate admin config from SQLite into ScyllaDB / Cassandra.
//!
//! ```text
//! PERTISK_SCYLLA_HOSTS=10.1.1.148:9042 \
//! PERTISK_SCYLLA_USER=pertisk PERTISK_SCYLLA_PASSWORD=... \
//! migrate-sqlite-to-scylla /var/lib/pertisk-proxy/proxy.sqlite
//! ```

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use tracing::info;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let sqlite_path = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(pertisk_proxy::config::resolve_db_path);

    if !sqlite_path.is_file() {
        eprintln!(
            "SQLite file not found: {} (pass path as argv[1] or set PERTISK_DB_PATH)",
            sqlite_path.display()
        );
        return ExitCode::from(2);
    }

    let cfg = pertisk_proxy::config::ScyllaDbConfig::from_env();
    info!(
        sqlite = %sqlite_path.display(),
        hosts = ?cfg.hosts,
        keyspace = %cfg.keyspace,
        user = ?cfg.username,
        "starting SQLite → Scylla migration"
    );

    match pertisk_proxy::db::migrate_sqlite_to_scylla(&sqlite_path, &cfg).await {
        Ok(stats) => {
            info!(?stats, "migration complete");
            println!(
                "ok proxy_config={} dns_providers={} users={} sessions={} certificates={} smtp={} s3={}",
                stats.proxy_config,
                stats.dns_providers,
                stats.users,
                stats.sessions,
                stats.certificates,
                stats.smtp_settings,
                stats.s3_settings
            );
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("migration failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
