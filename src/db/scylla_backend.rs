//! ScyllaDB / Cassandra implementation of admin config persistence.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use bcrypt::{hash, DEFAULT_COST};
use scylla::client::session::Session;
use scylla::client::session_builder::SessionBuilder;
use tracing::info;
use uuid::Uuid;

use crate::config::ScyllaDbConfig;

use super::{
    cert_expiry_from_pem, DnsProviderRow, S3SettingsRow, S3SettingsUpdate, SmtpSettingsRow,
    SmtpSettingsUpdate, CertificateRow, DEFAULT_ADMIN_PASSWORD, DEFAULT_ADMIN_USERNAME,
    PROXY_CONFIG_KEY,
};

pub async fn connect(cfg: &ScyllaDbConfig) -> Result<Arc<Session>> {
    connect_with_options(cfg, true).await
}

/// Connect and ensure schema. When `seed_defaults` is false, skip default admin /
/// singleton seeding (used by SQLite → Scylla migration).
pub async fn connect_with_options(
    cfg: &ScyllaDbConfig,
    seed_defaults: bool,
) -> Result<Arc<Session>> {
    let mut builder = SessionBuilder::new();
    for host in &cfg.hosts {
        builder = builder.known_node(host);
    }
    if let (Some(user), Some(pass)) = (&cfg.username, &cfg.password) {
        builder = builder.user(user, pass);
    }
    if let Some(dc) = &cfg.local_dc {
        builder = builder.prefer_datacenter(dc.clone());
    }
    let session = builder
        .build()
        .await
        .context("connect to ScyllaDB")?;
    let session = Arc::new(session);
    super::scylla_schema::init_schema(
        &session,
        &cfg.keyspace,
        cfg.local_dc.as_deref(),
    )
    .await?;
    if seed_defaults {
        seed_admin_user(&session).await?;
    }
    info!(
        hosts = ?cfg.hosts,
        keyspace = %cfg.keyspace,
        seed_defaults,
        "opened ScyllaDB admin config backend"
    );
    Ok(session)
}

async fn seed_admin_user(session: &Session) -> Result<()> {
    let result = session
        .query_unpaged("SELECT id FROM users LIMIT 1", &[])
        .await?
        .into_rows_result()?;
    if result.rows_num() > 0 {
        return Ok(());
    }
    let id = Uuid::new_v4().to_string();
    let password_hash = hash(DEFAULT_ADMIN_PASSWORD, DEFAULT_COST)?;
    let created_at = chrono::Utc::now().to_rfc3339();
    session
        .query_unpaged(
            "INSERT INTO users (id, username, password_hash, created_at) VALUES (?, ?, ?, ?)",
            (&id, DEFAULT_ADMIN_USERNAME, &password_hash, &created_at),
        )
        .await?;
    info!(
        username = DEFAULT_ADMIN_USERNAME,
        "seeded default admin user (change password after first login)"
    );
    Ok(())
}

pub async fn get_user_password_hash(
    session: &Session,
    username: &str,
) -> Result<Option<String>> {
    let result = session
        .query_unpaged(
            "SELECT password_hash FROM users WHERE username = ? ALLOW FILTERING",
            (username,),
        )
        .await?
        .into_rows_result()?;
    Ok(result.maybe_first_row::<(String,)>()?.map(|(h,)| h))
}

pub async fn update_user_password(
    session: &Session,
    username: &str,
    new_password: &str,
) -> Result<bool> {
    let Some(_) = get_user_password_hash(session, username).await? else {
        return Ok(false);
    };
    let password_hash = hash(new_password, DEFAULT_COST)?;
    // Secondary index lookup — fetch id then update by PK.
    let result = session
        .query_unpaged(
            "SELECT id FROM users WHERE username = ? ALLOW FILTERING",
            (username,),
        )
        .await?
        .into_rows_result()?;
    let Some((id,)) = result.maybe_first_row::<(String,)>()? else {
        return Ok(false);
    };
    session
        .query_unpaged(
            "UPDATE users SET password_hash = ? WHERE id = ?",
            (&password_hash, &id),
        )
        .await?;
    Ok(true)
}

pub async fn insert_session(
    session: &Session,
    token: &str,
    username: &str,
    expires_at: chrono::DateTime<chrono::Utc>,
) -> Result<()> {
    let expires_at = expires_at.to_rfc3339();
    session
        .query_unpaged(
            "INSERT INTO sessions (session_token, username, expires_at) VALUES (?, ?, ?)",
            (token, username, &expires_at),
        )
        .await?;
    Ok(())
}

pub async fn get_session(
    session: &Session,
    token: &str,
) -> Result<Option<(String, chrono::DateTime<chrono::Utc>)>> {
    let result = session
        .query_unpaged(
            "SELECT username, expires_at FROM sessions WHERE session_token = ?",
            (token,),
        )
        .await?
        .into_rows_result()?;
    let Some((username, expires_at)) = result.maybe_first_row::<(String, String)>()? else {
        return Ok(None);
    };
    let expires_at = chrono::DateTime::parse_from_rfc3339(&expires_at)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| chrono::Utc::now());
    if expires_at <= chrono::Utc::now() {
        return Ok(None);
    }
    Ok(Some((username, expires_at)))
}

pub async fn load_active_sessions(
    session: &Session,
) -> Result<Vec<(String, String, chrono::DateTime<chrono::Utc>)>> {
    let result = session
        .query_unpaged("SELECT session_token, username, expires_at FROM sessions", &[])
        .await?
        .into_rows_result()?;
    let now = chrono::Utc::now();
    let mut out = Vec::new();
    for row in result.rows::<(String, String, String)>()? {
        let (token, username, expires_at) = row?;
        let expires_at = chrono::DateTime::parse_from_rfc3339(&expires_at)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now());
        if expires_at > now {
            out.push((token, username, expires_at));
        }
    }
    Ok(out)
}

pub async fn get_config_json(session: &Session, key: &str) -> Result<Option<String>> {
    let result = session
        .query_unpaged("SELECT value FROM proxy_config WHERE key = ?", (key,))
        .await?
        .into_rows_result()?;
    Ok(result.maybe_first_row::<(String,)>()?.map(|(v,)| v))
}

pub async fn save_config_json(session: &Session, key: &str, value: &str) -> Result<()> {
    let updated_at = chrono::Utc::now().to_rfc3339();
    session
        .query_unpaged(
            "INSERT INTO proxy_config (key, value, updated_at) VALUES (?, ?, ?)",
            (key, value, &updated_at),
        )
        .await?;
    Ok(())
}

pub async fn get_proxy_config(
    session: &Session,
) -> Result<Option<crate::proxy_config::Config>> {
    if let Some(json) = get_config_json(session, PROXY_CONFIG_KEY).await? {
        return Ok(Some(serde_json::from_str(&json)?));
    }
    if let Some(json) = get_config_json(session, "sites").await? {
        if let Ok(cfg) = serde_json::from_str::<crate::proxy_config::Config>(&json) {
            let json = serde_json::to_string(&cfg)?;
            save_config_json(session, PROXY_CONFIG_KEY, &json).await?;
            return Ok(Some(cfg));
        }
    }
    Ok(None)
}

pub async fn save_proxy_config(
    session: &Session,
    config: &crate::proxy_config::Config,
) -> Result<()> {
    let json = serde_json::to_string(config)?;
    save_config_json(session, PROXY_CONFIG_KEY, &json).await
}

pub async fn list_dns_providers(session: &Session) -> Result<Vec<DnsProviderRow>> {
    let result = session
        .query_unpaged(
            "SELECT id, name, provider_type, created_at FROM dns_providers",
            &[],
        )
        .await?
        .into_rows_result()?;
    let mut out = Vec::new();
    for row in result.rows::<(String, String, String, String)>()? {
        let (id, name, provider_type, created_at) = row?;
        out.push(DnsProviderRow {
            id,
            name,
            provider_type,
            credentials: None,
            created_at,
        });
    }
    out.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(out)
}

pub async fn get_dns_provider(session: &Session, id: &str) -> Result<Option<DnsProviderRow>> {
    let result = session
        .query_unpaged(
            "SELECT id, name, provider_type, credentials, created_at FROM dns_providers WHERE id = ?",
            (id,),
        )
        .await?
        .into_rows_result()?;
    let Some((id, name, provider_type, credentials, created_at)) =
        result.maybe_first_row::<(String, String, String, Option<String>, String)>()?
    else {
        return Ok(None);
    };
    Ok(Some(DnsProviderRow {
        id,
        name,
        provider_type,
        credentials: credentials.and_then(|s| serde_json::from_str(&s).ok()),
        created_at,
    }))
}

pub async fn create_dns_provider(
    session: &Session,
    name: String,
    provider_type: String,
    credentials: Option<HashMap<String, String>>,
) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let cred_json = credentials
        .as_ref()
        .and_then(|c| serde_json::to_string(c).ok());
    let created_at = chrono::Utc::now().to_rfc3339();
    session
        .query_unpaged(
            "INSERT INTO dns_providers (id, name, provider_type, credentials, created_at) VALUES (?, ?, ?, ?, ?)",
            (&id, &name, &provider_type, &cred_json, &created_at),
        )
        .await?;
    Ok(id)
}

pub async fn put_dns_provider(
    session: &Session,
    id: &str,
    name: String,
    provider_type: String,
    credentials: Option<HashMap<String, String>>,
) -> Result<bool> {
    if get_dns_provider(session, id).await?.is_none() {
        return Ok(false);
    }
    let cred_json = credentials
        .as_ref()
        .and_then(|c| serde_json::to_string(c).ok());
    session
        .query_unpaged(
            "UPDATE dns_providers SET name = ?, provider_type = ?, credentials = ? WHERE id = ?",
            (&name, &provider_type, &cred_json, id),
        )
        .await?;
    Ok(true)
}

pub async fn delete_dns_provider(session: &Session, id: &str) -> Result<bool> {
    if get_dns_provider(session, id).await?.is_none() {
        return Ok(false);
    }
    session
        .query_unpaged("DELETE FROM dns_providers WHERE id = ?", (id,))
        .await?;
    Ok(true)
}

pub async fn list_certificates(session: &Session) -> Result<Vec<CertificateRow>> {
    let result = session
        .query_unpaged(
            "SELECT id, hosts, source_type, created_at, expires_at, cert_pem FROM certificates",
            &[],
        )
        .await?
        .into_rows_result()?;
    let mut out = Vec::new();
    for row in result.rows::<(String, String, String, String, Option<String>, String)>()? {
        let (id, hosts, source_type, created_at, mut expires_at, cert_pem) = row?;
        if expires_at.is_none() {
            if let Some(parsed) = cert_expiry_from_pem(cert_pem.as_bytes()) {
                expires_at = Some(parsed.clone());
                let _ = session
                    .query_unpaged(
                        "UPDATE certificates SET expires_at = ? WHERE id = ?",
                        (&parsed, &id),
                    )
                    .await;
            }
        }
        out.push(CertificateRow {
            id,
            hosts: serde_json::from_str(&hosts).unwrap_or_default(),
            source_type,
            created_at,
            expires_at,
        });
    }
    out.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(out)
}

pub async fn get_all_certificates_for_store(
    session: &Session,
) -> Result<Vec<(String, Vec<String>, Vec<u8>, Vec<u8>)>> {
    let result = session
        .query_unpaged(
            "SELECT id, hosts, cert_pem, key_pem FROM certificates",
            &[],
        )
        .await?
        .into_rows_result()?;
    let mut out = Vec::new();
    for row in result.rows::<(String, String, String, String)>()? {
        let (id, hosts, cert_pem, key_pem) = row?;
        let hosts: Vec<String> = serde_json::from_str(&hosts).unwrap_or_default();
        out.push((id, hosts, cert_pem.into_bytes(), key_pem.into_bytes()));
    }
    Ok(out)
}

pub async fn add_certificate(
    session: &Session,
    hosts: Vec<String>,
    cert_pem: Vec<u8>,
    key_pem: Vec<u8>,
    source_type: &str,
) -> Result<String> {
    let expires_at = cert_expiry_from_pem(&cert_pem);
    let id = Uuid::new_v4().to_string();
    let hosts_json = serde_json::to_string(&hosts)?;
    let cert_str = String::from_utf8(cert_pem)?;
    let key_str = String::from_utf8(key_pem)?;
    let created_at = chrono::Utc::now().to_rfc3339();
    session
        .query_unpaged(
            "INSERT INTO certificates (id, hosts, cert_pem, key_pem, source_type, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
            (
                &id,
                &hosts_json,
                &cert_str,
                &key_str,
                source_type,
                &created_at,
                &expires_at,
            ),
        )
        .await?;
    Ok(id)
}

pub async fn delete_certificate(session: &Session, id: &str) -> Result<bool> {
    let result = session
        .query_unpaged("SELECT id FROM certificates WHERE id = ?", (id,))
        .await?
        .into_rows_result()?;
    if result.rows_num() == 0 {
        return Ok(false);
    }
    session
        .query_unpaged("DELETE FROM certificates WHERE id = ?", (id,))
        .await?;
    Ok(true)
}

pub async fn get_smtp_settings(session: &Session) -> Result<SmtpSettingsRow> {
    let result = session
        .query_unpaged(
            "SELECT enabled, host, port, username, password, from_email, from_name,
                    use_tls, alert_to, notify_login_failure, notify_login,
                    notify_password_change, updated_at
             FROM smtp_settings WHERE id = 1",
            &[],
        )
        .await?
        .into_rows_result()?;
    let row = result
        .maybe_first_row::<(
            bool,
            String,
            i32,
            String,
            String,
            String,
            String,
            bool,
            String,
            bool,
            bool,
            bool,
            String,
        )>()?
        .context("smtp_settings row missing")?;
    Ok(SmtpSettingsRow {
        enabled: row.0,
        host: row.1,
        port: i64::from(row.2),
        username: row.3,
        password: row.4,
        from_email: row.5,
        from_name: row.6,
        use_tls: row.7,
        alert_to: row.8,
        notify_login_failure: row.9,
        notify_login: row.10,
        notify_password_change: row.11,
        updated_at: row.12,
    })
}

pub async fn update_smtp_settings(
    session: &Session,
    update: SmtpSettingsUpdate,
) -> Result<SmtpSettingsRow> {
    let current = get_smtp_settings(session).await?;
    let password = match update.password {
        None => current.password,
        Some(p) => p,
    };
    let updated_at = chrono::Utc::now().to_rfc3339();
    session
        .query_unpaged(
            "INSERT INTO smtp_settings (
                id, enabled, host, port, username, password, from_email, from_name,
                use_tls, alert_to, notify_login_failure, notify_login,
                notify_password_change, updated_at
             ) VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            (
                update.enabled,
                &update.host,
                update.port as i32,
                &update.username,
                &password,
                &update.from_email,
                &update.from_name,
                update.use_tls,
                &update.alert_to,
                update.notify_login_failure,
                update.notify_login,
                update.notify_password_change,
                &updated_at,
            ),
        )
        .await?;
    get_smtp_settings(session).await
}

pub async fn get_s3_settings(session: &Session) -> Result<S3SettingsRow> {
    let result = session
        .query_unpaged(
            "SELECT enabled, endpoint, region, bucket, prefix, access_key_id,
                    secret_access_key, force_path_style, updated_at
             FROM s3_settings WHERE id = 1",
            &[],
        )
        .await?
        .into_rows_result()?;
    let row = result
        .maybe_first_row::<(
            bool,
            String,
            String,
            String,
            String,
            String,
            String,
            bool,
            String,
        )>()?
        .context("s3_settings row missing")?;
    Ok(S3SettingsRow {
        enabled: row.0,
        endpoint: row.1,
        region: row.2,
        bucket: row.3,
        prefix: row.4,
        access_key_id: row.5,
        secret_access_key: row.6,
        force_path_style: row.7,
        updated_at: row.8,
    })
}

pub async fn update_s3_settings(
    session: &Session,
    update: S3SettingsUpdate,
) -> Result<S3SettingsRow> {
    let current = get_s3_settings(session).await?;
    let secret = match update.secret_access_key {
        None => current.secret_access_key,
        Some(s) => s,
    };
    let updated_at = chrono::Utc::now().to_rfc3339();
    session
        .query_unpaged(
            "INSERT INTO s3_settings (
                id, enabled, endpoint, region, bucket, prefix,
                access_key_id, secret_access_key, force_path_style, updated_at
             ) VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            (
                update.enabled,
                &update.endpoint,
                &update.region,
                &update.bucket,
                &update.prefix,
                &update.access_key_id,
                &secret,
                update.force_path_style,
                &updated_at,
            ),
        )
        .await?;
    get_s3_settings(session).await
}
