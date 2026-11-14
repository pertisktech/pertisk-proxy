//! CQL schema for the ScyllaDB / Cassandra admin config backend.

use anyhow::{Context, Result};
use scylla::client::session::Session;

/// Create keyspace + tables and seed singleton rows.
///
/// Uses NetworkTopologyStrategy (required by Scylla with tablets enabled).
/// `local_dc` defaults to `datacenter1` when empty.
pub async fn init_schema(session: &Session, keyspace: &str, local_dc: Option<&str>) -> Result<()> {
    let ks = sanitize_keyspace(keyspace)?;
    let dc = sanitize_dc(local_dc.unwrap_or("datacenter1"))?;
    session
        .query_unpaged(
            format!(
                "CREATE KEYSPACE IF NOT EXISTS {ks} WITH replication = \
                 {{'class': 'NetworkTopologyStrategy', '{dc}': 1}}"
            ),
            &[],
        )
        .await
        .context("create keyspace")?;
    session
        .use_keyspace(&ks, false)
        .await
        .context("use keyspace")?;

    for stmt in TABLE_DDL {
        session
            .query_unpaged(*stmt, &[])
            .await
            .with_context(|| format!("DDL: {stmt}"))?;
    }

    seed_smtp_settings(session).await?;
    seed_s3_settings(session).await?;
    Ok(())
}

fn sanitize_keyspace(keyspace: &str) -> Result<String> {
    let ks = keyspace.trim();
    if ks.is_empty()
        || !ks
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        || !ks
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
    {
        anyhow::bail!("invalid PERTISK_SCYLLA_KEYSPACE: {keyspace:?}");
    }
    Ok(ks.to_string())
}

fn sanitize_dc(dc: &str) -> Result<String> {
    let dc = dc.trim();
    if dc.is_empty()
        || !dc
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        anyhow::bail!("invalid PERTISK_SCYLLA_DC: {dc:?}");
    }
    Ok(dc.to_string())
}

const TABLE_DDL: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS proxy_config (
        key text PRIMARY KEY,
        value text,
        updated_at text
    )",
    "CREATE TABLE IF NOT EXISTS dns_providers (
        id text PRIMARY KEY,
        name text,
        provider_type text,
        credentials text,
        created_at text
    )",
    "CREATE TABLE IF NOT EXISTS users (
        id text PRIMARY KEY,
        username text,
        password_hash text,
        created_at text
    )",
    "CREATE INDEX IF NOT EXISTS users_username_idx ON users (username)",
    // `token` is reserved in CQL — use session_token.
    "CREATE TABLE IF NOT EXISTS sessions (
        session_token text PRIMARY KEY,
        username text,
        expires_at text
    )",
    "CREATE TABLE IF NOT EXISTS certificates (
        id text PRIMARY KEY,
        hosts text,
        cert_pem text,
        key_pem text,
        source_type text,
        created_at text,
        expires_at text
    )",
    "CREATE TABLE IF NOT EXISTS smtp_settings (
        id int PRIMARY KEY,
        enabled boolean,
        host text,
        port int,
        username text,
        password text,
        from_email text,
        from_name text,
        use_tls boolean,
        alert_to text,
        notify_login_failure boolean,
        notify_login boolean,
        notify_password_change boolean,
        updated_at text
    )",
    "CREATE TABLE IF NOT EXISTS s3_settings (
        id int PRIMARY KEY,
        enabled boolean,
        endpoint text,
        region text,
        bucket text,
        prefix text,
        access_key_id text,
        secret_access_key text,
        force_path_style boolean,
        updated_at text
    )",
];

async fn seed_smtp_settings(session: &Session) -> Result<()> {
    let result = session
        .query_unpaged("SELECT id FROM smtp_settings WHERE id = 1", &[])
        .await?
        .into_rows_result()?;
    if result.rows_num() > 0 {
        return Ok(());
    }
    let updated_at = chrono::Utc::now().to_rfc3339();
    session
        .query_unpaged(
            "INSERT INTO smtp_settings (
                id, enabled, host, port, username, password, from_email, from_name,
                use_tls, alert_to, notify_login_failure, notify_login,
                notify_password_change, updated_at
             ) VALUES (1, false, '', 587, '', '', '', 'Pertisk Proxy', true, '', false, false, false, ?)",
            (&updated_at,),
        )
        .await?;
    Ok(())
}

async fn seed_s3_settings(session: &Session) -> Result<()> {
    let result = session
        .query_unpaged("SELECT id FROM s3_settings WHERE id = 1", &[])
        .await?
        .into_rows_result()?;
    if result.rows_num() > 0 {
        return Ok(());
    }
    let updated_at = chrono::Utc::now().to_rfc3339();
    session
        .query_unpaged(
            "INSERT INTO s3_settings (
                id, enabled, endpoint, region, bucket, prefix,
                access_key_id, secret_access_key, force_path_style, updated_at
             ) VALUES (1, false, '', 'us-east-1', '', '', '', '', false, ?)",
            (&updated_at,),
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_keyspace_accepts_safe_names() {
        assert_eq!(sanitize_keyspace("pertisk_proxy").unwrap(), "pertisk_proxy");
        assert_eq!(sanitize_keyspace(" KS1 ").unwrap(), "KS1");
        assert!(sanitize_keyspace("").is_err());
        assert!(sanitize_keyspace("bad-name").is_err());
        assert!(sanitize_keyspace("1leading").is_err());
        assert!(sanitize_keyspace("drop;keyspace").is_err());
    }

    #[test]
    fn sanitize_dc_accepts_safe_names() {
        assert_eq!(sanitize_dc("datacenter1").unwrap(), "datacenter1");
        assert_eq!(sanitize_dc(" dc-1 ").unwrap(), "dc-1");
        assert!(sanitize_dc("").is_err());
        assert!(sanitize_dc("bad;dc").is_err());
    }

    #[test]
    fn table_ddl_is_non_empty() {
        assert!(!TABLE_DDL.is_empty());
        assert!(TABLE_DDL.iter().any(|s| s.contains("proxy_config")));
        assert!(TABLE_DDL.iter().any(|s| s.contains("users")));
    }
}
