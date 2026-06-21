//! Kubernetes Ingress mode: reconcile Ingress + Gateway API into the Pingora router and admin UI.

#[cfg(feature = "ingress")]
mod controller;
#[cfg(feature = "ingress")]
pub mod gateway_api;
#[cfg(feature = "ingress")]
mod leader_election;

#[cfg(feature = "ingress")]
pub use controller::{IngressController, IngressControllerConfig};

#[cfg(feature = "ingress")]
fn load_controller_config() -> IngressControllerConfig {
    let ingress_class = std::env::var("PERTISK_INGRESS_CLASS")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| {
            std::env::var("INGRESS_CLASS")
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        })
        .or_else(|| Some("pertisk".to_string()));

    let gateway_class = std::env::var("PERTISK_GATEWAY_CLASS")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or(ingress_class.clone());

    let namespace = std::env::var("PERTISK_WATCH_NAMESPACE")
        .ok()
        .or_else(|| std::env::var("WATCH_NAMESPACE").ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());

    let gateway_api_enabled = std::env::var("PERTISK_GATEWAY_API_ENABLED")
        .ok()
        .map(|v| {
            !matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        })
        .unwrap_or(true);

    let gateway_controller_name = std::env::var("PERTISK_GATEWAY_CONTROLLER_NAME")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "pertisk.tech/ingress-controller".to_string());

    IngressControllerConfig {
        namespace,
        ingress_class,
        gateway_class,
        gateway_api_enabled,
        gateway_controller_name,
        default_backend_port: 80,
    }
}

#[cfg(feature = "ingress")]
pub fn run() -> anyhow::Result<()> {
    use std::net::SocketAddr;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::time::Duration;

    use crate::api::{self, AdminState, LeaderElectionState};
    use crate::config::IngressConfig;
    use crate::h3::{self, H3Config};
    use crate::log::{ProxyLog, ProxyLogEntry};
    use crate::logging;
    use crate::proxy_config::Config;
    use crate::runtime;
    use crate::server::{self, PendingH3};
    use crate::tls::{CertStore, Http01ChallengeStore};
    use crate::Router;
    use kube::Client;
    use tokio::sync::RwLock;
    use tracing::info;

    crate::tls::init_crypto_provider();

    let proxy_log = Arc::new(ProxyLog::new(10_000));
    logging::init(Some(Arc::clone(&proxy_log)));

    let runtime_cfg = runtime::runtime_config_from_env(&runtime::ingress_runtime_env())?;
    let tokio_runtime = runtime::build_runtime(&runtime_cfg, "pertisk-proxy-ingress-worker")?;

    let ingress_env = IngressConfig::from_env()?;
    let controller_config = load_controller_config();

    info!(
        binary = "pertisk-proxy-ingress",
        mode = "ingress",
        ingress_class = ?controller_config.ingress_class,
        gateway_class = ?controller_config.gateway_class,
        gateway_api_enabled = controller_config.gateway_api_enabled,
        "starting pertisk-proxy-ingress"
    );

    let leader_enabled = leader_election::env_flag("PERTISK_LEADER_ELECTION_ENABLED", true);
    let namespace = leader_election::resolve_namespace();
    let lease_name = leader_election::resolve_lease_name("pertisk-ingress-leader");

    let (client, leader_state) = tokio_runtime.block_on(async {
        let client = Client::try_default().await?;
        let leader_state = if leader_enabled {
            let config = leader_election::LeaderElectionConfig {
                namespace: namespace.clone(),
                lease_name: lease_name.clone(),
                holder_id: leader_election::resolve_holder_id(),
                lease_duration_seconds: leader_election::resolve_lease_duration_seconds(),
                renew_interval_seconds: leader_election::resolve_renew_interval_seconds(),
            };
            Some(leader_election::start_leader_election(client.clone(), config).await)
        } else {
            info!("Leader election disabled (PERTISK_LEADER_ELECTION_ENABLED=false)");
            None
        };
        Ok::<_, anyhow::Error>((client, leader_state))
    })?;

    let router = Router::new();
    let cert_store = Arc::new(CertStore::new());
    let runtime_config = Arc::new(RwLock::new(Config::default()));
    let certs_dir = std::env::var("PERTISK_CERTS_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("pertisk-proxy-certs"));
    std::fs::create_dir_all(&certs_dir).ok();

    let proxy_log_enabled = Arc::new(AtomicBool::new(
        std::env::var("PERTISK_PROXY_LOG")
            .ok()
            .map(|v| {
                !matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "0" | "false" | "no" | "off"
                )
            })
            .unwrap_or(false),
    ));

    let controller = IngressController::new(
        client.clone(),
        controller_config.clone(),
        Arc::clone(&router),
        Arc::clone(&runtime_config),
        Arc::clone(&cert_store),
        Arc::clone(&proxy_log),
    );

    let leader_election_state = Some(LeaderElectionState {
        enabled: leader_enabled,
        is_leader: leader_state,
        namespace: namespace.clone(),
        lease_name: lease_name.clone(),
    });

    tokio_runtime.block_on(async {
        if let Err(e) = controller.reconcile().await {
            tracing::error!("Initial reconcile error: {}", e);
        }
        proxy_log
            .push(ProxyLogEntry::config_reload(format!(
                "started pertisk-proxy-ingress {}",
                env!("pertisk_proxy_VERSION")
            )))
            .await;
    });

    let sessions = api::new_sessions();
    let http01_store = Arc::new(Http01ChallengeStore::new());
    let admin_state = api::build_ingress_state(
        Arc::clone(&router),
        Arc::clone(&cert_store),
        ingress_env.clone(),
        runtime_cfg.clone(),
        Some(client.clone()),
        controller_config.ingress_class.clone(),
        controller_config.gateway_class.clone(),
        controller_config.gateway_api_enabled,
        leader_election_state,
        Arc::clone(&runtime_config),
        Some(sessions),
        Arc::clone(&http01_store),
        Arc::clone(&proxy_log),
        Arc::clone(&proxy_log_enabled),
        certs_dir,
    );

    let management_addr = api::management_addr();
    tokio_runtime.spawn(async move {
        if let Err(err) = api::serve(admin_state, management_addr).await {
            tracing::error!(error = %err, "management API stopped");
        }
    });

    tokio_runtime.spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            if let Err(e) = controller.reconcile().await {
                tracing::error!("Reconcile error: {}", e);
            }
        }
    });

    if let (Ok(cert), Ok(key)) = (
        ingress_env.server.tls_cert_path(),
        ingress_env.server.tls_key_path(),
    ) {
        cert_store
            .set_global_fallback(cert.into(), key.into())
            .ok();
    }

    let pending_h3 = if ingress_env.server.enable_h3 {
        Some(PendingH3 {
            router: Arc::clone(&router),
            cert_store: Arc::clone(&cert_store),
            configs: vec![H3Config::from_server(&ingress_env.server)?],
            runtime_cfg: runtime_cfg.clone(),
        })
    } else {
        None
    };

    server::run(
        &ingress_env.server,
        router,
        cert_store,
        false,
        &runtime_cfg,
        None,
        pending_h3,
        proxy_log,
        proxy_log_enabled,
    )
}
