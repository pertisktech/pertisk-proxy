//! pertisk-tunnel-server — accept QUIC clients and expose TCP on 127.0.0.1.

use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::{bail, Context, Result};
use axum::{extract::State, routing::get, Json, Router};
use clap::Parser;
use pertisk_tunnel_proto as proto;
use quinn::{Endpoint, Incoming, ServerConfig};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::Deserialize;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::{TcpListener, TcpStream};
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
    #[serde(default = "default_bind")]
    bind: String,
    token: String,
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
    remote_port: u16,
}

struct ClientSession {
    conn: Arc<quinn::Connection>,
    tunnels: HashSet<String>,
    addr: String,
}

struct TunnelCounters {
    bytes_to_client: AtomicU64,
    bytes_from_client: AtomicU64,
    streams: AtomicU64,
}

#[derive(Clone)]
struct AppState {
    token: String,
    /// name → remote_port
    tunnels: HashMap<String, u16>,
    counters: Arc<HashMap<String, Arc<TunnelCounters>>>,
    /// Active client (replaced on reconnect).
    session: Arc<RwLock<Option<ClientSession>>>,
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
    let mut seen_ports = HashSet::new();
    for t in &cfg.tunnels {
        if t.name.trim().is_empty() {
            bail!("tunnel name must not be empty");
        }
        if t.remote_port == 0 {
            bail!("tunnel {} remote_port must be non-zero", t.name);
        }
        if !seen_ports.insert(t.remote_port) {
            bail!("duplicate remote_port {} (used by more than one tunnel)", t.remote_port);
        }
        if map.insert(t.name.clone(), t.remote_port).is_some() {
            bail!("duplicate tunnel name {}", t.name);
        }
    }

    let counters: HashMap<String, Arc<TunnelCounters>> = map
        .keys()
        .map(|name| {
            (
                name.clone(),
                Arc::new(TunnelCounters {
                    bytes_to_client: AtomicU64::new(0),
                    bytes_from_client: AtomicU64::new(0),
                    streams: AtomicU64::new(0),
                }),
            )
        })
        .collect();

    let state = AppState {
        token: cfg.token.clone(),
        tunnels: map.clone(),
        counters: Arc::new(counters),
        session: Arc::new(RwLock::new(None)),
    };

    // Bind loopback ports once at startup (survives client reconnects).
    for (name, port) in &map {
        let listener = bind_loopback(*port)
            .with_context(|| format!("bind 127.0.0.1:{port} for tunnel `{name}`"))?;
        info!("tunnel `{name}` listening on 127.0.0.1:{port} (waiting for client)");
        let st = state.clone();
        let name = name.clone();
        tokio::spawn(async move {
            accept_loop(name, listener, st).await;
        });
    }

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

fn bind_loopback(port: u16) -> Result<TcpListener> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let domain = Domain::IPV4;
    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    {
        let _ = socket.set_reuse_port(true);
    }
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(1024)?;
    let std_listener: std::net::TcpListener = socket.into();
    Ok(TcpListener::from_std(std_listener)?)
}

async fn accept_loop(name: String, listener: TcpListener, state: AppState) {
    loop {
        let (tcp, peer) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                warn!("accept on tunnel `{name}`: {e}");
                continue;
            }
        };
        let _ = tcp.set_nodelay(true);
        let st = state.clone();
        let name = name.clone();
        tokio::spawn(async move {
            if let Err(e) = forward_accepted(name, tcp, peer, st).await {
                tracing::debug!("forward ended: {e:#}");
            }
        });
    }
}

async fn forward_accepted(
    name: String,
    mut tcp: TcpStream,
    peer: SocketAddr,
    state: AppState,
) -> Result<()> {
    let (conn, _) = {
        let g = state.session.read().await;
        let Some(sess) = g.as_ref() else {
            // No client connected — close immediately.
            return Ok(());
        };
        if !sess.tunnels.contains(&name) {
            return Ok(());
        }
        (sess.conn.clone(), sess.addr.clone())
    };

    let (mut send, mut recv) = conn.open_bi().await.context("open data stream")?;
    proto::write_frame(&mut send, &proto::OpenConn { tunnel: name.clone() }).await?;
    tracing::debug!("relaying {peer} via tunnel `{name}`");
    let (to_client, from_client) = pipe_tcp_quic(&mut tcp, &mut send, &mut recv).await;
    if let Some(c) = state.counters.get(&name) {
        c.bytes_to_client.fetch_add(to_client, Ordering::Relaxed);
        c.bytes_from_client.fetch_add(from_client, Ordering::Relaxed);
        c.streams.fetch_add(1, Ordering::Relaxed);
    }
    if to_client + from_client > 50 * 1024 * 1024 {
        info!(
            "tunnel `{name}` stream {peer} transferred {} (to client) + {} (from client)",
            human_bytes(to_client),
            human_bytes(from_client)
        );
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
    let sess = state.session.read().await;
    let connected = sess.is_some();
    let client_addr = sess.as_ref().map(|s| s.addr.clone());
    let active = sess.as_ref().map(|s| s.tunnels.clone()).unwrap_or_default();
    let mut bytes_to_client = 0u64;
    let mut bytes_from_client = 0u64;
    let tunnels: Vec<_> = state
        .tunnels
        .iter()
        .map(|(name, port)| {
            let (btc, bfc, streams) = state
                .counters
                .get(name)
                .map(|c| {
                    (
                        c.bytes_to_client.load(Ordering::Relaxed),
                        c.bytes_from_client.load(Ordering::Relaxed),
                        c.streams.load(Ordering::Relaxed),
                    )
                })
                .unwrap_or((0, 0, 0));
            bytes_to_client += btc;
            bytes_from_client += bfc;
            proto::TunnelStatusEntry {
                name: name.clone(),
                remote_port: *port,
                connected: connected && active.contains(name),
                client_addr: if active.contains(name) {
                    client_addr.clone()
                } else {
                    None
                },
                bytes_to_client: btc,
                bytes_from_client: bfc,
                streams,
            }
        })
        .collect();
    Json(proto::TunnelStatus {
        online: connected,
        bytes_to_client,
        bytes_from_client,
        tunnels,
    })
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

    let mut server = ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(tls).context("quic server crypto")?,
    ));
    server.transport_config(Arc::new(tuned_transport()));

    Endpoint::server(server, bind).context("bind quinn endpoint")
}

fn tuned_transport() -> quinn::TransportConfig {
    use quinn::VarInt;
    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(Duration::from_secs(120).try_into().unwrap()));
    transport.keep_alive_interval(Some(Duration::from_secs(10)));
    transport.receive_window(VarInt::from_u32(8 * 1024 * 1024));
    transport.stream_receive_window(VarInt::from_u32(2 * 1024 * 1024));
    transport.send_window(8 * 1024 * 1024);
    transport.max_concurrent_bidi_streams(VarInt::from_u32(1024));
    transport.initial_rtt(Duration::from_millis(80));
    transport
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
        reject_hello(&mut send, &conn, "invalid token").await;
        bail!("auth failed from {remote}");
    }

    let mut granted = Vec::new();
    let mut names = HashSet::new();
    for name in &wanted {
        let Some(&port) = state.tunnels.get(name) else {
            let known: Vec<_> = state.tunnels.keys().cloned().collect();
            reject_hello(
                &mut send,
                &conn,
                &format!(
                    "unknown tunnel `{name}` (server has: {})",
                    if known.is_empty() {
                        "(none)".into()
                    } else {
                        known.join(", ")
                    }
                ),
            )
            .await;
            bail!("client requested unknown tunnel {name}");
        };
        names.insert(name.clone());
        granted.push(proto::TunnelGranted {
            name: name.clone(),
            remote_port: port,
        });
    }
    if granted.is_empty() {
        reject_hello(&mut send, &conn, "no tunnels requested").await;
        bail!("no tunnels");
    }

    // Replace any previous client (reconnect-safe; ports stay bound).
    {
        let mut slot = state.session.write().await;
        if let Some(old) = slot.take() {
            info!("replacing previous client {}", old.addr);
            old.conn
                .close(0u32.into(), b"replaced by new client");
        }
        *slot = Some(ClientSession {
            conn: Arc::new(conn.clone()),
            tunnels: names.clone(),
            addr: remote.to_string(),
        });
    }

    proto::write_frame(
        &mut send,
        &proto::ServerControl::HelloOk {
            tunnels: granted.clone(),
        },
    )
    .await?;

    info!(
        "authorized {remote} tunnels={}",
        granted
            .iter()
            .map(|g| format!("{}→127.0.0.1:{}", g.name, g.remote_port))
            .collect::<Vec<_>>()
            .join(", ")
    );

    // Keepalive / control until disconnect.
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

    // Clear session only if we are still the active client.
    {
        let mut slot = state.session.write().await;
        if slot
            .as_ref()
            .map(|s| s.addr == remote.to_string())
            .unwrap_or(false)
        {
            *slot = None;
        }
    }
    info!("client {remote} disconnected");
    Ok(())
}

/// Returns (bytes TCP→QUIC / to client, bytes QUIC→TCP / from client).
async fn pipe_tcp_quic(
    tcp: &mut TcpStream,
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
) -> (u64, u64) {
    let (mut tcp_r, mut tcp_w) = tcp.split();
    let to_client = AtomicU64::new(0);
    let from_client = AtomicU64::new(0);
    let c2s = async {
        let mut buf = vec![0u8; 64 * 1024];
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
                    to_client.fetch_add(n as u64, Ordering::Relaxed);
                    if tokio::io::AsyncWriteExt::flush(send).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = send.finish();
    };
    let s2c = async {
        let mut buf = vec![0u8; 64 * 1024];
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
                    from_client.fetch_add(n as u64, Ordering::Relaxed);
                    if tokio::io::AsyncWriteExt::flush(&mut tcp_w).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = tokio::io::AsyncWriteExt::shutdown(&mut tcp_w).await;
    };
    let _ = tokio::join!(c2s, s2c);
    (
        to_client.load(Ordering::Relaxed),
        from_client.load(Ordering::Relaxed),
    )
}

fn human_bytes(n: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let f = n as f64;
    if f >= GIB {
        format!("{:.2} GiB", f / GIB)
    } else if f >= MIB {
        format!("{:.1} MiB", f / MIB)
    } else if f >= KIB {
        format!("{:.0} KiB", f / KIB)
    } else {
        format!("{n} B")
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

async fn reject_hello(send: &mut quinn::SendStream, conn: &quinn::Connection, message: &str) {
    warn!("rejecting client: {message}");
    let _ = proto::write_frame(
        send,
        &proto::ServerControl::HelloErr {
            message: message.to_string(),
        },
    )
    .await;
    let _ = send.finish();
    tokio::time::sleep(Duration::from_millis(200)).await;
    conn.close(0u32.into(), message.as_bytes());
}
