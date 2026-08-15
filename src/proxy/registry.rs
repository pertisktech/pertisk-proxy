//! OCI Distribution / Harbor registry paths.
//!
//! Harbor behind a reverse proxy expects NGINX-equivalent `X-Forwarded-*` headers
//! (see pertisk-rproxy `build_upstream_request`). Without `X-Forwarded-Port` /
//! `X-Forwarded-Ssl`, token exchange and push authorization can fail even when
//! `docker login` succeeds.
//!
//! Harbor also sets a UI `sid` cookie on `/v2/` responses. Strip cookies on
//! registry paths so `/service/token` uses `Authorization` from `docker login`.
//!
//! Large layer uploads need long upstream/downstream timeouts (Pingora default
//! downstream read timeout is 60s).

use http::header;
use pingora_http::{RequestHeader, ResponseHeader};
use pingora_proxy::Session;

/// OCI registry API (`/v2`, `/v2/…`) or Harbor token endpoint.
pub fn is_oci_registry_path(path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    path == "/v2" || path.starts_with("/v2/") || path == "/service/token"
}

/// Layer blob upload/commit (large request bodies).
pub fn is_registry_blob_upload(method: &http::Method, path: &str) -> bool {
    if !matches!(method, &http::Method::PUT | &http::Method::PATCH) {
        return false;
    }
    let path = path.split('?').next().unwrap_or(path);
    (path == "/v2" || path.contains("/v2/")) && path.contains("/blobs/uploads/")
}

/// Upstream read/write timeout for registry traffic (default 1h; 0 = unlimited).
pub fn registry_upstream_timeout() -> std::time::Duration {
    let secs = std::env::var("PERTISK_REGISTRY_UPSTREAM_REQUEST_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(3600);
    if secs == 0 {
        std::time::Duration::MAX
    } else {
        std::time::Duration::from_secs(secs)
    }
}

fn forward_proto_port_ssl(downstream_tls: bool) -> (&'static str, &'static str, &'static str) {
    if downstream_tls {
        ("https", "443", "on")
    } else {
        ("http", "80", "off")
    }
}

/// Header pairs for registry upstream (H3 / reqwest).
pub fn registry_upstream_header_pairs(
    host: &str,
    method: &http::Method,
    path: &str,
    client_ip: Option<&str>,
    downstream_tls: bool,
) -> Vec<(&'static str, String)> {
    let (proto, port, ssl) = forward_proto_port_ssl(downstream_tls);
    let mut pairs = vec![
        ("X-Forwarded-Proto", proto.to_string()),
        ("X-Forwarded-Host", host.to_string()),
        ("X-Forwarded-Port", port.to_string()),
        ("X-Forwarded-Ssl", ssl.to_string()),
        ("Accept-Encoding", "identity".to_string()),
    ];
    if let Some(ip) = client_ip.filter(|s| !s.is_empty()) {
        pairs.push(("X-Real-IP", ip.to_string()));
        pairs.push(("X-Forwarded-For", ip.to_string()));
    }
    if is_registry_blob_upload(method, path) {
        pairs.push(("Connection", "close".to_string()));
    }
    pairs
}

/// Harbor returns 405 for POST `/service/token`; some clients POST with query params only.
pub fn normalize_registry_token_request(upstream: &mut RequestHeader) {
    if upstream.method != http::Method::POST {
        return;
    }
    let path = upstream.uri.path();
    if path != "/service/token" {
        return;
    }
    let has_query = upstream
        .uri
        .query()
        .is_some_and(|q| !q.is_empty());
    let content_length = upstream
        .headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("0");
    let body_empty = content_length == "0" || content_length.is_empty();
    if has_query || body_empty {
        upstream.set_method(http::Method::GET);
        upstream.remove_header(header::CONTENT_LENGTH.as_str());
    }
}

/// Remove session cookies; apply NGINX-style forwarding headers Harbor expects.
pub fn prepare_registry_upstream_request(
    upstream: &mut RequestHeader,
    method: &http::Method,
    path: &str,
    host: &str,
    client_ip: Option<&str>,
    downstream_tls: bool,
) {
    upstream.remove_header(header::COOKIE.as_str());
    for name in [
        "x-forwarded-host",
        "x-forwarded-port",
        "x-forwarded-proto",
        "x-forwarded-ssl",
        "x-real-ip",
        "x-forwarded-for",
    ] {
        upstream.remove_header(name);
    }

    for (name, value) in registry_upstream_header_pairs(host, method, path, client_ip, downstream_tls)
    {
        upstream.insert_header(name, value.as_str()).ok();
    }
}

/// Extend downstream read/write timeouts for registry traffic (Pingora default is 60s).
pub fn prepare_registry_downstream_session(session: &mut Session) {
    let timeout = registry_upstream_timeout();
    if timeout == std::time::Duration::MAX {
        session.set_read_timeout(None);
        session.set_write_timeout(None);
    } else {
        session.set_read_timeout(Some(timeout));
        session.set_write_timeout(Some(timeout));
    }
}

/// Remove `Set-Cookie` from registry responses and disable HTTP/3 upgrade for docker clients.
///
/// Harbor builds blob-upload `Location` URLs from `X-Forwarded-Proto`. When that is wrong
/// (HTTP/1.1 over TLS without h2), clients follow `http://` and push fails with 401.
pub fn prepare_registry_response_headers(resp: &mut ResponseHeader, downstream_tls: bool) {
    resp.remove_header(header::SET_COOKIE.as_str());
    resp.insert_header("Alt-Svc", "clear").ok();
    if downstream_tls {
        rewrite_registry_location_to_https(resp);
    }
}

fn rewrite_registry_location_to_https(resp: &mut ResponseHeader) {
    let Some(loc) = resp
        .headers
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
    else {
        return;
    };
    if let Some(https) = rewrite_registry_location_value(loc, true) {
        resp.remove_header(header::LOCATION.as_str());
        resp.insert_header(header::LOCATION, https.as_str()).ok();
    }
}

/// Harbor may emit `http://` blob upload locations when `X-Forwarded-Proto` is wrong.
pub fn rewrite_registry_location_value(loc: &str, downstream_tls: bool) -> Option<String> {
    if !downstream_tls {
        return None;
    }
    let rest = loc.strip_prefix("http://")?;
    Some(format!("https://{rest}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oci_paths() {
        assert!(is_oci_registry_path("/v2"));
        assert!(is_oci_registry_path("/v2/"));
        assert!(is_oci_registry_path("/v2/pertisksoft/hello-cicd/blobs/uploads/"));
        assert!(is_oci_registry_path("/service/token"));
        assert!(is_oci_registry_path("/service/token?service=harbor-registry"));
        assert!(!is_oci_registry_path("/api/v2/x"));
        assert!(!is_oci_registry_path("/harbor/v2/"));
    }

    #[test]
    fn post_token_with_query_becomes_get() {
        let mut req =
            RequestHeader::build(http::Method::POST, b"/service/token?service=harbor-registry", None)
                .unwrap();
        normalize_registry_token_request(&mut req);
        assert_eq!(req.method, http::Method::GET);
    }

    #[test]
    fn strips_cookie_and_sets_forward_headers() {
        let mut req = RequestHeader::build(http::Method::GET, b"/v2/", None).unwrap();
        req.insert_header(header::COOKIE, "sid=abc").unwrap();
        req.insert_header("X-Forwarded-Proto", "http").unwrap();
        prepare_registry_upstream_request(
            &mut req,
            &http::Method::GET,
            "/v2/",
            "registry.example.com",
            Some("203.0.113.1"),
            true,
        );
        assert!(req.headers.get(header::COOKIE).is_none());
        assert_eq!(
            req.headers.get("X-Forwarded-Proto").unwrap().to_str().unwrap(),
            "https"
        );
        assert_eq!(
            req.headers.get("X-Forwarded-Port").unwrap().to_str().unwrap(),
            "443"
        );
        assert_eq!(
            req.headers.get("X-Forwarded-Ssl").unwrap().to_str().unwrap(),
            "on"
        );
        assert_eq!(
            req.headers.get("X-Forwarded-Host").unwrap().to_str().unwrap(),
            "registry.example.com"
        );
        assert_eq!(
            req.headers.get("Accept-Encoding").unwrap().to_str().unwrap(),
            "identity"
        );
    }

    #[test]
    fn blob_upload_sets_connection_close() {
        let mut req = RequestHeader::build(http::Method::PUT, b"/v2/x/blobs/uploads/u", None)
            .unwrap();
        prepare_registry_upstream_request(
            &mut req,
            &http::Method::PUT,
            "/v2/x/blobs/uploads/u?digest=sha256:abc",
            "registry.example.com",
            Some("203.0.113.1"),
            true,
        );
        assert_eq!(
            req.headers.get(header::CONNECTION).unwrap().to_str().unwrap(),
            "close"
        );
    }

    #[test]
    fn blob_upload_paths() {
        assert!(is_registry_blob_upload(
            &http::Method::PUT,
            "/v2/pertisksoft/hello-cicd/blobs/uploads/uuid?digest=sha256:abc"
        ));
        assert!(!is_registry_blob_upload(
            &http::Method::HEAD,
            "/v2/pertisksoft/hello-cicd/blobs/sha256:abc"
        ));
    }

    #[test]
    fn registry_timeout_defaults() {
        let saved = std::env::var("PERTISK_REGISTRY_UPSTREAM_REQUEST_TIMEOUT_SECS").ok();
        std::env::remove_var("PERTISK_REGISTRY_UPSTREAM_REQUEST_TIMEOUT_SECS");
        assert_eq!(registry_upstream_timeout().as_secs(), 3600);
        if let Some(v) = saved {
            std::env::set_var("PERTISK_REGISTRY_UPSTREAM_REQUEST_TIMEOUT_SECS", v);
        }
    }

    #[test]
    fn strips_set_cookie_on_response() {
        let mut resp = ResponseHeader::build(http::StatusCode::OK, None).unwrap();
        resp.insert_header(header::SET_COOKIE, "sid=abc; Path=/; HttpOnly")
            .unwrap();
        prepare_registry_response_headers(&mut resp, true);
        assert!(resp.headers.get(header::SET_COOKIE).is_none());
        assert_eq!(
            resp.headers.get("Alt-Svc").unwrap().to_str().unwrap(),
            "clear"
        );
    }

    #[test]
    fn rewrites_http_location_to_https() {
        let mut resp = ResponseHeader::build(http::StatusCode::ACCEPTED, None).unwrap();
        resp.insert_header(
            header::LOCATION,
            "http://harbor.example.com/v2/foo/blobs/uploads/uuid",
        )
        .unwrap();
        prepare_registry_response_headers(&mut resp, true);
        assert_eq!(
            resp.headers.get(header::LOCATION).unwrap().to_str().unwrap(),
            "https://harbor.example.com/v2/foo/blobs/uploads/uuid"
        );
    }

    #[test]
    fn leaves_http_location_when_downstream_plain_http() {
        let mut resp = ResponseHeader::build(http::StatusCode::ACCEPTED, None).unwrap();
        resp.insert_header(
            header::LOCATION,
            "http://harbor.example.com/v2/foo/blobs/uploads/uuid",
        )
        .unwrap();
        prepare_registry_response_headers(&mut resp, false);
        assert_eq!(
            resp.headers.get(header::LOCATION).unwrap().to_str().unwrap(),
            "http://harbor.example.com/v2/foo/blobs/uploads/uuid"
        );
    }
}
