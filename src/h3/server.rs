use std::sync::Arc;

use anyhow::{Context, Result};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use reqwest::Client;
use tokio::net::UdpSocket;
use tokio_quiche::http3::driver::{
    H3Event, InboundFrame, IncomingH3Headers, OutboundFrame, OutboundFrameSender,
    ServerEventStream, ServerH3Event,
};
use tokio_quiche::http3::settings::Http3Settings;
use tokio_quiche::listen;
use tokio_quiche::metrics::DefaultMetrics;
use tokio_quiche::settings::{CertificateKind, ConnectionParams, Hooks, QuicSettings, TlsCertificatePaths};
use tokio_quiche::ServerH3Driver;
use tracing::{error, info, warn};

use crate::config::ServerConfig;
use crate::h3::headers::{error_response, h3_to_request, request_host, response_to_h3};
use crate::router::Router;

#[derive(Clone)]
pub struct H3Config {
    pub udp_listen: String,
    pub tls_cert_path: String,
    pub tls_key_path: String,
}

impl H3Config {
    pub fn from_tls_paths(cert: impl Into<String>, key: impl Into<String>, udp_listen: String) -> Self {
        Self {
            udp_listen,
            tls_cert_path: cert.into(),
            tls_key_path: key.into(),
        }
    }

    pub fn from_server(server: &ServerConfig) -> anyhow::Result<Self> {
        Ok(Self {
            udp_listen: server.h3_udp_listen.clone(),
            tls_cert_path: server.tls_cert_path()?.to_string(),
            tls_key_path: server.tls_key_path()?.to_string(),
        })
    }
}

pub async fn run(router: Arc<Router>, config: H3Config) -> Result<()> {
    let cert = &config.tls_cert_path;
    let key = &config.tls_key_path;

    let socket = UdpSocket::bind(&config.udp_listen)
        .await
        .with_context(|| format!("failed to bind UDP {}", config.udp_listen))?;

    info!(addr = %config.udp_listen, "HTTP/3 listener started");

    let mut listeners = listen(
        [socket],
        ConnectionParams::new_server(
            QuicSettings::default(),
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

    let client = Client::builder().build()?;
    let accept_stream = &mut listeners[0];

    while let Some(conn_res) = accept_stream.next().await {
        match conn_res {
            Ok(conn) => {
                let (driver, mut controller) = ServerH3Driver::new(Http3Settings::default());
                conn.start(driver);

                let router = Arc::clone(&router);
                let client = client.clone();
                tokio::spawn(async move {
                    if let Err(err) =
                        serve_connection(router, client, controller.event_receiver_mut()).await
                    {
                        warn!(error = %err, "HTTP/3 connection closed with error");
                    }
                });
            }
            Err(err) => error!(error = %err, "failed to accept QUIC connection"),
        }
    }

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
                let router = Arc::clone(&router);
                let client = client.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_request(router, client, incoming_headers).await {
                        warn!(error = %err, "HTTP/3 request failed");
                    }
                });
            }
            ServerH3Event::Core(other) => {
                info!(event = ?other, "unhandled HTTP/3 event");
            }
        }
    }
    Ok(())
}

async fn handle_request(
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

    if path == "/healthz" || path == "/readyz" {
        send_error(
            &mut send,
            error_response(http::StatusCode::OK, "ok"),
        )
        .await;
        return Ok(());
    }

    let table = router.snapshot();
    let backend = match table.match_route(&host, path) {
        Some(backend) => backend.clone(),
        None => {
            send_error(
                &mut send,
                error_response(http::StatusCode::NOT_FOUND, "no route"),
            )
            .await;
            return Ok(());
        }
    };

    info!(host = %host, path = %path, upstream = %backend.address, "HTTP/3 request");

    let body = read_request_body(&mut recv).await?;
    let upstream = build_upstream_url(&backend.address, req.uri())?;

    let mut upstream_req = client.request(req.method().clone(), upstream);
    for (name, value) in req.headers().iter() {
        if name == http::header::HOST {
            continue;
        }
        upstream_req = upstream_req.header(name, value);
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
    for (name, value) in response_headers.iter() {
        if name == http::header::SERVER {
            continue;
        }
        response = response.header(name, value);
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

fn build_upstream_url(backend_address: &str, uri: &http::Uri) -> Result<String> {
    let path_and_query = uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");

    if backend_address.contains("://") {
        Ok(format!("{backend_address}{path_and_query}"))
    } else {
        Ok(format!("http://{backend_address}{path_and_query}"))
    }
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
