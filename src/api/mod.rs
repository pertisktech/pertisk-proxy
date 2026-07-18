//! Management API + admin UI static file server (Axum on `PERTISK_MANAGEMENT_ADDR`).

#[cfg(all(feature = "admin", feature = "ingress"))]
pub mod kubernetes;

#[cfg(feature = "admin")]
pub mod backup;

#[cfg(all(feature = "admin", feature = "acme"))]
pub mod acme;

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::{
    body::Body,
    extract::{Path as AxumPath, Query, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::{delete, get, post},
    Json, Router,
};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};

use crate::config::ProxyConfig;
use crate::db::{CertificateRow, Database, DnsProviderRow};
use crate::log::{dedupe_consecutive_system_logs, ProxyLog, ProxyLogEntry};
use crate::metrics::ProxyMetrics;
use crate::proxy::apply;
use crate::proxy_config::{Config, TlsConfig, TlsSource};
use crate::runtime::RuntimeConfig;
use crate::tls::{CertStore, Http01ChallengeStore};
use crate::Router as ProxyRouter;

#[cfg(feature = "acme")]
use crate::tls::AcmeManager;

const VERSION: &str = env!("pertisk_proxy_VERSION");
const SESSION_TTL_SECS: u64 = 86_400;

#[derive(Clone)]
pub struct SessionEntry {
    pub username: String,
    pub expires_at: Instant,
}

pub type Sessions = Arc<DashMap<String, SessionEntry>>;

pub fn new_sessions() -> Sessions {
    Arc::new(DashMap::new())
}

#[derive(Clone)]
pub struct LeaderElectionState {
    pub enabled: bool,
    pub is_leader: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub namespace: String,
    pub lease_name: String,
}

#[derive(Clone)]
pub struct AdminState {
    pub router: Arc<ProxyRouter>,
    pub cert_store: Arc<CertStore>,
    pub proxy_config: ProxyConfig,
    pub runtime_cfg: RuntimeConfig,
    pub runtime_config: Arc<RwLock<Config>>,
    pub started_at: Instant,
    pub auth_required: bool,
    pub env_password: Option<String>,
    pub sessions: Option<Sessions>,
    pub admin_dist: PathBuf,
    pub dev_origin: Option<String>,
    pub db: Option<Arc<Database>>,
    pub certs_dir: PathBuf,
    pub http01_store: Arc<Http01ChallengeStore>,
    pub proxy_log: Arc<ProxyLog>,
    pub proxy_log_enabled: Arc<AtomicBool>,
    pub metrics: ProxyMetrics,
    pub viewer_mode: bool,
    #[cfg(feature = "ingress")]
    pub kube_client: Option<kube::Client>,
    #[cfg(feature = "ingress")]
    pub ingress_class: Option<String>,
    #[cfg(feature = "ingress")]
    pub gateway_class: Option<String>,
    #[cfg(feature = "ingress")]
    pub gateway_api_enabled: bool,
    #[cfg(feature = "ingress")]
    pub leader_election: Option<LeaderElectionState>,
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
        .route("/api/metrics", get(get_metrics))
        .route("/api/logs", get(get_logs))
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
        .route("/api/backup/export", get(backup::backup_export))
        .route("/api/backup/restore", post(backup::backup_restore));

    #[cfg(feature = "ingress")]
    let protected = protected
        .route("/api/kubernetes/namespaces", get(kubernetes::kubernetes_namespaces))
        .route("/api/kubernetes/pods", get(kubernetes::kubernetes_pods))
        .route("/api/kubernetes/deployments", get(kubernetes::kubernetes_deployments))
        .route("/api/kubernetes/services", get(kubernetes::kubernetes_services))
        .route("/api/kubernetes/configmaps", get(kubernetes::kubernetes_configmaps))
        .route("/api/kubernetes/secrets", get(kubernetes::kubernetes_secrets))
        .route("/api/kubernetes/tls-secrets", get(kubernetes::kubernetes_tls_secrets))
        .route(
            "/api/kubernetes/ingresses",
            get(kubernetes::kubernetes_ingresses).post(kubernetes::kubernetes_ingresses_create),
        )
        .route(
            "/api/kubernetes/ingresses/{namespace}/{name}",
            get(kubernetes::kubernetes_ingress_get)
                .put(kubernetes::kubernetes_ingress_update)
                .delete(kubernetes::kubernetes_ingress_delete),
        )
        .route(
            "/api/kubernetes/gateway-sites",
            post(kubernetes::kubernetes_gateway_sites_create),
        )
        .route(
            "/api/kubernetes/gateway-sites/{namespace}/{name}",
            get(kubernetes::kubernetes_gateway_site_get)
                .put(kubernetes::kubernetes_gateway_site_update)
                .delete(kubernetes::kubernetes_gateway_site_delete),
        )
        .route(
            "/api/kubernetes/gateways",
            get(kubernetes::kubernetes_gateways).post(kubernetes::kubernetes_gateways_create),
        )
        .route(
            "/api/kubernetes/gateways/{namespace}/{name}",
            get(kubernetes::kubernetes_gateway_get)
                .put(kubernetes::kubernetes_gateway_update)
                .delete(kubernetes::kubernetes_gateway_delete),
        )
        .route("/api/kubernetes/httproutes", get(kubernetes::kubernetes_httproutes))
        .route("/api/kubernetes/nodes", get(kubernetes::kubernetes_nodes))
        .route("/api/kubernetes/events", get(kubernetes::kubernetes_events))
        .route(
            "/api/kubernetes/cluster-summary",
            get(kubernetes::kubernetes_cluster_summary),
        );
    let protected = protected.layer(middleware::from_fn_with_state(
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
    if !state.auth_required {
        return Ok(next.run(req).await);
    }
    if is_authorized(&state, req.headers()).await {
        Ok(next.run(req).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

fn session_ttl_secs() -> u64 {
    std::env::var("PERTISK_SESSION_TTL_SECS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(SESSION_TTL_SECS)
}

fn session_username(sessions: &DashMap<String, SessionEntry>, token: &str) -> Option<String> {
    let now = Instant::now();
    let entry = sessions.get(token)?;
    if entry.expires_at > now {
        Some(entry.username.clone())
    } else {
        drop(entry);
        sessions.remove(token);
        None
    }
}

pub(super) async fn is_authorized(state: &AdminState, headers: &HeaderMap) -> bool {
    resolve_username(state, headers).await.is_some()
}

async fn resolve_username(state: &AdminState, headers: &HeaderMap) -> Option<String> {
    let token = extract_bearer(headers)?;
    if let Some(ref sessions) = state.sessions {
        if let Some(username) = session_username(sessions, token) {
            return Some(username);
        }
        if let Some(ref db) = state.db {
            if let Ok(Some((username, expires_at))) = db.get_session(token).await {
                let remaining_secs = (expires_at - chrono::Utc::now()).num_seconds().max(0) as u64;
                sessions.insert(
                    token.to_string(),
                    SessionEntry {
                        username: username.clone(),
                        expires_at: Instant::now() + Duration::from_secs(remaining_secs.max(1)),
                    },
                );
                return Some(username);
            }
        }
    }
    None
}

const DEFAULT_ADMIN_USERNAME: &str = crate::db::DEFAULT_ADMIN_USERNAME;

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
        auth_required: state.auth_required,
    })
}

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
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
    if !state.auth_required {
        return Ok(Json(LoginResponse {
            token: String::new(),
            username: body.username,
            expires_in: 0,
        }));
    }
    let Some(ref sessions) = state.sessions else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiError {
                error: "login not configured".into(),
            }),
        ));
    };

    let username = body.username.trim();
    if username.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                error: "username required".into(),
            }),
        ));
    }

    let authenticated = if let Some(ref db) = state.db {
        let Some(hash) = db
            .get_user_password_hash(username)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiError {
                        error: e.to_string(),
                    }),
                )
            })?
        else {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ApiError {
                    error: "invalid username or password".into(),
                }),
            ));
        };
        bcrypt::verify(&body.password, &hash).unwrap_or(false)
    } else if let Some(ref expected) = state.env_password {
        username == DEFAULT_ADMIN_USERNAME && body.password == *expected
    } else {
        false
    };

    if !authenticated {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiError {
                error: "invalid username or password".into(),
            }),
        ));
    }

    let ttl_secs = session_ttl_secs();
    let token = uuid::Uuid::new_v4().to_string();
    let expires_at = Instant::now() + Duration::from_secs(ttl_secs);
    sessions.insert(
        token.clone(),
        SessionEntry {
            username: username.to_string(),
            expires_at,
        },
    );
    if let Some(ref db) = state.db {
        let expires_at_chrono = chrono::Utc::now() + chrono::Duration::seconds(ttl_secs as i64);
        if let Err(err) = db
            .insert_session(&token, username, expires_at_chrono)
            .await
        {
            warn!(error = %err, "failed to persist session to database");
        }
    }

    Ok(Json(LoginResponse {
        token,
        username: username.to_string(),
        expires_in: ttl_secs,
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
    let username = resolve_username(&state, &headers).await;
    Json(AuthCheckResponse {
        authenticated: !state.auth_required || username.is_some(),
        username,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct ManagementInfo {
    mode: &'static str,
    version: &'static str,
    uptime_secs: u64,
    db_path: String,
    management_addr: String,
    route_count: usize,
    site_count: usize,
    backend_count: usize,
    tls_count: usize,
    tls_host_count: usize,
    enable_h3: bool,
    auto_https: bool,
    runtime_mode: String,
    tuning: TuningInfo,
    listeners: ListenerInfo,
    http3: crate::http3_options::Http3Options,
    hostname: Option<String>,
    os: Option<String>,
    cpu_count: Option<u32>,
    cpu_usage_percent: Option<f32>,
    memory_total_bytes: Option<u64>,
    memory_used_bytes: Option<u64>,
    disk_total_bytes: Option<u64>,
    disk_used_bytes: Option<u64>,
    disk_mount_point: Option<String>,
    process_cpu_usage_percent: Option<f32>,
    process_memory_bytes: Option<u64>,
    process_pid: u32,
    ipv4_addrs: Vec<String>,
    ipv6_addrs: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gateway_api_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    helm_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ingress_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gateway_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    leader_election: Option<LeaderElectionInfo>,
}

#[derive(Serialize)]
struct LeaderElectionInfo {
    enabled: bool,
    is_leader: bool,
    namespace: String,
    lease_name: String,
}

#[derive(Serialize)]
struct ListenerInfo {
    http: String,
    https: String,
    h3_udp: String,
}

#[derive(Serialize)]
struct TuningInfo {
    requested_mode: String,
    resolved_mode: String,
    tokio_worker_threads: usize,
    max_blocking_threads: usize,
    pingora_service_threads: usize,
    pingora_listener_tasks_per_fd: usize,
    pingora_upstream_keepalive_pool_size: usize,
    h3_worker_threads: usize,
    tcp_listen_backlog: i32,
    h3_stack: &'static str,
    udp_offload: &'static str,
    h3_upstream_pool: H3UpstreamPoolInfo,
    /// Effective transport for the compiled HTTP/3 stack (Quinn by default).
    /// Distinct from `ManagementInfo.http3`, which is route/config storage used by tokio-quiche.
    effective_quic: Option<EffectiveQuicInfo>,
    kernel: KernelTuningInfo,
}

#[derive(Serialize)]
struct H3UpstreamPoolInfo {
    max_idle_per_host: usize,
    idle_timeout_secs: u64,
    tcp_keepalive_secs: u64,
}

#[derive(Serialize)]
struct EffectiveQuicInfo {
    source: &'static str,
    idle_timeout_secs: u64,
    keepalive_secs: Option<u64>,
    max_streams_bidi: u64,
    stream_receive_window: u64,
    conn_receive_window: u64,
    udp_buffer_bytes: Option<usize>,
    congestion_control: Option<String>,
    enable_0rtt: Option<bool>,
    enable_pacing: Option<bool>,
    listeners: Option<usize>,
}

#[derive(Serialize)]
struct KernelTuningInfo {
    cpu_affinity: Option<String>,
    open_files_limit: Option<u64>,
    rmem_max: Option<u64>,
    wmem_max: Option<u64>,
    somaxconn: Option<u64>,
    netdev_max_backlog: Option<u64>,
    tcp_max_syn_backlog: Option<u64>,
    tcp_congestion_control: Option<String>,
    default_qdisc: Option<String>,
    ip_local_port_range: Option<String>,
    tcp_tw_reuse: Option<String>,
}

fn read_trimmed(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn read_u64(path: &str) -> Option<u64> {
    read_trimmed(path)?.parse().ok()
}

fn process_cpu_affinity() -> Option<String> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix("Cpus_allowed_list:")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn process_open_files_limit() -> Option<u64> {
    let limits = std::fs::read_to_string("/proc/self/limits").ok()?;
    limits.lines().find_map(|line| {
        let rest = line.strip_prefix("Max open files")?.trim();
        let soft = rest.split_whitespace().next()?;
        if soft.eq_ignore_ascii_case("unlimited") {
            Some(u64::MAX)
        } else {
            soft.parse().ok()
        }
    })
}

fn kernel_tuning_info() -> KernelTuningInfo {
    KernelTuningInfo {
        cpu_affinity: process_cpu_affinity(),
        open_files_limit: process_open_files_limit(),
        rmem_max: read_u64("/proc/sys/net/core/rmem_max"),
        wmem_max: read_u64("/proc/sys/net/core/wmem_max"),
        somaxconn: read_u64("/proc/sys/net/core/somaxconn"),
        netdev_max_backlog: read_u64("/proc/sys/net/core/netdev_max_backlog"),
        tcp_max_syn_backlog: read_u64("/proc/sys/net/ipv4/tcp_max_syn_backlog"),
        tcp_congestion_control: read_trimmed("/proc/sys/net/ipv4/tcp_congestion_control"),
        default_qdisc: read_trimmed("/proc/sys/net/core/default_qdisc"),
        ip_local_port_range: read_trimmed("/proc/sys/net/ipv4/ip_local_port_range"),
        tcp_tw_reuse: read_trimmed("/proc/sys/net/ipv4/tcp_tw_reuse"),
    }
}

fn effective_quic_info() -> Option<EffectiveQuicInfo> {
    #[cfg(feature = "h3-quinn")]
    {
        let q = crate::h3::effective_transport_config();
        Some(EffectiveQuicInfo {
            source: "env (Quinn)",
            idle_timeout_secs: q.idle_timeout_secs,
            keepalive_secs: Some(q.keepalive_secs),
            max_streams_bidi: u64::from(q.max_streams_bidi),
            stream_receive_window: u64::from(q.stream_receive_window),
            conn_receive_window: u64::from(q.conn_receive_window),
            udp_buffer_bytes: Some(q.udp_buffer_bytes),
            congestion_control: None,
            enable_0rtt: None,
            enable_pacing: None,
            listeners: None,
        })
    }
    #[cfg(not(feature = "h3-quinn"))]
    {
        None
    }
}

fn tuning_info(runtime_cfg: &RuntimeConfig) -> TuningInfo {
    let pingora = crate::runtime::pingora_server_conf(runtime_cfg);
    let h3_pool = crate::h3::upstream_pool_config(runtime_cfg);
    let h3_stack = if cfg!(feature = "h3-quinn") {
        "quinn"
    } else if cfg!(feature = "h3-quiche") {
        "tokio-quiche"
    } else {
        "disabled"
    };
    TuningInfo {
        requested_mode: runtime_cfg.requested_mode.as_str().to_string(),
        resolved_mode: runtime_cfg.resolved_mode.as_str().to_string(),
        tokio_worker_threads: runtime_cfg.worker_threads,
        max_blocking_threads: runtime_cfg.max_blocking_threads,
        pingora_service_threads: pingora.threads,
        pingora_listener_tasks_per_fd: pingora.listener_tasks_per_fd,
        pingora_upstream_keepalive_pool_size: pingora.upstream_keepalive_pool_size,
        h3_worker_threads: crate::runtime::h3_worker_threads(runtime_cfg),
        tcp_listen_backlog: crate::runtime::tcp_listen_backlog(runtime_cfg),
        h3_stack,
        udp_offload: if cfg!(target_os = "linux") {
            "automatic (GSO/GRO when supported)"
        } else {
            "platform dependent"
        },
        h3_upstream_pool: H3UpstreamPoolInfo {
            max_idle_per_host: h3_pool.max_idle_per_host,
            idle_timeout_secs: h3_pool.idle_timeout_secs,
            tcp_keepalive_secs: h3_pool.tcp_keepalive_secs,
        },
        effective_quic: effective_quic_info(),
        kernel: kernel_tuning_info(),
    }
}

#[cfg(feature = "ingress")]
fn management_ingress_fields(state: &AdminState) -> (Option<bool>, Option<String>, Option<String>, Option<LeaderElectionInfo>) {
    (
        if state.viewer_mode {
            Some(state.gateway_api_enabled)
        } else {
            None
        },
        if state.viewer_mode {
            state.ingress_class.clone()
        } else {
            None
        },
        if state.viewer_mode {
            state.gateway_class.clone()
        } else {
            None
        },
        state.leader_election.as_ref().map(|le| LeaderElectionInfo {
            enabled: le.enabled,
            is_leader: le
                .is_leader
                .as_ref()
                .map(|f| f.load(Ordering::Relaxed))
                .unwrap_or(true),
            namespace: le.namespace.clone(),
            lease_name: le.lease_name.clone(),
        }),
    )
}

#[derive(Serialize)]
struct MetricsResponse {
    uptime_secs: u64,
    log_entries: u64,
    http_requests_total: u64,
    https_requests_total: u64,
    grpc_requests_total: u64,
    h2_requests_total: u64,
    h3_requests_total: u64,
    h3_vs_h2_ratio: f64,
    active_connections: u64,
    bytes_sent_total: u64,
    bytes_received_total: u64,
    upstream_errors_total: u64,
    site_h2_requests_total: std::collections::HashMap<String, u64>,
    site_h3_requests_total: std::collections::HashMap<String, u64>,
    metrics_addr: String,
}

async fn get_metrics(
    State(state): State<AdminState>,
    headers: HeaderMap,
) -> Result<Json<MetricsResponse>, (StatusCode, Json<ApiError>)> {
    if !is_authorized(&state, &headers).await {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiError {
                error: "unauthorized".into(),
            }),
        ));
    }

    let m = &state.metrics;
    let h3 = m.h3_requests_total.load(Ordering::Relaxed) as f64;
    let h2 = m.h2_requests_total.load(Ordering::Relaxed) as f64;
    let h3_vs_h2_ratio = if h2 > 0.0 {
        h3 / h2
    } else if h3 > 0.0 {
        f64::INFINITY
    } else {
        0.0
    };

    let mut site_h2_requests_total = std::collections::HashMap::new();
    let mut site_h3_requests_total = std::collections::HashMap::new();
    for (host, (h2_count, h3_count)) in m.site_protocol_snapshot() {
        site_h2_requests_total.insert(host.clone(), h2_count);
        site_h3_requests_total.insert(host, h3_count);
    }

    Ok(Json(MetricsResponse {
        uptime_secs: state.started_at.elapsed().as_secs(),
        log_entries: state.proxy_log.len() as u64,
        http_requests_total: m.http_requests_total.load(Ordering::Relaxed),
        https_requests_total: m.https_requests_total.load(Ordering::Relaxed),
        grpc_requests_total: m.grpc_requests_total.load(Ordering::Relaxed),
        h2_requests_total: m.h2_requests_total.load(Ordering::Relaxed),
        h3_requests_total: m.h3_requests_total.load(Ordering::Relaxed),
        h3_vs_h2_ratio,
        active_connections: m.active_connections.load(Ordering::Relaxed),
        bytes_sent_total: m.bytes_sent_total.load(Ordering::Relaxed),
        bytes_received_total: m.bytes_received_total.load(Ordering::Relaxed),
        upstream_errors_total: m.upstream_errors_total.load(Ordering::Relaxed),
        site_h2_requests_total,
        site_h3_requests_total,
        metrics_addr: crate::metrics::metrics_addr_from_env().to_string(),
    }))
}

async fn get_management(State(state): State<AdminState>) -> Json<ManagementInfo> {
    let server = &state.proxy_config.server;
    let cfg = state.runtime_config.read().await;
    let data_path = state
        .db
        .as_ref()
        .map(|d| d.path().to_path_buf())
        .unwrap_or_else(|| state.certs_dir.clone());
    let (disk_total_bytes, disk_used_bytes, disk_mount_point) =
        disk_usage_for_path(&data_path);
    let (
        hostname,
        os,
        cpu_count,
        cpu_usage_percent,
        memory_total_bytes,
        memory_used_bytes,
        process_cpu_usage_percent,
        process_memory_bytes,
        ipv4_addrs,
        ipv6_addrs,
    ) = gather_system_info();
    let (gateway_api_enabled, ingress_class, gateway_class, leader_election) = {
        #[cfg(feature = "ingress")]
        {
            management_ingress_fields(&state)
        }
        #[cfg(not(feature = "ingress"))]
        {
            (None, None, None, None)
        }
    };
    Json(ManagementInfo {
        mode: if state.viewer_mode { "ingress" } else { "proxy" },
        version: VERSION,
        uptime_secs: state.started_at.elapsed().as_secs(),
        db_path: state
            .db
            .as_ref()
            .map(|d| d.path().display().to_string())
            .unwrap_or_else(|| "(kubernetes)".into()),
        management_addr: cfg.management_addr.to_string(),
        route_count: state.router.route_count(),
        site_count: cfg.sites.len(),
        backend_count: cfg.backends.len(),
        tls_count: cfg.tls.len(),
        tls_host_count: state.cert_store.host_count(),
        enable_h3: server.enable_h3,
        auto_https: state.proxy_config.auto_https,
        runtime_mode: state.runtime_cfg.resolved_mode.as_str().to_string(),
        tuning: tuning_info(&state.runtime_cfg),
        listeners: ListenerInfo {
            http: crate::h3::effective_listen_display(&server.http_listen),
            https: crate::h3::effective_listen_display(&server.https_listen),
            h3_udp: crate::h3::effective_udp_listen_display(&server.h3_udp_listen),
        },
        http3: cfg.http3.clone(),
        hostname,
        os,
        cpu_count,
        cpu_usage_percent,
        memory_total_bytes,
        memory_used_bytes,
        disk_total_bytes,
        disk_used_bytes,
        disk_mount_point,
        process_cpu_usage_percent,
        process_memory_bytes,
        process_pid: std::process::id(),
        ipv4_addrs,
        ipv6_addrs,
        gateway_api_enabled,
        helm_enabled: if state.viewer_mode {
            Some(std::env::var("PERTISK_HELM_ENABLED")
                .ok()
                .map(|v| !matches!(v.as_str(), "0" | "false" | "FALSE" | "no" | "NO"))
                .unwrap_or(false))
        } else {
            None
        },
        ingress_class,
        gateway_class,
        leader_election,
    })
}

fn disk_usage_for_path(data_path: &std::path::Path) -> (Option<u64>, Option<u64>, Option<String>) {
    if !sysinfo::IS_SUPPORTED_SYSTEM {
        return (None, None, None);
    }

    let path = std::fs::canonicalize(data_path).unwrap_or_else(|_| data_path.to_path_buf());
    let disks = sysinfo::Disks::new_with_refreshed_list();

    let mut best: Option<(&sysinfo::Disk, usize)> = None;
    for disk in disks.list() {
        let mount = disk.mount_point();
        if path.starts_with(mount) {
            let len = mount.as_os_str().len();
            if best.map(|(_, best_len)| len > best_len).unwrap_or(true) {
                best = Some((disk, len));
            }
        }
    }

    let Some((disk, _)) = best else {
        return (None, None, None);
    };

    let total = disk.total_space();
    let used = total.saturating_sub(disk.available_space());
    (
        Some(total),
        Some(used),
        Some(disk.mount_point().display().to_string()),
    )
}

fn gather_system_info() -> (
    Option<String>,
    Option<String>,
    Option<u32>,
    Option<f32>,
    Option<u64>,
    Option<u64>,
    Option<f32>,
    Option<u64>,
    Vec<String>,
    Vec<String>,
) {
    let mut hostname = None;
    let os = if sysinfo::IS_SUPPORTED_SYSTEM {
        sysinfo::System::name()
            .or_else(|| sysinfo::System::long_os_version())
            .or_else(|| Some(std::env::consts::OS.to_string()))
    } else {
        Some(std::env::consts::OS.to_string())
    };
    let mut cpu_count = None;
    let mut cpu_usage_percent = None;
    let mut memory_total_bytes = None;
    let mut memory_used_bytes = None;
    let mut process_cpu_usage_percent = None;
    let mut process_memory_bytes = None;

    if sysinfo::IS_SUPPORTED_SYSTEM {
        let mut sys = sysinfo::System::new_all();
        sys.refresh_memory();
        sys.refresh_cpu_all();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All);
        hostname = sysinfo::System::host_name();
        cpu_count = Some(sys.cpus().len() as u32);
        cpu_usage_percent = Some(sys.global_cpu_usage());
        memory_total_bytes = Some(sys.total_memory());
        memory_used_bytes = Some(sys.used_memory());

        let pid = sysinfo::Pid::from_u32(std::process::id());
        if let Some(process) = sys.process(pid) {
            process_cpu_usage_percent = Some(process.cpu_usage());
            process_memory_bytes = Some(process.memory());
        }
    }

    fn is_private_or_docker_ipv4(ip: &std::net::Ipv4Addr) -> bool {
        let o = ip.octets();
        o[0] == 127
            || (o[0] == 172 && o[1] >= 16 && o[1] <= 31)
            || (o[0] == 192 && o[1] == 168)
    }

    let (mut ipv4_addrs, mut ipv6_addrs): (Vec<String>, Vec<String>) =
        match local_ip_address::list_afinet_netifas() {
            Ok(list) => {
                let mut v4 = Vec::new();
                let mut v6 = Vec::new();
                for (_name, ip) in list {
                    match ip {
                        std::net::IpAddr::V4(a) => {
                            if !is_private_or_docker_ipv4(&a) {
                                v4.push(a.to_string());
                            }
                        }
                        std::net::IpAddr::V6(a) => {
                            let s = a.segments();
                            if s[0] >= 0x2000 && s[0] <= 0x3fff {
                                v6.push(a.to_string());
                            }
                        }
                    }
                }
                (v4, v6)
            }
            Err(_) => (Vec::new(), Vec::new()),
        };
    ipv4_addrs.sort();
    ipv6_addrs.sort();
    ipv4_addrs.dedup();
    ipv6_addrs.dedup();

    (
        hostname,
        os,
        cpu_count,
        cpu_usage_percent,
        memory_total_bytes,
        memory_used_bytes,
        process_cpu_usage_percent,
        process_memory_bytes,
        ipv4_addrs,
        ipv6_addrs,
    )
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
struct LogsQuery {
    #[serde(rename = "type")]
    log_type: Option<String>,
    host: Option<String>,
}

async fn get_logs(
    State(state): State<AdminState>,
    Query(q): Query<LogsQuery>,
) -> Json<Vec<ProxyLogEntry>> {
    let mut entries = state.proxy_log.recent(500).await;
    entries.retain(|e| e.entry_type != crate::log::LogEntryType::Request);
    entries.retain(|e| crate::log::ui_log_enabled(e.level));

    if let Some(ref t) = q.log_type {
        match t.as_str() {
            "system" => entries.retain(ProxyLogEntry::is_system),
            "proxy" | "domain" | "http" => entries.retain(ProxyLogEntry::has_domain),
            _ => {}
        }
    }

    if let Some(ref host) = q.host {
        let host_trim = host.trim().to_lowercase();
        if !host_trim.is_empty() {
            entries.retain(|e| {
                e.host
                    .as_ref()
                    .map(|h| {
                        let h = h.trim().to_lowercase();
                        h == host_trim || h.ends_with(&format!(".{host_trim}"))
                    })
                    .unwrap_or(false)
            });
        }
    }

    if q.log_type
        .as_deref()
        .is_some_and(|t| t.eq_ignore_ascii_case("system"))
    {
        entries = dedupe_consecutive_system_logs(entries);
    }

    Json(entries)
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
    crate::proxy_config::normalize_tls_config(&mut cfg.tls);
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
    if state.viewer_mode {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiError {
                error: "config is read-only in ingress mode; manage via Kubernetes".into(),
            }),
        ));
    }
    for tls in &mut body.tls {
        tls.expires_at = None;
    }
    crate::proxy_config::normalize_tls_config(&mut body.tls);
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
    backup::activate_proxy_config(&state, db.as_ref(), &body)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiError {
                    error: e,
                }),
            )
        })?;
    state
        .proxy_log
        .push(ProxyLogEntry::config_reload(format!(
            "config saved ({} sites, {} routes)",
            body.sites.len(),
            state.router.route_count()
        )))
        .await;
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
    state
        .proxy_log_enabled
        .store(cfg.proxy_log, Ordering::Relaxed);
    state
        .proxy_log
        .push(ProxyLogEntry::config_reload(format!(
            "config reloaded ({} sites, {} routes)",
            cfg.sites.len(),
            state.router.route_count()
        )))
        .await;
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
    for key in ["PERTISK_ADMIN_PASSWORD", "PERTISK_PASSWORD"] {
        if let Ok(s) = std::env::var(key) {
            let s = s.trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
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

pub async fn load_sessions_from_db(db: &Database, sessions: &Sessions) -> Result<()> {
    let rows = db.load_active_sessions().await?;
    let now = Instant::now();
    for (token, username, expires_at) in rows {
        let remaining_secs = (expires_at - chrono::Utc::now()).num_seconds().max(0) as u64;
        sessions.insert(
            token,
            SessionEntry {
                username,
                expires_at: now + Duration::from_secs(remaining_secs.max(1)),
            },
        );
    }
    Ok(())
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
    sessions: Option<Sessions>,
    proxy_log: Arc<ProxyLog>,
    proxy_log_enabled: Arc<AtomicBool>,
    metrics: ProxyMetrics,
) -> AdminState {
    let env_password = admin_password();
    let auth_required = db.is_some() || env_password.is_some();
    if !auth_required {
        warn!("PERTISK_ADMIN_PASSWORD is not set and no database configured; management API allows unauthenticated access");
    } else if db.is_some() {
        info!("management API requires login (users stored in database)");
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
        auth_required,
        env_password,
        sessions,
        admin_dist: resolve_admin_dist(),
        dev_origin: admin_dev_origin(),
        db,
        certs_dir,
        http01_store,
        proxy_log,
        proxy_log_enabled,
        metrics,
        viewer_mode: false,
        #[cfg(feature = "ingress")]
        kube_client: None,
        #[cfg(feature = "ingress")]
        ingress_class: None,
        #[cfg(feature = "ingress")]
        gateway_class: None,
        #[cfg(feature = "ingress")]
        gateway_api_enabled: false,
        #[cfg(feature = "ingress")]
        leader_election: None,
        #[cfg(feature = "acme")]
        acme_manager,
    }
}

#[cfg(feature = "ingress")]
pub fn build_ingress_state(
    router: Arc<ProxyRouter>,
    cert_store: Arc<CertStore>,
    ingress_config: crate::config::IngressConfig,
    runtime_cfg: RuntimeConfig,
    kube_client: Option<kube::Client>,
    ingress_class: Option<String>,
    gateway_class: Option<String>,
    gateway_api_enabled: bool,
    leader_election: Option<LeaderElectionState>,
    runtime_config: Arc<RwLock<Config>>,
    sessions: Option<Sessions>,
    http01_store: Arc<Http01ChallengeStore>,
    proxy_log: Arc<ProxyLog>,
    proxy_log_enabled: Arc<AtomicBool>,
    certs_dir: PathBuf,
    metrics: ProxyMetrics,
) -> AdminState {
    let env_password = admin_password();
    let auth_required = env_password.is_some()
        || std::env::var("PERTISK_API_TOKEN")
            .ok()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
    if !auth_required {
        warn!("PERTISK_PASSWORD / PERTISK_API_TOKEN not set; ingress management API allows unauthenticated access");
    }
    let proxy_config = ProxyConfig {
        db_path: certs_dir.clone(),
        server: ingress_config.server.clone(),
        auto_https: false,
        migrate_routes_path: None,
    };
    AdminState {
        router,
        cert_store,
        proxy_config,
        runtime_cfg,
        runtime_config,
        started_at: Instant::now(),
        auth_required,
        env_password,
        sessions,
        admin_dist: resolve_admin_dist(),
        dev_origin: admin_dev_origin(),
        db: None,
        certs_dir,
        http01_store,
        proxy_log,
        proxy_log_enabled,
        metrics,
        viewer_mode: true,
        kube_client,
        ingress_class,
        gateway_class,
        gateway_api_enabled,
        leader_election,
        #[cfg(feature = "acme")]
        acme_manager: None,
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
