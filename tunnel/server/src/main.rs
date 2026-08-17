//! pertisk-tunnel-server — accept QUIC clients and expose TCP on 127.0.0.1.

use std::{
    collections::HashMap,
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use anyhow::{bail, Context, Result};
use axum::{extract::State, routing::get, Json, Router};
use clap::Parser;
use pertisk_tunnel_proto as proto;
use quinn::{Endpoint, Incoming, ServerConfig};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

#[derive(Debug, Parser)]
#[command(name = "pertisk-tunnel-server", about = "Reverse tunnel server for pertisk-proxy")]
struct Args {
    /// Path to TOML config (token + tunnel allowlist).
    #[arg(short, long, env = "PERTISK_TUNNEL_CONFIG")]
    config: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
struct Config {
    /// QUIC listen address (UDP), e.g. 0.0.0.0:7000
    #[serde(default = "default_bind")]
    bind: String,
    /// Shared secret; clients must present this token.
    token: String,
    /// Optional HTTP status JSON on loopback (default 127.0.0.1:7700). Empty disables.
    #[serde(default = "default_status")]
    status_bind: String,
    tunnels: Vec<TunnelDef>,
}

fn default_bind() -> String {
    format!("0.0.0.0:{}", proto::DEFAULT_CONTROL_PORT)
}

fn default_status() -> String {
    proto::DEFAULT_STATUS_ADDR.to_string()
}

#[derive(Debug, Clone, Deserialize)]
struct TunnelDef {
    name: String,
    /// Loopback TCP port for pertisk-proxy Site upstreams.
    remote_port: u16,
}

#[derive(Debug, Clone, Serialize)]
struct LiveTunnel {
    name: String,
    remote_port: u16,
    connected: bool,
    client_addr: Option<String>,
}

#[derive(Clone)]
struct AppState {
    token: String,
    tunnels: HashMap<String, u16>,
    live: Arc<RwLock<HashMap<String, LiveTunnel>>>,
}

#[tokio::main]
async fn main() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "pertisk_tunnel_server=info,info".into()),
        )
        .init();

    let args = Args::parse();
    let raw = std::fs::read_to_string(&args.config)
        .with_context(|| format!("read config {}", args.config.display()))?;
    let cfg: Config = toml::from_str(&raw).context("parse tunnel server config")?;
    if cfg.token.trim().is_empty() || cfg.token.contains("change-me") {
        warn!("tunnel token looks weak — set a strong random token before production");
    }
    if cfg.tunnels.is_empty() {
        bail!("config must list at least one [[tunnels]] entry");
    }

    let mut map = HashMap::new();
    let mut live = HashMap::new();
    for t in &cfg.tunnels {
        if t.name.trim().is_empty() {
            bail!("tunnel name must not be empty");
        }
        if t.remote_port == 0 {
            bail!("tunnel {} remote_port must be non-zero", t.name);
        }
        if map.insert(t.name.clone(), t.remote_port).is_some() {
            bail!("duplicate tunnel name {}", t.name);
        }
        live.insert(
            t.name.clone(),
            LiveTunnel {
                name: t.name.clone(),
                remote_port: t.remote_port,
                connected: false,
                client_addr: None,
            },
        );
    }

    let state = AppState {
        token: cfg.token.clone(),
        tunnels: map,
        live: Arc::new(RwLock::new(live)),
    };

    if !cfg.status_bind.trim().is_empty() {
        let status_addr: SocketAddr = cfg
            .status_bind
            .parse()
            .with_context(|| format!("status_bind {}", cfg.status_bind))?;
        if !status_addr.ip().is_loopback() {
            warn!(
                "status_bind {} is not loopback — prefer 127.0.0.1 for admin status only",
                status_addr
            );
        }
        let st = state.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_status(status_addr, st).await {
                error!("status server exited: {e:#}");
            }
        });
        info!("tunnel status JSON on http://{status_addr}/status");
    }

    let bind: SocketAddr = cfg.bind.parse().with_context(|| format!("bind {}", cfg.bind))?;
    let endpoint = make_endpoint(bind)?;
    info!(
        "pertisk-tunnel-server listening on UDP {bind} (ALPN={})",
        String::from_utf8_lossy(proto::ALPN)
    );

    while let Some(incoming) = endpoint.accept().await {
        let st = state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(incoming, st).await {
                warn!("client session ended: {e:#}");
            }
        });
    }
    Ok(())
}

async fn serve_status(addr: SocketAddr, state: AppState) -> Result<()> {
    let app = Router::new()
        .route("/status", get(status_handler))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state);
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn status_handler(State(state): State<AppState>) -> Json<proto::TunnelStatus> {
    let live = state.live.read().await;
    let tunnels: Vec<_> = live
        .values()
        .map(|t| proto::TunnelStatusEntry {
            name: t.name.clone(),
            remote_port: t.remote_port,
            connected: t.connected,
            client_addr: t.client_addr.clone(),
        })
        .collect();
    let online = tunnels.iter().any(|t| t.connected);
    Json(proto::TunnelStatus { online, tunnels })
}

fn make_endpoint(bind: SocketAddr) -> Result<Endpoint> {
    let cert = rcgen::generate_simple_self_signed(vec!["pertisk-tunnel".into()])
        .context("generate self-signed cert")?;
    let cert_der = CertificateDer::from(cert.cert);
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()));

    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .context("rustls server config")?;
    tls.alpn_protocols = vec![proto::ALPN.to_vec()];
    tls.max_early_data_size = 0;

    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(Duration::from_secs(60).try_into().unwrap()));
    transport.keep_alive_interval(Some(Duration::from_secs(15)));

    let mut server = ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(tls).context("quic server crypto")?,
    ));
    server.transport_config(Arc::new(transport));

    Endpoint::server(server, bind).context("bind quinn endpoint")
}

async fn handle_connection(incoming: Incoming, state: AppState) -> Result<()> {
    let conn = incoming.await.context("accept quic")?;
    let remote = conn.remote_address();
    info!("client connected from {remote}");

    let (mut send, mut recv) = conn.accept_bi().await.context("accept control stream")?;
    let hello: proto::ClientControl = proto::read_frame(&mut recv).await.context("read hello")?;
    let (token, wanted) = match hello {
        proto::ClientControl::Hello { token, tunnels } => (token, tunnels),
        other => bail!("expected hello, got {other:?}"),
    };

    if !constant_time_eq(token.as_bytes(), state.token.as_bytes()) {
        let _ = proto::write_frame(
            &mut send,
            &proto::ServerControl::HelloErr {
                message: "invalid token".into(),
            },
        )
        .await;
        bail!("auth failed from {remote}");
    }

    let mut granted = Vec::new();
    for name in &wanted {
        let Some(&port) = state.tunnels.get(name) else {
            let _ = proto::write_frame(
                &mut send,
                &proto::ServerControl::HelloErr {
                    message: format!("unknown tunnel {name}"),
                },
            )
            .await;
            bail!("client requested unknown tunnel {name}");
        };
        granted.push(proto::TunnelGranted {
            name: name.clone(),
            remote_port: port,
        });
    }
    if granted.is_empty() {
        let _ = proto::write_frame(
            &mut send,
            &proto::ServerControl::HelloErr {
                message: "no tunnels requested".into(),
            },
        )
        .await;
        bail!("no tunnels");
    }

    proto::write_frame(
        &mut send,
        &proto::ServerControl::HelloOk {
            tunnels: granted.clone(),
        },
    )
    .await?;

    // Mark live
    {
        let mut live = state.live.write().await;
        for g in &granted {
            if let Some(e) = live.get_mut(&g.name) {
                e.connected = true;
                e.client_addr = Some(remote.to_string());
            }
        }
    }

    let names: Vec<String> = granted.iter().map(|g| g.name.clone()).collect();
    info!(
        "authorized {remote} tunnels={}",
        names.join(",")
    );

    let mut listeners = Vec::new();
    for g in granted {
        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], g.remote_port)))
            .await
            .with_context(|| format!("bind 127.0.0.1:{}", g.remote_port))?;
        info!(
            "tunnel {} listening on 127.0.0.1:{} → client {remote}",
            g.name, g.remote_port
        );
        listeners.push((g.name, listener));
    }

    let conn = Arc::new(conn);
    let mut accept_tasks = Vec::new();
    for (name, listener) in listeners {
        let conn = conn.clone();
        accept_tasks.push(tokio::spawn(async move {
            loop {
                let (tcp, peer) = match listener.accept().await {
                    Ok(v) => v,
                    Err(e) => {
                        warn!("accept on tunnel {name}: {e}");
                        break;
                    }
                };
                let conn = conn.clone();
                let name = name.clone();
                tokio::spawn(async move {
                    if let Err(e) = relay_public_to_client(conn, name, tcp, peer).await {
                        warn!("relay error: {e:#}");
                    }
                });
            }
        }));
    }

    // Keep control stream alive (ping/pong) until connection closes.
    let control = tokio::spawn(async move {
        loop {
            match proto::read_frame::<_, proto::ClientControl>(&mut recv).await {
                Ok(proto::ClientControl::Ping) => {
                    if proto::write_frame(&mut send, &proto::ServerControl::Pong)
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(proto::ClientControl::Hello { .. }) => {
                    warn!("unexpected second hello");
                    break;
                }
                Err(_) => break,
            }
        }
    });

    tokio::select! {
        _ = conn.closed() => {}
        _ = control => {}
    }

    for t in accept_tasks {
        t.abort();
    }

    {
        let mut live = state.live.write().await;
        for name in names {
            if let Some(e) = live.get_mut(&name) {
                e.connected = false;
                e.client_addr = None;
            }
        }
    }
    info!("client {remote} disconnected");
    Ok(())
}

async fn relay_public_to_client(
    conn: Arc<quinn::Connection>,
    tunnel: String,
    mut tcp: tokio::net::TcpStream,
    peer: SocketAddr,
) -> Result<()> {
    let (mut send, mut recv) = conn.open_bi().await.context("open data stream")?;
    proto::write_frame(&mut send, &proto::OpenConn { tunnel: tunnel.clone() }).await?;
    tracing::debug!("relaying {peer} via tunnel {tunnel}");
    pipe_tcp_quic(&mut tcp, &mut send, &mut recv).await;
    Ok(())
}

async fn pipe_tcp_quic(
    tcp: &mut tokio::net::TcpStream,
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
) {
    let (mut tcp_r, mut tcp_w) = tcp.split();
    let c2s = async {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match tokio::io::AsyncReadExt::read(&mut tcp_r, &mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if tokio::io::AsyncWriteExt::write_all(send, &buf[..n])
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = tokio::io::AsyncWriteExt::shutdown(send).await;
    };
    let s2c = async {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match tokio::io::AsyncReadExt::read(recv, &mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if tokio::io::AsyncWriteExt::write_all(&mut tcp_w, &buf[..n])
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = tokio::io::AsyncWriteExt::shutdown(&mut tcp_w).await;
    };
    tokio::select! {
        _ = c2s => {}
        _ = s2c => {}
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
