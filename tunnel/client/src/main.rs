//! pertisk-tunnel-client — dial VPS and forward local TCP services.

use std::{
    collections::HashMap,
    net::{SocketAddr, ToSocketAddrs},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use anyhow::{bail, Context, Result};
use clap::Parser;
use pertisk_tunnel_proto as proto;
use quinn::Endpoint;
use rustls::pki_types::ServerName;
use serde::Deserialize;
use tokio::net::TcpStream;
use tracing::{error, info, warn};

#[derive(Debug, Parser)]
#[command(name = "pertisk-tunnel-client", about = "Reverse tunnel client for pertisk-proxy")]
struct Args {
    /// Path to TOML config.
    #[arg(short, long, env = "PERTISK_TUNNEL_CONFIG")]
    config: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
struct Config {
    /// Host:port of the tunnel server (UDP/QUIC), e.g. vps.example.com:7000
    server: String,
    token: String,
    /// Accept self-signed server cert (required for default server setup).
    #[serde(default = "default_true")]
    insecure_skip_verify: bool,
    /// Reconnect delay seconds after disconnect.
    #[serde(default = "default_reconnect")]
    reconnect_secs: u64,
    tunnels: Vec<ClientTunnel>,
}

fn default_true() -> bool {
    true
}

fn default_reconnect() -> u64 {
    3
}

#[derive(Debug, Clone, Deserialize)]
struct ClientTunnel {
    name: String,
    /// Backend to dial on this host/LAN.
    /// Examples: `127.0.0.1:3000`, `10.1.1.195:8006`, `https://10.1.1.195:8006` (Proxmox).
    /// Scheme is only for default ports; the tunnel forwards raw TCP (TLS stays end-to-end
    /// when the Site upstream is also `https://127.0.0.1:<remote_port>`).
    #[serde(alias = "target")]
    local: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "pertisk_tunnel_client=info,info".into()),
        )
        .init();

    let args = Args::parse();
    let raw = std::fs::read_to_string(&args.config)
        .with_context(|| format!("read config {}", args.config.display()))?;
    let cfg: Config = toml::from_str(&raw).context("parse tunnel client config")?;
    if cfg.tunnels.is_empty() {
        bail!("config must list at least one [[tunnels]] entry");
    }

    loop {
        match run_session(&cfg).await {
            Ok(()) => warn!("session ended cleanly; reconnecting"),
            Err(e) => error!("session error: {e:#}; reconnecting"),
        }
        tokio::time::sleep(Duration::from_secs(cfg.reconnect_secs.max(1))).await;
    }
}

async fn run_session(cfg: &Config) -> Result<()> {
    let server_addr = resolve_server(&cfg.server)?;
    let endpoint = make_client_endpoint(cfg.insecure_skip_verify)?;
    let server_name = host_name(&cfg.server);

    info!("connecting to {} ({server_addr})", cfg.server);
    let conn = endpoint
        .connect(server_addr, &server_name)
        .context("connect builder")?
        .await
        .context("quic connect")?;

    let (mut send, mut recv) = conn.open_bi().await.context("open control stream")?;
    let names: Vec<String> = cfg.tunnels.iter().map(|t| t.name.clone()).collect();
    proto::write_frame(
        &mut send,
        &proto::ClientControl::Hello {
            token: cfg.token.clone(),
            tunnels: names,
        },
    )
    .await?;

    let reply: proto::ServerControl = match proto::read_frame(&mut recv).await {
        Ok(r) => r,
        Err(e) => {
            bail!(
                "hello reply failed ({e}). Check VPS: matching token, and server.toml [[tunnels]] names \
                 must include every client tunnel name. See: journalctl -u pertisk-tunnel-server -n 50"
            );
        }
    };
    let granted = match reply {
        proto::ServerControl::HelloOk { tunnels } => tunnels,
        proto::ServerControl::HelloErr { message } => bail!("server rejected: {message}"),
        other => bail!("unexpected hello reply: {other:?}"),
    };

    let local_map: HashMap<String, String> = cfg
        .tunnels
        .iter()
        .map(|t| (t.name.clone(), t.local.clone()))
        .collect();

    for g in &granted {
        let local = local_map
            .get(&g.name)
            .cloned()
            .unwrap_or_else(|| "?".into());
        let dial = match dial_target(&local) {
            Ok(d) => d,
            Err(e) => {
                warn!("tunnel {}: invalid target {local}: {e:#}", g.name);
                local.clone()
            }
        };
        info!(
            "tunnel {} ready: VPS 127.0.0.1:{} → {local} (dial {dial})",
            g.name, g.remote_port
        );
    }

    let locals = Arc::new(local_map);
    let conn = Arc::new(conn);

    let accept = {
        let conn = conn.clone();
        let locals = locals.clone();
        tokio::spawn(async move {
            loop {
                match conn.accept_bi().await {
                    Ok((send, recv)) => {
                        let locals = locals.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_open_conn(send, recv, locals).await {
                                warn!("data stream: {e:#}");
                            }
                        });
                    }
                    Err(e) => {
                        warn!("accept_bi ended: {e}");
                        break;
                    }
                }
            }
        })
    };

    let keepalive = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(20)).await;
            if proto::write_frame(&mut send, &proto::ClientControl::Ping)
                .await
                .is_err()
            {
                break;
            }
            match proto::read_frame::<_, proto::ServerControl>(&mut recv).await {
                Ok(proto::ServerControl::Pong) => {}
                Ok(_) | Err(_) => break,
            }
        }
    });

    tokio::select! {
        _ = conn.closed() => {}
        _ = accept => {}
        _ = keepalive => {}
    }
    Ok(())
}

async fn handle_open_conn(
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
    locals: Arc<HashMap<String, String>>,
) -> Result<()> {
    let open: proto::OpenConn = proto::read_frame(&mut recv).await.context("open frame")?;
    let Some(local) = locals.get(&open.tunnel) else {
        bail!("no local mapping for tunnel {}", open.tunnel);
    };
    let dial = dial_target(local)?;
    let mut tcp = TcpStream::connect(&dial)
        .await
        .with_context(|| format!("connect backend {local} (dial {dial})"))?;
    let _ = tcp.set_nodelay(true);
    tracing::debug!("opened backend {local} (dial {dial}) for tunnel {}", open.tunnel);
    pipe_tcp_quic(&mut tcp, &mut send, &mut recv).await;
    Ok(())
}

/// Parse `host:port`, `http(s)://host[:port][/…]` into a TCP dial address.
fn dial_target(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("empty tunnel target");
    }

    let (scheme, rest) = if let Some(r) = trimmed.strip_prefix("https://") {
        (Some("https"), r)
    } else if let Some(r) = trimmed.strip_prefix("http://") {
        (Some("http"), r)
    } else if let Some(r) = trimmed.strip_prefix("tcp://") {
        (None, r)
    } else {
        (None, trimmed)
    };

    let hostport = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(rest)
        .trim();
    if hostport.is_empty() {
        bail!("missing host in tunnel target {raw}");
    }

    // Already has an explicit port (including [ipv6]:port).
    if host_has_port(hostport) {
        return Ok(hostport.to_string());
    }

    let default_port = match scheme {
        Some("https") => 443,
        Some("http") => 80,
        _ => bail!("tunnel target {raw} needs host:port (or http(s)://host)"),
    };
    Ok(format!("{hostport}:{default_port}"))
}

fn host_has_port(hostport: &str) -> bool {
    if hostport.starts_with('[') {
        return hostport
            .find(']')
            .and_then(|i| hostport.get(i + 1..))
            .is_some_and(|s| s.starts_with(':') && s[1..].parse::<u16>().is_ok());
    }
    hostport
        .rsplit_once(':')
        .map(|(h, p)| !h.is_empty() && !h.contains(':') && p.parse::<u16>().is_ok())
        == Some(true)
}

async fn pipe_tcp_quic(
    tcp: &mut TcpStream,
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
) {
    let (mut tcp_r, mut tcp_w) = tcp.split();
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
}

fn resolve_server(server: &str) -> Result<SocketAddr> {
    let with_port = if server.contains(']') {
        server.to_string()
    } else if server
        .rsplit_once(':')
        .map(|(_, p)| p.parse::<u16>().is_ok())
        == Some(true)
    {
        server.to_string()
    } else {
        format!("{server}:{}", proto::DEFAULT_CONTROL_PORT)
    };
    with_port
        .to_socket_addrs()
        .with_context(|| format!("resolve {with_port}"))?
        .next()
        .with_context(|| format!("no addresses for {with_port}"))
}

fn host_name(server: &str) -> String {
    if let Some(h) = server.strip_prefix('[') {
        return h.split(']').next().unwrap_or(h).to_string();
    }
    if let Some((h, port)) = server.rsplit_once(':') {
        if port.parse::<u16>().is_ok() {
            return h.to_string();
        }
    }
    server.to_string()
}

fn make_client_endpoint(insecure: bool) -> Result<Endpoint> {
    let mut crypto = if insecure {
        let provider = rustls::crypto::ring::default_provider();
        let mut cfg = rustls::ClientConfig::builder_with_provider(Arc::new(provider))
            .with_safe_default_protocol_versions()
            .context("rustls versions")?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
            .with_no_client_auth();
        cfg.alpn_protocols = vec![proto::ALPN.to_vec()];
        cfg
    } else {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let mut cfg = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        cfg.alpn_protocols = vec![proto::ALPN.to_vec()];
        cfg
    };
    crypto.enable_early_data = false;

    let mut client_cfg = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(crypto).context("quic client crypto")?,
    ));
    client_cfg.transport_config(Arc::new(tuned_transport()));

    let mut endpoint = Endpoint::client(SocketAddr::from(([0, 0, 0, 0], 0)))?;
    endpoint.set_default_client_config(client_cfg);
    Ok(endpoint)
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

#[derive(Debug)]
struct SkipServerVerification;

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dial_proxmox_https_url() {
        assert_eq!(
            dial_target("https://10.1.1.195:8006").unwrap(),
            "10.1.1.195:8006"
        );
    }

    #[test]
    fn dial_bare_host_port() {
        assert_eq!(dial_target("10.1.1.195:8006").unwrap(), "10.1.1.195:8006");
    }

    #[test]
    fn dial_https_default_port() {
        assert_eq!(dial_target("https://10.1.1.195").unwrap(), "10.1.1.195:443");
    }
}
