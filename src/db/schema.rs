//! SQLite schema: proxy_config, dns_providers, users, sessions, certificates, smtp_settings, s3_settings.

use rusqlite::Connection;

pub fn init_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS proxy_config (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS dns_providers (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            provider_type TEXT NOT NULL,
            credentials TEXT,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS users (
            id TEXT PRIMARY KEY,
            username TEXT NOT NULL UNIQUE,
            password_hash TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_users_username ON users(username);
        CREATE TABLE IF NOT EXISTS sessions (
            token TEXT PRIMARY KEY,
            username TEXT NOT NULL,
            expires_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_sessions_expires_at ON sessions(expires_at);
        CREATE TABLE IF NOT EXISTS certificates (
            id TEXT PRIMARY KEY,
            hosts TEXT NOT NULL,
            cert_pem TEXT NOT NULL,
            key_pem TEXT NOT NULL,
            source_type TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS smtp_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            enabled INTEGER NOT NULL DEFAULT 0,
            host TEXT NOT NULL DEFAULT '',
            port INTEGER NOT NULL DEFAULT 587,
            username TEXT NOT NULL DEFAULT '',
            password TEXT NOT NULL DEFAULT '',
            from_email TEXT NOT NULL DEFAULT '',
            from_name TEXT NOT NULL DEFAULT '',
            use_tls INTEGER NOT NULL DEFAULT 1,
            alert_to TEXT NOT NULL DEFAULT '',
            notify_login_failure INTEGER NOT NULL DEFAULT 0,
            notify_login INTEGER NOT NULL DEFAULT 0,
            notify_password_change INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS s3_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            enabled INTEGER NOT NULL DEFAULT 0,
            endpoint TEXT NOT NULL DEFAULT '',
            region TEXT NOT NULL DEFAULT 'us-east-1',
            bucket TEXT NOT NULL DEFAULT '',
            prefix TEXT NOT NULL DEFAULT '',
            access_key_id TEXT NOT NULL DEFAULT '',
            secret_access_key TEXT NOT NULL DEFAULT '',
            force_path_style INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL
        );
        ",
    )?;
    migrate_certificates_expires_at(conn)?;
    migrate_smtp_notify_flags(conn)?;
    seed_smtp_settings(conn)?;
    seed_s3_settings(conn)?;
    Ok(())
}

fn seed_smtp_settings(conn: &Connection) -> Result<(), rusqlite::Error> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM smtp_settings WHERE id = 1", [], |r| {
        r.get(0)
    })?;
    if count > 0 {
        return Ok(());
    }
    let updated_at = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO smtp_settings (
            id, enabled, host, port, username, password, from_email, from_name,
            use_tls, alert_to, notify_login_failure, notify_login, notify_password_change, updated_at
         ) VALUES (1, 0, '', 587, '', '', '', 'Pertisk Proxy', 1, '', 0, 0, 0, ?1)",
        rusqlite::params![updated_at],
    )?;
    Ok(())
}

fn seed_s3_settings(conn: &Connection) -> Result<(), rusqlite::Error> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM s3_settings WHERE id = 1", [], |r| {
        r.get(0)
    })?;
    if count > 0 {
        return Ok(());
    }
    let updated_at = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO s3_settings (
            id, enabled, endpoint, region, bucket, prefix,
            access_key_id, secret_access_key, force_path_style, updated_at
         ) VALUES (1, 0, '', 'us-east-1', '', '', '', '', 0, ?1)",
        rusqlite::params![updated_at],
    )?;
    Ok(())
}

fn migrate_certificates_expires_at(conn: &Connection) -> Result<(), rusqlite::Error> {
    add_column_if_missing(conn, "certificates", "expires_at", "TEXT")?;
    Ok(())
}

fn migrate_smtp_notify_flags(conn: &Connection) -> Result<(), rusqlite::Error> {
    add_column_if_missing(
        conn,
        "smtp_settings",
        "notify_login",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        conn,
        "smtp_settings",
        "notify_password_change",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    Ok(())
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    decl: &str,
) -> Result<(), rusqlite::Error> {
    let has_col: bool = conn.query_row(
        &format!("SELECT COUNT(1) FROM pragma_table_info('{table}') WHERE name = '{column}'"),
        [],
        |r| r.get(0),
    )?;
    if !has_col {
        conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"), [])?;
    }
    Ok(())
}
