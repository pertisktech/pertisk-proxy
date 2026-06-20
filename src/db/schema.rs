//! SQLite schema: proxy_config, dns_providers, certificates.

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
        CREATE TABLE IF NOT EXISTS certificates (
            id TEXT PRIMARY KEY,
            hosts TEXT NOT NULL,
            cert_pem TEXT NOT NULL,
            key_pem TEXT NOT NULL,
            source_type TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        ",
    )?;
    migrate_certificates_expires_at(conn)?;
    Ok(())
}

fn migrate_certificates_expires_at(conn: &Connection) -> Result<(), rusqlite::Error> {
    let has_col: bool = conn.query_row(
        "SELECT COUNT(1) FROM pragma_table_info('certificates') WHERE name = 'expires_at'",
        [],
        |r| r.get(0),
    )?;
    if !has_col {
        conn.execute("ALTER TABLE certificates ADD COLUMN expires_at TEXT", [])?;
    }
    Ok(())
}
