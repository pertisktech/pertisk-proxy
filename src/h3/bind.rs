use std::net::SocketAddr;

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

/// Effective listen address(es) for admin UI and logs.
/// `0.0.0.0:port` is shown as `[::]:port` on Linux (single dual-stack socket).
/// On macOS, both `0.0.0.0:port` and `[::]:port` are listed when applicable.
pub fn effective_listen_display(listen: &str) -> String {
    let addrs = tcp_bind_addrs(listen);
    if addrs.len() == 1 {
        addrs[0].clone()
    } else {
        addrs.join(", ")
    }
}

/// Effective UDP listen address for admin UI (always one dual-stack socket).
pub fn effective_udp_listen_display(listen: &str) -> String {
    h3_bind_addrs(listen)
        .into_iter()
        .next()
        .unwrap_or_else(|| listen.to_string())
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

    #[test]
    fn effective_listen_display_unspecified() {
        #[cfg(target_os = "macos")]
        assert_eq!(
            effective_listen_display("0.0.0.0:443"),
            "0.0.0.0:443, [::]:443"
        );
        #[cfg(not(target_os = "macos"))]
        assert_eq!(effective_listen_display("0.0.0.0:443"), "[::]:443");
        assert_eq!(effective_udp_listen_display("0.0.0.0:443"), "[::]:443");
        assert_eq!(effective_listen_display("[::]:80"), "[::]:80");
    }

    #[test]
    fn invalid_listen_passthrough() {
        assert_eq!(tcp_bind_addrs("not-an-addr"), vec!["not-an-addr"]);
        assert_eq!(h3_bind_addrs("not-an-addr"), vec!["not-an-addr"]);
        assert_eq!(h3_bind_addrs("127.0.0.1:443"), vec!["127.0.0.1:443"]);
        assert_eq!(effective_listen_display("not-an-addr"), "not-an-addr");
        assert_eq!(effective_udp_listen_display("not-an-addr"), "not-an-addr");
    }
}
