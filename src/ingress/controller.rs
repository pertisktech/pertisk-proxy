//! Kubernetes Ingress controller: reconcile Ingress and Gateway API resources into proxy config and TLS from Secrets.

use crate::db::{cert_expiry_from_pem, cert_hosts_from_pem};
use crate::ingress::gateway_api::{
    Gateway, GatewayClass, HTTPBackendRef, HTTPRoute, HTTPRouteMatch, HTTPRouteRule,
    ParentReference, SecretObjectReference,
};
use crate::log::{ProxyLog, ProxyLogEntry};
use crate::proxy::apply;
use crate::proxy_config::{
    Backend, Config, LoadBalancerAlgorithm, PathMatchType, PathRewrite, Site, TlsConfig,
    TlsSource, Upstream,
};
use crate::tls::CertStore;
use crate::Router;
use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::api::networking::v1::{Ingress, IngressBackend};
use kube::{
    api::{Api, ListParams, Patch, PatchParams},
    Client, ResourceExt,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::RwLock;
use tokio::sync::RwLock as AsyncRwLock;
use tracing::{info, warn};

/// Config for the Ingress controller (namespace filter, class name, etc.).
#[derive(Debug, Clone)]
pub struct IngressControllerConfig {
    pub namespace: Option<String>,
    pub ingress_class: Option<String>,
    pub gateway_class: Option<String>,
    pub gateway_api_enabled: bool,
    pub gateway_controller_name: String,
    pub default_backend_port: u16,
}

impl Default for IngressControllerConfig {
    fn default() -> Self {
        Self {
            namespace: None,
            ingress_class: Some("pertisk-proxy".to_string()),
            gateway_class: Some("pertisk-proxy".to_string()),
            gateway_api_enabled: true,
            gateway_controller_name: "pertisk.tech/ingress-controller".to_string(),
            default_backend_port: 80,
        }
    }
}

/// Controller that watches Kubernetes resources and updates Router, runtime config, and CertStore.
pub struct IngressController {
    client: Client,
    config: IngressControllerConfig,
    router: Arc<Router>,
    runtime_config: Arc<AsyncRwLock<Config>>,
    cert_store: Arc<CertStore>,
    proxy_log: Arc<ProxyLog>,
    /// Hosts for which we inserted certs from Ingress TLS Secrets; cleared and repopulated each reconcile.
    last_ingress_tls_hosts: RwLock<Vec<String>>,
}

impl IngressController {
    pub fn new(
        client: Client,
        config: IngressControllerConfig,
        router: Arc<Router>,
        runtime_config: Arc<AsyncRwLock<Config>>,
        cert_store: Arc<CertStore>,
        proxy_log: Arc<ProxyLog>,
    ) -> Self {
        Self {
            client,
            config,
            router,
            runtime_config,
            cert_store,
            proxy_log,
            last_ingress_tls_hosts: RwLock::new(Vec::new()),
        }
    }

    /// Run one reconciliation pass: list Ingress, build Config, sync TLS from Secrets, reload state.
    pub async fn reconcile(&self) -> Result<(), kube::Error> {
        let ingresses_api: Api<Ingress> = if let Some(ref ns) = self.config.namespace {
            Api::namespaced(self.client.clone(), &ns)
        } else {
            Api::all(self.client.clone())
        };

        let list_params = ListParams::default();
        let list = ingresses_api.list(&list_params).await?;

        let mut backends: Vec<Backend> = Vec::new();
        let mut sites: Vec<Site> = Vec::new();
        let mut backend_names: HashMap<String, String> = HashMap::new();
        // (namespace, secret_name) -> list of hostnames to use for this TLS secret
        let mut tls_secrets: HashMap<(String, String), Vec<String>> = HashMap::new();

        for ingress in list {
            let spec = match ingress.spec.as_ref() {
                Some(s) => s,
                None => continue,
            };

            if let Some(ref class) = spec.ingress_class_name.as_ref() {
                if let Some(ref want) = self.config.ingress_class {
                    if class.as_str() != want.as_str() {
                        continue;
                    }
                }
            }

            let ns = ingress.namespace().unwrap_or_else(|| "default".to_string());
            let ingress_name = ingress.name_any();
            let geoip = crate::geoip::policy_from_annotations(
                ingress.metadata.annotations.as_ref(),
            );
            let security = crate::security::policy_from_annotations(
                ingress.metadata.annotations.as_ref(),
            );
            let rule_hosts: Vec<String> = spec
                .rules
                .as_deref()
                .unwrap_or_default()
                .iter()
                .filter_map(|r| r.host.as_ref().cloned())
                .collect();

            for rule in spec.rules.as_deref().unwrap_or_default() {
                let host = rule.host.as_deref().unwrap_or("*");
                let http = match rule.http.as_ref() {
                    Some(h) => h,
                    None => continue,
                };
                for path in http.paths.as_slice() {
                    let path_match = match path.path_type.as_str() {
                        "Exact" => PathMatchType::Exact,
                        "Prefix" => PathMatchType::Prefix,
                        _ => PathMatchType::ImplementationSpecific,
                    };
                    let backend_name = backend_name_for(&path.backend, &ingress_name);
                    if !backend_names.contains_key(&backend_name) {
                        let upstreams = upstreams_from_backend(
                            &path.backend,
                            Some(ns.as_str()),
                            self.config.default_backend_port,
                        );
                        backends.push(Backend {
                            name: backend_name.clone(),
                            upstreams,
                            algorithm: crate::proxy_config::LoadBalancerAlgorithm::RoundRobin,
                            health_path: None,
                            health_interval_secs: 10,
                        });
                        backend_names.insert(backend_name.clone(), backend_name.clone());
                    }
                    sites.push(Site {
                        host: host.to_string(),
                        routes: vec![PathRewrite {
                            path_type: path_match,
                            path: path.path.clone().unwrap_or_else(|| "/".to_string()),
                            rewrite: None,
                            upstream: None,
                        }],
                        backend: backend_name,
                        security_headers: None,
                        ingress_namespace: Some(ns.clone()),
                        ingress_name: Some(ingress_name.clone()),
                        k8s_resource_kind: Some("ingress".to_string()),
                        http3_alt_svc_enabled: true,
                        forward_client_ip: true,
                        geoip: geoip.clone(),
                        security: security.clone(),
                    });
                }
            }

            // Collect TLS: secretName -> hosts (from spec.tls or from rule hosts if tls.hosts empty)
            for tls in spec.tls.as_deref().unwrap_or_default() {
                let secret_name = match tls.secret_name.as_deref() {
                    Some(s) if !s.is_empty() => s.to_string(),
                    _ => continue,
                };
                let mut hosts: Vec<String> = if let Some(ref h) = tls.hosts {
                    if h.is_empty() {
                        rule_hosts.clone()
                    } else {
                        h.clone()
                    }
                } else {
                    rule_hosts.clone()
                };
                // Include all rule hosts so site rows match TLS (e.g. wildcard in spec.tls.hosts).
                hosts.extend(rule_hosts.iter().cloned());
                if hosts.is_empty() {
                    continue;
                }
                tls_secrets
                    .entry((ns.clone(), secret_name))
                    .or_default()
                    .extend(hosts.into_iter());
            }
        }

        // Reconcile Gateway API (Gateway + HTTPRoute) when enabled.
        let (n_gateways, n_httproutes) = if self.config.gateway_api_enabled {
            if let Err(e) = self.reconcile_gateway_class_status().await {
                warn!("Failed to update GatewayClass status: {}", e);
            }
            self.reconcile_gateway_api(
                &mut backends,
                &mut sites,
                &mut backend_names,
                &mut tls_secrets,
            )
            .await
        } else {
            (0, 0)
        };

        // Dedupe and sort hosts per (namespace, secret_name) so UI never shows duplicates.
        for hosts in tls_secrets.values_mut() {
            let set: HashSet<_> = hosts.drain(..).collect();
            let mut list: Vec<_> = set.into_iter().collect();
            list.sort();
            *hosts = list;
        }

        // Snapshot previous ingress TLS hosts so we can remove obsolete entries after
        // new certs are inserted (prevents temporary "no cert" windows during reconcile).
        let previous = self
            .last_ingress_tls_hosts
            .read()
            .map(|g| g.clone())
            .unwrap_or_default();

        let n_tls_secrets = tls_secrets.len();
        let mut new_ingress_hosts: Vec<String> = Vec::new();
        let mut tls_from_secrets: Vec<TlsConfig> = Vec::new();

        for ((namespace, secret_name), hosts) in tls_secrets {
            let ns_api: Api<Secret> = Api::namespaced(self.client.clone(), &namespace);
            let secret = match ns_api.get(&secret_name).await {
                Ok(s) => s,
                Err(e) => {
                    warn!("Failed to get Secret {}/{}: {}", namespace, secret_name, e);
                    continue;
                }
            };
            let type_ = secret.type_.as_deref().unwrap_or("");
            if type_ != "kubernetes.io/tls" && !secret.data.as_ref().map_or(false, |d| d.contains_key("tls.crt")) {
                continue;
            }
            let data = match secret.data.as_ref() {
                Some(d) => d,
                None => continue,
            };
            let cert_pem = match data.get("tls.crt") {
                Some(b) => b.0.clone(),
                None => continue,
            };
            let key_pem = match data.get("tls.key") {
                Some(b) => b.0.clone(),
                None => continue,
            };
            if cert_pem.is_empty() || key_pem.is_empty() {
                warn!("Secret {}/{} has empty tls.crt or tls.key", namespace, secret_name);
                continue;
            }
            let expires_at = cert_expiry_from_pem(&cert_pem);
            let mut merged_hosts = hosts;
            for host in cert_hosts_from_pem(&cert_pem) {
                if !merged_hosts.iter().any(|h| h == &host) {
                    merged_hosts.push(host);
                }
            }
            merged_hosts.sort();
            merged_hosts.dedup();
            tls_from_secrets.push(TlsConfig {
                hosts: merged_hosts.clone(),
                source: TlsSource::Kubernetes,
                expires_at,
            });
            if let Err(e) = self.cert_store.insert_pem_in_memory_for_hosts(
                &merged_hosts,
                &cert_pem,
                &key_pem,
            ) {
                warn!(
                    "Failed to load TLS from Secret {}/{}: {}",
                    namespace, secret_name, e
                );
                continue;
            }
            new_ingress_hosts.extend(merged_hosts);
        }

        if !new_ingress_hosts.is_empty() {
            let new_set: HashSet<String> = new_ingress_hosts.iter().cloned().collect();
            let obsolete_hosts: Vec<String> = previous
                .iter()
                .filter(|h| !new_set.contains(*h))
                .cloned()
                .collect();
            if !obsolete_hosts.is_empty() {
                self.cert_store.remove_for_hosts(&obsolete_hosts);
            }
            if let Ok(mut g) = self.last_ingress_tls_hosts.write() {
                *g = new_ingress_hosts;
            }
        } else if !previous.is_empty() {
            warn!(
                "No TLS certs loaded in this reconcile pass; keeping previous certificates to avoid HTTPS/HTTP3 downtime"
            );
        }

        let n_backends = backends.len();
        let n_sites = sites.len();
        let config = Config {
            backends,
            sites,
            tls: tls_from_secrets,
            ..Config::default()
        };
        if let Err(e) = apply::apply_config(self.router.as_ref(), &config) {
            warn!("Failed to apply ingress config to router: {}", e);
        } else {
            *self.runtime_config.write().await = config;
        }
        info!(
            "Ingress reconciliation complete ({} backends, {} sites, TLS from {} Secrets, {} Gateways, {} HTTPRoutes)",
            n_backends,
            n_sites,
            n_tls_secrets,
            n_gateways,
            n_httproutes
        );
        let _ = self
            .proxy_log
            .push(ProxyLogEntry::config_reload(format!(
                "Ingress reconciliation: {} backends, {} sites, TLS from {} Secrets, {} Gateways, {} HTTPRoutes",
                n_backends, n_sites, n_tls_secrets, n_gateways, n_httproutes
            )))
            .await;
        Ok(())
    }

    /// Set GatewayClass status Accepted=True for classes managed by this controller.
    async fn reconcile_gateway_class_status(&self) -> Result<(), kube::Error> {
        let api: Api<GatewayClass> = Api::all(self.client.clone());
        let list = api.list(&ListParams::default()).await?;
        let now = chrono::Utc::now()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        for gc in list {
            if gc.spec.controller_name != self.config.gateway_controller_name {
                continue;
            }
            let generation = gc.metadata.generation.unwrap_or(0);
            if gateway_class_already_accepted(&gc, generation) {
                continue;
            }
            let name = gc.name_any();
            let patch = serde_json::json!({
                "status": {
                    "conditions": [{
                        "type": "Accepted",
                        "status": "True",
                        "reason": "Accepted",
                        "message": "Handled by pertisk-proxy ingress controller",
                        "lastTransitionTime": now,
                        "observedGeneration": generation,
                    }]
                }
            });
            api.patch_status(
                &name,
                &PatchParams::default(),
                &Patch::Merge(&patch),
            )
            .await?;
            info!("GatewayClass {} marked Accepted", name);
        }
        Ok(())
    }

    /// Reconcile Gateway API resources into proxy config.
    async fn reconcile_gateway_api(
        &self,
        backends: &mut Vec<Backend>,
        sites: &mut Vec<Site>,
        backend_names: &mut HashMap<String, String>,
        tls_secrets: &mut HashMap<(String, String), Vec<String>>,
    ) -> (usize, usize) {
        let gw_api: Api<Gateway> = if let Some(ref ns) = self.config.namespace {
            Api::namespaced(self.client.clone(), ns)
        } else {
            Api::all(self.client.clone())
        };
        let route_api: Api<HTTPRoute> = if let Some(ref ns) = self.config.namespace {
            Api::namespaced(self.client.clone(), ns)
        } else {
            Api::all(self.client.clone())
        };
        let list_params = ListParams::default();

        let gw_list = match gw_api.list(&list_params).await {
            Ok(l) => l,
            Err(e) => {
                warn!(
                    "Failed to list Gateway resources (install Gateway API CRDs and RBAC): {}",
                    e
                );
                return (0, 0);
            }
        };
        let gateways = gw_list.items;

        let mut managed_gateways: HashMap<(String, String), ManagedGateway> = HashMap::new();
        for gw in &gateways {
            let spec = &gw.spec;
            if let Some(ref want) = self.config.gateway_class {
                match spec.gateway_class_name.as_deref() {
                    Some(class) if class == want.as_str() => {}
                    Some(class) => {
                        tracing::debug!(
                            "Skipping Gateway {} (class {} != {})",
                            gw.name_any(),
                            class,
                            want
                        );
                        continue;
                    }
                    None => continue,
                }
            }
            let ns = gw.namespace().unwrap_or_else(|| "default".to_string());
            let name = gw.name_any();
            let mut listener_hostnames: Vec<Option<String>> = Vec::new();
            let mut tls_secret_refs: Vec<(String, String)> = Vec::new();
            for listener in &spec.listeners {
                if !is_http_listener_protocol(&listener.protocol) {
                    continue;
                }
                listener_hostnames.push(listener.hostname.clone());
                if let Some(ref tls) = listener.tls {
                    let listener_hosts = listener
                        .hostname
                        .as_ref()
                        .map(|h| vec![h.clone()])
                        .unwrap_or_default();
                    for cert_ref in &tls.certificate_refs {
                        if !is_secret_ref(cert_ref) {
                            continue;
                        }
                        let secret_ns = cert_ref
                            .namespace
                            .as_deref()
                            .unwrap_or(ns.as_str())
                            .to_string();
                        tls_secret_refs.push((secret_ns.clone(), cert_ref.name.clone()));
                        let hosts = if listener_hosts.is_empty() {
                            vec!["*".to_string()]
                        } else {
                            listener_hosts.clone()
                        };
                        tls_secrets
                            .entry((secret_ns, cert_ref.name.clone()))
                            .or_default()
                            .extend(hosts);
                    }
                }
            }
            managed_gateways.insert(
                (ns.clone(), name.clone()),
                ManagedGateway {
                    name,
                    listener_hostnames,
                    tls_secret_refs,
                },
            );
        }

        let n_gateways = managed_gateways.len();
        if n_gateways == 0 {
            return (0, 0);
        }

        let route_list = match route_api.list(&list_params).await {
            Ok(l) => l,
            Err(e) => {
                warn!("Failed to list HTTPRoute resources: {}", e);
                if let Err(e) = self
                    .reconcile_gateway_statuses(&gateways, &[])
                    .await
                {
                    warn!("Failed to update Gateway status: {}", e);
                }
                return (n_gateways, 0);
            }
        };
        let routes = route_list.items;

        let mut n_httproutes = 0usize;
        for route in &routes {
            let route_ns = route.namespace().unwrap_or_else(|| "default".to_string());
            let route_name = route.name_any();
            let geoip = crate::geoip::policy_from_annotations(route.metadata.annotations.as_ref());
            let security =
                crate::security::policy_from_annotations(route.metadata.annotations.as_ref());
            let attached = attached_gateways(&route, &managed_gateways);
            if attached.is_empty() {
                continue;
            }
            n_httproutes += 1;

            let hosts = resolve_route_hostnames(&route.spec.hostnames, &attached);
            for host in &hosts {
                for gw in &attached {
                    for (secret_ns, secret_name) in &gw.tls_secret_refs {
                        tls_secrets
                            .entry((secret_ns.clone(), secret_name.clone()))
                            .or_default()
                            .push(host.clone());
                    }
                }
            }
            for host in &hosts {
                let rules = if route.spec.rules.is_empty() {
                    vec![HTTPRouteRule {
                        matches: vec![],
                        backend_refs: vec![],
                    }]
                } else {
                    route.spec.rules.clone()
                };
                for (rule_idx, rule) in rules.iter().enumerate() {
                    let matches: Vec<HTTPRouteMatch> = if rule.matches.is_empty() {
                        vec![HTTPRouteMatch { path: None }]
                    } else {
                        rule.matches.clone()
                    };
                    for (match_idx, m) in matches.iter().enumerate() {
                        let (path_type, path) = path_from_match(m);
                        let backend_refs = if rule.backend_refs.is_empty() {
                            continue;
                        } else {
                            &rule.backend_refs
                        };
                        for (backend_idx, backend_ref) in backend_refs.iter().enumerate() {
                            if !is_service_backend_ref(backend_ref) {
                                warn!(
                                    "HTTPRoute {}/{} backendRef kind {:?} not supported (Service only)",
                                    route_ns,
                                    route_name,
                                    backend_ref.kind
                                );
                                continue;
                            }
                            let backend_name = gateway_backend_name(
                                backend_ref,
                                &route_name,
                                rule_idx,
                                match_idx,
                                backend_idx,
                            );
                            if !backend_names.contains_key(&backend_name) {
                                let upstreams = upstreams_from_gateway_backend(
                                    backend_ref,
                                    Some(route_ns.as_str()),
                                    self.config.default_backend_port,
                                );
                                if upstreams.is_empty() {
                                    continue;
                                }
                                backends.push(Backend {
                                    name: backend_name.clone(),
                                    upstreams,
                                    algorithm: LoadBalancerAlgorithm::RoundRobin,
                                    health_path: None,
                                    health_interval_secs: 10,
                                });
                                backend_names.insert(backend_name.clone(), backend_name.clone());
                            }
                            sites.push(Site {
                                host: host.clone(),
                                routes: vec![PathRewrite {
                                    path_type,
                                    path: path.clone(),
                                    rewrite: None,
                                    upstream: None,
                                }],
                                backend: backend_name,
                                security_headers: None,
                                ingress_namespace: Some(route_ns.clone()),
                                ingress_name: Some(route_name.clone()),
                                k8s_resource_kind: Some("httproute".to_string()),
                                http3_alt_svc_enabled: true,
                                forward_client_ip: true,
                                geoip: geoip.clone(),
                                security: security.clone(),
                            });
                        }
                    }
                }
            }
        }

        if let Err(e) = self
            .reconcile_gateway_statuses(&gateways, &routes)
            .await
        {
            warn!("Failed to update Gateway status: {}", e);
        }

        (n_gateways, n_httproutes)
    }

    /// Patch Gateway status so UIs show Accepted/Programmed instead of Pending/Unknown.
    async fn reconcile_gateway_statuses(
        &self,
        gw_list: &[Gateway],
        route_list: &[HTTPRoute],
    ) -> Result<(), kube::Error> {
        let now = chrono::Utc::now()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let addresses = self.gateway_status_addresses().await;

        for gw in gw_list {
            let spec = &gw.spec;
            if let Some(ref want) = self.config.gateway_class {
                match spec.gateway_class_name.as_deref() {
                    Some(class) if class == want.as_str() => {}
                    _ => continue,
                }
            } else if spec.gateway_class_name.is_none() {
                continue;
            }
            let ns = gw.namespace().unwrap_or_else(|| "default".to_string());
            let name = gw.name_any();
            let generation = gw.metadata.generation.unwrap_or(0);

            let mut listener_statuses = Vec::new();
            let mut all_listeners_programmed = true;
            let mut all_listeners_accepted = true;

            for listener in &spec.listeners {
                if !is_http_listener_protocol(&listener.protocol) {
                    continue;
                }
                let attached_routes =
                    count_attached_routes(route_list, &ns, &name, &listener.name);
                let tls_ok = match listener.tls.as_ref() {
                    None => true,
                    Some(tls) => {
                        let mut ok = true;
                        for cert_ref in &tls.certificate_refs {
                            if !is_secret_ref(cert_ref) {
                                ok = false;
                                continue;
                            }
                            if !self.gateway_tls_secret_ready(&ns, cert_ref).await {
                                ok = false;
                            }
                        }
                        ok
                    }
                };
                let accepted = tls_ok;
                let programmed = accepted && attached_routes > 0;
                all_listeners_accepted &= accepted;
                all_listeners_programmed &= programmed;

                let mut conditions = vec![gateway_condition(
                    "Accepted",
                    accepted,
                    if accepted {
                        "Accepted"
                    } else {
                        "InvalidCertificateRef"
                    },
                    if accepted {
                        "Listener accepted by pertisk-proxy ingress controller"
                    } else {
                        "TLS certificate Secret missing or not ready"
                    },
                    &now,
                    generation,
                )];
                conditions.push(gateway_condition(
                    "Programmed",
                    programmed,
                    if programmed {
                        "Programmed"
                    } else if !accepted {
                        "Invalid"
                    } else {
                        "NoRoutesAttached"
                    },
                    if programmed {
                        "Listener programmed with attached HTTPRoutes"
                    } else if !accepted {
                        "Listener not accepted"
                    } else {
                        "No HTTPRoutes attached to this listener"
                    },
                    &now,
                    generation,
                ));

                listener_statuses.push(serde_json::json!({
                    "name": listener.name,
                    "supportedKinds": [{
                        "group": "gateway.networking.k8s.io",
                        "kind": "HTTPRoute"
                    }],
                    "attachedRoutes": attached_routes,
                    "conditions": conditions,
                }));
            }

            if listener_statuses.is_empty() {
                continue;
            }

            let gateway_conditions = vec![
                gateway_condition(
                    "Accepted",
                    all_listeners_accepted,
                    if all_listeners_accepted {
                        "Accepted"
                    } else {
                        "ListenersNotValid"
                    },
                    if all_listeners_accepted {
                        "All listeners accepted"
                    } else {
                        "One or more listeners are not accepted"
                    },
                    &now,
                    generation,
                ),
                gateway_condition(
                    "Programmed",
                    all_listeners_programmed,
                    if all_listeners_programmed {
                        "Programmed"
                    } else {
                        "ListenersNotProgrammed"
                    },
                    if all_listeners_programmed {
                        "All listeners programmed"
                    } else {
                        "One or more listeners are not programmed"
                    },
                    &now,
                    generation,
                ),
            ];

            if gateway_status_up_to_date(gw.status.as_ref(), generation, all_listeners_programmed) {
                continue;
            }

            let mut status = serde_json::json!({
                "listeners": listener_statuses,
                "conditions": gateway_conditions,
            });
            if !addresses.is_empty() {
                status["addresses"] = serde_json::json!(addresses
                    .iter()
                    .map(|value| serde_json::json!({ "type": "IPAddress", "value": value }))
                    .collect::<Vec<_>>());
            }

            let patch = serde_json::json!({ "status": status });
            let api: Api<Gateway> = Api::namespaced(self.client.clone(), &ns);
            api.patch_status(&name, &PatchParams::default(), &Patch::Merge(&patch))
                .await?;
            info!(
                "Gateway {}/{} status updated (Accepted={}, Programmed={})",
                ns, name, all_listeners_accepted, all_listeners_programmed
            );
        }
        Ok(())
    }

    async fn gateway_tls_secret_ready(&self, gw_ns: &str, cert_ref: &SecretObjectReference) -> bool {
        let secret_ns = cert_ref
            .namespace
            .as_deref()
            .unwrap_or(gw_ns)
            .to_string();
        let api: Api<Secret> = Api::namespaced(self.client.clone(), &secret_ns);
        match api.get(&cert_ref.name).await {
            Ok(secret) => secret
                .data
                .as_ref()
                .map(|data| data.contains_key("tls.crt") && data.contains_key("tls.key"))
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    async fn gateway_status_addresses(&self) -> Vec<String> {
        use k8s_openapi::api::core::v1::Service;
        let release = match std::env::var("PERTISK_HELM_RELEASE") {
            Ok(v) if !v.trim().is_empty() => v,
            _ => return Vec::new(),
        };
        let ns = std::env::var("PERTISK_HELM_NAMESPACE")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "pertisk-proxy".to_string());
        let api: Api<Service> = Api::namespaced(self.client.clone(), &ns);
        let service = match api.get(&release).await {
            Ok(svc) => svc,
            Err(_) => return Vec::new(),
        };
        service
            .status
            .and_then(|status| status.load_balancer)
            .map(|lb| lb.ingress.unwrap_or_default())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|ingress| {
                ingress
                    .ip
                    .or(ingress.hostname)
                    .filter(|value| !value.is_empty())
            })
            .collect()
    }
}

fn backend_name_for(backend: &IngressBackend, ingress_name: &str) -> String {
    if let Some(ref svc) = backend.service {
        let name = svc.name.as_str();
        let port = svc
            .port
            .as_ref()
            .and_then(|p| p.number)
            .unwrap_or(80i32);
        format!("{}.{}:{}", name, ingress_name, port)
    } else if let Some(ref res) = backend.resource {
        format!("resource.{}.{}", res.name.as_str(), ingress_name)
    } else {
        format!("default.{}", ingress_name)
    }
}

fn resolve_upstream_addr(service_name: &str, namespace: &str, port: u16) -> String {
    if is_self_management_service(service_name, namespace, port) {
        format!("127.0.0.1:{port}")
    } else {
        format!("{service_name}.{namespace}.svc.cluster.local:{port}")
    }
}

/// When an Ingress/Gateway routes to this release's Service on the management port, proxy
/// loopback — the admin UI runs in-process on the same pod (ClusterIP hairpin often fails).
fn is_self_management_service(service_name: &str, namespace: &str, port: u16) -> bool {
    if port != crate::api::management_addr().port() {
        return false;
    }
    let helm_ns = std::env::var("PERTISK_HELM_NAMESPACE")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "pertisk-proxy".to_string());
    if namespace != helm_ns {
        return false;
    }
    if let Ok(service) = std::env::var("PERTISK_HELM_SERVICE_NAME") {
        let service = service.trim();
        if !service.is_empty() && service_name == service {
            return true;
        }
    }
    std::env::var("PERTISK_HELM_RELEASE")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .is_some_and(|release| service_name == release)
}

fn upstreams_from_backend(
    backend: &IngressBackend,
    namespace: Option<&str>,
    default_port: u16,
) -> Vec<Upstream> {
    if let Some(ref svc) = backend.service {
        let name = svc.name.as_str();
        let port = svc
            .port
            .as_ref()
            .and_then(|p| p.number)
            .map(|p| p as u16)
            .unwrap_or(default_port);
        let ns = namespace.unwrap_or("default");
        let addr = resolve_upstream_addr(name, ns, port);
        vec![Upstream { addr, weight: 1 }]
    } else {
        Vec::new()
    }
}

#[derive(Clone)]
struct ManagedGateway {
    name: String,
    listener_hostnames: Vec<Option<String>>,
    tls_secret_refs: Vec<(String, String)>,
}

fn is_http_listener_protocol(protocol: &str) -> bool {
    matches!(
        protocol.to_ascii_uppercase().as_str(),
        "HTTP" | "HTTPS"
    )
}

fn is_secret_ref(cert_ref: &SecretObjectReference) -> bool {
    let group = cert_ref.group.as_deref().unwrap_or("");
    let kind = cert_ref.kind.as_deref().unwrap_or("Secret");
    group.is_empty() && kind == "Secret"
}

fn is_service_backend_ref(backend_ref: &HTTPBackendRef) -> bool {
    let group = backend_ref.group.as_deref().unwrap_or("");
    let kind = backend_ref.kind.as_deref().unwrap_or("Service");
    group.is_empty() && kind == "Service"
}

fn parent_ref_matches_gateway(parent: &ParentReference, gw_ns: &str, gw_name: &str) -> bool {
    let group = parent
        .group
        .as_deref()
        .unwrap_or("gateway.networking.k8s.io");
    let kind = parent.kind.as_deref().unwrap_or("Gateway");
    if group != "gateway.networking.k8s.io" || kind != "Gateway" {
        return false;
    }
    if parent.name != gw_name {
        return false;
    }
    let parent_ns = parent.namespace.as_deref().unwrap_or(gw_ns);
    parent_ns == gw_ns
}

fn attached_gateways(
    route: &HTTPRoute,
    managed: &HashMap<(String, String), ManagedGateway>,
) -> Vec<ManagedGateway> {
    let route_ns = route.namespace().unwrap_or_else(|| "default".to_string());
    let route_name = route.name_any();
    let mut attached = Vec::new();

    if route.spec.parent_refs.is_empty() {
        // Default attachment: Gateway with the same name in the same namespace.
        if let Some(gw) = managed.get(&(route_ns.clone(), route_name.clone())) {
            attached.push(gw.clone());
        }
        return attached;
    }

    for parent in &route.spec.parent_refs {
        let gw_ns = parent
            .namespace
            .as_deref()
            .unwrap_or(route_ns.as_str())
            .to_string();
        if let Some(gw) = managed.get(&(gw_ns.clone(), parent.name.clone())) {
            if parent_ref_matches_gateway(parent, &gw_ns, &gw.name) {
                attached.push(gw.clone());
            }
        }
    }
    attached
}

fn hostname_matches_listener(route_host: &str, listener_hostname: &Option<String>) -> bool {
    match listener_hostname {
        None => true,
        Some(listener) => {
            if listener == route_host {
                return true;
            }
            if listener.starts_with("*.") {
                let suffix = &listener[1..];
                return route_host.ends_with(suffix) && route_host.len() > suffix.len();
            }
            false
        }
    }
}

fn resolve_route_hostnames(
    route_hostnames: &[String],
    attached_gateways: &[ManagedGateway],
) -> Vec<String> {
    let mut listener_hosts: Vec<String> = attached_gateways
        .iter()
        .flat_map(|gw| {
            gw.listener_hostnames
                .iter()
                .filter_map(|h| h.as_ref().cloned())
        })
        .collect();

    if route_hostnames.is_empty() {
        if listener_hosts.is_empty() {
            return vec!["*".to_string()];
        }
        listener_hosts.sort();
        listener_hosts.dedup();
        return listener_hosts;
    }

    if listener_hosts.is_empty() {
        return route_hostnames.to_vec();
    }

    route_hostnames
        .iter()
        .filter(|host| {
            attached_gateways.iter().any(|gw| {
                gw.listener_hostnames.is_empty()
                    || gw
                        .listener_hostnames
                        .iter()
                        .any(|lh| hostname_matches_listener(host, lh))
            })
        })
        .cloned()
        .collect()
}

fn path_from_match(m: &HTTPRouteMatch) -> (PathMatchType, String) {
    match m.path.as_ref() {
        Some(path) => {
            let path_type = match path.match_type.as_deref().unwrap_or("PathPrefix") {
                "Exact" => PathMatchType::Exact,
                "PathPrefix" | "Prefix" => PathMatchType::Prefix,
                _ => PathMatchType::ImplementationSpecific,
            };
            let value = path.value.clone().unwrap_or_else(|| "/".to_string());
            (path_type, value)
        }
        None => (PathMatchType::Prefix, "/".to_string()),
    }
}

fn gateway_backend_name(
    backend_ref: &HTTPBackendRef,
    route_name: &str,
    rule_idx: usize,
    match_idx: usize,
    backend_idx: usize,
) -> String {
    let port = backend_ref.port.unwrap_or(80);
    format!(
        "{}.{}:{}:{}:{}",
        backend_ref.name, route_name, port, rule_idx, match_idx * 10 + backend_idx
    )
}

fn upstreams_from_gateway_backend(
    backend_ref: &HTTPBackendRef,
    namespace: Option<&str>,
    default_port: u16,
) -> Vec<Upstream> {
    let port = backend_ref
        .port
        .map(|p| p as u16)
        .unwrap_or(default_port);
    let ns = backend_ref
        .namespace
        .as_deref()
        .or(namespace)
        .unwrap_or("default");
    let addr = resolve_upstream_addr(&backend_ref.name, ns, port);
    vec![Upstream { addr, weight: 1 }]
}

fn gateway_class_already_accepted(gc: &GatewayClass, generation: i64) -> bool {
    gc.status
        .as_ref()
        .map(|status| {
            status.conditions.iter().any(|c| {
                c.condition_type == "Accepted"
                    && c.status == "True"
                    && c.observed_generation.unwrap_or(0) >= generation
            })
        })
        .unwrap_or(false)
}

fn gateway_condition(
    condition_type: &str,
    ok: bool,
    reason: &str,
    message: &str,
    now: &str,
    generation: i64,
) -> serde_json::Value {
    serde_json::json!({
        "type": condition_type,
        "status": if ok { "True" } else { "False" },
        "reason": reason,
        "message": message,
        "lastTransitionTime": now,
        "observedGeneration": generation,
    })
}

fn count_attached_routes(
    route_list: &[HTTPRoute],
    gw_ns: &str,
    gw_name: &str,
    listener_name: &str,
) -> i32 {
    route_list
        .iter()
        .filter(|route| {
            route.spec.parent_refs.iter().any(|parent| {
                if !parent_ref_matches_gateway(parent, gw_ns, gw_name) {
                    return false;
                }
                match parent.section_name.as_deref() {
                    None | Some("") => true,
                    Some(section) => section == listener_name,
                }
            })
        })
        .count() as i32
}

fn gateway_status_up_to_date(
    status: Option<&crate::ingress::gateway_api::GatewayStatus>,
    generation: i64,
    programmed: bool,
) -> bool {
    let Some(status) = status else {
        return false;
    };
    let gateway_programmed = status.conditions.iter().any(|c| {
        c.condition_type == "Programmed"
            && c.status == (if programmed { "True" } else { "False" })
            && c.observed_generation.unwrap_or(0) >= generation
    });
    gateway_programmed
        && !status.listeners.is_empty()
        && status.listeners.iter().all(|listener| {
            listener.conditions.iter().any(|c| {
                c.condition_type == "Programmed"
                    && c.status == (if programmed { "True" } else { "False" })
                    && c.observed_generation.unwrap_or(0) >= generation
            })
        })
}
