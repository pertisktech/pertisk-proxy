//! Admin config persistence: SQLite (default) or optional ScyllaDB / Cassandra.

mod schema;
#[cfg(feature = "scylla-db")]
mod migrate;
#[cfg(feature = "scylla-db")]
mod scylla_backend;
#[cfg(feature = "scylla-db")]
mod scylla_schema;

#[cfg(feature = "scylla-db")]
pub use migrate::{migrate_sqlite_to_scylla, MigrateStats};

use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(feature = "scylla-db")]
use std::sync::Arc;

use anyhow::Result;
use bcrypt::{hash, DEFAULT_COST};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tokio::task::spawn_blocking;
use tracing::info;
use uuid::Uuid;

pub use schema::init_schema;

pub const DEFAULT_ADMIN_USERNAME: &str = "admin";
pub const DEFAULT_ADMIN_PASSWORD: &str = "admin";

pub(crate) const PROXY_CONFIG_KEY: &str = "current";

#[cfg(feature = "scylla-db")]
async fn attach_sqlite_logs_session(
) -> Result<Arc<::scylla::client::session::Session>> {
    let cfg = crate::config::ScyllaDbConfig::from_env();
    scylla_backend::connect(&cfg).await
}

fn seed_admin_user(conn: &Connection) -> Result<()> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
    if count > 0 {
        return Ok(());
    }
    let id = Uuid::new_v4().to_string();
    let password_hash = hash(DEFAULT_ADMIN_PASSWORD, DEFAULT_COST)?;
    let created_at = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO users (id, username, password_hash, created_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![id, DEFAULT_ADMIN_USERNAME, password_hash, created_at],
    )?;
    info!(
        username = DEFAULT_ADMIN_USERNAME,
        "seeded default admin user (change password after first login)"
    );
    Ok(())
}

#[derive(Clone)]
enum DbBackend {
    Sqlite {
        path: PathBuf,
    },
    #[cfg(feature = "scylla-db")]
    Scylla {
        session: Arc<::scylla::client::session::Session>,
        data_dir: PathBuf,
    },
}

#[derive(Clone)]
pub struct Database {
    backend: DbBackend,
    /// Optional Scylla session for access-log warehouse when config backend is SQLite
    /// but `PERTISK_SCYLLA_*` is set (same cluster).
    #[cfg(feature = "scylla-db")]
    logs_session: Option<Arc<::scylla::client::session::Session>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsProviderRow {
    pub id: String,
    pub name: String,
    pub provider_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials: Option<HashMap<String, String>>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertificateRow {
    pub id: String,
    pub hosts: Vec<String>,
    pub source_type: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// Singleton SMTP / notification settings (`id = 1`). Password is never serialized to API clients.
#[derive(Debug, Clone)]
pub struct SmtpSettingsRow {
    pub enabled: bool,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub password: String,
    pub from_email: String,
    pub from_name: String,
    pub use_tls: bool,
    pub alert_to: String,
    pub notify_login_failure: bool,
    pub notify_login: bool,
    pub notify_password_change: bool,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct SmtpSettingsUpdate {
    pub enabled: bool,
    pub host: String,
    pub port: i64,
    pub username: String,
    /// `None` = leave unchanged; `Some("")` = clear; `Some(value)` = set.
    pub password: Option<String>,
    pub from_email: String,
    pub from_name: String,
    pub use_tls: bool,
    pub alert_to: String,
    pub notify_login_failure: bool,
    pub notify_login: bool,
    pub notify_password_change: bool,
}

/// Singleton S3 / object-storage settings (`id = 1`). Secret is never serialized to API clients.
#[derive(Debug, Clone)]
pub struct S3SettingsRow {
    pub enabled: bool,
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub force_path_style: bool,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct S3SettingsUpdate {
    pub enabled: bool,
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub access_key_id: String,
    /// `None` = leave unchanged; `Some("")` = clear; `Some(value)` = set.
    pub secret_access_key: Option<String>,
    pub force_path_style: bool,
}

/// Access-log storage backend for the Logs UI / warehouse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogStorageBackend {
    #[default]
    Memory,
    Scylla,
}

impl LogStorageBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Scylla => "scylla",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "scylla" | "scylladb" | "cassandra" => Self::Scylla,
            _ => Self::Memory,
        }
    }
}

/// Singleton logging settings (`id = 1`).
#[derive(Debug, Clone, Serialize)]
pub struct LoggingSettingsRow {
    pub enabled: bool,
    pub storage: LogStorageBackend,
    pub ttl_days: i32,
    pub updated_at: String,
    /// True when a Scylla session is available for access-log warehouse writes.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub scylla_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scylla_hosts: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scylla_keyspace: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LoggingSettingsUpdate {
    pub enabled: bool,
    pub storage: LogStorageBackend,
    pub ttl_days: i32,
}

impl Database {
    /// Open SQLite admin DB at `path` (legacy helper).
    pub fn open(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path)?;
        init_schema(&conn)?;
        seed_admin_user(&conn)?;
        Ok(Self {
            backend: DbBackend::Sqlite { path },
            #[cfg(feature = "scylla-db")]
            logs_session: None,
        })
    }

    /// Open admin DB from `PERTISK_DB_BACKEND` / Scylla / SQLite env.
    pub async fn open_from_env() -> Result<Self> {
        use crate::config::{resolve_db_backend, resolve_db_path, DbBackendKind, ScyllaDbConfig};

        match resolve_db_backend() {
            DbBackendKind::Sqlite => {
                let mut db = Self::open(resolve_db_path())?;
                #[cfg(feature = "scylla-db")]
                {
                    let hosts = std::env::var("PERTISK_SCYLLA_HOSTS").unwrap_or_default();
                    if !hosts.trim().is_empty() {
                        match attach_sqlite_logs_session().await {
                            Ok(session) => {
                                info!("attached Scylla session for access-log warehouse (SQLite config backend)");
                                db.logs_session = Some(session);
                            }
                            Err(err) => {
                                tracing::warn!(
                                    error = %err,
                                    "PERTISK_SCYLLA_HOSTS set but could not open logs session"
                                );
                            }
                        }
                    }
                }
                Ok(db)
            }
            #[cfg(feature = "scylla-db")]
            DbBackendKind::Scylla => {
                let cfg = ScyllaDbConfig::from_env();
                let session = scylla_backend::connect(&cfg).await?;
                let data_dir = crate::config::default_data_dir();
                std::fs::create_dir_all(&data_dir)?;
                // Sentinel path so `path().parent()` remains the data dir (certs/, etc.).
                let sentinel = data_dir.join("proxy.scylla");
                Ok(Self {
                    backend: DbBackend::Scylla {
                        session,
                        data_dir: sentinel,
                    },
                    logs_session: None,
                })
            }
            #[cfg(not(feature = "scylla-db"))]
            DbBackendKind::Scylla => {
                anyhow::bail!(
                    "PERTISK_DB_BACKEND=scylla but this binary was built without the scylla-db feature"
                )
            }
        }
    }

    /// SQLite file path, or a sentinel under the Scylla data dir.
    pub fn path(&self) -> &PathBuf {
        match &self.backend {
            DbBackend::Sqlite { path } => path,
            #[cfg(feature = "scylla-db")]
            DbBackend::Scylla { data_dir, .. } => data_dir,
        }
    }

    /// Directory for on-disk artifacts (certs PEM files, etc.).
    pub fn data_dir(&self) -> &Path {
        match &self.backend {
            DbBackend::Sqlite { path } => path.parent().unwrap_or_else(|| Path::new(".")),
            #[cfg(feature = "scylla-db")]
            DbBackend::Scylla { data_dir, .. } => {
                data_dir.parent().unwrap_or_else(|| Path::new("."))
            }
        }
    }

    pub fn is_scylla(&self) -> bool {
        match &self.backend {
            DbBackend::Sqlite { .. } => false,
            #[cfg(feature = "scylla-db")]
            DbBackend::Scylla { .. } => true,
        }
    }

    fn sqlite_path(&self) -> Result<PathBuf> {
        match &self.backend {
            DbBackend::Sqlite { path } => Ok(path.clone()),
            #[cfg(feature = "scylla-db")]
            DbBackend::Scylla { .. } => {
                anyhow::bail!("internal: SQLite path requested on Scylla backend")
            }
        }
    }

    /// Shared Scylla session for access-log warehouse (config backend or SQLite side session).
    #[cfg(feature = "scylla-db")]
    pub fn scylla_session_arc(&self) -> Option<Arc<::scylla::client::session::Session>> {
        match &self.backend {
            DbBackend::Scylla { session, .. } => Some(Arc::clone(session)),
            DbBackend::Sqlite { .. } => self.logs_session.clone(),
        }
    }

    #[cfg(feature = "scylla-db")]
    pub async fn query_access_logs(
        &self,
        buckets: &[String],
        limit: usize,
    ) -> Result<Vec<crate::log::ProxyLogEntry>> {
        let session = self.access_log_session()?;
        scylla_backend::query_access_logs(session, buckets, limit).await
    }

    #[cfg(feature = "scylla-db")]
    pub async fn insert_access_log_row(
        session: &::scylla::client::session::Session,
        bucket: &str,
        written_at: chrono::DateTime<chrono::Utc>,
        id: uuid::Uuid,
        entry: &crate::log::ProxyLogEntry,
    ) -> Result<()> {
        scylla_backend::insert_access_log(session, bucket, written_at, id, entry).await
    }

    #[cfg(feature = "scylla-db")]
    fn scylla_session(&self) -> Result<&::scylla::client::session::Session> {
        match &self.backend {
            DbBackend::Scylla { session, .. } => Ok(session.as_ref()),
            DbBackend::Sqlite { .. } => {
                anyhow::bail!("internal: Scylla session requested on SQLite backend")
            }
        }
    }

    #[cfg(feature = "scylla-db")]
    fn access_log_session(&self) -> Result<&::scylla::client::session::Session> {
        match &self.backend {
            DbBackend::Scylla { session, .. } => Ok(session.as_ref()),
            DbBackend::Sqlite { .. } => self
                .logs_session
                .as_ref()
                .map(|s| s.as_ref())
                .ok_or_else(|| anyhow::anyhow!("Scylla access-log session not available")),
        }
    }

    pub async fn get_user_password_hash(&self, username: &str) -> Result<Option<String>> {
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            return scylla_backend::get_user_password_hash(self.scylla_session()?, username).await;
        }
        let path = self.sqlite_path()?;
        let username = username.to_string();
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            let mut stmt =
                conn.prepare_cached("SELECT password_hash FROM users WHERE username = ?1")?;
            let mut rows = stmt.query(rusqlite::params![username])?;
            if let Some(row) = rows.next()? {
                Ok(Some(row.get(0)?))
            } else {
                Ok(None)
            }
        })
        .await?
    }

    /// Update password hash for an existing user. Returns false if the user does not exist.
    pub async fn update_user_password(&self, username: &str, new_password: &str) -> Result<bool> {
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            return scylla_backend::update_user_password(self.scylla_session()?, username, new_password)
                .await;
        }
        let path = self.sqlite_path()?;
        let username = username.to_string();
        let password_hash = hash(new_password, DEFAULT_COST)?;
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            let n = conn.execute(
                "UPDATE users SET password_hash = ?1 WHERE username = ?2",
                rusqlite::params![password_hash, username],
            )?;
            Ok::<_, anyhow::Error>(n > 0)
        })
        .await?
    }

    pub async fn insert_session(
        &self,
        token: &str,
        username: &str,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            return scylla_backend::insert_session(self.scylla_session()?, token, username, expires_at)
                .await;
        }
        let path = self.sqlite_path()?;
        let token = token.to_string();
        let username = username.to_string();
        let expires_at = expires_at.to_rfc3339();
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            conn.execute(
                "INSERT OR REPLACE INTO sessions (token, username, expires_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![token, username, expires_at],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await??;
        Ok(())
    }

    pub async fn get_session(
        &self,
        token: &str,
    ) -> Result<Option<(String, chrono::DateTime<chrono::Utc>)>> {
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            return scylla_backend::get_session(self.scylla_session()?, token).await;
        }
        let path = self.sqlite_path()?;
        let token = token.to_string();
        let now = chrono::Utc::now().to_rfc3339();
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            let mut stmt = conn.prepare_cached(
                "SELECT username, expires_at FROM sessions WHERE token = ?1 AND expires_at > ?2",
            )?;
            let mut rows = stmt.query(rusqlite::params![token, now])?;
            if let Some(row) = rows.next()? {
                let username: String = row.get(0)?;
                let expires_at: String = row.get(1)?;
                let expires_at = chrono::DateTime::parse_from_rfc3339(&expires_at)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now());
                Ok(Some((username, expires_at)))
            } else {
                Ok(None)
            }
        })
        .await?
    }

    pub async fn load_active_sessions(
        &self,
    ) -> Result<Vec<(String, String, chrono::DateTime<chrono::Utc>)>> {
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            return scylla_backend::load_active_sessions(self.scylla_session()?).await;
        }
        let path = self.sqlite_path()?;
        let now = chrono::Utc::now().to_rfc3339();
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            let mut stmt = conn.prepare_cached(
                "SELECT token, username, expires_at FROM sessions WHERE expires_at > ?1",
            )?;
            let mut rows = stmt.query(rusqlite::params![now])?;
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                let token: String = row.get(0)?;
                let username: String = row.get(1)?;
                let expires_at: String = row.get(2)?;
                let expires_at = chrono::DateTime::parse_from_rfc3339(&expires_at)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now());
                out.push((token, username, expires_at));
            }
            Ok(out)
        })
        .await?
    }

    pub async fn get_proxy_config(&self) -> Result<Option<crate::proxy_config::Config>> {
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            return scylla_backend::get_proxy_config(self.scylla_session()?).await;
        }
        if let Some(json) = self.get_config_json(PROXY_CONFIG_KEY).await? {
            return Ok(Some(serde_json::from_str(&json)?));
        }
        if let Some(json) = self.get_config_json("sites").await? {
            if let Ok(cfg) = serde_json::from_str::<crate::proxy_config::Config>(&json) {
                self.save_proxy_config(&cfg).await?;
                return Ok(Some(cfg));
            }
        }
        Ok(None)
    }

    pub async fn save_proxy_config(&self, config: &crate::proxy_config::Config) -> Result<()> {
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            return scylla_backend::save_proxy_config(self.scylla_session()?, config).await;
        }
        let json = serde_json::to_string(config)?;
        self.save_config_json(PROXY_CONFIG_KEY, &json).await
    }

    pub async fn get_config_json(&self, key: &str) -> Result<Option<String>> {
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            return scylla_backend::get_config_json(self.scylla_session()?, key).await;
        }
        let path = self.sqlite_path()?;
        let key = key.to_string();
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            let mut stmt = conn.prepare_cached("SELECT value FROM proxy_config WHERE key = ?1")?;
            let mut rows = stmt.query(rusqlite::params![key])?;
            if let Some(row) = rows.next()? {
                Ok(Some(row.get::<_, String>(0)?))
            } else {
                Ok(None)
            }
        })
        .await?
    }

    pub async fn save_config_json(&self, key: &str, value: &str) -> Result<()> {
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            return scylla_backend::save_config_json(self.scylla_session()?, key, value).await;
        }
        let path = self.sqlite_path()?;
        let key = key.to_string();
        let value = value.to_string();
        let updated_at = chrono::Utc::now().to_rfc3339();
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            conn.execute(
                "INSERT INTO proxy_config (key, value, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = ?3",
                rusqlite::params![key, value, updated_at],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await??;
        Ok(())
    }

    pub async fn get_runtime_env_overrides(&self) -> Result<HashMap<String, String>> {
        let Some(json) = self
            .get_config_json(crate::runtime_env::RUNTIME_ENV_KEY)
            .await?
        else {
            return Ok(HashMap::new());
        };
        crate::runtime_env::parse_overrides_json(&json)
    }

    pub async fn save_runtime_env_overrides(
        &self,
        overrides: &HashMap<String, String>,
    ) -> Result<()> {
        let sanitized = crate::runtime_env::sanitize_overrides(overrides.clone());
        let json = serde_json::to_string(&sanitized)?;
        self.save_config_json(crate::runtime_env::RUNTIME_ENV_KEY, &json)
            .await
    }

    pub async fn list_dns_providers(&self) -> Result<Vec<DnsProviderRow>> {
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            return scylla_backend::list_dns_providers(self.scylla_session()?).await;
        }
        let path = self.sqlite_path()?;
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            let mut stmt = conn.prepare_cached(
                "SELECT id, name, provider_type, created_at FROM dns_providers ORDER BY created_at",
            )?;
            let mut out = Vec::new();
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                out.push(DnsProviderRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    provider_type: row.get(2)?,
                    credentials: None,
                    created_at: row.get(3)?,
                });
            }
            Ok(out)
        })
        .await?
    }

    pub async fn get_dns_provider(&self, id: &str) -> Result<Option<DnsProviderRow>> {
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            return scylla_backend::get_dns_provider(self.scylla_session()?, id).await;
        }
        let path = self.sqlite_path()?;
        let id = id.to_string();
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            let mut stmt = conn.prepare_cached(
                "SELECT id, name, provider_type, credentials, created_at FROM dns_providers WHERE id = ?1",
            )?;
            let mut rows = stmt.query(rusqlite::params![id])?;
            if let Some(row) = rows.next()? {
                let credentials: Option<String> = row.get(3)?;
                Ok(Some(DnsProviderRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    provider_type: row.get(2)?,
                    credentials: credentials.and_then(|s| serde_json::from_str(&s).ok()),
                    created_at: row.get(4)?,
                }))
            } else {
                Ok(None)
            }
        })
        .await?
    }

    pub async fn create_dns_provider(
        &self,
        name: String,
        provider_type: String,
        credentials: Option<HashMap<String, String>>,
    ) -> Result<String> {
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            return scylla_backend::create_dns_provider(
                self.scylla_session()?,
                name,
                provider_type,
                credentials,
            )
            .await;
        }
        let id = Uuid::new_v4().to_string();
        let path = self.sqlite_path()?;
        let cred_json = credentials
            .as_ref()
            .and_then(|c| serde_json::to_string(c).ok());
        let created_at = chrono::Utc::now().to_rfc3339();
        let id_for_insert = id.clone();
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            conn.execute(
                "INSERT INTO dns_providers (id, name, provider_type, credentials, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![id_for_insert, name, provider_type, cred_json, created_at],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await??;
        Ok(id)
    }

    pub async fn put_dns_provider(
        &self,
        id: &str,
        name: String,
        provider_type: String,
        credentials: Option<HashMap<String, String>>,
    ) -> Result<bool> {
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            return scylla_backend::put_dns_provider(
                self.scylla_session()?,
                id,
                name,
                provider_type,
                credentials,
            )
            .await;
        }
        let path = self.sqlite_path()?;
        let id = id.to_string();
        let cred_json = credentials
            .as_ref()
            .and_then(|c| serde_json::to_string(c).ok());
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            let n = conn.execute(
                "UPDATE dns_providers SET name = ?1, provider_type = ?2, credentials = ?3 WHERE id = ?4",
                rusqlite::params![name, provider_type, cred_json, id],
            )?;
            Ok(n > 0)
        })
        .await?
    }

    pub async fn delete_dns_provider(&self, id: &str) -> Result<bool> {
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            return scylla_backend::delete_dns_provider(self.scylla_session()?, id).await;
        }
        let path = self.sqlite_path()?;
        let id = id.to_string();
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            let n = conn.execute(
                "DELETE FROM dns_providers WHERE id = ?1",
                rusqlite::params![id],
            )?;
            Ok(n > 0)
        })
        .await?
    }

    pub async fn list_certificates(&self) -> Result<Vec<CertificateRow>> {
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            return scylla_backend::list_certificates(self.scylla_session()?).await;
        }
        let path = self.sqlite_path()?;
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            let mut stmt = conn.prepare_cached(
                "SELECT id, hosts, source_type, created_at, expires_at, cert_pem FROM certificates ORDER BY created_at",
            )?;
            let mut out = Vec::new();
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let id: String = row.get(0)?;
                let mut expires_at: Option<String> = row.get::<_, Option<String>>(4).ok().flatten();
                if expires_at.is_none() {
                    if let Ok(cert_pem) = row.get::<_, String>(5) {
                        if let Some(parsed) = cert_expiry_from_pem(cert_pem.as_bytes()) {
                            expires_at = Some(parsed.clone());
                            let _ = conn.execute(
                                "UPDATE certificates SET expires_at = ?1 WHERE id = ?2",
                                rusqlite::params![parsed, id],
                            );
                        }
                    }
                }
                out.push(CertificateRow {
                    id,
                    hosts: serde_json::from_str(row.get::<_, String>(1)?.as_str())
                        .unwrap_or_default(),
                    source_type: row.get(2)?,
                    created_at: row.get(3)?,
                    expires_at,
                });
            }
            Ok(out)
        })
        .await?
    }

    pub async fn get_all_certificates_for_store(
        &self,
    ) -> Result<Vec<(String, Vec<String>, Vec<u8>, Vec<u8>)>> {
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            return scylla_backend::get_all_certificates_for_store(self.scylla_session()?).await;
        }
        let path = self.sqlite_path()?;
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            let mut stmt =
                conn.prepare_cached("SELECT id, hosts, cert_pem, key_pem FROM certificates")?;
            let mut out = Vec::new();
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let id: String = row.get(0)?;
                let hosts: Vec<String> =
                    serde_json::from_str(row.get::<_, String>(1)?.as_str()).unwrap_or_default();
                let cert_pem: String = row.get(2)?;
                let key_pem: String = row.get(3)?;
                out.push((id, hosts, cert_pem.into_bytes(), key_pem.into_bytes()));
            }
            Ok(out)
        })
        .await?
    }

    pub async fn add_certificate(
        &self,
        hosts: Vec<String>,
        cert_pem: Vec<u8>,
        key_pem: Vec<u8>,
        source_type: &str,
    ) -> Result<String> {
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            return scylla_backend::add_certificate(
                self.scylla_session()?,
                hosts,
                cert_pem,
                key_pem,
                source_type,
            )
            .await;
        }
        let expires_at = cert_expiry_from_pem(&cert_pem);
        let id = Uuid::new_v4().to_string();
        let path = self.sqlite_path()?;
        let hosts_json = serde_json::to_string(&hosts)?;
        let cert_str = String::from_utf8(cert_pem)?;
        let key_str = String::from_utf8(key_pem)?;
        let source_type = source_type.to_string();
        let created_at = chrono::Utc::now().to_rfc3339();
        let id_for_insert = id.clone();
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            conn.execute(
                "INSERT INTO certificates (id, hosts, cert_pem, key_pem, source_type, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![id_for_insert, hosts_json, cert_str, key_str, source_type, created_at, expires_at],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await??;
        Ok(id)
    }

    pub async fn delete_certificate(&self, id: &str) -> Result<bool> {
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            return scylla_backend::delete_certificate(self.scylla_session()?, id).await;
        }
        let path = self.sqlite_path()?;
        let id = id.to_string();
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            let n = conn.execute(
                "DELETE FROM certificates WHERE id = ?1",
                rusqlite::params![id],
            )?;
            Ok(n > 0)
        })
        .await?
    }

    pub async fn get_smtp_settings(&self) -> Result<SmtpSettingsRow> {
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            return scylla_backend::get_smtp_settings(self.scylla_session()?).await;
        }
        let path = self.sqlite_path()?;
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            conn.query_row(
                "SELECT enabled, host, port, username, password, from_email, from_name,
                        use_tls, alert_to, notify_login_failure, notify_login,
                        notify_password_change, updated_at
                 FROM smtp_settings WHERE id = 1",
                [],
                |row| {
                    Ok(SmtpSettingsRow {
                        enabled: row.get::<_, i64>(0)? != 0,
                        host: row.get(1)?,
                        port: row.get(2)?,
                        username: row.get(3)?,
                        password: row.get(4)?,
                        from_email: row.get(5)?,
                        from_name: row.get(6)?,
                        use_tls: row.get::<_, i64>(7)? != 0,
                        alert_to: row.get(8)?,
                        notify_login_failure: row.get::<_, i64>(9)? != 0,
                        notify_login: row.get::<_, i64>(10)? != 0,
                        notify_password_change: row.get::<_, i64>(11)? != 0,
                        updated_at: row.get(12)?,
                    })
                },
            )
            .map_err(Into::into)
        })
        .await?
    }

    pub async fn update_smtp_settings(
        &self,
        update: SmtpSettingsUpdate,
    ) -> Result<SmtpSettingsRow> {
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            return scylla_backend::update_smtp_settings(self.scylla_session()?, update).await;
        }
        let path = self.sqlite_path()?;
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            let updated_at = chrono::Utc::now().to_rfc3339();
            match update.password {
                None => {
                    conn.execute(
                        "UPDATE smtp_settings SET
                            enabled = ?1, host = ?2, port = ?3, username = ?4,
                            from_email = ?5, from_name = ?6, use_tls = ?7,
                            alert_to = ?8, notify_login_failure = ?9, notify_login = ?10,
                            notify_password_change = ?11, updated_at = ?12
                         WHERE id = 1",
                        rusqlite::params![
                            update.enabled as i64,
                            update.host,
                            update.port,
                            update.username,
                            update.from_email,
                            update.from_name,
                            update.use_tls as i64,
                            update.alert_to,
                            update.notify_login_failure as i64,
                            update.notify_login as i64,
                            update.notify_password_change as i64,
                            updated_at,
                        ],
                    )?;
                }
                Some(password) => {
                    conn.execute(
                        "UPDATE smtp_settings SET
                            enabled = ?1, host = ?2, port = ?3, username = ?4, password = ?5,
                            from_email = ?6, from_name = ?7, use_tls = ?8,
                            alert_to = ?9, notify_login_failure = ?10, notify_login = ?11,
                            notify_password_change = ?12, updated_at = ?13
                         WHERE id = 1",
                        rusqlite::params![
                            update.enabled as i64,
                            update.host,
                            update.port,
                            update.username,
                            password,
                            update.from_email,
                            update.from_name,
                            update.use_tls as i64,
                            update.alert_to,
                            update.notify_login_failure as i64,
                            update.notify_login as i64,
                            update.notify_password_change as i64,
                            updated_at,
                        ],
                    )?;
                }
            }
            conn.query_row(
                "SELECT enabled, host, port, username, password, from_email, from_name,
                        use_tls, alert_to, notify_login_failure, notify_login,
                        notify_password_change, updated_at
                 FROM smtp_settings WHERE id = 1",
                [],
                |row| {
                    Ok(SmtpSettingsRow {
                        enabled: row.get::<_, i64>(0)? != 0,
                        host: row.get(1)?,
                        port: row.get(2)?,
                        username: row.get(3)?,
                        password: row.get(4)?,
                        from_email: row.get(5)?,
                        from_name: row.get(6)?,
                        use_tls: row.get::<_, i64>(7)? != 0,
                        alert_to: row.get(8)?,
                        notify_login_failure: row.get::<_, i64>(9)? != 0,
                        notify_login: row.get::<_, i64>(10)? != 0,
                        notify_password_change: row.get::<_, i64>(11)? != 0,
                        updated_at: row.get(12)?,
                    })
                },
            )
            .map_err(Into::into)
        })
        .await?
    }

    pub async fn get_s3_settings(&self) -> Result<S3SettingsRow> {
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            return scylla_backend::get_s3_settings(self.scylla_session()?).await;
        }
        let path = self.sqlite_path()?;
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            conn.query_row(
                "SELECT enabled, endpoint, region, bucket, prefix, access_key_id,
                        secret_access_key, force_path_style, updated_at
                 FROM s3_settings WHERE id = 1",
                [],
                |row| {
                    Ok(S3SettingsRow {
                        enabled: row.get::<_, i64>(0)? != 0,
                        endpoint: row.get(1)?,
                        region: row.get(2)?,
                        bucket: row.get(3)?,
                        prefix: row.get(4)?,
                        access_key_id: row.get(5)?,
                        secret_access_key: row.get(6)?,
                        force_path_style: row.get::<_, i64>(7)? != 0,
                        updated_at: row.get(8)?,
                    })
                },
            )
            .map_err(Into::into)
        })
        .await?
    }

    pub async fn update_s3_settings(&self, update: S3SettingsUpdate) -> Result<S3SettingsRow> {
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            return scylla_backend::update_s3_settings(self.scylla_session()?, update).await;
        }
        let path = self.sqlite_path()?;
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            let updated_at = chrono::Utc::now().to_rfc3339();
            match update.secret_access_key {
                None => {
                    conn.execute(
                        "UPDATE s3_settings SET
                            enabled = ?1, endpoint = ?2, region = ?3, bucket = ?4, prefix = ?5,
                            access_key_id = ?6, force_path_style = ?7, updated_at = ?8
                         WHERE id = 1",
                        rusqlite::params![
                            update.enabled as i64,
                            update.endpoint,
                            update.region,
                            update.bucket,
                            update.prefix,
                            update.access_key_id,
                            update.force_path_style as i64,
                            updated_at,
                        ],
                    )?;
                }
                Some(secret) => {
                    conn.execute(
                        "UPDATE s3_settings SET
                            enabled = ?1, endpoint = ?2, region = ?3, bucket = ?4, prefix = ?5,
                            access_key_id = ?6, secret_access_key = ?7, force_path_style = ?8,
                            updated_at = ?9
                         WHERE id = 1",
                        rusqlite::params![
                            update.enabled as i64,
                            update.endpoint,
                            update.region,
                            update.bucket,
                            update.prefix,
                            update.access_key_id,
                            secret,
                            update.force_path_style as i64,
                            updated_at,
                        ],
                    )?;
                }
            }
            conn.query_row(
                "SELECT enabled, endpoint, region, bucket, prefix, access_key_id,
                        secret_access_key, force_path_style, updated_at
                 FROM s3_settings WHERE id = 1",
                [],
                |row| {
                    Ok(S3SettingsRow {
                        enabled: row.get::<_, i64>(0)? != 0,
                        endpoint: row.get(1)?,
                        region: row.get(2)?,
                        bucket: row.get(3)?,
                        prefix: row.get(4)?,
                        access_key_id: row.get(5)?,
                        secret_access_key: row.get(6)?,
                        force_path_style: row.get::<_, i64>(7)? != 0,
                        updated_at: row.get(8)?,
                    })
                },
            )
            .map_err(Into::into)
        })
        .await?
    }

    pub fn logging_scylla_meta(&self) -> (bool, Option<String>, Option<String>) {
        #[cfg(feature = "scylla-db")]
        {
            let available = self.scylla_session_arc().is_some();
            if available {
                let hosts = std::env::var("PERTISK_SCYLLA_HOSTS").ok();
                let keyspace = std::env::var("PERTISK_SCYLLA_KEYSPACE")
                    .ok()
                    .or_else(|| Some("pertisk_proxy".into()));
                return (true, hosts, keyspace);
            }
            return (false, None, None);
        }
        #[cfg(not(feature = "scylla-db"))]
        {
            (false, None, None)
        }
    }

    pub async fn get_logging_settings(&self) -> Result<LoggingSettingsRow> {
        let (scylla_available, scylla_hosts, scylla_keyspace) = self.logging_scylla_meta();
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            let mut row = scylla_backend::get_logging_settings(self.scylla_session()?).await?;
            row.scylla_available = scylla_available;
            row.scylla_hosts = scylla_hosts;
            row.scylla_keyspace = scylla_keyspace;
            return Ok(row);
        }
        let path = self.sqlite_path()?;
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            conn.query_row(
                "SELECT enabled, storage, ttl_days, updated_at FROM logging_settings WHERE id = 1",
                [],
                |row| {
                    Ok(LoggingSettingsRow {
                        enabled: row.get::<_, i64>(0)? != 0,
                        storage: LogStorageBackend::parse(&row.get::<_, String>(1)?),
                        ttl_days: row.get::<_, i64>(2)? as i32,
                        updated_at: row.get(3)?,
                        scylla_available,
                        scylla_hosts: scylla_hosts.clone(),
                        scylla_keyspace: scylla_keyspace.clone(),
                    })
                },
            )
            .map_err(Into::into)
        })
        .await?
    }

    pub async fn update_logging_settings(
        &self,
        update: LoggingSettingsUpdate,
    ) -> Result<LoggingSettingsRow> {
        let ttl_days = update.ttl_days.clamp(1, 365);
        #[cfg(feature = "scylla-db")]
        if self.is_scylla() {
            let session = self.scylla_session()?;
            let row = scylla_backend::update_logging_settings(
                session,
                LoggingSettingsUpdate {
                    enabled: update.enabled,
                    storage: update.storage,
                    ttl_days,
                },
            )
            .await?;
            if update.storage == LogStorageBackend::Scylla {
                let ttl_secs = ttl_days.saturating_mul(86400);
                if let Err(err) =
                    scylla_backend::set_access_logs_ttl_secs(session, ttl_secs).await
                {
                    tracing::warn!(error = %err, "failed to alter access_logs TTL");
                }
            }
            let (scylla_available, scylla_hosts, scylla_keyspace) = self.logging_scylla_meta();
            return Ok(LoggingSettingsRow {
                scylla_available,
                scylla_hosts,
                scylla_keyspace,
                ..row
            });
        }
        if update.storage == LogStorageBackend::Scylla && !self.logging_scylla_meta().0 {
            anyhow::bail!("ScyllaDB is not configured (set PERTISK_SCYLLA_HOSTS)");
        }
        let path = self.sqlite_path()?;
        let storage = update.storage.as_str().to_string();
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            let updated_at = chrono::Utc::now().to_rfc3339();
            conn.execute(
                "UPDATE logging_settings SET enabled = ?1, storage = ?2, ttl_days = ?3, updated_at = ?4
                 WHERE id = 1",
                rusqlite::params![update.enabled as i64, storage, ttl_days, updated_at],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await??;
        #[cfg(feature = "scylla-db")]
        if update.storage == LogStorageBackend::Scylla {
            if let Some(session) = self.scylla_session_arc() {
                let ttl_secs = ttl_days.saturating_mul(86400);
                if let Err(err) =
                    scylla_backend::set_access_logs_ttl_secs(session.as_ref(), ttl_secs).await
                {
                    tracing::warn!(error = %err, "failed to alter access_logs TTL");
                }
            }
        }
        self.get_logging_settings().await
    }
}

/// Parse first certificate in PEM and return its not_after as RFC2822 string.
pub fn cert_expiry_from_pem(pem: &[u8]) -> Option<String> {
    use std::io::Cursor;
    use x509_parser::pem::Pem;
    use x509_parser::prelude::{FromDer, X509Certificate};
    let mut reader = Cursor::new(pem);
    let (pem, _) = Pem::read(&mut reader).ok()?;
    let (_, cert) = X509Certificate::from_der(&pem.contents).ok()?;
    cert.validity()
        .not_after
        .to_rfc2822()
        .ok()
        .map(|s| s.to_string())
}

/// Parse CN and DNS SAN names from the first certificate in PEM.
pub fn cert_hosts_from_pem(pem: &[u8]) -> Vec<String> {
    use std::collections::HashSet;
    use std::io::Cursor;
    use x509_parser::pem::Pem;
    use x509_parser::prelude::{FromDer, GeneralName, X509Certificate};
    let mut reader = Cursor::new(pem);
    let (pem, _) = match Pem::read(&mut reader) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let (_, cert) = match X509Certificate::from_der(&pem.contents) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut hosts = HashSet::new();
    for cn in cert
        .subject()
        .iter_common_name()
        .flat_map(|cn| cn.as_str().ok())
    {
        let cn = cn.trim();
        if !cn.is_empty() {
            hosts.insert(cn.to_string());
        }
    }
    if let Ok(Some(san)) = cert.subject_alternative_name() {
        for name in san.value.general_names.iter() {
            if let GeneralName::DNSName(dns) = name {
                let host = dns.trim();
                if !host.is_empty() {
                    hosts.insert(host.to_string());
                }
            }
        }
    }
    hosts.into_iter().collect()
}

#[derive(Debug, Clone, Default)]
pub struct CertValidity {
    pub issued_at: Option<String>,
    pub expires_at: Option<String>,
}

/// Parse first certificate in PEM and return issued/expiry timestamps.
pub fn cert_validity_from_pem(cert_pem: &[u8]) -> CertValidity {
    CertValidity {
        issued_at: None,
        expires_at: cert_expiry_from_pem(cert_pem),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn logging_settings_round_trip_sqlite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let db = Database::open(path).unwrap();
        let row = db.get_logging_settings().await.unwrap();
        assert!(!row.enabled);
        assert_eq!(row.storage, LogStorageBackend::Memory);
        assert_eq!(row.ttl_days, 7);

        let updated = db
            .update_logging_settings(LoggingSettingsUpdate {
                enabled: true,
                storage: LogStorageBackend::Memory,
                ttl_days: 14,
            })
            .await
            .unwrap();
        assert!(updated.enabled);
        assert_eq!(updated.storage, LogStorageBackend::Memory);
        assert_eq!(updated.ttl_days, 14);

        let again = db.get_logging_settings().await.unwrap();
        assert!(again.enabled);
        assert_eq!(again.ttl_days, 14);
    }

    #[test]
    fn log_storage_backend_parse() {
        assert_eq!(LogStorageBackend::parse("scylla"), LogStorageBackend::Scylla);
        assert_eq!(LogStorageBackend::parse("memory"), LogStorageBackend::Memory);
        assert_eq!(LogStorageBackend::parse("other"), LogStorageBackend::Memory);
    }
}
