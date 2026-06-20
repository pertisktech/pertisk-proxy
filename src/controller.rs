use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use futures::StreamExt;
use k8s_openapi::api::networking::v1::Ingress;
use kube::api::{Api, ListParams, ResourceExt};
use kube::runtime::watcher::{self, Event};
use kube::runtime::WatchStreamExt;
use kube::Client;
use tracing::{info, warn};

use crate::config::Config;
use crate::router::{build_route_table_from_ingresses, ingress_matches_class, Router};

pub async fn run(router: Arc<Router>, config: Config) -> Result<()> {
    let client = Client::try_default().await?;
    info!("connected to Kubernetes API");

    let api: Api<Ingress> = if config.watch_all_namespaces {
        Api::all(client.clone())
    } else {
        let namespace = config
            .watch_namespace
            .as_deref()
            .unwrap_or("default");
        Api::namespaced(client.clone(), namespace)
    };

    let class = config.ingress_class.clone();
    let mut stream = watcher::watcher(api, watcher::Config::default())
        .default_backoff()
        .boxed();

    let mut ingresses: HashMap<String, Ingress> = HashMap::new();

    while let Some(event) = stream.next().await {
        match event {
            Ok(Event::Init) => {
                info!("ingress watch restarting, clearing local cache");
                ingresses.clear();
            }
            Ok(Event::InitApply(ingress)) | Ok(Event::Apply(ingress)) => {
                if !ingress_matches_class(&ingress, class.as_deref()) {
                    let key = ingress_key(&ingress);
                    if ingresses.remove(&key).is_some() {
                        rebuild_router(&router, &ingresses);
                    }
                    continue;
                }

                let key = ingress_key(&ingress);
                info!(ingress = key, "ingress applied");
                ingresses.insert(key, ingress);
                rebuild_router(&router, &ingresses);
            }
            Ok(Event::InitDone) => {
                info!(
                    ingresses = ingresses.len(),
                    "ingress watch initial sync complete"
                );
                rebuild_router(&router, &ingresses);
            }
            Ok(Event::Delete(ingress)) => {
                let key = ingress_key(&ingress);
                info!(ingress = key, "ingress deleted");
                ingresses.remove(&key);
                rebuild_router(&router, &ingresses);
            }
            Err(err) => {
                warn!(error = %err, "ingress watch error");
            }
        }
    }

    Ok(())
}

fn ingress_key(ingress: &Ingress) -> String {
    format!(
        "{}/{}",
        ingress.namespace().unwrap_or_else(|| "default".into()),
        ingress.name_any()
    )
}

fn rebuild_router(router: &Router, ingresses: &HashMap<String, Ingress>) {
    let table = build_route_table_from_ingresses(ingresses.values().cloned());
    info!(routes = table.route_count(), "rebuilt routing table");
    router.replace(table);
}

/// Bootstrap routing from a full list when running outside the watch loop (tests/local dev).
pub async fn bootstrap_from_list(client: Client, config: &Config, router: Arc<Router>) -> Result<()> {
    let api: Api<Ingress> = if config.watch_all_namespaces {
        Api::all(client)
    } else {
        let namespace = config
            .watch_namespace
            .as_deref()
            .unwrap_or("default");
        Api::namespaced(client, namespace)
    };

    let list = api.list(&ListParams::default()).await?;
    let mut ingresses = HashMap::new();

    for ingress in list.items {
        if !ingress_matches_class(&ingress, config.ingress_class.as_deref()) {
            continue;
        }
        ingresses.insert(ingress_key(&ingress), ingress);
    }

    rebuild_router(&router, &ingresses);
    info!(ingresses = ingresses.len(), "bootstrapped ingress routes");
    Ok(())
}
