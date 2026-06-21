//! gRPC and gRPC-Web detection and upstream header handling.
//!
//! Omni serves Connect JSON and gRPC-Web on HTTP/1.1 at `/api/<Service>/<Method>`. Classification
//! follows pertisk-rproxy (header-based). Connect JSON (`Connect-Protocol-Version`) is plain HTTP
//! passthrough; only native `application/grpc` on `/api/` is forced to HTTP/1.1 gRPC-Web mode so
//! the path is not stripped and h2c is not used.

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

/// Classify gRPC traffic (aligned with pertisk-rproxy header detection).
pub fn classify_grpc_request(
    headers: &http::HeaderMap,
    method: &Method,
    path: &str,
) -> (bool, bool) {
    let on_api_rpc = *method == Method::POST && is_grpc_rpc_path(path);
    let mut is_grpc_web = is_grpc_web_request(headers, method, path);
    // Omni unary `/api/` RPCs may send Content-Type: application/grpc; keep HTTP/1.1 + full path.
    if on_api_rpc && is_grpc_request(headers) {
        is_grpc_web = true;
    }
    let is_grpc = is_grpc_request(headers) || is_grpc_web;
    (is_grpc, is_grpc_web)
}

/// Connect RPC (Omni UI uses Connect JSON over HTTP/1.1 for Watch/Get).
pub fn is_connect_request(headers: &http::HeaderMap) -> bool {
    if headers.contains_key("connect-protocol-version") {
        return true;
    }
    headers
        .get(header::CONTENT_TYPE.as_str())
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.starts_with("application/connect"))
        .unwrap_or(false)
}

/// Long-lived `/api/` streams (Connect Watch, gRPC Watch, etc.).
pub fn is_long_lived_api_stream(method: &Method, path: &str, headers: &http::HeaderMap) -> bool {
    *method == Method::POST
        && is_grpc_rpc_path(path)
        && (is_grpc_server_streaming(path) || is_connect_request(headers))
}

/// gRPC method name from an RPC path (`/api/foo.Bar/Baz` → `Baz`).
pub fn grpc_rpc_method(path: &str) -> Option<&str> {
    let path = path.split('?').next().unwrap_or(path);
    path.rsplit('/').next().filter(|m| !m.is_empty())
}

/// Long-lived server-streaming RPCs (`Watch`, etc.).
pub fn is_grpc_server_streaming(path: &str) -> bool {
    matches!(
        grpc_rpc_method(path),
        Some("Watch" | "Subscribe" | "Listen" | "Stream" | "Tail" | "Events")
    )
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

/// HTTP/3 cannot proxy Omni `/api/` RPC streams (buffering + wrong semantics). Force HTTP/2.
pub fn is_h3_incompatible_request(headers: &http::HeaderMap, method: &Method, path: &str) -> bool {
    is_grpc_like_request(headers, method, path)
        || (*method == Method::POST && is_grpc_rpc_path(path))
}

/// h2c upstream: native gRPC only (never for `/api/` gRPC-Web/Connect paths).
pub fn uses_h2c_upstream(is_grpc: bool, is_grpc_web: bool, _path: &str) -> bool {
    is_grpc && !is_grpc_web
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

pub fn prepare_upstream_request(upstream: &mut RequestHeader, is_grpc_web: bool) {
    upstream.remove_header(header::CONNECTION.as_str());
    upstream.remove_header(header::UPGRADE.as_str());
    upstream.remove_header(header::TRANSFER_ENCODING.as_str());
    upstream.remove_header("keep-alive");
    upstream.remove_header("proxy-connection");

    if !is_grpc_web && !upstream.headers.contains_key(header::TE.as_str()) {
        upstream.insert_header(header::TE.as_str(), "trailers").ok();
    }
}

pub fn strip_hop_by_hop_response_headers(resp: &mut ResponseHeader) {
    resp.remove_header(header::TRANSFER_ENCODING.as_str());
    resp.remove_header(header::CONNECTION.as_str());
    resp.remove_header("keep-alive");
    resp.remove_header("proxy-connection");
}

pub fn prepare_streaming_response_headers(resp: &mut ResponseHeader) {
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

/// Client cancelled an HTTP/2 stream (common when Connect Watch retries or tab navigates away).
pub fn is_benign_downstream_disconnect(error: &pingora_error::Error) -> bool {
    use pingora_error::{ErrorSource, ErrorType};

    if error.esource() != &ErrorSource::Downstream {
        return false;
    }

    if matches!(
        error.etype(),
        ErrorType::ConnectionClosed | ErrorType::ReadError | ErrorType::WriteError
    ) {
        return true;
    }

    if matches!(error.etype(), ErrorType::H2Error) {
        let detail = format!("{error}");
        return detail.contains("stream no longer needed")
            || detail.contains("Client closed H2")
            || detail.contains("CANCEL")
            || detail.contains("connection reset");
    }

    false
}

/// Join HTTP/2 multiple Cookie headers for HTTP/1.1 upstream (Omni Auth0 sessions).
pub fn merge_cookie_headers(upstream: &mut RequestHeader) {
    let values: Vec<String> = upstream
        .headers
        .get_all(header::COOKIE.as_str())
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if values.len() <= 1 {
        return;
    }
    upstream.remove_header(header::COOKIE.as_str());
    upstream
        .insert_header(header::COOKIE.as_str(), values.join("; "))
        .ok();
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
    fn connect_json_watch_is_not_grpc_but_long_lived() {
        let watch = "/api/omni.resources.ResourceService/Watch";
        let mut headers = http::HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
        headers.insert("connect-protocol-version", "1".parse().unwrap());
        let (is_grpc, is_grpc_web) = classify_grpc_request(&headers, &Method::POST, watch);
        assert!(!is_grpc);
        assert!(!is_grpc_web);
        assert!(is_long_lived_api_stream(&Method::POST, watch, &headers));
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
