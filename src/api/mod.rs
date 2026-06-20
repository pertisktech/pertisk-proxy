//! Management API + admin UI static file server (Axum on `PERTISK_MANAGEMENT_ADDR`).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};

use crate::config::ProxyConfig;
use crate::http3_options::Http3Options;
use crate::proxy::routes;
use crate::routes_config;
use crate::runtime::RuntimeConfig;
use crate::tls::CertStore;
use crate::Router as ProxyRouter;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct AdminState {
    pub router: Arc<ProxyRouter>,
    pub cert_store: Arc<CertStore>,
    pub proxy_config: ProxyConfig,
    pub runtime_cfg: RuntimeConfig,
    pub routes_path: PathBuf,
    pub started_at: Instant,
    pub auth_password: Option<String>,
    pub session_token: Arc<RwLock<Option<String>>>,
    pub admin_dist: PathBuf,
    pub dev_origin: Option<String>,
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
        .route("/live", get(|| async { "ok" }))
        .route("/ready", get(|| async { "ok" }))
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ok" }));

    let protected = Router::new()
        .route("/api/management", get(get_management))
        .route("/api/config", get(get_config).put(put_config))
        .route("/api/config/yaml", get(get_config_yaml))
        .route("/api/reload", post(reload_config))
        .route("/api/tls", get(get_tls))
        .route("/api/routes", get(get_routes))
        .route("/api/auth/check", get(auth_check))
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
    routes_path: String,
    route_count: usize,
    tls_host_count: usize,
    enable_h3: bool,
    auto_https: bool,
    runtime_mode: String,
    listeners: ListenerInfo,
    http3: Http3Options,
}

#[derive(Serialize)]
struct ListenerInfo {
    http: String,
    https: String,
    h3_udp: String,
}

async fn get_management(State(state): State<AdminState>) -> Json<ManagementInfo> {
    let server = &state.proxy_config.server;
    Json(ManagementInfo {
        mode: "proxy",
        version: VERSION,
        uptime_secs: state.started_at.elapsed().as_secs(),
        routes_path: state.routes_path.display().to_string(),
        route_count: state.router.route_count(),
        tls_host_count: state.cert_store.host_count(),
        enable_h3: server.enable_h3,
        auto_https: state.proxy_config.auto_https,
        runtime_mode: state.runtime_cfg.resolved_mode.as_str().to_string(),
        listeners: ListenerInfo {
            http: server.http_listen.clone(),
            https: server.https_listen.clone(),
            h3_udp: server.h3_udp_listen.clone(),
        },
        http3: (*state.router.http3_options()).clone(),
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

#[derive(Serialize)]
struct ConfigYamlResponse {
    path: String,
    yaml: String,
}

async fn get_config_yaml(
    State(state): State<AdminState>,
) -> Result<Json<ConfigYamlResponse>, (StatusCode, Json<ApiError>)> {
    let yaml = std::fs::read_to_string(&state.routes_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
    })?;
    Ok(Json(ConfigYamlResponse {
        path: state.routes_path.display().to_string(),
        yaml,
    }))
}

async fn get_config(
    State(state): State<AdminState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let loaded = routes_config::load(&state.routes_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
    })?;
    let value = serde_json::json!({
        "routes_path": state.routes_path.display().to_string(),
        "route_count": loaded.table.route_count(),
        "tls_entries": loaded.tls.len(),
        "http3": loaded.http3,
    });
    Ok(Json(value))
}

#[derive(Deserialize)]
struct PutConfigRequest {
    yaml: String,
}

#[derive(Serialize)]
struct ReloadResponse {
    ok: bool,
    route_count: usize,
}

async fn put_config(
    State(state): State<AdminState>,
    Json(body): Json<PutConfigRequest>,
) -> Result<Json<ReloadResponse>, (StatusCode, Json<ApiError>)> {
    routes_config::validate_yaml(&body.yaml).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
    })?;
    std::fs::write(&state.routes_path, &body.yaml).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
    })?;
    routes::reload_from_path(
        state.router.as_ref(),
        state.cert_store.as_ref(),
        &state.routes_path,
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
    })?;
    Ok(Json(ReloadResponse {
        ok: true,
        route_count: state.router.route_count(),
    }))
}

async fn reload_config(
    State(state): State<AdminState>,
) -> Result<Json<ReloadResponse>, (StatusCode, Json<ApiError>)> {
    routes::reload_from_path(
        state.router.as_ref(),
        state.cert_store.as_ref(),
        &state.routes_path,
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
    })?;
    Ok(Json(ReloadResponse {
        ok: true,
        route_count: state.router.route_count(),
    }))
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
    let loaded = routes_config::load(&state.routes_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
    })?;
    let entries = loaded
        .tls
        .into_iter()
        .filter_map(|entry| {
            let cert = entry.source.cert_path()?.display().to_string();
            let key = entry.source.key_path()?.display().to_string();
            Some(TlsEntryView {
                hosts: entry.hosts,
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
) -> AdminState {
    let password = admin_password();
    if password.is_none() {
        warn!("PERTISK_ADMIN_PASSWORD is not set; management API allows unauthenticated access");
    }
    AdminState {
        router,
        cert_store,
        routes_path: proxy_config.routes_config.clone(),
        proxy_config,
        runtime_cfg,
        started_at: Instant::now(),
        auth_password: password,
        session_token: Arc::new(RwLock::new(Some(session_token()))),
        admin_dist: resolve_admin_dist(),
        dev_origin: admin_dev_origin(),
    }
}
