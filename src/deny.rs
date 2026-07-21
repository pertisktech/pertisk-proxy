//! Fast reject for requests to hosts with no configured route.

use std::sync::LazyLock;

use bytes::Bytes;
use http::StatusCode;
use pingora_http::ResponseHeader;
use pingora_proxy::Session;

const BODY: &[u8] = b"not found";

static PLAIN_404_HEADERS: LazyLock<ResponseHeader> = LazyLock::new(|| {
    // tarpaulin::skip_start
    let mut resp = ResponseHeader::build(StatusCode::NOT_FOUND, Some(3)).unwrap();
    resp.insert_header("Content-Type", "text/plain").unwrap();
    resp.insert_header("Content-Length", BODY.len()).unwrap();
    resp
    // tarpaulin::skip_end
});

static TLS_421_HEADERS: LazyLock<ResponseHeader> = LazyLock::new(|| {
    // tarpaulin::skip_start
    let mut resp = ResponseHeader::build(StatusCode::MISDIRECTED_REQUEST, Some(3)).unwrap();
    resp.insert_header("Content-Type", "text/plain").unwrap();
    resp.insert_header("Content-Length", BODY.len()).unwrap();
    resp
    // tarpaulin::skip_end
});

/// When true (default), unknown hosts get an immediate 404/421 without upstream routing.
pub fn enabled() -> bool {
    static DEFAULT: LazyLock<bool> = LazyLock::new(|| {
        // tarpaulin::skip_start
        match std::env::var("PERTISK_DEFAULT_DENY") {
            Ok(raw) => {
                let v = raw.trim().to_ascii_lowercase();
                !(v == "0" || v == "false" || v == "off" || v == "no")
            }
            Err(_) => true,
        }
        // tarpaulin::skip_end
    });
    *DEFAULT
}

pub fn unknown_host_status(tls: bool) -> StatusCode {
    if tls {
        StatusCode::MISDIRECTED_REQUEST
    } else {
        StatusCode::NOT_FOUND
    }
}

/// Respond on the Pingora HTTP/1 + HTTP/2 path for an unconfigured host.
pub async fn respond_pingora(session: &mut Session, tls: bool, server: &str) -> pingora_core::Result<()> {
    // tarpaulin::skip_start
    let template = if tls { &*TLS_421_HEADERS } else { &*PLAIN_404_HEADERS };
    let mut resp = template.clone();
    resp.insert_header("Server", server)?;
    resp.insert_header("X-App-Name", crate::app_name())?;
    session.write_response_header(Box::new(resp), false).await?;
    session
        .write_response_body(Some(Bytes::from_static(BODY)), true)
        .await?;
    Ok(())
    // tarpaulin::skip_end
}

pub fn h3_response(tls: bool) -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(unknown_host_status(tls))
        .header("content-type", "text/plain")
        .header("content-length", BODY.len())
        .header("server", "pertisk-proxy/h3")
        .header("x-app-name", crate::app_name())
        .body(BODY.to_vec())
        .unwrap()
}

const GEOIP_BODY: &[u8] = b"forbidden";

/// Respond 403 for GeoIP / policy blocks on the Pingora path.
pub async fn respond_forbidden(
    session: &mut Session,
    server: &str,
    reason: &str,
) -> pingora_core::Result<()> {
    let mut resp = ResponseHeader::build(StatusCode::FORBIDDEN, Some(4))?;
    resp.insert_header("Content-Type", "text/plain")?;
    resp.insert_header("Content-Length", GEOIP_BODY.len())?;
    resp.insert_header("Server", server)?;
    resp.insert_header("X-App-Name", crate::app_name())?;
    resp.insert_header("X-Pertisk-Block", reason)?;
    session.write_response_header(Box::new(resp), false).await?;
    session
        .write_response_body(Some(Bytes::from_static(GEOIP_BODY)), true)
        .await?;
    Ok(())
}

pub async fn respond_html(
    session: &mut Session,
    server: &str,
    status: StatusCode,
    content_type: &str,
    body: &str,
    extra_headers: &[(&'static str, String)],
) -> pingora_core::Result<()> {
    let mut resp = ResponseHeader::build(status, Some(4 + extra_headers.len()))?;
    resp.insert_header("Content-Type", content_type)?;
    resp.insert_header("Content-Length", body.len())?;
    resp.insert_header("Server", server)?;
    resp.insert_header("X-App-Name", crate::app_name())?;
    for (name, value) in extra_headers {
        resp.insert_header(*name, value.as_str())?;
    }
    session.write_response_header(Box::new(resp), false).await?;
    session
        .write_response_body(Some(Bytes::copy_from_slice(body.as_bytes())), true)
        .await?;
    Ok(())
}

pub async fn respond_redirect(
    session: &mut Session,
    server: &str,
    location: &str,
    set_cookie: Option<&str>,
) -> pingora_core::Result<()> {
    let mut resp = ResponseHeader::build(StatusCode::FOUND, Some(5))?;
    resp.insert_header("Location", location)?;
    resp.insert_header("Content-Length", "0")?;
    resp.insert_header("Server", server)?;
    resp.insert_header("X-App-Name", crate::app_name())?;
    if let Some(cookie) = set_cookie {
        resp.insert_header("Set-Cookie", cookie)?;
    }
    session.write_response_header(Box::new(resp), true).await?;
    Ok(())
}

pub fn h3_forbidden(reason: &str) -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(StatusCode::FORBIDDEN)
        .header("content-type", "text/plain")
        .header("content-length", GEOIP_BODY.len())
        .header("server", "pertisk-proxy/h3")
        .header("x-app-name", crate::app_name())
        .header("x-pertisk-block", reason)
        .body(GEOIP_BODY.to_vec())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_host_status_codes() {
        assert_eq!(unknown_host_status(false), StatusCode::NOT_FOUND);
        assert_eq!(unknown_host_status(true), StatusCode::MISDIRECTED_REQUEST);
    }

    #[test]
    fn h3_response_builds_body() {
        let resp = h3_response(false);
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert_eq!(resp.body(), BODY);

        let resp_tls = h3_response(true);
        assert_eq!(resp_tls.status(), StatusCode::MISDIRECTED_REQUEST);
    }

    #[test]
    fn enabled_defaults_to_true() {
        assert!(enabled());
    }
}
