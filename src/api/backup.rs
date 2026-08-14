//! Backup export/restore for proxy mode (SQLite + TLS) and ingress mode (Kubernetes + Helm).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;

use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::info;

use super::AdminState;
use crate::db::{DnsProviderRow, S3SettingsUpdate};
use crate::log::ProxyLogEntry;
use crate::proxy_config::Config;
use crate::storage::s3 as s3_storage;

#[derive(Debug, Deserialize)]
pub struct BackupExportQuery {
    pub namespace: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RestoreBackupBody {
    pub data: String,
    #[serde(default)]
    pub merge: bool,
}

#[derive(Debug, Serialize)]
pub struct RestoreBackupResponse {
    pub message: String,
    pub restored_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct S3SettingsResponse {
    pub enabled: bool,
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub access_key_id: String,
    pub has_secret_access_key: bool,
    pub force_path_style: bool,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateS3SettingsRequest {
    pub enabled: bool,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub bucket: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub access_key_id: String,
    #[serde(default)]
    pub secret_access_key: Option<String>,
    #[serde(default)]
    pub force_path_style: bool,
}

#[derive(Debug, Deserialize)]
pub struct ExportS3Body {
    #[serde(default)]
    pub namespace: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExportS3Response {
    pub ok: bool,
    pub bucket: String,
    pub key: String,
}

#[derive(Debug, Serialize)]
pub struct OkResponse {
    pub ok: bool,
}

struct BuiltBackup {
    filename: String,
    content_type: &'static str,
    body: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProxyBackup {
    config: Config,
    certificates: Vec<BackupCertificate>,
    dns_providers: Vec<DnsProviderRow>,
    created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BackupCertificate {
    hosts: Vec<String>,
    cert_pem: String,
    key_pem: String,
    source_type: String,
}

#[cfg(feature = "ingress")]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HelmBackupSnapshot {
    release: String,
    namespace: String,
    values: String,
    history: serde_json::Value,
}

#[cfg(feature = "ingress")]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct IngressBackup {
    ingresses: Vec<serde_json::Value>,
    secrets: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    gateways: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    httproutes: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    helm: Option<HelmBackupSnapshot>,
    created_at: String,
}

pub async fn backup_export(
    State(state): State<AdminState>,
    headers: HeaderMap,
    _query: Query<BackupExportQuery>,
) -> Response {
    if !super::is_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    if state.viewer_mode {
        #[cfg(feature = "ingress")]
        {
            return ingress_backup_export(&state, _query.namespace.as_deref()).await;
        }
        #[cfg(not(feature = "ingress"))]
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Ingress mode not available" })),
            )
                .into_response();
        }
    }

    proxy_backup_export(&state).await
}

pub async fn backup_restore(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Json(body): Json<RestoreBackupBody>,
) -> Response {
    if !super::is_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    if state.viewer_mode {
        #[cfg(feature = "ingress")]
        {
            return ingress_backup_restore(&state, body).await;
        }
        #[cfg(not(feature = "ingress"))]
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Ingress mode not available" })),
            )
                .into_response();
        }
    }

    proxy_backup_restore(&state, body).await
}

fn require_db(state: &AdminState) -> Result<&crate::db::Database, Response> {
    state.db.as_deref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "database not configured" })),
        )
            .into_response()
    })
}

fn row_to_s3_response(row: &crate::db::S3SettingsRow) -> S3SettingsResponse {
    S3SettingsResponse {
        enabled: row.enabled,
        endpoint: row.endpoint.clone(),
        region: row.region.clone(),
        bucket: row.bucket.clone(),
        prefix: row.prefix.clone(),
        access_key_id: row.access_key_id.clone(),
        has_secret_access_key: !row.secret_access_key.trim().is_empty(),
        force_path_style: row.force_path_style,
        updated_at: row.updated_at.clone(),
    }
}

pub async fn s3_settings_get(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    if !super::is_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let db = match require_db(&state) {
        Ok(db) => db,
        Err(r) => return r,
    };
    match db.get_s3_settings().await {
        Ok(row) => Json(row_to_s3_response(&row)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn s3_settings_put(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Json(body): Json<UpdateS3SettingsRequest>,
) -> Response {
    if !super::is_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let db = match require_db(&state) {
        Ok(db) => db,
        Err(r) => return r,
    };
    match db
        .update_s3_settings(S3SettingsUpdate {
            enabled: body.enabled,
            endpoint: body.endpoint.trim().to_string(),
            region: body.region.trim().to_string(),
            bucket: body.bucket.trim().to_string(),
            prefix: body.prefix.trim().to_string(),
            access_key_id: body.access_key_id.trim().to_string(),
            secret_access_key: body.secret_access_key,
            force_path_style: body.force_path_style,
        })
        .await
    {
        Ok(row) => Json(row_to_s3_response(&row)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn s3_settings_test(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    if !super::is_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let db = match require_db(&state) {
        Ok(db) => db,
        Err(r) => return r,
    };
    let settings = match db.get_s3_settings().await {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    match s3_storage::test_connection(&settings).await {
        Ok(()) => Json(OkResponse { ok: true }).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn backup_export_s3(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Json(body): Json<ExportS3Body>,
) -> Response {
    if !super::is_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let namespace = body.namespace;
    let db = match require_db(&state) {
        Ok(db) => db,
        Err(r) => return r,
    };
    let settings = match db.get_s3_settings().await {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    if !settings.enabled {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "S3 backup is disabled" })),
        )
            .into_response();
    }
    if settings.bucket.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "S3 bucket is not configured" })),
        )
            .into_response();
    }

    let built = if state.viewer_mode {
        #[cfg(feature = "ingress")]
        {
            match build_ingress_backup(&state, namespace.as_deref()).await {
                Ok(b) => b,
                Err(r) => return r,
            }
        }
        #[cfg(not(feature = "ingress"))]
        {
            let _ = namespace;
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Ingress mode not available" })),
            )
                .into_response();
        }
    } else {
        let _ = namespace;
        match build_proxy_backup(&state).await {
            Ok(b) => b,
            Err(r) => return r,
        }
    };

    let key = s3_storage::object_key(&settings.prefix, &built.filename);
    match s3_storage::put_object(&settings, &key, built.body, built.content_type).await {
        Ok(()) => Json(ExportS3Response {
            ok: true,
            bucket: settings.bucket.trim().to_string(),
            key,
        })
        .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn proxy_backup_export(state: &AdminState) -> Response {
    match build_proxy_backup(state).await {
        Ok(built) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, built.content_type)
            .header(
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", built.filename),
            )
            .body(Body::from(built.body))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        Err(r) => r,
    }
}

async fn build_proxy_backup(state: &AdminState) -> Result<BuiltBackup, Response> {
    let Some(db) = &state.db else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Database not configured" })),
        )
            .into_response());
    };

    let config = state.runtime_config.read().await.clone();
    let pem_by_id: HashMap<String, (Vec<String>, Vec<u8>, Vec<u8>)> = match
        db.get_all_certificates_for_store().await
    {
        Ok(rows) => rows
            .into_iter()
            .map(|(id, hosts, cert_pem, key_pem)| (id, (hosts, cert_pem, key_pem)))
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "backup: failed to read certificate PEM data");
            HashMap::new()
        }
    };

    let certificates = match db.list_certificates().await {
        Ok(rows) => rows
            .into_iter()
            .filter_map(|row| {
                let (hosts, cert_pem, key_pem) = pem_by_id.get(&row.id)?;
                Some(BackupCertificate {
                    hosts: hosts.clone(),
                    cert_pem: String::from_utf8_lossy(cert_pem).into_owned(),
                    key_pem: String::from_utf8_lossy(key_pem).into_owned(),
                    source_type: row.source_type,
                })
            })
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "backup: failed to list certificates");
            Vec::new()
        }
    };

    let dns_providers = match db.list_dns_providers().await {
        Ok(providers) => providers
            .into_iter()
            .map(|mut p| {
                p.credentials = None;
                p
            })
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "backup: failed to list DNS providers");
            Vec::new()
        }
    };

    let backup = ProxyBackup {
        config,
        certificates,
        dns_providers,
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    let filename = format!(
        "pertisk-proxy-backup-{}.json",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    );
    let body = serde_json::to_vec_pretty(&backup).unwrap_or_default();
    Ok(BuiltBackup {
        filename,
        content_type: "application/json",
        body,
    })
}

async fn proxy_backup_restore(state: &AdminState, body: RestoreBackupBody) -> Response {
    let Some(db) = &state.db else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Database not configured" })),
        )
            .into_response();
    };

    let backup: ProxyBackup = match serde_json::from_str(&body.data) {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("Invalid backup format: {e}") })),
            )
                .into_response();
        }
    };

    let mut restored_count = 0usize;
    let mut errors = Vec::new();

    let existing_cert_hosts: HashSet<String> = if body.merge {
        db.list_certificates()
            .await
            .unwrap_or_default()
            .into_iter()
            .flat_map(|r| r.hosts)
            .collect()
    } else {
        HashSet::new()
    };

    for cert in backup.certificates {
        if body.merge {
            let dup = cert.hosts.iter().any(|h| existing_cert_hosts.contains(h));
            if dup {
                continue;
            }
        }
        let cert_pem = cert.cert_pem.into_bytes();
        let key_pem = cert.key_pem.into_bytes();
        match db
            .add_certificate(
                cert.hosts.clone(),
                cert_pem.clone(),
                key_pem.clone(),
                &cert.source_type,
            )
            .await
        {
            Ok(id) => {
                restored_count += 1;
                if let Err(e) = state.cert_store.insert_pem_for_hosts(
                    &cert.hosts,
                    &cert_pem,
                    &key_pem,
                    &state.certs_dir,
                    &id,
                ) {
                    errors.push(format!("Certificate loaded to DB but cert store failed: {e}"));
                } else {
                    info!(hosts = ?cert.hosts, "backup: restored certificate");
                }
            }
            Err(e) => errors.push(format!("Failed to restore certificate: {e}")),
        }
    }

    let config_to_save = if body.merge {
        let current = state.runtime_config.read().await.clone();
        merge_proxy_config(current, backup.config)
    } else {
        backup.config
    };

    let mut normalized = config_to_save;
    for tls in &mut normalized.tls {
        tls.expires_at = None;
    }
    crate::proxy_config::normalize_tls_config(&mut normalized.tls);

    match persist_proxy_config(state, db.as_ref(), &normalized).await {
        Ok(_) => {
            restored_count += 1;
            state
                .proxy_log
                .push(ProxyLogEntry::config_reload(format!(
                    "backup restored ({} sites, {} routes)",
                    normalized.sites.len(),
                    state.router.route_count()
                )))
                .await;
        }
        Err(e) => errors.push(format!("Failed to restore config: {e}")),
    }

    restore_response(
        restored_count,
        errors,
        Some(
            "DNS provider credentials are not included in backups. Re-add DNS providers manually after restore."
                .into(),
        ),
    )
}

fn merge_proxy_config(mut merged: Config, backup: Config) -> Config {
    let existing_hosts: HashSet<String> = merged.sites.iter().map(|s| s.host.clone()).collect();
    for site in backup.sites {
        if !existing_hosts.contains(&site.host) {
            merged.sites.push(site);
        }
    }

    let existing_backends: HashSet<String> = merged.backends.iter().map(|b| b.name.clone()).collect();
    for backend in backup.backends {
        if !existing_backends.contains(&backend.name) {
            merged.backends.push(backend);
        }
    }

    let existing_tls: HashSet<Vec<String>> = merged
        .tls
        .iter()
        .map(|t| {
            let mut h = t.hosts.clone();
            h.sort();
            h
        })
        .collect();
    for tls in backup.tls {
        let mut hosts = tls.hosts.clone();
        hosts.sort();
        if !existing_tls.contains(&hosts) {
            merged.tls.push(tls);
        }
    }

    merged
}

#[cfg(feature = "ingress")]
async fn ingress_backup_export(state: &AdminState, namespace: Option<&str>) -> Response {
    match build_ingress_backup(state, namespace).await {
        Ok(built) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, built.content_type)
            .header(
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", built.filename),
            )
            .body(Body::from(built.body))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        Err(r) => r,
    }
}

#[cfg(feature = "ingress")]
async fn build_ingress_backup(
    state: &AdminState,
    namespace: Option<&str>,
) -> Result<BuiltBackup, Response> {
    let Some(client) = &state.kube_client else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Kubernetes client not available" })),
        )
            .into_response());
    };

    let ingress_api: kube::Api<k8s_openapi::api::networking::v1::Ingress> = match namespace {
        Some(ns) => kube::Api::namespaced(client.clone(), ns),
        None => kube::Api::all(client.clone()),
    };

    let ingresses = match ingress_api.list(&kube::api::ListParams::default()).await {
        Ok(list) => list
            .items
            .into_iter()
            .map(|ing| serde_json::to_value(ing).unwrap_or(serde_json::Value::Null))
            .collect(),
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to list ingresses: {e}") })),
            )
                .into_response());
        }
    };

    let secret_api: kube::Api<k8s_openapi::api::core::v1::Secret> = match namespace {
        Some(ns) => kube::Api::namespaced(client.clone(), ns),
        None => kube::Api::all(client.clone()),
    };

    let secrets = match secret_api.list(&kube::api::ListParams::default()).await {
        Ok(list) => list
            .items
            .into_iter()
            .filter(|sec| {
                let typ = sec.type_.as_deref().unwrap_or("");
                let has_tls = sec
                    .data
                    .as_ref()
                    .is_some_and(|d| d.contains_key("tls.crt"));
                typ == "kubernetes.io/tls" || has_tls
            })
            .map(|sec| serde_json::to_value(sec).unwrap_or(serde_json::Value::Null))
            .collect(),
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to list secrets: {e}") })),
            )
                .into_response());
        }
    };

    let mut gateways = Vec::new();
    let mut httproutes = Vec::new();
    if state.gateway_api_enabled {
        use crate::ingress::gateway_api::{Gateway, HTTPRoute};

        let gw_api: kube::Api<Gateway> = match namespace {
            Some(ns) => kube::Api::namespaced(client.clone(), ns),
            None => kube::Api::all(client.clone()),
        };
        if let Ok(list) = gw_api.list(&kube::api::ListParams::default()).await {
            gateways = list
                .items
                .into_iter()
                .map(|gw| serde_json::to_value(gw).unwrap_or(serde_json::Value::Null))
                .collect();
        }

        let route_api: kube::Api<HTTPRoute> = match namespace {
            Some(ns) => kube::Api::namespaced(client.clone(), ns),
            None => kube::Api::all(client.clone()),
        };
        if let Ok(list) = route_api.list(&kube::api::ListParams::default()).await {
            httproutes = list
                .items
                .into_iter()
                .map(|r| serde_json::to_value(r).unwrap_or(serde_json::Value::Null))
                .collect();
        }
    }

    let helm = fetch_helm_backup_snapshot().await;

    let backup = IngressBackup {
        ingresses,
        secrets,
        gateways,
        httproutes,
        helm,
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    let filename = format!(
        "pertisk-ingress-backup-{}.yaml",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    );

    match serde_yaml::to_string(&backup) {
        Ok(yaml) => Ok(BuiltBackup {
            filename,
            content_type: "application/x-yaml",
            body: yaml.into_bytes(),
        }),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to serialize backup: {e}") })),
        )
            .into_response()),
    }
}

#[cfg(feature = "ingress")]
async fn ingress_backup_restore(state: &AdminState, body: RestoreBackupBody) -> Response {
    let Some(client) = &state.kube_client else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Kubernetes client not available" })),
        )
            .into_response();
    };

    let backup: IngressBackup = match serde_yaml::from_str(&body.data) {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("Invalid backup format: {e}") })),
            )
                .into_response();
        }
    };

    let mut restored_count = 0usize;
    let mut errors = Vec::new();

    for secret_value in backup.secrets {
        match restore_secret(client, secret_value, body.merge).await {
            Ok(applied) => {
                if applied {
                    restored_count += 1;
                }
            }
            Err(e) => errors.push(e),
        }
    }

    for gw_value in backup.gateways {
        match restore_gateway(client, gw_value, body.merge).await {
            Ok(applied) => {
                if applied {
                    restored_count += 1;
                }
            }
            Err(e) => errors.push(e),
        }
    }

    for route_value in backup.httproutes {
        match restore_httproute(client, route_value, body.merge).await {
            Ok(applied) => {
                if applied {
                    restored_count += 1;
                }
            }
            Err(e) => errors.push(e),
        }
    }

    for ingress_value in backup.ingresses {
        match restore_ingress(client, ingress_value, body.merge).await {
            Ok(applied) => {
                if applied {
                    restored_count += 1;
                }
            }
            Err(e) => errors.push(e),
        }
    }

    let note = backup.helm.as_ref().map(|h| {
        format!(
            "Helm release {} in namespace {} was included for reference. Re-install with: helm upgrade --install {} <chart> -n {} -f values-from-backup.yaml",
            h.release, h.namespace, h.release, h.namespace
        )
    });

    restore_response(restored_count, errors, note)
}

#[cfg(feature = "ingress")]
async fn restore_secret(
    client: &kube::Client,
    secret_value: serde_json::Value,
    merge: bool,
) -> Result<bool, String> {
    let secret: k8s_openapi::api::core::v1::Secret =
        serde_json::from_value(secret_value).map_err(|e| format!("Failed to parse secret: {e}"))?;
    let namespace = secret
        .metadata
        .namespace
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let name = secret
        .metadata
        .name
        .clone()
        .unwrap_or_else(|| "unnamed".to_string());
    let api: kube::Api<k8s_openapi::api::core::v1::Secret> =
        kube::Api::namespaced(client.clone(), &namespace);
    let exists = api.get(&name).await.is_ok();
    if exists && !merge {
        return Ok(false);
    }
    if exists {
        api.replace(&name, &kube::api::PostParams::default(), &secret)
            .await
            .map_err(|e| format!("Failed to restore secret {namespace}/{name}: {e}"))?;
    } else {
        api.create(&kube::api::PostParams::default(), &secret)
            .await
            .map_err(|e| format!("Failed to restore secret {namespace}/{name}: {e}"))?;
    }
    Ok(true)
}

#[cfg(feature = "ingress")]
async fn restore_ingress(
    client: &kube::Client,
    ingress_value: serde_json::Value,
    merge: bool,
) -> Result<bool, String> {
    let ingress: k8s_openapi::api::networking::v1::Ingress = serde_json::from_value(ingress_value)
        .map_err(|e| format!("Failed to parse ingress: {e}"))?;
    let namespace = ingress
        .metadata
        .namespace
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let name = ingress
        .metadata
        .name
        .clone()
        .unwrap_or_else(|| "unnamed".to_string());
    let api: kube::Api<k8s_openapi::api::networking::v1::Ingress> =
        kube::Api::namespaced(client.clone(), &namespace);
    let exists = api.get(&name).await.is_ok();
    if exists && !merge {
        return Ok(false);
    }
    if exists {
        api.replace(&name, &kube::api::PostParams::default(), &ingress)
            .await
            .map_err(|e| format!("Failed to restore ingress {namespace}/{name}: {e}"))?;
    } else {
        api.create(&kube::api::PostParams::default(), &ingress)
            .await
            .map_err(|e| format!("Failed to restore ingress {namespace}/{name}: {e}"))?;
    }
    Ok(true)
}

#[cfg(feature = "ingress")]
async fn restore_gateway(
    client: &kube::Client,
    value: serde_json::Value,
    merge: bool,
) -> Result<bool, String> {
    use crate::ingress::gateway_api::Gateway;
    let gw: Gateway =
        serde_json::from_value(value).map_err(|e| format!("Failed to parse gateway: {e}"))?;
    let namespace = gw
        .metadata
        .namespace
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let name = gw
        .metadata
        .name
        .clone()
        .unwrap_or_else(|| "unnamed".to_string());
    let api: kube::Api<Gateway> = kube::Api::namespaced(client.clone(), &namespace);
    let exists = api.get(&name).await.is_ok();
    if exists && !merge {
        return Ok(false);
    }
    if exists {
        api.replace(&name, &kube::api::PostParams::default(), &gw)
            .await
            .map_err(|e| format!("Failed to restore gateway {namespace}/{name}: {e}"))?;
    } else {
        api.create(&kube::api::PostParams::default(), &gw)
            .await
            .map_err(|e| format!("Failed to restore gateway {namespace}/{name}: {e}"))?;
    }
    Ok(true)
}

#[cfg(feature = "ingress")]
async fn restore_httproute(
    client: &kube::Client,
    value: serde_json::Value,
    merge: bool,
) -> Result<bool, String> {
    use crate::ingress::gateway_api::HTTPRoute;
    let route: HTTPRoute =
        serde_json::from_value(value).map_err(|e| format!("Failed to parse httproute: {e}"))?;
    let namespace = route
        .metadata
        .namespace
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let name = route
        .metadata
        .name
        .clone()
        .unwrap_or_else(|| "unnamed".to_string());
    let api: kube::Api<HTTPRoute> = kube::Api::namespaced(client.clone(), &namespace);
    let exists = api.get(&name).await.is_ok();
    if exists && !merge {
        return Ok(false);
    }
    if exists {
        api.replace(&name, &kube::api::PostParams::default(), &route)
            .await
            .map_err(|e| format!("Failed to restore httproute {namespace}/{name}: {e}"))?;
    } else {
        api.create(&kube::api::PostParams::default(), &route)
            .await
            .map_err(|e| format!("Failed to restore httproute {namespace}/{name}: {e}"))?;
    }
    Ok(true)
}

#[cfg(feature = "ingress")]
fn helm_env_value(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(feature = "ingress")]
async fn fetch_helm_backup_snapshot() -> Option<HelmBackupSnapshot> {
    let enabled = helm_env_value("PERTISK_HELM_ENABLED")
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false);
    if !enabled {
        return None;
    }

    let release = helm_env_value("PERTISK_HELM_RELEASE")?;
    let namespace = helm_env_value("PERTISK_HELM_NAMESPACE")
        .or_else(|| std::env::var("POD_NAMESPACE").ok())
        .unwrap_or_else(|| "default".to_string());
    let helm_path = helm_env_value("PERTISK_HELM_PATH").unwrap_or_else(|| "helm".to_string());

    let values_output = tokio::process::Command::new(&helm_path)
        .args(["get", "values", &release, "-n", &namespace, "--output", "yaml"])
        .output()
        .await
        .ok()?;
    if !values_output.status.success() {
        tracing::warn!(
            stderr = %String::from_utf8_lossy(&values_output.stderr),
            "backup: helm get values failed"
        );
        return None;
    }

    let mut history_cmd = tokio::process::Command::new(&helm_path);
    history_cmd.args(["history", &release, "-n", &namespace, "--output", "json"]);
    if let Some(max) = helm_env_value("PERTISK_HELM_HISTORY_MAX") {
        history_cmd.args(["--max", &max]);
    }
    let history_output = history_cmd.output().await.ok()?;
    let history = if history_output.status.success() {
        serde_json::from_slice(&history_output.stdout).unwrap_or(serde_json::Value::Null)
    } else {
        serde_json::Value::Null
    };

    Some(HelmBackupSnapshot {
        release,
        namespace,
        values: String::from_utf8_lossy(&values_output.stdout).into_owned(),
        history,
    })
}

fn restore_response(restored_count: usize, errors: Vec<String>, note: Option<String>) -> Response {
    let body = RestoreBackupResponse {
        message: if errors.is_empty() {
            format!("Successfully restored backup ({restored_count} item(s))")
        } else {
            format!(
                "Restored {restored_count} item(s) with {} error(s)",
                errors.len()
            )
        },
        restored_count,
        errors: if errors.is_empty() { None } else { Some(errors) },
        note,
    };
    let status = if body.errors.is_some() {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    (status, Json(body)).into_response()
}

/// Apply proxy config to the running data plane (routes, cert store, ACME sweep).
pub async fn activate_proxy_config(
    state: &AdminState,
    db: &crate::db::Database,
    config: &Config,
) -> Result<(), String> {
    crate::proxy::apply::apply_config(state.router.as_ref(), config).map_err(|e| e.to_string())?;
    state
        .cert_store
        .reload_from_configs(&config.tls)
        .map_err(|e| e.to_string())?;
    state.cert_store.set_expected_from_config(config);
    if let Err(err) =
        super::load_db_certs_into_store(db, state.cert_store.as_ref(), &state.certs_dir).await
    {
        tracing::warn!(error = %err, "config apply: failed to reload certificates from database");
    }
    *state.runtime_config.write().await = config.clone();
    state.proxy_log_enabled.store(config.proxy_log, Ordering::Relaxed);
    #[cfg(feature = "acme")]
    if let Some(acme) = state.acme_manager.clone() {
        let cfg = config.clone();
        let db_c = std::sync::Arc::new(db.clone());
        let store_c = state.cert_store.clone();
        let dir = state.certs_dir.clone();
        tokio::spawn(async move {
            super::acme::spawn_auto_ssl_for_config(&cfg, db_c, acme, store_c, dir).await;
        });
    }
    Ok(())
}

/// Save config to SQLite and activate on the data plane.
pub async fn persist_proxy_config(
    state: &AdminState,
    db: &crate::db::Database,
    config: &Config,
) -> Result<(), String> {
    db.save_proxy_config(config)
        .await
        .map_err(|e| e.to_string())?;
    activate_proxy_config(state, db, config).await
}
