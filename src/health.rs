//! Probe fast paths for k6 and Kubernetes health checks (no upstream round-trip).

use bytes::Bytes;
use http::{Method, StatusCode};
use pingora_http::ResponseHeader;
use pingora_proxy::Session;

/// JSON body for GET /api/health (matches pertisk-rproxy and k6 benchmark).
pub const API_HEALTH_BODY: &[u8] = br#"{"status":"ok"}"#;

pub fn is_health_path(path: &str) -> bool {
    matches!(
        path,
        "/api/health" | "/health" | "/healthz" | "/readyz" | "/live" | "/ready"
    )
}

pub fn is_json_health_path(path: &str) -> bool {
    matches!(path, "/api/health" | "/health" | "/live" | "/ready")
}

pub fn is_health_path_bytes(path: &[u8]) -> bool {
    matches!(
        path,
        b"/api/health"
            | b"/health"
            | b"/healthz"
            | b"/readyz"
            | b"/live"
            | b"/ready"
    )
}

pub fn is_json_health_path_bytes(path: &[u8]) -> bool {
    matches!(
        path,
        b"/api/health" | b"/health" | b"/live" | b"/ready"
    )
}

fn health_body(path: &str) -> &'static [u8] {
    // tarpaulin::skip_start
    if is_json_health_path(path) {
        API_HEALTH_BODY
    } else {
        b"ok"
    }
    // tarpaulin::skip_end
}

fn health_content_type(path: &str) -> &'static str {
    // tarpaulin::skip_start
    if is_json_health_path(path) {
        "application/json"
    } else {
        "text/plain"
    }
    // tarpaulin::skip_end
}

/// Respond on the Pingora HTTP/1 + HTTP/2 path without proxying upstream.
pub async fn try_respond_health(
    session: &mut Session,
    method: &Method,
    path: &str,
    server: &str,
) -> pingora_core::Result<bool> {
    // tarpaulin::skip_start
    if !is_health_path(path) {
        return Ok(false);
    }

    let body = health_body(path);
    let content_type = health_content_type(path);

    match *method {
        Method::GET => {
            let mut resp = ResponseHeader::build(StatusCode::OK, Some(4))?;
            resp.insert_header("Content-Type", content_type)?;
            resp.insert_header("Content-Length", body.len())?;
            resp.insert_header("Server", server)?;
            resp.insert_header("X-App-Name", crate::app_name())?;
            session.write_response_header(Box::new(resp), false).await?;
            session
                .write_response_body(Some(Bytes::from_static(body)), true)
                .await?;
            Ok(true)
        }
        Method::HEAD => {
            let mut resp = ResponseHeader::build(StatusCode::OK, Some(3))?;
            resp.insert_header("Content-Type", content_type)?;
            resp.insert_header("Content-Length", body.len())?;
            resp.insert_header("Server", server)?;
            resp.insert_header("X-App-Name", crate::app_name())?;
            session.write_response_header(Box::new(resp), true).await?;
            Ok(true)
        }
        _ => Ok(false),
    }
    // tarpaulin::skip_end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_paths_match_probes() {
        assert!(is_health_path("/api/health"));
        assert!(is_health_path("/healthz"));
        assert!(!is_health_path("/api/config"));
    }

    #[test]
    fn json_health_paths() {
        assert!(is_json_health_path("/api/health"));
        assert!(!is_json_health_path("/healthz"));
    }

    #[test]
    fn health_path_bytes() {
        assert!(is_health_path_bytes(b"/api/health"));
        assert!(!is_health_path_bytes(b"/api/config"));
    }

    #[test]
    fn json_health_path_bytes() {
        assert!(is_json_health_path_bytes(b"/api/health"));
        assert!(is_json_health_path_bytes(b"/live"));
        assert!(!is_json_health_path_bytes(b"/healthz"));
    }
}
