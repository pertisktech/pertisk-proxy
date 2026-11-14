//! One-shot SQLite → ScyllaDB admin config migration.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;
use tracing::info;

use crate::config::ScyllaDbConfig;

use super::scylla_backend;

/// Copy all admin-config tables from a SQLite file into ScyllaDB / Cassandra.
///
/// Creates the keyspace and schema if needed. Existing Scylla rows with the same
/// primary keys are overwritten (INSERT upsert).
pub async fn migrate_sqlite_to_scylla(
    sqlite_path: &Path,
    cfg: &ScyllaDbConfig,
) -> Result<MigrateStats> {
    let conn = Connection::open(sqlite_path)
        .with_context(|| format!("open SQLite {}", sqlite_path.display()))?;
    super::init_schema(&conn)?;

    // Schema only — do not seed default admin (would collide with migrated users).
    let session = scylla_backend::connect_with_options(cfg, false).await?;

    // Clear target tables so a re-run does not leave stale / seeded rows.
    for table in [
        "proxy_config",
        "dns_providers",
        "users",
        "sessions",
        "certificates",
        "smtp_settings",
        "s3_settings",
    ] {
        session
            .query_unpaged(format!("TRUNCATE TABLE {table}"), &[])
            .await
            .with_context(|| format!("TRUNCATE {table}"))?;
    }

    let mut stats = MigrateStats::default();

    {
        let mut stmt = conn.prepare_cached(
            "SELECT key, value, updated_at FROM proxy_config",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (key, value, updated_at) = row?;
            session
                .query_unpaged(
                    "INSERT INTO proxy_config (key, value, updated_at) VALUES (?, ?, ?)",
                    (&key, &value, &updated_at),
                )
                .await?;
            stats.proxy_config += 1;
        }
    }

    {
        let mut stmt = conn.prepare_cached(
            "SELECT id, name, provider_type, credentials, created_at FROM dns_providers",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        for row in rows {
            let (id, name, provider_type, credentials, created_at) = row?;
            session
                .query_unpaged(
                    "INSERT INTO dns_providers (id, name, provider_type, credentials, created_at) \
                     VALUES (?, ?, ?, ?, ?)",
                    (&id, &name, &provider_type, credentials.as_deref(), &created_at),
                )
                .await?;
            stats.dns_providers += 1;
        }
    }

    {
        let mut stmt = conn.prepare_cached(
            "SELECT id, username, password_hash, created_at FROM users",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        for row in rows {
            let (id, username, password_hash, created_at) = row?;
            session
                .query_unpaged(
                    "INSERT INTO users (id, username, password_hash, created_at) VALUES (?, ?, ?, ?)",
                    (&id, &username, &password_hash, &created_at),
                )
                .await?;
            stats.users += 1;
        }
    }

    {
        let mut stmt =
            conn.prepare_cached("SELECT token, username, expires_at FROM sessions")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (token, username, expires_at) = row?;
            session
                .query_unpaged(
                    "INSERT INTO sessions (session_token, username, expires_at) VALUES (?, ?, ?)",
                    (&token, &username, &expires_at),
                )
                .await?;
            stats.sessions += 1;
        }
    }

    {
        let mut stmt = conn.prepare_cached(
            "SELECT id, hosts, cert_pem, key_pem, source_type, created_at, expires_at \
             FROM certificates",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
            ))
        })?;
        for row in rows {
            let (id, hosts, cert_pem, key_pem, source_type, created_at, expires_at) = row?;
            session
                .query_unpaged(
                    "INSERT INTO certificates \
                     (id, hosts, cert_pem, key_pem, source_type, created_at, expires_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?)",
                    (
                        &id,
                        &hosts,
                        &cert_pem,
                        &key_pem,
                        &source_type,
                        &created_at,
                        expires_at.as_deref(),
                    ),
                )
                .await?;
            stats.certificates += 1;
        }
    }

    {
        let mut stmt = conn.prepare_cached(
            "SELECT enabled, host, port, username, password, from_email, from_name,
                    use_tls, alert_to, notify_login_failure, notify_login,
                    notify_password_change, updated_at
             FROM smtp_settings WHERE id = 1",
        )?;
        let mut rows = stmt.query([])?;
        if let Some(r) = rows.next()? {
            let enabled: i64 = r.get(0)?;
            let host: String = r.get(1)?;
            let port: i64 = r.get(2)?;
            let username: String = r.get(3)?;
            let password: String = r.get(4)?;
            let from_email: String = r.get(5)?;
            let from_name: String = r.get(6)?;
            let use_tls: i64 = r.get(7)?;
            let alert_to: String = r.get(8)?;
            let notify_login_failure: i64 = r.get(9)?;
            let notify_login: i64 = r.get(10)?;
            let notify_password_change: i64 = r.get(11)?;
            let updated_at: String = r.get(12)?;
            session
                .query_unpaged(
                    "INSERT INTO smtp_settings (
                        id, enabled, host, port, username, password, from_email, from_name,
                        use_tls, alert_to, notify_login_failure, notify_login,
                        notify_password_change, updated_at
                     ) VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    (
                        enabled != 0,
                        &host,
                        port as i32,
                        &username,
                        &password,
                        &from_email,
                        &from_name,
                        use_tls != 0,
                        &alert_to,
                        notify_login_failure != 0,
                        notify_login != 0,
                        notify_password_change != 0,
                        &updated_at,
                    ),
                )
                .await?;
            stats.smtp_settings = 1;
        }
    }

    {
        let mut stmt = conn.prepare_cached(
            "SELECT enabled, endpoint, region, bucket, prefix, access_key_id,
                    secret_access_key, force_path_style, updated_at
             FROM s3_settings WHERE id = 1",
        )?;
        let mut rows = stmt.query([])?;
        if let Some(r) = rows.next()? {
            let enabled: i64 = r.get(0)?;
            let endpoint: String = r.get(1)?;
            let region: String = r.get(2)?;
            let bucket: String = r.get(3)?;
            let prefix: String = r.get(4)?;
            let access_key_id: String = r.get(5)?;
            let secret_access_key: String = r.get(6)?;
            let force_path_style: i64 = r.get(7)?;
            let updated_at: String = r.get(8)?;
            session
                .query_unpaged(
                    "INSERT INTO s3_settings (
                        id, enabled, endpoint, region, bucket, prefix,
                        access_key_id, secret_access_key, force_path_style, updated_at
                     ) VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    (
                        enabled != 0,
                        &endpoint,
                        &region,
                        &bucket,
                        &prefix,
                        &access_key_id,
                        &secret_access_key,
                        force_path_style != 0,
                        &updated_at,
                    ),
                )
                .await?;
            stats.s3_settings = 1;
        }
    }

    info!(
        sqlite = %sqlite_path.display(),
        keyspace = %cfg.keyspace,
        ?stats,
        "migrated SQLite admin config to ScyllaDB"
    );
    Ok(stats)
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MigrateStats {
    pub proxy_config: usize,
    pub dns_providers: usize,
    pub users: usize,
    pub sessions: usize,
    pub certificates: usize,
    pub smtp_settings: usize,
    pub s3_settings: usize,
}
