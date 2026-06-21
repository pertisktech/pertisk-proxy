//! SQLite persistence for sites config, certificates, DNS providers, and admin users.

mod schema;

use std::collections::HashMap;
use std::path::PathBuf;

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
pub struct Database {
    path: PathBuf,
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

const PROXY_CONFIG_KEY: &str = "current";

impl Database {
    pub fn open(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path)?;
        init_schema(&conn)?;
        seed_admin_user(&conn)?;
        Ok(Self { path })
    }

    pub async fn get_user_password_hash(&self, username: &str) -> Result<Option<String>> {
        let path = self.path.clone();
        let username = username.to_string();
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            let mut stmt = conn.prepare_cached(
                "SELECT password_hash FROM users WHERE username = ?1",
            )?;
            let mut rows = stmt.query(rusqlite::params![username])?;
            if let Some(row) = rows.next()? {
                Ok(Some(row.get(0)?))
            } else {
                Ok(None)
            }
        })
        .await?
    }

    pub async fn insert_session(
        &self,
        token: &str,
        username: &str,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        let path = self.path.clone();
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
        let path = self.path.clone();
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
        let path = self.path.clone();
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

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub async fn get_proxy_config(&self) -> Result<Option<crate::proxy_config::Config>> {
        if let Some(json) = self.get_config_json(PROXY_CONFIG_KEY).await? {
            return Ok(Some(serde_json::from_str(&json)?));
        }
        // Migrate legacy "sites" JSON blob from earlier admin iteration.
        if let Some(json) = self.get_config_json("sites").await? {
            if let Ok(cfg) = serde_json::from_str::<crate::proxy_config::Config>(&json) {
                self.save_proxy_config(&cfg).await?;
                return Ok(Some(cfg));
            }
        }
        Ok(None)
    }

    pub async fn save_proxy_config(&self, config: &crate::proxy_config::Config) -> Result<()> {
        let json = serde_json::to_string(config)?;
        self.save_config_json(PROXY_CONFIG_KEY, &json).await
    }

    pub async fn get_config_json(&self, key: &str) -> Result<Option<String>> {
        let path = self.path.clone();
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
        let path = self.path.clone();
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

    pub async fn list_dns_providers(&self) -> Result<Vec<DnsProviderRow>> {
        let path = self.path.clone();
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
        let path = self.path.clone();
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
        let id = Uuid::new_v4().to_string();
        let path = self.path.clone();
        let cred_json = credentials.as_ref().and_then(|c| serde_json::to_string(c).ok());
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
        let path = self.path.clone();
        let id = id.to_string();
        let cred_json = credentials.as_ref().and_then(|c| serde_json::to_string(c).ok());
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
        let path = self.path.clone();
        let id = id.to_string();
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            let n = conn.execute("DELETE FROM dns_providers WHERE id = ?1", rusqlite::params![id])?;
            Ok(n > 0)
        })
        .await?
    }

    pub async fn list_certificates(&self) -> Result<Vec<CertificateRow>> {
        let path = self.path.clone();
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
                    hosts: serde_json::from_str(row.get::<_, String>(1)?.as_str()).unwrap_or_default(),
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
        let path = self.path.clone();
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
        let expires_at = cert_expiry_from_pem(&cert_pem);
        let id = Uuid::new_v4().to_string();
        let path = self.path.clone();
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
        let path = self.path.clone();
        let id = id.to_string();
        spawn_blocking(move || {
            let conn = Connection::open(path)?;
            let n = conn.execute("DELETE FROM certificates WHERE id = ?1", rusqlite::params![id])?;
            Ok(n > 0)
        })
        .await?
    }
}

fn cert_expiry_from_pem(pem: &[u8]) -> Option<String> {
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
