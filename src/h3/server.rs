use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use reqwest::Client;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio_quiche::http3::driver::{
    H3Event, InboundFrame, IncomingH3Headers, OutboundFrame, OutboundFrameSender,
    ServerEventStream, ServerH3Event,
};
use tokio_quiche::http3::settings::Http3Settings;
use tokio_quiche::listen;
use tokio_quiche::metrics::DefaultMetrics;
use tokio_quiche::settings::{CertificateKind, ConnectionParams, Hooks, TlsCertificatePaths};
use tokio_quiche::ServerH3Driver;
use tracing::{error, info, warn};

use crate::deny;
use crate::h3::headers::{error_response, h3_to_request, pseudo_authority, request_host, response_to_h3};
use crate::h3::health;
use crate::h3::settings::{listener_count, quic_settings};
use crate::proxy::forward::resolve_forward;
use crate::router::Router;
use crate::runtime::RuntimeConfig;

const UDP_BUFFER_BYTES: usize = 7 * 1024 * 1024;

fn tune_udp_socket(socket: &Socket) -> Result<()> {
    let _ = socket.set_reuse_address(true);
    #[cfg(all(unix, not(target_os = "solaris")))]
    let _ = socket.set_reuse_port(true);
    let _ = socket.set_recv_buffer_size(UDP_BUFFER_BYTES);
    let _ = socket.set_send_buffer_size(UDP_BUFFER_BYTES);
    Ok(())
}

fn create_bound_socket(addr: SocketAddr) -> Result<Socket> {
    let domain = if addr.is_ipv6() {
        Domain::IPV6
    } else {
        Domain::IPV4
    };

    let socket = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))
        .with_context(|| format!("failed to create UDP socket for {addr}"))?;
    #[cfg(all(unix, not(target_os = "solaris"), not(target_os = "fuchsia")))]
    if addr.is_ipv6() {
        // Allow native IPv6 and IPv4-mapped clients on one QUIC socket.
        let _ = socket.set_only_v6(false);
    }
    tune_udp_socket(&socket)?;
    socket
        .bind(&addr.into())
        .with_context(|| format!("failed to bind UDP {addr}"))?;
    Ok(socket)
}

/// Bind one or more UDP sockets. When `count > 1`, uses SO_REUSEPORT for parallel recv.
async fn bind_udp_sockets(listen: &str, count: usize) -> Result<Vec<UdpSocket>> {
    let addr: SocketAddr = listen
        .parse()
        .with_context(|| format!("invalid UDP listen address: {listen}"))?;
    let count = count.max(1);

    let mut sockets = Vec::with_capacity(count);
    for _ in 0..count {
        let socket = create_bound_socket(addr)?;
        socket.set_nonblocking(true)?;
        sockets.push(UdpSocket::from_std(socket.into())?);
    }

    Ok(sockets)
}

pub async fn run(router: Arc<Router>, config: H3Config, runtime_cfg: &RuntimeConfig) -> Result<()> {
    let cert = &config.tls_cert_path;
    let key = &config.tls_key_path;
    let http3_opts = router.http3_options();
    let quic = quic_settings(runtime_cfg, http3_opts.as_ref());
    let listeners_n = listener_count(runtime_cfg, http3_opts.as_ref());

    info!(
        addr = %config.udp_listen,
        listeners = listeners_n,
        max_streams_bidi = quic.initial_max_streams_bidi,
        conn_window = quic.initial_max_data,
        stream_window = quic.initial_max_stream_data_bidi_remote,
        cc_algorithm = %quic.cc_algorithm,
        enable_0rtt = quic.enable_early_data,
        enable_pacing = quic.enable_pacing,
        idle_timeout_secs = quic.max_idle_timeout.map(|d| d.as_secs()),
        "HTTP/3 listener started"
    );

    let sockets = bind_udp_sockets(&config.udp_listen, listeners_n)
        .await
        .with_context(|| format!("failed to bind UDP {}", config.udp_listen))?;

    let listeners = listen(
        sockets,
        ConnectionParams::new_server(
            quic,
            TlsCertificatePaths {
                cert,
                private_key: key,
                kind: CertificateKind::X509,
            },
            Hooks::default(),
        ),
        DefaultMetrics,
    )
    .context("failed to create QUIC listener")?;

    let client = crate::h3::upstream_client::build_upstream_client()?;

    for mut accept_stream in listeners {
        let router = Arc::clone(&router);
        let client = client.clone();
        tokio::spawn(async move {
            while let Some(conn_res) = accept_stream.next().await {
                match conn_res {
                    Ok(conn) => {
                        let (driver, mut controller) =
                            ServerH3Driver::new(Http3Settings::default());
                        conn.start(driver);

                        let router = Arc::clone(&router);
                        let client = client.clone();
                        tokio::spawn(async move {
                            if let Err(err) = serve_connection(
                                router,
                                client,
                                controller.event_receiver_mut(),
                            )
                            .await
                            {
                                warn!(error = %err, "HTTP/3 connection closed with error");
                            }
                        });
                    }
                    Err(err) => error!(error = %err, "failed to accept QUIC connection"),
                }
            }
        });
    }

    // Keep the H3 task alive until the process exits.
    std::future::pending::<()>().await;
    Ok(())
}

async fn serve_connection(
    router: Arc<Router>,
    client: Client,
    events: &mut ServerEventStream,
) -> Result<()> {
    while let Some(event) = events.recv().await {
        match event {
            ServerH3Event::Core(H3Event::ConnectionError(err)) => {
                anyhow::bail!("HTTP/3 connection error: {err:?}");
            }
            ServerH3Event::Core(H3Event::ConnectionShutdown(_)) => break,
            ServerH3Event::Headers {
                incoming_headers,
                ..
            } => {
                if health::matches_request(&incoming_headers.headers) {
                    health::try_serve(incoming_headers).await;
                    continue;
                }

                if deny::enabled() {
                    if let Some(host) = pseudo_authority(&incoming_headers.headers) {
                        if let Ok(host) = std::str::from_utf8(host) {
                            if !host.is_empty() && !router.snapshot().has_host(host) {
                                deny_h3(incoming_headers).await;
                                continue;
                            }
                        }
                    }
                }

                let router = Arc::clone(&router);
                let client = client.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_proxied_request(router, client, incoming_headers).await {
                        warn!(error = %err, "HTTP/3 proxied request failed");
                    }
                });
            }
            ServerH3Event::Core(_) => {}
        }
    }
    Ok(())
}

async fn deny_h3(headers: IncomingH3Headers) {
    let IncomingH3Headers { mut send, mut recv, .. } = headers;
    drain_request_body(&mut recv).await;
    send_error(&mut send, deny::h3_response(true)).await;
}

async fn drain_request_body(recv: &mut tokio_quiche::http3::driver::InboundFrameStream) {
    while let Some(frame) = recv.recv().await {
        if matches!(frame, InboundFrame::Body(_, true)) {
            break;
        }
    }
}

async fn handle_proxied_request(
    router: Arc<Router>,
    client: Client,
    headers: IncomingH3Headers,
) -> Result<()> {
    let IncomingH3Headers {
        headers: h3_headers,
        mut send,
        mut recv,
        ..
    } = headers;

    let req = match h3_to_request(h3_headers) {
        Ok(req) => req,
        Err(err) => {
            send_error(&mut send, error_response(http::StatusCode::BAD_REQUEST, &err.to_string()))
                .await;
            return Ok(());
        }
    };

    let path = req.uri().path();
    let host = request_host(&req);
    let path_q = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or(path);

    let plan = match resolve_forward(router.snapshot().as_ref(), &host, path_q) {
        Ok(plan) => plan,
        Err(_) => {
            send_error(
                &mut send,
                error_response(http::StatusCode::NOT_FOUND, "no route"),
            )
            .await;
            return Ok(());
        }
    };

    tracing::trace!(
        host = %host,
        path = %path_q,
        upstream = %plan.upstream_url,
        "HTTP/3 proxied request"
    );

    let body = read_request_body(&mut recv).await?;

    let mut upstream_req = client.request(req.method().clone(), plan.upstream_url);
    let oci_registry = crate::proxy::registry::is_oci_registry_path(path);
    for (name, value) in req.headers().iter() {
        if name == http::header::HOST {
            continue;
        }
        if oci_registry
            && (name == http::header::COOKIE
                || name.as_str().eq_ignore_ascii_case("x-forwarded-host")
                || name.as_str().eq_ignore_ascii_case("x-forwarded-port")
                || name.as_str().eq_ignore_ascii_case("x-forwarded-proto")
                || name.as_str().eq_ignore_ascii_case("x-forwarded-ssl")
                || name.as_str().eq_ignore_ascii_case("x-real-ip")
                || name.as_str().eq_ignore_ascii_case("x-forwarded-for"))
        {
            continue;
        }
        upstream_req = upstream_req.header(name, value);
    }
    if oci_registry {
        for (name, value) in crate::proxy::registry::registry_upstream_header_pairs(
            &host,
            req.method(),
            path,
            None,
            true,
        ) {
            upstream_req = upstream_req.header(name, value);
        }
    }
    for (name, value) in &plan.middleware.request_headers {
        upstream_req = upstream_req.header(name.as_str(), value.as_str());
    }
    upstream_req = upstream_req.header(HOST, host);
    upstream_req = upstream_req.body(body);

    let upstream_res = match upstream_req.send().await {
        Ok(res) => res,
        Err(err) => {
            send_error(
                &mut send,
                error_response(http::StatusCode::BAD_GATEWAY, &err.to_string()),
            )
            .await;
            return Ok(());
        }
    };

    let status = upstream_res.status();
    let response_headers = upstream_res.headers().clone();
    let response_body = upstream_res.bytes().await.unwrap_or_default();

    let mut response = http::Response::builder().status(status);
    response = response.header("Server", "pertisk-proxy/h3");
    response = response.header("x-app-name", crate::app_name());
    for (name, value) in response_headers.iter() {
        if name == http::header::SERVER {
            continue;
        }
        if oci_registry && name == http::header::SET_COOKIE {
            continue;
        }
        let value_str = value.to_str().unwrap_or_default();
        let out = if oci_registry && name == http::header::LOCATION {
            crate::proxy::registry::rewrite_registry_location_value(value_str, true)
                .unwrap_or_else(|| value_str.to_string())
        } else {
            value_str.to_string()
        };
        response = response.header(name, out.as_str());
    }
    if oci_registry {
        response = response.header("Alt-Svc", "clear");
    }
    for (name, value) in &plan.middleware.response_headers {
        response = response.header(name.as_str(), value.as_str());
    }
    let response = response.body(response_body.to_vec()).unwrap();

    let h3_headers = response_to_h3(&response);
    send.send(OutboundFrame::Headers(h3_headers, None))
        .await
        .ok();

    send.send(OutboundFrame::Body(Bytes::from(response.into_body()), true))
        .await
        .ok();

    Ok(())
}

async fn read_request_body(recv: &mut tokio_quiche::http3::driver::InboundFrameStream) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(frame) = recv.recv().await {
        match frame {
            InboundFrame::Body(chunk, fin) => {
                body.extend_from_slice(&chunk);
                if fin {
                    break;
                }
            }
            InboundFrame::Datagram(_) => {}
        }
    }
    Ok(body)
}

async fn send_error(send: &mut OutboundFrameSender, response: http::Response<Vec<u8>>) {
    let h3_headers = response_to_h3(&response);
    let body = response.into_body();
    let _ = send.send(OutboundFrame::Headers(h3_headers, None)).await;
    let _ = send
        .send(OutboundFrame::Body(Bytes::from(body), true))
        .await;
}

use http::header::HOST;
