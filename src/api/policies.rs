//! Named Access Control Lists and WAF policy CRUD (stored in proxy_config JSON).

use axum::{
    extract::{Path as AxumPath, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{ApiError, AdminState};
use crate::proxy_config::{AccessList, Config, NamedWafPolicy};

#[derive(Deserialize)]
pub struct AccessListBody {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub geoip: crate::geoip::GeoIpPolicy,
}

#[derive(Deserialize)]
pub struct WafPolicyBody {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub security: crate::security::SecurityPolicy,
}

#[derive(Serialize)]
pub struct IdResponse {
    pub id: String,
}

#[derive(Serialize)]
pub struct OkResponse {
    pub ok: bool,
}

fn require_db(state: &AdminState) -> Result<&crate::db::Database, (StatusCode, Json<ApiError>)> {
    state.db.as_deref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiError {
                error: "database not configured".into(),
            }),
        )
    })
}

/// Load working config: runtime sites/backends plus policy lists (from DB in ingress).
async fn load_config(state: &AdminState) -> Result<Config, (StatusCode, Json<ApiError>)> {
    let mut cfg = state.runtime_config.read().await.clone();
    if state.viewer_mode {
        if let Some(db) = state.db.as_ref() {
            if let Ok(Some(stored)) = db.get_proxy_config().await {
                cfg.access_lists = stored.access_lists;
                cfg.waf_policies = stored.waf_policies;
            }
        }
    }
    Ok(cfg)
}

/// Persist lists and activate. In ingress mode, only lists are written to SQLite
/// (sites come from Kubernetes); runtime is updated and middleware re-applied.
async fn save_and_activate(
    state: &AdminState,
    db: &crate::db::Database,
    cfg: &Config,
) -> Result<(), (StatusCode, Json<ApiError>)> {
    if state.viewer_mode {
        let mut stored = db.get_proxy_config().await.unwrap_or(None).unwrap_or_default();
        stored.access_lists = cfg.access_lists.clone();
        stored.waf_policies = cfg.waf_policies.clone();
        db.save_proxy_config(&stored).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: e.to_string(),
                }),
            )
        })?;
        crate::proxy::apply::apply_config(state.router.as_ref(), cfg).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiError {
                    error: e.to_string(),
                }),
            )
        })?;
        *state.runtime_config.write().await = cfg.clone();
        return Ok(());
    }

    db.save_proxy_config(cfg).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
    })?;
    crate::api::backup::activate_proxy_config(state, db, cfg)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiError { error: e }),
            )
        })?;
    Ok(())
}

fn normalize_name(name: &str) -> Result<String, (StatusCode, Json<ApiError>)> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                error: "name is required".into(),
            }),
        ));
    }
    Ok(name)
}

pub(crate) async fn access_lists_list(State(state): State<AdminState>) -> Json<Vec<AccessList>> {
    if let Some(db) = state.db.as_ref() {
        if let Ok(Some(stored)) = db.get_proxy_config().await {
            return Json(stored.access_lists);
        }
    }
    let cfg = state.runtime_config.read().await;
    Json(cfg.access_lists.clone())
}

pub(crate) async fn access_lists_create(
    State(state): State<AdminState>,
    Json(body): Json<AccessListBody>,
) -> Result<(StatusCode, Json<IdResponse>), (StatusCode, Json<ApiError>)> {
    let db = require_db(&state)?;
    let name = normalize_name(&body.name)?;
    let id = Uuid::new_v4().to_string();
    let mut cfg = load_config(&state).await?;
    cfg.access_lists.push(AccessList {
        id: id.clone(),
        name,
        description: body
            .description
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        geoip: body.geoip.normalized(),
    });
    save_and_activate(&state, db, &cfg).await?;
    Ok((StatusCode::CREATED, Json(IdResponse { id })))
}

pub(crate) async fn access_lists_get(
    State(state): State<AdminState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<AccessList>, (StatusCode, Json<ApiError>)> {
    let cfg = load_config(&state).await?;
    cfg.access_lists
        .iter()
        .find(|l| l.id == id)
        .cloned()
        .map(Json)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiError {
                    error: "not found".into(),
                }),
            )
        })
}

pub(crate) async fn access_lists_put(
    State(state): State<AdminState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<AccessListBody>,
) -> Result<Json<OkResponse>, (StatusCode, Json<ApiError>)> {
    let db = require_db(&state)?;
    let name = normalize_name(&body.name)?;
    let mut cfg = load_config(&state).await?;
    let Some(list) = cfg.access_lists.iter_mut().find(|l| l.id == id) else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "not found".into(),
            }),
        ));
    };
    list.name = name;
    list.description = body
        .description
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    list.geoip = body.geoip.normalized();
    // Refresh embedded snapshots on sites that reference this list.
    let geoip = list.geoip.clone();
    for site in &mut cfg.sites {
        if site.access_list_id.as_deref() == Some(id.as_str()) {
            site.geoip = geoip.clone();
        }
    }
    save_and_activate(&state, db, &cfg).await?;
    Ok(Json(OkResponse { ok: true }))
}

pub(crate) async fn access_lists_delete(
    State(state): State<AdminState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<OkResponse>, (StatusCode, Json<ApiError>)> {
    let db = require_db(&state)?;
    let mut cfg = load_config(&state).await?;
    let before = cfg.access_lists.len();
    cfg.access_lists.retain(|l| l.id != id);
    if cfg.access_lists.len() == before {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "not found".into(),
            }),
        ));
    }
    for site in &mut cfg.sites {
        if site.access_list_id.as_deref() == Some(id.as_str()) {
            site.access_list_id = None;
        }
    }
    save_and_activate(&state, db, &cfg).await?;
    Ok(Json(OkResponse { ok: true }))
}

pub(crate) async fn waf_policies_list(State(state): State<AdminState>) -> Json<Vec<NamedWafPolicy>> {
    if let Some(db) = state.db.as_ref() {
        if let Ok(Some(stored)) = db.get_proxy_config().await {
            return Json(stored.waf_policies);
        }
    }
    let cfg = state.runtime_config.read().await;
    Json(cfg.waf_policies.clone())
}

pub(crate) async fn waf_policies_create(
    State(state): State<AdminState>,
    Json(body): Json<WafPolicyBody>,
) -> Result<(StatusCode, Json<IdResponse>), (StatusCode, Json<ApiError>)> {
    let db = require_db(&state)?;
    let name = normalize_name(&body.name)?;
    let id = Uuid::new_v4().to_string();
    let mut cfg = load_config(&state).await?;
    cfg.waf_policies.push(NamedWafPolicy {
        id: id.clone(),
        name,
        description: body
            .description
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        security: body.security.normalized(),
    });
    save_and_activate(&state, db, &cfg).await?;
    Ok((StatusCode::CREATED, Json(IdResponse { id })))
}

pub(crate) async fn waf_policies_get(
    State(state): State<AdminState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<NamedWafPolicy>, (StatusCode, Json<ApiError>)> {
    let cfg = load_config(&state).await?;
    cfg.waf_policies
        .iter()
        .find(|p| p.id == id)
        .cloned()
        .map(Json)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiError {
                    error: "not found".into(),
                }),
            )
        })
}

pub(crate) async fn waf_policies_put(
    State(state): State<AdminState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<WafPolicyBody>,
) -> Result<Json<OkResponse>, (StatusCode, Json<ApiError>)> {
    let db = require_db(&state)?;
    let name = normalize_name(&body.name)?;
    let mut cfg = load_config(&state).await?;
    let Some(policy) = cfg.waf_policies.iter_mut().find(|p| p.id == id) else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "not found".into(),
            }),
        ));
    };
    policy.name = name;
    policy.description = body
        .description
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    policy.security = body.security.normalized();
    let security = policy.security.clone();
    for site in &mut cfg.sites {
        if site.waf_policy_id.as_deref() == Some(id.as_str()) {
            site.security = security.clone();
        }
    }
    save_and_activate(&state, db, &cfg).await?;
    Ok(Json(OkResponse { ok: true }))
}

pub(crate) async fn waf_policies_delete(
    State(state): State<AdminState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<OkResponse>, (StatusCode, Json<ApiError>)> {
    let db = require_db(&state)?;
    let mut cfg = load_config(&state).await?;
    let before = cfg.waf_policies.len();
    cfg.waf_policies.retain(|p| p.id != id);
    if cfg.waf_policies.len() == before {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "not found".into(),
            }),
        ));
    }
    for site in &mut cfg.sites {
        if site.waf_policy_id.as_deref() == Some(id.as_str()) {
            site.waf_policy_id = None;
        }
    }
    save_and_activate(&state, db, &cfg).await?;
    Ok(Json(OkResponse { ok: true }))
}
