//! Shared wire protocol for pertisk-tunnel (QUIC ALPN + length-prefixed JSON).

use bytes::{Buf, BufMut, BytesMut};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// QUIC ALPN identifier (not HTTP/3).
pub const ALPN: &[u8] = b"pertisk-tunnel";

/// Default control port on the VPS.
pub const DEFAULT_CONTROL_PORT: u16 = 7000;

/// Default HTTP status bind for the tunnel server (loopback only).
pub const DEFAULT_STATUS_ADDR: &str = "127.0.0.1:7700";

const MAX_FRAME: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum ProtoError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("frame too large ({0} bytes)")]
    FrameTooLarge(usize),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unexpected end of stream")]
    UnexpectedEof,
}

/// Client → server on the control stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientControl {
    Hello {
        token: String,
        /// Tunnel names the client wants to open (must match server allowlist).
        tunnels: Vec<String>,
    },
    Ping,
}

/// Server → client on the control stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerControl {
    HelloOk {
        tunnels: Vec<TunnelGranted>,
    },
    HelloErr {
        message: String,
    },
    Pong,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelGranted {
    pub name: String,
    pub remote_port: u16,
}

/// First JSON frame on a data stream opened by the server, then raw TCP bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenConn {
    pub tunnel: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelStatus {
    pub online: bool,
    /// Bytes since tunnel-server process start (QUIC payload, both directions).
    #[serde(default)]
    pub bytes_to_client: u64,
    #[serde(default)]
    pub bytes_from_client: u64,
    pub tunnels: Vec<TunnelStatusEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelStatusEntry {
    pub name: String,
    pub remote_port: u16,
    pub connected: bool,
    pub client_addr: Option<String>,
    #[serde(default)]
    pub bytes_to_client: u64,
    #[serde(default)]
    pub bytes_from_client: u64,
    #[serde(default)]
    pub streams: u64,
}

pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, value: &impl Serialize) -> Result<(), ProtoError> {
    let body = serde_json::to_vec(value)?;
    if body.len() > MAX_FRAME {
        return Err(ProtoError::FrameTooLarge(body.len()));
    }
    let mut hdr = [0u8; 4];
    (&mut hdr[..]).put_u32(body.len() as u32);
    w.write_all(&hdr).await?;
    w.write_all(&body).await?;
    w.flush().await?;
    Ok(())
}

pub async fn read_frame<R: AsyncRead + Unpin, T: for<'de> Deserialize<'de>>(
    r: &mut R,
) -> Result<T, ProtoError> {
    let mut hdr = [0u8; 4];
    r.read_exact(&mut hdr).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            ProtoError::UnexpectedEof
        } else {
            ProtoError::Io(e)
        }
    })?;
    let len = (&hdr[..]).get_u32() as usize;
    if len > MAX_FRAME {
        return Err(ProtoError::FrameTooLarge(len));
    }
    let mut body = BytesMut::with_capacity(len);
    body.resize(len, 0);
    r.read_exact(&mut body).await?;
    Ok(serde_json::from_slice(&body)?)
}

/// Copy bidirectionally until either side closes; returns bytes (a→b, b→a).
pub async fn copy_bidirectional<A, B>(a: &mut A, b: &mut B) -> (u64, u64)
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    match tokio::io::copy_bidirectional(a, b).await {
        Ok(n) => n,
        Err(_) => (0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::DuplexStream;

    #[tokio::test]
    async fn frame_roundtrip() {
        let (mut a, mut b): (DuplexStream, DuplexStream) = tokio::io::duplex(4096);
        let msg = ClientControl::Hello {
            token: "secret".into(),
            tunnels: vec!["app".into()],
        };
        write_frame(&mut a, &msg).await.unwrap();
        let got: ClientControl = read_frame(&mut b).await.unwrap();
        match got {
            ClientControl::Hello { token, tunnels } => {
                assert_eq!(token, "secret");
                assert_eq!(tunnels, vec!["app"]);
            }
            _ => panic!("wrong variant"),
        }
    }
}
