use std::net::{SocketAddr, Ipv6Addr};

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
        return vec![format!("[{}]:{port}", Ipv6Addr::UNSPECIFIED)];
    }

    // 0.0.0.0 — prefer dual-stack wildcard; on Linux also bind IPv4 explicitly.
    #[cfg(not(target_os = "macos"))]
    {
        return vec![
            format!("[{}]:{port}", Ipv6Addr::UNSPECIFIED),
            format!("{}:{port}", Ipv4Addr::UNSPECIFIED),
        ];
    }
    #[cfg(target_os = "macos")]
    {
        vec![format!("[{}]:{port}", Ipv6Addr::UNSPECIFIED)]
    }
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
