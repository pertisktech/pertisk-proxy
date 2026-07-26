//! SMTP settings API and fire-and-forget login-failure alerts.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use super::{ApiError, AdminState};
use crate::db::{Database, SmtpSettingsUpdate};
use crate::email::{smtp, templates};

#[derive(Serialize)]
pub struct SmtpSettingsResponse {
    pub enabled: bool,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub has_password: bool,
    pub from_email: String,
    pub from_name: String,
    pub use_tls: bool,
    pub alert_to: String,
    pub notify_login_failure: bool,
    pub updated_at: String,
}

#[derive(Deserialize)]
pub struct UpdateSmtpSettingsRequest {
    pub enabled: bool,
    pub host: String,
    pub port: i64,
    #[serde(default)]
    pub username: String,
    /// Omit to leave unchanged; empty string clears the stored password.
    #[serde(default)]
    pub password: Option<String>,
    pub from_email: String,
    #[serde(default)]
    pub from_name: String,
    pub use_tls: bool,
    #[serde(default)]
    pub alert_to: String,
    #[serde(default)]
    pub notify_login_failure: bool,
}

#[derive(Deserialize)]
pub struct TestSmtpRequest {
    #[serde(default)]
    pub to: Option<String>,
}

#[derive(Deserialize)]
pub struct PreviewSmtpQuery {
    #[serde(default = "default_preview_template")]
    pub template: String,
}

fn default_preview_template() -> String {
    "test".into()
}

#[derive(Serialize)]
pub struct PreviewResponse {
    pub html: String,
}

#[derive(Serialize)]
pub struct TestSmtpResponse {
    pub ok: bool,
    pub to: String,
}

fn require_db(state: &AdminState) -> Result<&Database, (StatusCode, Json<ApiError>)> {
    state.db.as_deref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiError {
                error: "database not configured".into(),
            }),
        )
    })
}

fn row_to_response(row: &crate::db::SmtpSettingsRow) -> SmtpSettingsResponse {
    SmtpSettingsResponse {
        enabled: row.enabled,
        host: row.host.clone(),
        port: row.port,
        username: row.username.clone(),
        has_password: !row.password.trim().is_empty(),
        from_email: row.from_email.clone(),
        from_name: row.from_name.clone(),
        use_tls: row.use_tls,
        alert_to: row.alert_to.clone(),
        notify_login_failure: row.notify_login_failure,
        updated_at: row.updated_at.clone(),
    }
}

pub(crate) async fn smtp_get(
    State(state): State<AdminState>,
) -> Result<Json<SmtpSettingsResponse>, (StatusCode, Json<ApiError>)> {
    let db = require_db(&state)?;
    let row = db.get_smtp_settings().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
    })?;
    Ok(Json(row_to_response(&row)))
}

pub(crate) async fn smtp_put(
    State(state): State<AdminState>,
    Json(body): Json<UpdateSmtpSettingsRequest>,
) -> Result<Json<SmtpSettingsResponse>, (StatusCode, Json<ApiError>)> {
    let db = require_db(&state)?;
    if body.port < 1 || body.port > 65535 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                error: "port must be between 1 and 65535".into(),
            }),
        ));
    }
    let row = db
        .update_smtp_settings(SmtpSettingsUpdate {
            enabled: body.enabled,
            host: body.host.trim().to_string(),
            port: body.port,
            username: body.username.trim().to_string(),
            password: body.password,
            from_email: body.from_email.trim().to_string(),
            from_name: body.from_name.trim().to_string(),
            use_tls: body.use_tls,
            alert_to: body.alert_to.trim().to_string(),
            notify_login_failure: body.notify_login_failure,
        })
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: e.to_string(),
                }),
            )
        })?;
    Ok(Json(row_to_response(&row)))
}

pub(crate) async fn smtp_test(
    State(state): State<AdminState>,
    Json(body): Json<TestSmtpRequest>,
) -> Result<Json<TestSmtpResponse>, (StatusCode, Json<ApiError>)> {
    let db = require_db(&state)?;
    let settings = db.get_smtp_settings().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
    })?;

    let to = body
        .to
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| settings.alert_to.trim())
        .to_string();
    if to.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                error: "set Alert to address or pass { to } in the request body".into(),
            }),
        ));
    }

    let (subject, content) = templates::sample_content("test");
    smtp::send_email(&settings, &to, &subject, content, false)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiError {
                    error: e.to_string(),
                }),
            )
        })?;

    Ok(Json(TestSmtpResponse { ok: true, to }))
}

pub(crate) async fn smtp_preview(
    State(state): State<AdminState>,
    Query(query): Query<PreviewSmtpQuery>,
) -> Result<Json<PreviewResponse>, (StatusCode, Json<ApiError>)> {
    let db = require_db(&state)?;
    let settings = db.get_smtp_settings().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
    })?;
    let kind = query.template.trim();
    if kind != "test" && kind != "login_failure" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                error: "template must be test or login_failure".into(),
            }),
        ));
    }
    let from_name = if settings.from_name.trim().is_empty() {
        "Pertisk Proxy"
    } else {
        settings.from_name.trim()
    };
    let (_subject, content) = templates::sample_content(kind);
    Ok(Json(PreviewResponse {
        html: templates::render_html(from_name, &content),
    }))
}

/// Fire-and-forget alert when a management UI login fails.
pub fn notify_login_failure(
    db: Arc<Database>,
    username: String,
    ip_address: String,
    user_agent: String,
) {
    tokio::spawn(async move {
        let settings = match db.get_smtp_settings().await {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!(error = %err, "smtp: failed to load settings for login failure alert");
                return;
            }
        };
        if !settings.enabled {
            return;
        }
        if !settings.notify_login_failure {
            return;
        }
        let to = settings.alert_to.trim();
        if to.is_empty() {
            tracing::warn!("smtp: login failure notify enabled but alert_to is empty");
            return;
        }
        let attempted_at = chrono::Utc::now()
            .format("%Y-%m-%d %H:%M:%S UTC")
            .to_string();
        let content = templates::login_failure_content(&templates::LoginFailureDetails {
            username: username.clone(),
            ip_address,
            attempted_at,
            user_agent,
        });
        if let Err(err) = smtp::send_email(
            &settings,
            to,
            "Failed management login",
            content,
            true,
        )
        .await
        {
            tracing::warn!(error = %err, username = %username, "smtp: login failure alert failed");
        }
    });
}
