//! Fast reject for requests to hosts with no configured route.

use std::sync::LazyLock;

use bytes::Bytes;
use http::StatusCode;
use pingora_http::ResponseHeader;
use pingora_proxy::Session;

const BODY: &[u8] = b"not found";

static PLAIN_404_HEADERS: LazyLock<ResponseHeader> = LazyLock::new(|| {
    let mut resp = ResponseHeader::build(StatusCode::NOT_FOUND, Some(3)).unwrap();
    resp.insert_header("Content-Type", "text/plain").unwrap();
    resp.insert_header("Content-Length", BODY.len()).unwrap();
    resp
});

static TLS_421_HEADERS: LazyLock<ResponseHeader> = LazyLock::new(|| {
    let mut resp = ResponseHeader::build(StatusCode::MISDIRECTED_REQUEST, Some(3)).unwrap();
    resp.insert_header("Content-Type", "text/plain").unwrap();
    resp.insert_header("Content-Length", BODY.len()).unwrap();
    resp
});

/// When true (default), unknown hosts get an immediate 404/421 without upstream routing.
pub fn enabled() -> bool {
    static DEFAULT: LazyLock<bool> = LazyLock::new(|| {
        match std::env::var("PERTISK_DEFAULT_DENY") {
            Ok(raw) => {
                let v = raw.trim().to_ascii_lowercase();
                !(v == "0" || v == "false" || v == "off" || v == "no")
            }
            Err(_) => true,
        }
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
    let template = if tls { &*TLS_421_HEADERS } else { &*PLAIN_404_HEADERS };
    let mut resp = template.clone();
    resp.insert_header("Server", server)?;
    session.write_response_header(Box::new(resp), false).await?;
    session
        .write_response_body(Some(Bytes::from_static(BODY)), true)
        .await?;
    Ok(())
}

pub fn h3_response(tls: bool) -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(unknown_host_status(tls))
        .header("content-type", "text/plain")
        .header("content-length", BODY.len())
        .header("server", "pertisk-proxy/h3")
        .body(BODY.to_vec())
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
}
