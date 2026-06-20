use std::net::SocketAddr;

use anyhow::{Context, Result};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;

/// Dual-stack TCP listen addresses for unspecified binds.
/// Linux: `[::]:port` accepts IPv4 + IPv6 (when `net.ipv6.bindv6only=0`).
/// macOS: bind `0.0.0.0` and `[::]` separately — avoids IPv4-mapped `[::ffff:x]` quirks.
/// Do not bind both on Linux — that conflicts with the dual-stack socket (EADDRINUSE).
fn dual_stack_tcp_bind_addrs(listen: &str) -> Vec<String> {
    let Ok(addr) = listen.parse::<SocketAddr>() else {
        return vec![listen.to_string()];
    };

    if !addr.ip().is_unspecified() {
        return vec![listen.to_string()];
    }

    let port = addr.port();
    // Explicit `[::]:port` input — keep single dual-stack socket (used for H3-aligned TCP config).
    if addr.is_ipv6() {
        return vec![listen.to_string()];
    }

    #[cfg(target_os = "macos")]
    {
        vec![format!("0.0.0.0:{port}"), format!("[::]:{port}")]
    }
    #[cfg(not(target_os = "macos"))]
    {
        vec![format!("[{}]:{port}", std::net::Ipv6Addr::UNSPECIFIED)]
    }
}

/// UDP (QUIC) bind address. One dual-stack `[::]:port` socket handles both families.
fn dual_stack_udp_bind_addr(listen: &str) -> Vec<String> {
    let Ok(addr) = listen.parse::<SocketAddr>() else {
        return vec![listen.to_string()];
    };

    if !addr.ip().is_unspecified() {
        return vec![listen.to_string()];
    }

    let port = addr.port();
    vec![format!("[{}]:{port}", std::net::Ipv6Addr::UNSPECIFIED)]
}

/// UDP bind addresses for HTTP/3 (QUIC).
pub fn h3_bind_addrs(listen: &str) -> Vec<String> {
    dual_stack_udp_bind_addr(listen)
}

/// TCP bind addresses for HTTP/1 and HTTP/2.
pub fn tcp_bind_addrs(listen: &str) -> Vec<String> {
    dual_stack_tcp_bind_addrs(listen)
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
        #[cfg(target_os = "macos")]
        assert_eq!(
            tcp_bind_addrs("0.0.0.0:443"),
            vec!["0.0.0.0:443", "[::]:443"]
        );
        #[cfg(not(target_os = "macos"))]
        assert_eq!(tcp_bind_addrs("0.0.0.0:443"), vec!["[::]:443"]);
        assert_eq!(h3_bind_addrs("0.0.0.0:443"), vec!["[::]:443"]);
        assert_eq!(tcp_bind_addrs("[::]:80"), vec!["[::]:80"]);
        assert_eq!(tcp_bind_addrs("127.0.0.1:8080"), vec!["127.0.0.1:8080"]);
    }
}
