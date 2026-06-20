use std::net::SocketAddr;

use anyhow::{Context, Result};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;

#[cfg(not(target_os = "macos"))]
use std::net::Ipv4Addr;

/// UDP bind addresses for HTTP/3 (QUIC). On macOS, `[::]:port` is dual-stack (IPv4 + IPv6).
pub fn h3_bind_addrs(listen: &str) -> Vec<String> {
    let Ok(addr) = listen.parse::<SocketAddr>() else {
        return vec![listen.to_string()];
    };

    if !addr.ip().is_unspecified() {
        return vec![listen.to_string()];
    }

    let port = addr.port();
    if addr.is_ipv6() {
        return vec![format!("[{}]:{port}", std::net::Ipv6Addr::UNSPECIFIED)];
    }

    #[cfg(not(target_os = "macos"))]
    {
        return vec![
            format!("[{}]:{port}", std::net::Ipv6Addr::UNSPECIFIED),
            format!("{}:{port}", Ipv4Addr::UNSPECIFIED),
        ];
    }
    #[cfg(target_os = "macos")]
    {
        vec![format!("[{}]:{port}", std::net::Ipv6Addr::UNSPECIFIED)]
    }
}

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
    tune_udp_socket(&socket)?;
    socket
        .bind(&addr.into())
        .with_context(|| format!("failed to bind UDP {addr}"))?;
    Ok(socket)
}

/// Bind one or more UDP sockets. When `count > 1`, uses SO_REUSEPORT for parallel recv.
pub async fn bind_udp_sockets(listen: &str, count: usize) -> Result<Vec<UdpSocket>> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dual_stack_from_ipv4_unspecified() {
        let addrs = h3_bind_addrs("0.0.0.0:443");
        assert!(addrs.iter().any(|a| a.starts_with("[::]")));
    }
}
