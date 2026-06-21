//! pertisk-proxy-ingress — Kubernetes Ingress + Gateway API controller (ingress mode).

#[cfg(feature = "ingress")]
fn main() -> anyhow::Result<()> {
    pertisk_proxy::ingress::run()
}

#[cfg(not(feature = "ingress"))]
fn main() {
    eprintln!("Build with --features ingress to run pertisk-proxy-ingress");
    std::process::exit(1);
}
