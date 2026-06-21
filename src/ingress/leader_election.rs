//! Simple leader election using coordination.k8s.io Lease objects.

use k8s_openapi::api::coordination::v1::{Lease, LeaseSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{MicroTime, ObjectMeta};
use k8s_openapi::jiff::Timestamp;
use kube::api::PostParams;
use kube::{Api, Client, Error as KubeError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration as StdDuration;
use tracing::{info, warn};

const DEFAULT_LEASE_DURATION_SECONDS: i32 = 15;
const DEFAULT_RENEW_INTERVAL_SECONDS: u64 = 5;

pub struct LeaderElectionConfig {
    pub namespace: String,
    pub lease_name: String,
    pub holder_id: String,
    pub lease_duration_seconds: i32,
    pub renew_interval_seconds: u64,
}

pub fn env_flag(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(default)
}

pub fn resolve_namespace() -> String {
    if let Ok(ns) = std::env::var("PERTISK_LEADER_ELECTION_NAMESPACE") {
        if !ns.trim().is_empty() {
            return ns;
        }
    }
    if let Ok(ns) = std::env::var("POD_NAMESPACE") {
        if !ns.trim().is_empty() {
            return ns;
        }
    }
    if let Ok(ns) = std::fs::read_to_string("/var/run/secrets/kubernetes.io/serviceaccount/namespace") {
        let ns = ns.trim();
        if !ns.is_empty() {
            return ns.to_string();
        }
    }
    "default".to_string()
}

pub fn resolve_holder_id() -> String {
    for key in ["PERTISK_POD_NAME", "POD_NAME", "HOSTNAME"] {
        if let Ok(value) = std::env::var(key) {
            if !value.trim().is_empty() {
                return value;
            }
        }
    }
    format!("pertisk-{}", uuid::Uuid::new_v4())
}

pub fn resolve_lease_name(default_name: &str) -> String {
    std::env::var("PERTISK_LEADER_ELECTION_NAME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default_name.to_string())
}

pub fn resolve_lease_duration_seconds() -> i32 {
    std::env::var("PERTISK_LEADER_ELECTION_LEASE_DURATION_SECONDS")
        .ok()
        .and_then(|v| v.parse::<i32>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_LEASE_DURATION_SECONDS)
}

pub fn resolve_renew_interval_seconds() -> u64 {
    std::env::var("PERTISK_LEADER_ELECTION_RENEW_INTERVAL_SECONDS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_RENEW_INTERVAL_SECONDS)
}

pub async fn start_leader_election(client: Client, config: LeaderElectionConfig) -> Arc<AtomicBool> {
    let is_leader = Arc::new(AtomicBool::new(false));
    let is_leader_task = is_leader.clone();
    let namespace = config.namespace.clone();
    let lease_name = config.lease_name.clone();
    info!(
        "Leader election enabled: lease {}/{} (holder={})",
        namespace, lease_name, config.holder_id
    );

    tokio::spawn(async move {
        let leases: Api<Lease> = Api::namespaced(client, &config.namespace);
        let mut last_state = None::<bool>;

        loop {
            let now = Timestamp::now();
            let mut leader = false;
            match try_acquire_or_renew(&leases, &config, now).await {
                Ok(state) => leader = state,
                Err(err) => warn!("Leader election error: {}", err),
            }

            if last_state != Some(leader) {
                if leader {
                    info!("Leader election: acquired leadership for {}", config.lease_name);
                } else {
                    info!("Leader election: not leader for {}", config.lease_name);
                }
                last_state = Some(leader);
            }

            is_leader_task.store(leader, Ordering::Relaxed);
            tokio::time::sleep(StdDuration::from_secs(config.renew_interval_seconds)).await;
        }
    });

    is_leader
}

async fn try_acquire_or_renew(
    leases: &Api<Lease>,
    config: &LeaderElectionConfig,
    now: Timestamp,
) -> Result<bool, KubeError> {
    let lease = leases.get_opt(&config.lease_name).await?;

    match lease {
        None => {
            let lease = new_lease(config, now, true);
            match leases.create(&PostParams::default(), &lease).await {
                Ok(_) => Ok(true),
                Err(KubeError::Api(ae)) if ae.code == 409 => Ok(false),
                Err(e) => Err(e),
            }
        }
        Some(current) => {
            let (current_holder, renew_time, lease_duration) = current_lease_state(&current);
            let expired = is_expired(renew_time, lease_duration, now);
            let is_owner = current_holder.as_deref() == Some(config.holder_id.as_str());
            if !expired && !is_owner {
                return Ok(false);
            }

            let mut next = current.clone();
            next.metadata.resource_version = current.metadata.resource_version.clone();
            next.spec = Some(LeaseSpec {
                holder_identity: Some(config.holder_id.clone()),
                lease_duration_seconds: Some(config.lease_duration_seconds),
                renew_time: Some(MicroTime(now)),
                acquire_time: if is_owner {
                    renew_time.map(MicroTime)
                } else {
                    Some(MicroTime(now))
                },
                ..Default::default()
            });

            match leases
                .replace(&config.lease_name, &PostParams::default(), &next)
                .await
            {
                Ok(_) => Ok(true),
                Err(KubeError::Api(ae)) if ae.code == 409 => Ok(false),
                Err(e) => Err(e),
            }
        }
    }
}

fn new_lease(config: &LeaderElectionConfig, now: Timestamp, acquired: bool) -> Lease {
    Lease {
        metadata: ObjectMeta {
            name: Some(config.lease_name.clone()),
            namespace: Some(config.namespace.clone()),
            ..Default::default()
        },
        spec: Some(LeaseSpec {
            holder_identity: Some(config.holder_id.clone()),
            lease_duration_seconds: Some(config.lease_duration_seconds),
            acquire_time: if acquired { Some(MicroTime(now)) } else { None },
            renew_time: Some(MicroTime(now)),
            ..Default::default()
        }),
    }
}

fn current_lease_state(lease: &Lease) -> (Option<String>, Option<Timestamp>, i32) {
    let lease_duration = lease
        .spec
        .as_ref()
        .and_then(|spec| spec.lease_duration_seconds)
        .unwrap_or(DEFAULT_LEASE_DURATION_SECONDS);
    let holder = lease
        .spec
        .as_ref()
        .and_then(|spec| spec.holder_identity.clone());
    let renew_time = lease
        .spec
        .as_ref()
        .and_then(|spec| spec.renew_time.as_ref())
        .map(|t| t.0);
    (holder, renew_time, lease_duration)
}

fn is_expired(renew_time: Option<Timestamp>, lease_duration: i32, now: Timestamp) -> bool {
    let renew = renew_time.unwrap_or_else(|| {
        now.checked_sub(std::time::Duration::from_secs(lease_duration as u64 * 2))
            .unwrap_or(now)
    });
    let expiry = renew
        .checked_add(std::time::Duration::from_secs(lease_duration as u64))
        .unwrap_or(now);
    expiry < now
}
