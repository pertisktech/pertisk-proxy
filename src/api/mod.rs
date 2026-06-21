//! Management API + admin UI static file server (Axum on `PERTISK_MANAGEMENT_ADDR`).

#[cfg(all(feature = "admin", feature = "acme"))]
pub mod acme;

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use axum::{
    body::Body,
    extract::{Path as AxumPath, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};

use crate::config::ProxyConfig;
use crate::db::{CertificateRow, Database, DnsProviderRow};
use crate::proxy::apply;
use crate::proxy_config::{Config, TlsConfig, TlsSource};
use crate::runtime::RuntimeConfig;
use crate::tls::{CertStore, Http01ChallengeStore};
use crate::Router as ProxyRouter;

#[cfg(feature = "acme")]
use crate::tls::AcmeManager;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct AdminState {
    pub router: Arc<ProxyRouter>,
    pub cert_store: Arc<CertStore>,
    pub proxy_config: ProxyConfig,
    pub runtime_cfg: RuntimeConfig,
    pub runtime_config: Arc<RwLock<Config>>,
    pub started_at: Instant,
    pub auth_password: Option<String>,
    pub session_token: Arc<RwLock<Option<String>>>,
    pub admin_dist: PathBuf,
    pub dev_origin: Option<String>,
    pub db: Option<Arc<Database>>,
    pub certs_dir: PathBuf,
    pub http01_store: Arc<Http01ChallengeStore>,
    #[cfg(feature = "acme")]
    pub acme_manager: Option<Arc<AcmeManager>>,
}

pub async fn serve(state: AdminState, addr: SocketAddr) -> Result<()> {
    let app = router(state);
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind management API on {addr}"))?;
    info!("Management API listening on http://{addr}");
    axum::serve(listener, app)
        .await
        .context("management API server stopped")?;
    Ok(())
}

pub fn router(state: AdminState) -> Router {
    let public = Router::new()
        .route("/api/health", get(api_health))
        .route("/api/version", get(api_version))
        .route("/api/auth/config", get(auth_config))
        .route("/api/auth/login", post(auth_login))
        .route("/api/auth/check", get(auth_check))
        .route("/live", get(|| async { "ok" }))
        .route("/ready", get(|| async { "ok" }))
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ok" }))
        .route(
            "/.well-known/acme-challenge/{token}",
            get(acme_http01_challenge),
        );

    let protected = Router::new()
        .route("/api/management", get(get_management))
        .route("/api/config", get(get_config).put(put_config))
        .route("/api/reload", post(reload_config))
        .route("/api/tls", get(get_tls))
        .route("/api/routes", get(get_routes))
        .route("/api/certificates", get(certificates_list).post(certificates_upload))
        .route("/api/certificates/{id}", delete(certificates_delete))
        .route("/api/dns-providers", get(dns_providers_list).post(dns_providers_create))
        .route("/api/dns-providers/supported", get(dns_providers_supported))
        .route(
            "/api/dns-providers/{id}",
            get(dns_providers_get)
                .put(dns_providers_put)
                .delete(dns_providers_delete),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_auth_middleware,
        ));

    public
        .merge(protected)
        .fallback(admin_spa_fallback)
        .layer(CorsLayer::permissive())
        .with_state(state)
}

async fn require_auth_middleware(
    State(state): State<AdminState>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if state.auth_password.is_none() {
        return Ok(next.run(req).await);
    }
    if is_authorized(&state, req.headers()) {
        Ok(next.run(req).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

fn is_authorized(state: &AdminState, headers: &HeaderMap) -> bool {
    let token = extract_bearer(headers);
    let Some(token) = token else {
        return false;
    };
    if let Ok(guard) = state.session_token.try_read() {
        if guard.as_deref() == Some(token) {
            return true;
        }
    }
    false
}

fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ").map(str::trim)
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

async fn api_health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

#[derive(Serialize)]
struct VersionResponse {
    version: &'static str,
    binary: &'static str,
}

async fn api_version() -> Json<VersionResponse> {
    Json(VersionResponse {
        version: VERSION,
        binary: "pertisk-proxy",
    })
}

#[derive(Serialize)]
struct AuthConfigResponse {
    mode: &'static str,
    supports_local: bool,
    auth_required: bool,
}

async fn auth_config(State(state): State<AdminState>) -> Json<AuthConfigResponse> {
    Json(AuthConfigResponse {
        mode: "local",
        supports_local: true,
        auth_required: state.auth_password.is_some(),
    })
}

#[derive(Deserialize)]
struct LoginRequest {
    username: Option<String>,
    password: String,
}

#[derive(Serialize)]
struct LoginResponse {
    token: String,
    username: String,
    expires_in: u64,
}

async fn auth_login(
    State(state): State<AdminState>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, Json<ApiError>)> {
    let Some(expected) = state.auth_password.as_deref() else {
        return Ok(Json(LoginResponse {
            token: String::new(),
            username: body.username.unwrap_or_else(|| "admin".into()),
            expires_in: 0,
        }));
    };
    if body.password != expected {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiError {
                error: "invalid credentials".into(),
            }),
        ));
    }
    let token = state
        .session_token
        .read()
        .await
        .clone()
        .unwrap_or_default();
    Ok(Json(LoginResponse {
        token,
        username: body.username.unwrap_or_else(|| "admin".into()),
        expires_in: 86_400,
    }))
}

#[derive(Serialize)]
struct AuthCheckResponse {
    authenticated: bool,
    username: Option<String>,
}

async fn auth_check(
    State(state): State<AdminState>,
    headers: HeaderMap,
) -> Json<AuthCheckResponse> {
    Json(AuthCheckResponse {
        authenticated: state.auth_password.is_none() || is_authorized(&state, &headers),
        username: Some("admin".into()),
    })
}

#[derive(Serialize)]
struct ManagementInfo {
    mode: &'static str,
    version: &'static str,
    uptime_secs: u64,
    db_path: String,
    route_count: usize,
    site_count: usize,
    tls_host_count: usize,
    enable_h3: bool,
    auto_https: bool,
    runtime_mode: String,
    listeners: ListenerInfo,
    http3: crate::http3_options::Http3Options,
}

#[derive(Serialize)]
struct ListenerInfo {
    http: String,
    https: String,
    h3_udp: String,
}

async fn get_management(State(state): State<AdminState>) -> Json<ManagementInfo> {
    let server = &state.proxy_config.server;
    let cfg = state.runtime_config.read().await;
    Json(ManagementInfo {
        mode: "proxy",
        version: VERSION,
        uptime_secs: state.started_at.elapsed().as_secs(),
        db_path: state.proxy_config.db_path.display().to_string(),
        route_count: state.router.route_count(),
        site_count: cfg.sites.len(),
        tls_host_count: state.cert_store.host_count(),
        enable_h3: server.enable_h3,
        auto_https: state.proxy_config.auto_https,
        runtime_mode: state.runtime_cfg.resolved_mode.as_str().to_string(),
        listeners: ListenerInfo {
            http: server.http_listen.clone(),
            https: server.https_listen.clone(),
            h3_udp: server.h3_udp_listen.clone(),
        },
        http3: cfg.http3.clone(),
    })
}

#[derive(Serialize)]
struct RoutesResponse {
    routes: Vec<RouteView>,
    count: usize,
}

#[derive(Serialize)]
struct RouteView {
    host: String,
    path: String,
    path_type: String,
    upstream: String,
    middlewares: usize,
}

async fn get_routes(State(state): State<AdminState>) -> Json<RoutesResponse> {
    let mut routes = Vec::new();
    for (host, route) in state.router.snapshot().all_routes() {
        let path_type = match route.path_type {
            crate::router::PathMatchType::Exact => "exact",
            crate::router::PathMatchType::Prefix => "prefix",
            crate::router::PathMatchType::ImplementationSpecific => "implementation_specific",
        };
        routes.push(RouteView {
            host: host.clone(),
            path: route.path.clone(),
            path_type: path_type.into(),
            upstream: format!("{}:{}", route.backend.address, route.backend.port),
            middlewares: route.middlewares.len(),
        });
    }
    let count = routes.len();
    Json(RoutesResponse { routes, count })
}

async fn acme_http01_challenge(
    State(state): State<AdminState>,
    AxumPath(token): AxumPath<String>,
) -> Response {
    if let Some(body) = state.http01_store.get(&token) {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/plain")
            .body(Body::from(body))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    StatusCode::NOT_FOUND.into_response()
}

async fn get_config(State(state): State<AdminState>) -> Result<Json<Config>, (StatusCode, Json<ApiError>)> {
    let mut cfg = state.runtime_config.read().await.clone();
    if let Some(db) = &state.db {
        enrich_tls_expiries(&mut cfg, db).await;
    }
    Ok(Json(cfg))
}

async fn enrich_tls_expiries(cfg: &mut Config, db: &Database) {
    if let Ok(rows) = db.list_certificates().await {
        for tls in &mut cfg.tls {
            tls.expires_at = rows
                .iter()
                .find(|r| acme::cert_row_matches_tls_config(r, &tls.hosts))
                .and_then(|r| r.expires_at.clone());
        }
    }
}

async fn put_config(
    State(state): State<AdminState>,
    Json(mut body): Json<Config>,
) -> Result<Json<ReloadResponse>, (StatusCode, Json<ApiError>)> {
    for tls in &mut body.tls {
        tls.expires_at = None;
    }
    let Some(db) = &state.db else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiError {
                error: "database not configured".into(),
            }),
        ));
    };
    db.save_proxy_config(&body).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
    })?;
    apply::apply_config(state.router.as_ref(), &body).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
    })?;
    state
        .cert_store
        .reload_from_configs(&body.tls)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: e.to_string(),
                }),
            )
        })?;
    *state.runtime_config.write().await = body.clone();
    #[cfg(feature = "acme")]
    if let Some(acme) = state.acme_manager.clone() {
        let cfg = body.clone();
        let db_c = db.clone();
        let store_c = state.cert_store.clone();
        let dir = state.certs_dir.clone();
        tokio::spawn(async move {
            acme::spawn_auto_ssl_for_config(&cfg, db_c, acme, store_c, dir).await;
        });
    }
    Ok(Json(ReloadResponse {
        ok: true,
        route_count: state.router.route_count(),
    }))
}

async fn reload_config(State(state): State<AdminState>) -> Result<Json<ReloadResponse>, (StatusCode, Json<ApiError>)> {
    let Some(db) = &state.db else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiError {
                error: "database not configured".into(),
            }),
        ));
    };
    let cfg = db.get_proxy_config().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
    })?.unwrap_or_default();
    apply::apply_config(state.router.as_ref(), &cfg).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
    })?;
    state.cert_store.reload_from_configs(&cfg.tls).ok();
    if let Err(err) = load_db_certs_into_store(db.as_ref(), state.cert_store.as_ref(), &state.certs_dir).await
    {
        tracing::warn!(error = %err, "reload: failed to load certificates from database");
    }
    *state.runtime_config.write().await = cfg.clone();
    #[cfg(feature = "acme")]
    if let Some(acme) = state.acme_manager.clone() {
        let db_c = db.clone();
        let store_c = state.cert_store.clone();
        let dir = state.certs_dir.clone();
        tokio::spawn(async move {
            acme::spawn_auto_ssl_for_config(&cfg, db_c, acme, store_c, dir).await;
        });
    }
    Ok(Json(ReloadResponse {
        ok: true,
        route_count: state.router.route_count(),
    }))
}

#[derive(Serialize)]
struct ReloadResponse {
    ok: bool,
    route_count: usize,
}

async fn certificates_list(
    State(state): State<AdminState>,
) -> Result<Json<Vec<CertificateRow>>, (StatusCode, Json<ApiError>)> {
    let Some(db) = &state.db else {
        return Ok(Json(Vec::new()));
    };
    let rows = db.list_certificates().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
    })?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
struct CertificatesUploadBody {
    hosts: Vec<String>,
    cert_pem: String,
    key_pem: String,
}

async fn certificates_upload(
    State(state): State<AdminState>,
    Json(body): Json<CertificatesUploadBody>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<ApiError>)> {
    let Some(db) = &state.db else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiError {
                error: "database not configured".into(),
            }),
        ));
    };
    let hosts: Vec<String> = body
        .hosts
        .into_iter()
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .collect();
    if hosts.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                error: "hosts required".into(),
            }),
        ));
    }
    let cert_pem = body.cert_pem.into_bytes();
    let key_pem = body.key_pem.into_bytes();
    let id = db
        .add_certificate(hosts.clone(), cert_pem.clone(), key_pem.clone(), "uploaded")
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: e.to_string(),
                }),
            )
        })?;
    state
        .cert_store
        .insert_pem_for_hosts(&hosts, &cert_pem, &key_pem, &state.certs_dir, &id)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: e.to_string(),
                }),
            )
        })?;
    let mut cfg = state.runtime_config.read().await.clone();
    sync_uploaded_cert_tls(&mut cfg, &hosts, &id, &state.certs_dir);
    db.save_proxy_config(&cfg).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
    })?;
    *state.runtime_config.write().await = cfg;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "id": id, "message": "Certificate saved and loaded." })),
    ))
}

async fn certificates_delete(
    State(state): State<AdminState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let Some(db) = &state.db else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiError {
                error: "database not configured".into(),
            }),
        ));
    };
    let rows = db.list_certificates().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
    })?;
    let row = rows.iter().find(|r| r.id == id).cloned().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "not found".into(),
            }),
        )
    })?;
    let deleted = db.delete_certificate(&id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
    })?;
    if !deleted {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "not found".into(),
            }),
        ));
    }
    state.cert_store.remove_for_hosts(&row.hosts);
    let cert_path = state.certs_dir.join(format!("{id}.pem"));
    let key_path = state.certs_dir.join(format!("{id}.key"));
    let _ = std::fs::remove_file(cert_path);
    let _ = std::fs::remove_file(key_path);
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Serialize)]
struct SupportedDnsProviderField {
    key: String,
    label: String,
    field_type: String,
    required: bool,
}

#[derive(Serialize)]
struct SupportedDnsProvider {
    id: String,
    name: String,
    fields: Vec<SupportedDnsProviderField>,
}

fn supported_dns_providers() -> Vec<SupportedDnsProvider> {
    vec![
        SupportedDnsProvider {
            id: "cloudflare".into(),
            name: "Cloudflare".into(),
            fields: vec![
                SupportedDnsProviderField {
                    key: "api_token".into(),
                    label: "API Token".into(),
                    field_type: "password".into(),
                    required: true,
                },
                SupportedDnsProviderField {
                    key: "zone_id".into(),
                    label: "Zone ID (optional)".into(),
                    field_type: "text".into(),
                    required: false,
                },
            ],
        },
        SupportedDnsProvider {
            id: "digitalocean".into(),
            name: "DigitalOcean".into(),
            fields: vec![SupportedDnsProviderField {
                key: "api_token".into(),
                label: "API Token".into(),
                field_type: "password".into(),
                required: true,
            }],
        },
        SupportedDnsProvider {
            id: "route53".into(),
            name: "AWS Route 53".into(),
            fields: vec![
                SupportedDnsProviderField {
                    key: "access_key_id".into(),
                    label: "Access Key ID".into(),
                    field_type: "text".into(),
                    required: true,
                },
                SupportedDnsProviderField {
                    key: "secret_access_key".into(),
                    label: "Secret Access Key".into(),
                    field_type: "password".into(),
                    required: true,
                },
            ],
        },
        SupportedDnsProvider {
            id: "duckdns".into(),
            name: "DuckDNS".into(),
            fields: vec![
                SupportedDnsProviderField {
                    key: "domain".into(),
                    label: "Subdomain".into(),
                    field_type: "text".into(),
                    required: true,
                },
                SupportedDnsProviderField {
                    key: "token".into(),
                    label: "Token".into(),
                    field_type: "password".into(),
                    required: true,
                },
            ],
        },
        SupportedDnsProvider {
            id: "hetzner".into(),
            name: "Hetzner DNS".into(),
            fields: vec![SupportedDnsProviderField {
                key: "api_token".into(),
                label: "API Token".into(),
                field_type: "password".into(),
                required: true,
            }],
        },
    ]
}

async fn dns_providers_supported() -> Json<Vec<SupportedDnsProvider>> {
    Json(supported_dns_providers())
}

async fn dns_providers_list(
    State(state): State<AdminState>,
) -> Result<Json<Vec<DnsProviderRow>>, (StatusCode, Json<ApiError>)> {
    let Some(db) = &state.db else {
        return Ok(Json(Vec::new()));
    };
    let rows = db.list_dns_providers().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
    })?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
struct CreateDnsProviderBody {
    name: String,
    provider_type: String,
    credentials: Option<std::collections::HashMap<String, String>>,
}

async fn dns_providers_create(
    State(state): State<AdminState>,
    Json(body): Json<CreateDnsProviderBody>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<ApiError>)> {
    let Some(db) = &state.db else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiError {
                error: "database not configured".into(),
            }),
        ));
    };
    let id = db
        .create_dns_provider(body.name, body.provider_type, body.credentials)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: e.to_string(),
                }),
            )
        })?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

async fn dns_providers_get(
    State(state): State<AdminState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<DnsProviderRow>, (StatusCode, Json<ApiError>)> {
    let Some(db) = &state.db else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiError {
                error: "database not configured".into(),
            }),
        ));
    };
    match db.get_dns_provider(&id).await {
        Ok(Some(row)) => Ok(Json(row)),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "not found".into(),
            }),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )),
    }
}

#[derive(Deserialize)]
struct PutDnsProviderBody {
    name: String,
    provider_type: String,
    credentials: Option<std::collections::HashMap<String, String>>,
}

async fn dns_providers_put(
    State(state): State<AdminState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<PutDnsProviderBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let Some(db) = &state.db else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiError {
                error: "database not configured".into(),
            }),
        ));
    };
    let updated = db
        .put_dns_provider(&id, body.name, body.provider_type, body.credentials)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: e.to_string(),
                }),
            )
        })?;
    if updated {
        Ok(Json(serde_json::json!({ "ok": true })))
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "not found".into(),
            }),
        ))
    }
}

async fn dns_providers_delete(
    State(state): State<AdminState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let Some(db) = &state.db else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiError {
                error: "database not configured".into(),
            }),
        ));
    };
    let deleted = db.delete_dns_provider(&id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
    })?;
    if deleted {
        Ok(Json(serde_json::json!({ "ok": true })))
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "not found".into(),
            }),
        ))
    }
}

#[derive(Serialize)]
struct TlsEntryView {
    hosts: Vec<String>,
    cert: String,
    key: String,
}

#[derive(Serialize)]
struct TlsResponse {
    entries: Vec<TlsEntryView>,
    host_count: usize,
}

async fn get_tls(State(state): State<AdminState>) -> Result<Json<TlsResponse>, (StatusCode, Json<ApiError>)> {
    let cfg = state.runtime_config.read().await;
    let entries = cfg
        .tls
        .iter()
        .filter_map(|entry| {
            let cert = entry.source.cert_path()?.display().to_string();
            let key = entry.source.key_path()?.display().to_string();
            Some(TlsEntryView {
                hosts: entry.hosts.clone(),
                cert,
                key,
            })
        })
        .collect();
    Ok(Json(TlsResponse {
        host_count: state.cert_store.host_count(),
        entries,
    }))
}

#[derive(Serialize)]
struct ApiError {
    error: String,
}

async fn admin_spa_fallback(
    State(state): State<AdminState>,
    req: Request,
) -> Response {
    if let Some(origin) = state.dev_origin.as_ref() {
        let path = req.uri().path();
        let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
        let location = format!("{origin}{path}{query}");
        return Redirect::temporary(&location).into_response();
    }

    let dist = &state.admin_dist;
    let index = dist.join("index.html");
    if !index.is_file() {
        return build_stub_page(dist).into_response();
    }

    let path = req.uri().path().trim_start_matches('/');
    let file_path = if path.is_empty() {
        index.clone()
    } else {
        let candidate = dist.join(path);
        if candidate.is_file() {
            candidate
        } else {
            index
        }
    };

    match tokio::fs::read(&file_path).await {
        Ok(bytes) => {
            let mime = mime_guess(&file_path);
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                .body(Body::from(bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

fn mime_guess(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "application/javascript",
        Some("css") => "text/css",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn build_stub_page(dist: &Path) -> Response {
    let body = format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"><title>pertisk-proxy Admin</title></head>
<body style="font-family:system-ui;background:#0c0d18;color:#e6e7f0;padding:2rem">
<h1>pertisk-proxy Admin UI</h1>
<p>Build the admin UI: <code>make install-admin && make admin-dist</code></p>
<p>Expected files at <code>{}</code></p>
</body></html>"#,
        dist.display()
    );
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(body))
        .unwrap()
}

pub fn resolve_admin_dist() -> PathBuf {
    if let Ok(path) = std::env::var("PERTISK_ADMIN_DIST") {
        return PathBuf::from(path);
    }
    for candidate in [
        PathBuf::from("admin/dist"),
        PathBuf::from("/usr/share/pertisk-proxy/admin/dist"),
    ] {
        if candidate.join("index.html").is_file() {
            return candidate;
        }
    }
    PathBuf::from("admin/dist")
}

pub fn admin_password() -> Option<String> {
    std::env::var("PERTISK_ADMIN_PASSWORD")
        .ok()
        .filter(|s| !s.is_empty())
}

pub fn session_token() -> String {
    std::env::var("PERTISK_ADMIN_TOKEN").unwrap_or_else(|_| {
        use std::time::{SystemTime, UNIX_EPOCH};
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("ptproxy-{seed:x}")
    })
}

pub fn management_addr() -> SocketAddr {
    std::env::var("PERTISK_MANAGEMENT_ADDR")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| "127.0.0.1:9080".parse().expect("valid default addr"))
}

pub fn admin_dev_origin() -> Option<String> {
    std::env::var("PERTISK_ADMIN_UI_DEV_ORIGIN")
        .ok()
        .filter(|s| !s.is_empty())
}

pub fn build_state(
    router: Arc<ProxyRouter>,
    cert_store: Arc<CertStore>,
    proxy_config: ProxyConfig,
    runtime_cfg: RuntimeConfig,
    db: Option<Arc<Database>>,
    http01_store: Arc<Http01ChallengeStore>,
    #[cfg(feature = "acme")] acme_manager: Option<Arc<AcmeManager>>,
    runtime_config: Config,
) -> AdminState {
    let password = admin_password();
    if password.is_none() {
        warn!("PERTISK_ADMIN_PASSWORD is not set; management API allows unauthenticated access");
    }
    let certs_dir = db
        .as_ref()
        .map(|d| certs_dir_for_db(d.path()))
        .unwrap_or_else(|| PathBuf::from("./data/certs"));
    AdminState {
        router,
        cert_store,
        proxy_config,
        runtime_cfg,
        runtime_config: Arc::new(RwLock::new(runtime_config)),
        started_at: Instant::now(),
        auth_password: password,
        session_token: Arc::new(RwLock::new(Some(session_token()))),
        admin_dist: resolve_admin_dist(),
        dev_origin: admin_dev_origin(),
        db,
        certs_dir,
        http01_store,
        #[cfg(feature = "acme")]
        acme_manager,
    }
}

pub fn resolve_db_path() -> PathBuf {
    crate::config::resolve_db_path()
}

pub fn certs_dir_for_db(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("certs")
}

fn tls_hosts_sorted(hosts: &[String]) -> Vec<String> {
    let mut h: Vec<String> = hosts
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    h.sort();
    h
}

/// Map an uploaded certificate into `config.tls` so the admin UI and reload path stay in sync.
pub fn sync_uploaded_cert_tls(cfg: &mut Config, hosts: &[String], id: &str, certs_dir: &Path) {
    let cert_path = certs_dir.join(format!("{id}.pem"));
    let key_path = certs_dir.join(format!("{id}.key"));
    let file_source = TlsSource::File {
        cert: cert_path,
        key: key_path,
    };

    let uploaded = tls_hosts_sorted(hosts);
    if uploaded.is_empty() {
        return;
    }
    let uploaded_set: HashSet<String> = uploaded.iter().cloned().collect();

    if let Some(tls) = cfg
        .tls
        .iter_mut()
        .find(|t| tls_hosts_sorted(&t.hosts) == uploaded)
    {
        tls.source = file_source;
        return;
    }

    if let Some(tls) = cfg.tls.iter_mut().find(|t| {
        let tls_set: HashSet<String> = tls_hosts_sorted(&t.hosts).into_iter().collect();
        uploaded_set.is_subset(&tls_set)
    }) {
        tls.source = file_source;
        return;
    }

    cfg.tls.push(TlsConfig {
        hosts: uploaded,
        source: file_source,
        expires_at: None,
    });
}

/// Ensure uploaded DB certificates have matching `config.tls` entries (e.g. after restart).
pub async fn reconcile_uploaded_certs_in_config(
    db: &Database,
    cfg: &mut Config,
    certs_dir: &Path,
) -> Result<bool> {
    let rows = db.list_certificates().await?;
    let mut changed = false;
    for row in rows {
        if !row.source_type.eq_ignore_ascii_case("uploaded") {
            continue;
        }
        let before = serde_json::to_string(&cfg.tls).unwrap_or_default();
        sync_uploaded_cert_tls(cfg, &row.hosts, &row.id, certs_dir);
        let after = serde_json::to_string(&cfg.tls).unwrap_or_default();
        if before != after {
            changed = true;
        }
    }
    if changed {
        db.save_proxy_config(cfg).await?;
    }
    Ok(changed)
}

/// Load certificates from SQLite into CertStore (PEM files under `certs_dir`).
pub async fn load_db_certs_into_store(
    db: &Database,
    store: &CertStore,
    certs_dir: &Path,
) -> Result<()> {
    let rows = db.get_all_certificates_for_store().await?;
    for (id, hosts, cert_pem, key_pem) in rows {
        store.insert_pem_for_hosts(&hosts, &cert_pem, &key_pem, certs_dir, &id)?;
    }
    Ok(())
}
