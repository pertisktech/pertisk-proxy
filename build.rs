//! Injects version at build time so release packages can pass VERSION and the admin UI
//! shows the correct release (e.g. 0.0.22). Falls back to Cargo.toml when unset.

fn main() {
    println!("cargo:rerun-if-env-changed=pertisk_proxy_VERSION");
    let version = std::env::var("pertisk_proxy_VERSION")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    println!("cargo:rustc-env=pertisk_proxy_VERSION={version}");
}
