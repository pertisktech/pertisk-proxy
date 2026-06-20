//! HTTP/3 probe fast path — no spawn, no router, no upstream.

use std::sync::{Arc, LazyLock};

use bytes::Bytes;
use futures::SinkExt;
use quiche::h3::Header as H3Header;
use tokio_quiche::http3::driver::{InboundFrame, IncomingH3Headers, OutboundFrame};

use crate::health::{is_health_path_bytes, is_json_health_path_bytes, API_HEALTH_BODY};
use crate::h3::headers::pseudo_method_path;

const SERVER: &[u8] = b"pertisk-proxy/h3";

static API_HEALTH_GET_HEADERS: LazyLock<Arc<Vec<H3Header>>> = LazyLock::new(|| {
    Arc::new(vec![
        H3Header::new(b":status", b"200"),
        H3Header::new(b"content-type", b"application/json"),
        H3Header::new(b"content-length", b"15"),
        H3Header::new(b"server", SERVER),
    ])
});

static API_HEALTH_HEAD_HEADERS: LazyLock<Arc<Vec<H3Header>>> = LazyLock::new(|| {
    Arc::new(vec![
        H3Header::new(b":status", b"200"),
        H3Header::new(b"content-type", b"application/json"),
        H3Header::new(b"content-length", b"15"),
        H3Header::new(b"server", SERVER),
    ])
});

static PLAIN_OK_GET_HEADERS: LazyLock<Arc<Vec<H3Header>>> = LazyLock::new(|| {
    Arc::new(vec![
        H3Header::new(b":status", b"200"),
        H3Header::new(b"content-type", b"text/plain"),
        H3Header::new(b"content-length", b"2"),
        H3Header::new(b"server", SERVER),
    ])
});

static PLAIN_OK_HEAD_HEADERS: LazyLock<Arc<Vec<H3Header>>> = LazyLock::new(|| {
    Arc::new(vec![
        H3Header::new(b":status", b"200"),
        H3Header::new(b"content-type", b"text/plain"),
        H3Header::new(b"content-length", b"2"),
        H3Header::new(b"server", SERVER),
    ])
});

/// Returns `true` when `:method` + `:path` match a probe endpoint.
pub fn matches_request(headers: &[quiche::h3::Header]) -> bool {
    pseudo_method_path(headers)
        .map(|(method, path)| {
            matches!(method, b"GET" | b"HEAD") && is_health_path_bytes(path)
        })
        .unwrap_or(false)
}

/// Serve a probe response inline (no spawn, no router, no upstream).
pub async fn try_serve(incoming: IncomingH3Headers) {
    let IncomingH3Headers {
        headers,
        mut send,
        mut recv,
        ..
    } = incoming;

    let Some((method, path)) = pseudo_method_path(&headers) else {
        return;
    };

    if !is_health_path_bytes(path) {
        return;
    }

    let (h3_headers, body) = match method {
        b"GET" => health_get_response(path),
        b"HEAD" => health_head_response(path),
        _ => return,
    };

    let headers = (*h3_headers).clone();

    // Respond immediately; drain request body in the background so we don't
    // add recv latency to the probe hot path.
    let _ = send.send(OutboundFrame::Headers(headers, None)).await;
    let _ = send.send(OutboundFrame::Body(body, true)).await;

    tokio::spawn(async move {
        while let Some(frame) = recv.recv().await {
            if matches!(frame, InboundFrame::Body(_, true)) {
                break;
            }
        }
    });
}

fn health_get_response(path: &[u8]) -> (Arc<Vec<H3Header>>, Bytes) {
    if is_json_health_path_bytes(path) {
        (
            Arc::clone(&API_HEALTH_GET_HEADERS),
            Bytes::from_static(API_HEALTH_BODY),
        )
    } else {
        (
            Arc::clone(&PLAIN_OK_GET_HEADERS),
            Bytes::from_static(b"ok"),
        )
    }
}

fn health_head_response(path: &[u8]) -> (Arc<Vec<H3Header>>, Bytes) {
    if is_json_health_path_bytes(path) {
        (Arc::clone(&API_HEALTH_HEAD_HEADERS), Bytes::new())
    } else {
        (Arc::clone(&PLAIN_OK_HEAD_HEADERS), Bytes::new())
    }
}
