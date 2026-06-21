//! gRPC and gRPC-Web detection and upstream header handling.
//!
//! Omni exposes gRPC-Web on HTTP/1.1 at `/api/<Service>/<Method>` (unary RPCs like `Get`) and
//! native gRPC (h2c) at `/<Service>/<Method>`. Server-streaming RPCs such as `Watch` need the
//! Pingora GrpcWeb bridge plus h2c upstream; unary calls must pass through on HTTP/1.1 with the
//! `/api/` prefix intact (same as pertisk-rproxy).

use http::{header, Method, Uri};
use pingora_http::{RequestHeader, ResponseHeader};
use pingora_proxy::Session;

use pingora_core::Result;

/// gRPC uses `application/grpc`, `application/grpc+proto`, etc. (not gRPC-Web).
pub fn is_grpc_request(headers: &http::HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE.as_str())
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.starts_with("application/grpc") && !ct.starts_with("application/grpc-web"))
        .unwrap_or(false)
}

/// Native gRPC (not gRPC-Web).
pub fn is_native_grpc_request(headers: &http::HeaderMap) -> bool {
    is_grpc_request(headers)
}

/// Omni Connect/gRPC-Web RPC paths: `/api/<package.Service>/<Method>`.
pub fn is_grpc_rpc_path(path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    path.starts_with("/api/")
        && path.len() > 5
        && path.contains('.')
        && path.contains('/')
}

/// Classify gRPC traffic. All POST `/api/<Service>/<Method>` calls use gRPC-Web upstream
/// (HTTP/1.1, keep `/api/`) except server-streaming methods that need the bridge + h2c.
pub fn classify_grpc_request(
    headers: &http::HeaderMap,
    method: &Method,
    path: &str,
) -> (bool, bool) {
    let api_rpc = *method == Method::POST && is_grpc_rpc_path(path);
    let is_grpc_web = is_grpc_web_request(headers, method, path) || api_rpc;
    let is_grpc = is_grpc_request(headers) || is_grpc_web;
    (is_grpc, is_grpc_web)
}

/// gRPC method name from an RPC path (`/api/foo.Bar/Baz` → `Baz`).
pub fn grpc_rpc_method(path: &str) -> Option<&str> {
    let path = path.split('?').next().unwrap_or(path);
    path.rsplit('/').next().filter(|m| !m.is_empty())
}

/// Server-streaming gRPC-Web RPCs need bridge + h2c + `/api/` strip (Omni `Watch`, etc.).
pub fn is_grpc_server_streaming(path: &str) -> bool {
    matches!(
        grpc_rpc_method(path),
        Some("Watch" | "Subscribe" | "Listen" | "Stream" | "Tail" | "Events")
    )
}

/// Pingora GrpcWeb bridge: only for server-streaming gRPC-Web over HTTP/2 downstream.
pub fn uses_grpc_web_bridge(is_grpc_web: bool, path: &str) -> bool {
    is_grpc_web && is_grpc_server_streaming(path)
}

/// gRPC-Web detection (aligned with pertisk-rproxy, scoped to RPC paths for grpc-metadata).
pub fn is_grpc_web_request(headers: &http::HeaderMap, method: &Method, path: &str) -> bool {
    if headers
        .get(header::CONTENT_TYPE.as_str())
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.starts_with("application/grpc-web"))
        .unwrap_or(false)
    {
        return *method == Method::POST && is_grpc_rpc_path(path);
    }

    if headers
        .get("x-grpc-web")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim() == "1")
        .unwrap_or(false)
    {
        return *method == Method::POST && is_grpc_rpc_path(path);
    }

    if *method != Method::POST || !is_grpc_rpc_path(path) {
        return false;
    }

    headers
        .keys()
        .any(|name| name.as_str().starts_with("grpc-metadata-"))
}

pub fn is_grpc_like_request(headers: &http::HeaderMap, method: &Method, path: &str) -> bool {
    is_grpc_request(headers) || is_grpc_web_request(headers, method, path)
}

/// h2c upstream: native gRPC always; gRPC-Web only for server-streaming (with bridge).
pub fn uses_h2c_upstream(is_grpc: bool, is_grpc_web: bool, path: &str) -> bool {
    if !is_grpc {
        return false;
    }
    if is_grpc_web {
        uses_grpc_web_bridge(true, path)
    } else {
        true
    }
}

/// Strip `/api` prefix for native gRPC upstream paths (Omni h2c layout).
pub fn rewrite_upstream_grpc_path(upstream: &mut RequestHeader) -> Result<()> {
    let path = upstream.uri.path();
    let stripped = if let Some(rest) = path.strip_prefix("/api/") {
        rest
    } else if path == "/api" {
        return rebuild_upstream_path(upstream, "/");
    } else {
        return Ok(());
    };

    let new_path = if stripped.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", stripped.trim_start_matches('/'))
    };
    rebuild_upstream_path(upstream, &new_path)
}

fn rebuild_upstream_path(upstream: &mut RequestHeader, new_path: &str) -> Result<()> {
    let mut parts = upstream.uri.clone().into_parts();
    let query = parts
        .path_and_query
        .as_ref()
        .and_then(|pq| pq.query())
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    parts.path_and_query = Some(format!("{new_path}{query}").parse().map_err(|_| {
        pingora_error::Error::explain(
            pingora_error::ErrorType::InternalError,
            "invalid gRPC upstream path",
        )
    })?);
    upstream.set_uri(Uri::from_parts(parts).map_err(|_| {
        pingora_error::Error::explain(
            pingora_error::ErrorType::InternalError,
            "failed to rebuild gRPC upstream uri",
        )
    })?);
    Ok(())
}

/// Ensure Pingora's GrpcWeb bridge recognizes Omni requests (text/plain + grpc-metadata-*).
pub fn normalize_grpc_web_content_type(req: &mut RequestHeader) {
    if !uses_grpc_web_bridge(true, req.uri.path()) {
        return;
    }
    if req
        .headers
        .get(header::CONTENT_TYPE.as_str())
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("application/grpc-web"))
    {
        return;
    }
    req.insert_header(header::CONTENT_TYPE.as_str(), "application/grpc-web+proto")
        .ok();
}

pub fn validate_downstream(
    req: &RequestHeader,
    session: &Session,
    is_grpc_web: bool,
) -> Result<(), &'static str> {
    if req.method != Method::POST {
        return Err("gRPC requires POST");
    }

    if !is_grpc_web && !session.as_downstream().is_http2() {
        return Err("gRPC requires HTTP/2");
    }

    Ok(())
}

pub fn prepare_upstream_request(upstream: &mut RequestHeader, is_grpc_web: bool, path: &str) {
    upstream.remove_header(header::CONNECTION.as_str());
    upstream.remove_header(header::UPGRADE.as_str());
    upstream.remove_header(header::TRANSFER_ENCODING.as_str());
    upstream.remove_header("keep-alive");
    upstream.remove_header("proxy-connection");

    let native_upstream = !is_grpc_web || uses_grpc_web_bridge(is_grpc_web, path);
    if native_upstream && !upstream.headers.contains_key(header::TE.as_str()) {
        upstream.insert_header(header::TE.as_str(), "trailers").ok();
    }
}

pub fn strip_hop_by_hop_response_headers(resp: &mut ResponseHeader) {
    resp.remove_header(header::TRANSFER_ENCODING.as_str());
    resp.remove_header(header::CONNECTION.as_str());
    resp.remove_header("keep-alive");
    resp.remove_header("proxy-connection");
}

pub fn prepare_streaming_grpc_response_headers(resp: &mut ResponseHeader) {
    strip_hop_by_hop_response_headers(resp);
    resp.remove_header(header::CONTENT_LENGTH.as_str());
    resp.insert_header(header::CACHE_CONTROL.as_str(), "no-cache, no-transform")
        .ok();
    resp.insert_header("x-accel-buffering", "no").ok();
    resp.insert_header("Alt-Svc", "clear").ok();
}

pub async fn respond_error(
    session: &mut Session,
    grpc_status: u16,
    message: &str,
    is_grpc_web: bool,
) -> Result<()> {
    let content_type = if is_grpc_web {
        "application/grpc-web+proto"
    } else {
        "application/grpc"
    };
    let mut resp = ResponseHeader::build(http::StatusCode::OK, Some(3))?;
    resp.insert_header(header::CONTENT_TYPE.as_str(), content_type)?;
    resp.insert_header("grpc-status", grpc_status.to_string())?;
    if !message.is_empty() {
        resp.insert_header("grpc-message", message)?;
    }
    session.write_response_header(Box::new(resp), true).await?;
    Ok(())
}

pub fn grpc_upstream_timeout() -> std::time::Duration {
    let secs = std::env::var("PERTISK_GRPC_UPSTREAM_REQUEST_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(3600);
    if secs == 0 {
        std::time::Duration::MAX
    } else {
        std::time::Duration::from_secs(secs)
    }
}

pub fn grpc_h2_ping_interval() -> std::time::Duration {
    let secs = std::env::var("PERTISK_GRPC_H2C_KEEPALIVE_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(20);
    std::time::Duration::from_secs(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grpc_web_content_type_is_not_native_grpc() {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            "application/grpc-web+proto".parse().unwrap(),
        );
        assert!(!is_grpc_request(&headers));
        assert!(is_grpc_web_request(
            &headers,
            &Method::POST,
            "/api/omni.resources.ResourceService/Get"
        ));
    }

    #[test]
    fn detects_grpc_web_by_metadata_on_rpc_path_only() {
        let mut headers = http::HeaderMap::new();
        headers.insert("grpc-metadata-foo", "bar".parse().unwrap());
        assert!(!is_grpc_web_request(
            &headers,
            &Method::GET,
            "/assets/config.json"
        ));
        assert!(is_grpc_web_request(
            &headers,
            &Method::POST,
            "/api/omni.resources.ResourceService/Get"
        ));
    }

    #[test]
    fn get_uses_h1_passthrough_watch_uses_bridge() {
        let get = "/api/omni.resources.ResourceService/Get";
        let watch = "/api/omni.resources.ResourceService/Watch";
        assert!(!uses_grpc_web_bridge(true, get));
        assert!(uses_grpc_web_bridge(true, watch));
        assert!(!uses_h2c_upstream(true, true, get));
        assert!(uses_h2c_upstream(true, true, watch));
    }

    #[test]
    fn api_rpc_paths_force_grpc_web_even_with_native_content_type() {
        let get = "/api/omni.resources.ResourceService/Get";
        let mut headers = http::HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, "application/grpc".parse().unwrap());
        let (is_grpc, is_grpc_web) = classify_grpc_request(&headers, &Method::POST, get);
        assert!(is_grpc);
        assert!(is_grpc_web);
        assert!(!uses_h2c_upstream(is_grpc, is_grpc_web, get));
    }

    #[test]
    fn native_grpc_without_api_prefix_uses_h2c() {
        let path = "/omni.resources.ResourceService/Watch";
        let mut headers = http::HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, "application/grpc".parse().unwrap());
        let (is_grpc, is_grpc_web) = classify_grpc_request(&headers, &Method::POST, path);
        assert!(is_grpc);
        assert!(!is_grpc_web);
        assert!(uses_h2c_upstream(is_grpc, is_grpc_web, path));
    }

    #[test]
    fn rewrites_api_prefix_for_native_grpc() {
        let mut req = RequestHeader::build(
            http::Method::POST,
            b"/api/omni.resources.ResourceService/Watch",
            None,
        )
        .unwrap();
        rewrite_upstream_grpc_path(&mut req).unwrap();
        assert_eq!(req.uri.path(), "/omni.resources.ResourceService/Watch");
    }

    #[test]
    fn omni_rpc_path_detection() {
        assert!(is_grpc_rpc_path("/api/omni.resources.ResourceService/Watch"));
        assert!(!is_grpc_rpc_path("/api/health"));
    }
}
