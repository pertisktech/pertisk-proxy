//! Kubernetes dashboard API for ingress mode: list namespaces, pods, deployments, services, ingresses, nodes; create Ingress.

use super::AdminState;
use crate::db::cert_validity_from_pem;
use axum::{
    extract::{Path as AxumPath, Query, State},
    response::{IntoResponse, Response},
    Json,
};
use http::StatusCode;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{ConfigMap, Event, Namespace, Node, Pod, Secret, Service};
use k8s_openapi::api::networking::v1::{
    HTTPIngressPath, HTTPIngressRuleValue, Ingress, IngressBackend, IngressRule, IngressSpec,
    IngressTLS, IngressServiceBackend, ServiceBackendPort,
};
use k8s_openapi::jiff::Timestamp;
use kube::api::{Api, DeleteParams, ListParams, PostParams};
use kube::core::{ApiResource, DynamicObject, GroupVersionKind};
use kube::ResourceExt;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use crate::ingress::gateway_api::{
    AllowedRoutes, Gateway, GatewayListener, GatewayListenerTls, GatewaySpec, HTTPBackendRef,
    HTTPPathMatch, RouteNamespaces,
    HTTPRoute, HTTPRouteMatch, HTTPRouteRule, HTTPRouteSpec, ParentReference, SecretObjectReference,
};

fn format_k8s_time(ts: &Timestamp) -> String {
    ts.to_string()
}

#[derive(Serialize)]
pub struct K8sNamespaceRow {
    pub name: String,
    pub created_at: Option<String>,
}

#[derive(Serialize)]
pub struct K8sPodRow {
    pub name: String,
    pub namespace: String,
    pub phase: String,
    pub node: Option<String>,
    pub node_name: Option<String>,
    pub node_status: Option<String>,
    pub pod_ip: Option<String>,
    pub ready: String,
    pub restarts: u32,
    pub cpu_request_millicores: i64,
    pub memory_request_bytes: i64,
    pub cpu_usage_millicores: Option<i64>,
    pub memory_usage_bytes: Option<i64>,
    pub created_at: Option<String>,
}

#[derive(Serialize)]
pub struct K8sDeploymentRow {
    pub name: String,
    pub namespace: String,
    pub ready: String,
    pub replicas: i32,
    pub available: i32,
    pub created_at: Option<String>,
}

#[derive(Serialize)]
pub struct K8sServicePortDetail {
    pub port: i32,
    pub name: Option<String>,
    pub protocol: String,
}

#[derive(Serialize)]
pub struct K8sServiceRow {
    pub name: String,
    pub namespace: String,
    pub r#type: String,
    pub cluster_ip: Option<String>,
    pub external_ip: Option<String>,
    pub ports: Vec<String>,
    /// Port details for backend selection (port number, optional name, protocol).
    pub ports_detail: Vec<K8sServicePortDetail>,
    pub created_at: Option<String>,
}

#[derive(Serialize)]
pub struct K8sTlsSecretRow {
    pub namespace: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

#[derive(Serialize)]
pub struct K8sConfigMapRow {
    pub namespace: String,
    pub name: String,
    pub data_keys: Vec<String>,
    pub created_at: Option<String>,
}

#[derive(Serialize)]
pub struct K8sSecretRow {
    pub namespace: String,
    pub name: String,
    pub r#type: String,
    pub data_keys: Vec<String>,
    pub created_at: Option<String>,
    pub tls_expires_at: Option<String>,
}

#[derive(Serialize)]
pub struct K8sIngressRow {
    pub name: String,
    pub namespace: String,
    pub class: Option<String>,
    pub hosts: Vec<String>,
    pub created_at: Option<String>,
}

#[derive(Serialize)]
pub struct K8sNodeRow {
    pub name: String,
    pub ready: String,
    pub capacity_cpu: Option<String>,
    pub capacity_memory: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Serialize)]
pub struct K8sEventRow {
    pub namespace: String,
    pub name: String,
    pub r#type: Option<String>,
    pub reason: Option<String>,
    pub message: Option<String>,
    pub involved_kind: Option<String>,
    pub involved_name: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Serialize)]
pub struct K8sClusterSummary {
    // Node capacity (total)
    pub cpu_capacity: i64,      // in millicores
    pub memory_capacity: i64,   // in bytes
    
    // Node allocatable (usable)
    pub cpu_allocatable: i64,   // in millicores
    pub memory_allocatable: i64, // in bytes
    
    // Pod requests (sum of all pod requests)
    pub cpu_requests: i64,      // in millicores
    pub memory_requests: i64,   // in bytes
    
    // Pod limits (sum of all pod limits)
    pub cpu_limits: i64,        // in millicores
    pub memory_limits: i64,     // in bytes
    
    // Counts
    pub node_count: i32,
    pub pod_count: i32,
    pub pod_capacity: i64,
    pub pod_allocatable: i64,
    
    // Storage (from nodes)
    pub storage_capacity: i64,     // in bytes
    pub storage_allocatable: i64,  // in bytes
    pub storage_requests: i64,     // in bytes
    
    // Calculated percentages (0-100)
    pub cpu_requests_percent: f64,
    pub memory_requests_percent: f64,
    pub cpu_limits_percent: f64,
    pub memory_limits_percent: f64,
    pub storage_requests_percent: f64,
}

impl Default for K8sClusterSummary {
    fn default() -> Self {
        Self {
            cpu_capacity: 0,
            memory_capacity: 0,
            cpu_allocatable: 0,
            memory_allocatable: 0,
            cpu_requests: 0,
            memory_requests: 0,
            cpu_limits: 0,
            memory_limits: 0,
            node_count: 0,
            pod_count: 0,
            pod_capacity: 0,
            pod_allocatable: 0,
            storage_capacity: 0,
            storage_allocatable: 0,
            storage_requests: 0,
            cpu_requests_percent: 0.0,
            memory_requests_percent: 0.0,
            cpu_limits_percent: 0.0,
            memory_limits_percent: 0.0,
            storage_requests_percent: 0.0,
        }
    }
}

/// Parse Kubernetes CPU Quantity strings and return millicores.
/// Examples: "100m" => 100, "4" => 4000, "0.5" => 500.
fn parse_cpu_quantity(qty: &str) -> Option<i64> {
    let qty = qty.trim();
    if qty.is_empty() {
        return None;
    }

    let (num_str, multiplier) = if qty.ends_with('n') {
        (&qty[..qty.len() - 1], 0.000_001_f64) // nanocores -> millicores
    } else if qty.ends_with('u') {
        (&qty[..qty.len() - 1], 0.001_f64) // microcores -> millicores
    } else if qty.ends_with('m') {
        (&qty[..qty.len() - 1], 1.0_f64) // millicores
    } else {
        (qty, 1000.0_f64) // cores -> millicores
    };

    num_str
        .parse::<f64>()
        .ok()
        .map(|n| (n * multiplier).round() as i64)
}

/// Parse Kubernetes memory/storage Quantity strings and return bytes.
fn parse_bytes_quantity(qty: &str) -> Option<i64> {
    let qty = qty.trim();
    if qty.is_empty() {
        return None;
    }

    let (num_str, multiplier) = if qty.ends_with("Ei") {
        (&qty[..qty.len() - 2], 1_024_f64.powi(6))
    } else if qty.ends_with("Pi") {
        (&qty[..qty.len() - 2], 1_024_f64.powi(5))
    } else if qty.ends_with("Ti") {
        (&qty[..qty.len() - 2], 1_024_f64.powi(4))
    } else if qty.ends_with("Gi") {
        (&qty[..qty.len() - 2], 1_024_f64.powi(3))
    } else if qty.ends_with("Mi") {
        (&qty[..qty.len() - 2], 1_024_f64.powi(2))
    } else if qty.ends_with("Ki") {
        (&qty[..qty.len() - 2], 1_024_f64)
    } else if qty.ends_with('E') {
        (&qty[..qty.len() - 1], 1_000_f64.powi(6))
    } else if qty.ends_with('P') {
        (&qty[..qty.len() - 1], 1_000_f64.powi(5))
    } else if qty.ends_with('T') {
        (&qty[..qty.len() - 1], 1_000_f64.powi(4))
    } else if qty.ends_with('G') {
        (&qty[..qty.len() - 1], 1_000_f64.powi(3))
    } else if qty.ends_with('M') {
        (&qty[..qty.len() - 1], 1_000_f64.powi(2))
    } else if qty.ends_with('K') {
        (&qty[..qty.len() - 1], 1_000_f64)
    } else {
        (qty, 1.0_f64)
    };

    num_str
        .parse::<f64>()
        .ok()
        .map(|n| (n * multiplier).round() as i64)
}

fn not_available() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "Kubernetes dashboard is only available in ingress mode" })),
    )
        .into_response()
}

fn gateway_api_disabled() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "Gateway API is disabled (set PERTISK_GATEWAY_API_ENABLED=true or gatewayApi.enabled=true)" })),
    )
        .into_response()
}

pub async fn kubernetes_namespaces(State(state): State<AdminState>) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let api: Api<Namespace> = Api::all(client.clone());
    let list = match api.list(&ListParams::default()).await {
        Ok(l) => l,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let rows: Vec<K8sNamespaceRow> = list
        .items
        .into_iter()
        .map(|n| {
            let created_at = n
                .metadata
                .creation_timestamp
                .as_ref()
                .map(|t| format_k8s_time(&t.0));
            K8sNamespaceRow {
                name: n.name_any(),
                created_at,
            }
        })
        .collect();
    Json(rows).into_response()
}

#[derive(serde::Deserialize)]
pub struct NamespaceQuery {
    pub namespace: Option<String>,
}

pub async fn kubernetes_pods(State(state): State<AdminState>, Query(q): Query<NamespaceQuery>) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let api: Api<Pod> = if let Some(ref ns) = q.namespace {
        Api::namespaced(client.clone(), ns)
    } else {
        Api::all(client.clone())
    };
    let list = match api.list(&ListParams::default()).await {
        Ok(l) => l,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let nodes_api: Api<Node> = Api::all(client.clone());
    let node_status_map: std::collections::HashMap<String, String> = match nodes_api.list(&ListParams::default()).await {
        Ok(nodes) => nodes
            .items
            .into_iter()
            .map(|n| {
                let status = n
                    .status
                    .as_ref()
                    .and_then(|s| s.conditions.as_ref())
                    .and_then(|conds| conds.iter().find(|x| x.type_ == "Ready"))
                    .map(|c| c.status.clone())
                    .unwrap_or_else(|| "Unknown".to_string());
                (n.name_any(), status)
            })
            .collect(),
        Err(_) => std::collections::HashMap::new(),
    };

    let mut pod_usage_map: std::collections::HashMap<(String, String), (i64, i64)> = std::collections::HashMap::new();
    let pod_metrics_gvk = GroupVersionKind::gvk("metrics.k8s.io", "v1beta1", "PodMetrics");
    let mut pod_metrics_ar = ApiResource::from_gvk(&pod_metrics_gvk);
    pod_metrics_ar.plural = "pods".to_string();
    let pod_metrics_api: Api<DynamicObject> = Api::all_with(client.clone(), &pod_metrics_ar);
    if let Ok(metrics_list) = pod_metrics_api.list(&ListParams::default()).await {
        for metric in metrics_list.items {
            let namespace = metric
                .metadata
                .namespace
                .clone()
                .unwrap_or_else(|| "default".to_string());
            let name = metric
                .metadata
                .name
                .clone()
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }

            let mut cpu_total = 0_i64;
            let mut memory_total = 0_i64;
            if let Ok(v) = serde_json::to_value(&metric) {
                if let Some(containers) = v.get("containers").and_then(|c| c.as_array()) {
                    for container in containers {
                        if let Some(cpu_qty) = container
                            .get("usage")
                            .and_then(|u| u.get("cpu"))
                            .and_then(|x| x.as_str())
                        {
                            if let Some(val) = parse_cpu_quantity(cpu_qty) {
                                cpu_total += val;
                            }
                        }
                        if let Some(mem_qty) = container
                            .get("usage")
                            .and_then(|u| u.get("memory"))
                            .and_then(|x| x.as_str())
                        {
                            if let Some(val) = parse_bytes_quantity(mem_qty) {
                                memory_total += val;
                            }
                        }
                    }
                }
            }
            pod_usage_map.insert((namespace, name), (cpu_total, memory_total));
        }
    }

    let rows: Vec<K8sPodRow> = list
        .items
        .into_iter()
        .map(|p| {
            let pod_name = p.name_any();
            let pod_namespace = p.namespace().unwrap_or_else(|| "default".to_string());
            let phase = p.status.as_ref().and_then(|s| s.phase.clone()).unwrap_or_else(|| "Unknown".to_string());
            let node_name = p.spec.as_ref().and_then(|sp| sp.node_name.clone());
            let node_status = node_name
                .as_ref()
                .and_then(|n| node_status_map.get(n).cloned());
            let host_ip = p.status.as_ref().and_then(|s| s.host_ip.clone());
            let node = node_name.clone().or(host_ip.clone());
            let pod_ip = p.status.as_ref().and_then(|s| s.pod_ip.clone());
            let created_at = p
                .metadata
                .creation_timestamp
                .as_ref()
                .map(|t| format_k8s_time(&t.0));
            let (ready, restarts) = p
                .status
                .as_ref()
                .and_then(|s| s.container_statuses.as_ref())
                .map(|s| {
                    let total = s.len();
                    let ready_count = s.iter().filter(|c| c.ready).count();
                    let restarts = s.iter().map(|c| c.restart_count).sum::<i32>();
                    (format!("{}/{}", ready_count, total), restarts.max(0) as u32)
                })
                .unwrap_or_else(|| ("?/0".to_string(), 0));

            let (cpu_request_millicores, memory_request_bytes) = p
                .spec
                .as_ref()
                .map(|sp| {
                    let mut cpu_total = 0_i64;
                    let mut memory_total = 0_i64;
                    for c in &sp.containers {
                        if let Some(resources) = &c.resources {
                            if let Some(requests) = &resources.requests {
                                if let Some(cpu) = requests.get(&String::from("cpu")) {
                                    if let Some(val) = parse_cpu_quantity(&cpu.0) {
                                        cpu_total += val;
                                    }
                                }
                                if let Some(memory) = requests.get(&String::from("memory")) {
                                    if let Some(val) = parse_bytes_quantity(&memory.0) {
                                        memory_total += val;
                                    }
                                }
                            }
                        }
                    }
                    (cpu_total, memory_total)
                })
                .unwrap_or((0, 0));

            let usage = pod_usage_map
                .get(&(pod_namespace.clone(), pod_name.clone()))
                .copied();

            K8sPodRow {
                name: pod_name,
                namespace: pod_namespace,
                phase,
                node,
                node_name,
                node_status,
                pod_ip,
                ready,
                restarts,
                cpu_request_millicores,
                memory_request_bytes,
                cpu_usage_millicores: usage.map(|(cpu, _)| cpu),
                memory_usage_bytes: usage.map(|(_, mem)| mem),
                created_at,
            }
        })
        .collect();
    Json(rows).into_response()
}

pub async fn kubernetes_deployments(State(state): State<AdminState>, Query(q): Query<NamespaceQuery>) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let api: Api<Deployment> = if let Some(ref ns) = q.namespace {
        Api::namespaced(client.clone(), ns)
    } else {
        Api::all(client.clone())
    };
    let list = match api.list(&ListParams::default()).await {
        Ok(l) => l,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let rows: Vec<K8sDeploymentRow> = list
        .items
        .into_iter()
        .map(|d| {
            let (replicas, available) = d
                .status
                .as_ref()
                .map(|st| (st.replicas.unwrap_or(0), st.available_replicas.unwrap_or(0)))
                .unwrap_or((0, 0));
            let ready = format!("{}/{}", available, replicas);
            let created_at = d
                .metadata
                .creation_timestamp
                .as_ref()
                .map(|t| format_k8s_time(&t.0));
            K8sDeploymentRow {
                name: d.name_any(),
                namespace: d.namespace().unwrap_or_else(|| "default".to_string()),
                ready,
                replicas,
                available,
                created_at,
            }
        })
        .collect();
    Json(rows).into_response()
}

pub async fn kubernetes_services(State(state): State<AdminState>, Query(q): Query<NamespaceQuery>) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let api: Api<Service> = if let Some(ref ns) = q.namespace {
        Api::namespaced(client.clone(), ns)
    } else {
        Api::all(client.clone())
    };
    let list = match api.list(&ListParams::default()).await {
        Ok(l) => l,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let rows: Vec<K8sServiceRow> = list
        .items
        .into_iter()
        .map(|svc| {
            let spec = svc.spec.as_ref();
            let typ = spec.and_then(|s| s.type_.clone()).unwrap_or_else(|| "ClusterIP".to_string());
            let cluster_ip = spec.and_then(|s| s.cluster_ip.clone());
            let external_ip = svc
                .status
                .as_ref()
                .and_then(|st| st.load_balancer.as_ref())
                .and_then(|lb| lb.ingress.as_ref())
                .map(|ing| {
                    ing.iter()
                        .filter_map(|entry| {
                            entry
                                .ip
                                .as_ref()
                                .filter(|s| !s.is_empty())
                                .cloned()
                                .or_else(|| {
                                    entry
                                        .hostname
                                        .as_ref()
                                        .filter(|s| !s.is_empty())
                                        .cloned()
                                })
                        })
                        .collect::<Vec<String>>()
                })
                .and_then(|items| if items.is_empty() { None } else { Some(items.join(", ")) });
            let created_at = svc
                .metadata
                .creation_timestamp
                .as_ref()
                .map(|t| format_k8s_time(&t.0));
            let (ports, ports_detail): (Vec<String>, Vec<K8sServicePortDetail>) = spec
                .and_then(|s| s.ports.as_ref())
                .map(|p| {
                    let port_strs: Vec<String> = p
                        .iter()
                        .map(|x| format!("{}/{}", x.port, x.protocol.as_deref().unwrap_or("TCP")))
                        .collect();
                    let details: Vec<K8sServicePortDetail> = p
                        .iter()
                        .map(|x| K8sServicePortDetail {
                            port: x.port,
                            name: x.name.clone(),
                            protocol: x.protocol.as_deref().unwrap_or("TCP").to_string(),
                        })
                        .collect();
                    (port_strs, details)
                })
                .unwrap_or_default();
            K8sServiceRow {
                name: svc.name_any(),
                namespace: svc.namespace().unwrap_or_else(|| "default".to_string()),
                r#type: typ,
                cluster_ip,
                external_ip,
                ports,
                ports_detail,
                created_at,
            }
        })
        .collect();
    Json(rows).into_response()
}

pub async fn kubernetes_tls_secrets(
    State(state): State<AdminState>,
    Query(q): Query<NamespaceQuery>,
) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let api: Api<Secret> = if let Some(ref ns) = q.namespace {
        Api::namespaced(client.clone(), ns)
    } else {
        Api::all(client.clone())
    };
    let list = match api.list(&ListParams::default()).await {
        Ok(l) => l,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let rows: Vec<K8sTlsSecretRow> = list
        .items
        .into_iter()
        .filter(|sec| {
            let typ = sec.type_.as_deref().unwrap_or("");
            let has_tls = sec
                .data
                .as_ref()
                .map_or(false, |d| d.contains_key("tls.crt"));
            typ == "kubernetes.io/tls" || has_tls
        })
        .map(|sec| {
            let (issued_at, expires_at) = sec
                .data
                .as_ref()
                .and_then(|d| d.get("tls.crt"))
                .map(|b| {
                    let validity = cert_validity_from_pem(&b.0);
                    (validity.issued_at, validity.expires_at)
                })
                .unwrap_or((None, None));
            K8sTlsSecretRow {
                namespace: sec.namespace().unwrap_or_else(|| "default".to_string()),
                name: sec.name_any(),
                issued_at,
                expires_at,
            }
        })
        .collect();
    Json(rows).into_response()
}

pub async fn kubernetes_configmaps(
    State(state): State<AdminState>,
    Query(q): Query<NamespaceQuery>,
) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let api: Api<ConfigMap> = if let Some(ref ns) = q.namespace {
        Api::namespaced(client.clone(), ns)
    } else {
        Api::all(client.clone())
    };
    let list = match api.list(&ListParams::default()).await {
        Ok(l) => l,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let rows: Vec<K8sConfigMapRow> = list
        .items
        .into_iter()
        .map(|cm| {
            let created_at = cm
                .metadata
                .creation_timestamp
                .as_ref()
                .map(|t| format_k8s_time(&t.0));
            let mut keys: Vec<String> = cm
                .data
                .as_ref()
                .map(|d| d.keys().cloned().collect())
                .unwrap_or_default();
            keys.sort();
            K8sConfigMapRow {
                namespace: cm.namespace().unwrap_or_else(|| "default".to_string()),
                name: cm.name_any(),
                data_keys: keys,
                created_at,
            }
        })
        .collect();
    Json(rows).into_response()
}

pub async fn kubernetes_secrets(
    State(state): State<AdminState>,
    Query(q): Query<NamespaceQuery>,
) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let api: Api<Secret> = if let Some(ref ns) = q.namespace {
        Api::namespaced(client.clone(), ns)
    } else {
        Api::all(client.clone())
    };
    let list = match api.list(&ListParams::default()).await {
        Ok(l) => l,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let rows: Vec<K8sSecretRow> = list
        .items
        .into_iter()
        .map(|sec| {
            let created_at = sec
                .metadata
                .creation_timestamp
                .as_ref()
                .map(|t| format_k8s_time(&t.0));
            let tls_expires_at = sec
                .data
                .as_ref()
                .and_then(|d| {
                    if d.contains_key("tls.crt") && d.contains_key("tls.key") {
                        d.get("tls.crt")
                    } else {
                        None
                    }
                })
                .and_then(|b| cert_validity_from_pem(&b.0).expires_at);
            let mut keys: Vec<String> = sec
                .data
                .as_ref()
                .map(|d| d.keys().cloned().collect())
                .unwrap_or_default();
            keys.sort();
            K8sSecretRow {
                namespace: sec.namespace().unwrap_or_else(|| "default".to_string()),
                name: sec.name_any(),
                r#type: sec.type_.clone().unwrap_or_else(|| "Opaque".to_string()),
                data_keys: keys,
                created_at,
                tls_expires_at,
            }
        })
        .collect();
    Json(rows).into_response()
}

/// Response for GET one Ingress (edit form data).
#[derive(Clone, Serialize)]
pub struct IngressFormRouteRow {
    pub path: String,
    pub path_type: String,
    pub service_name: String,
    pub service_port: Option<i32>,
    pub service_port_name: Option<String>,
}

#[derive(Serialize)]
pub struct IngressFormRow {
    pub namespace: String,
    pub name: String,
    pub host: String,
    pub routes: Vec<IngressFormRouteRow>,
    pub path: String,
    pub path_type: String,
    pub tls_secret_name: Option<String>,
    pub service_name: String,
    pub service_port: Option<i32>,
    pub service_port_name: Option<String>,
    pub ingress_class_name: Option<String>,
    /// Parent Gateway name when this row is an HTTPRoute (Gateway API site).
    pub gateway_name: Option<String>,
    pub gateway_namespace: Option<String>,
    #[serde(skip_serializing_if = "crate::geoip::GeoIpPolicy::is_default")]
    pub geoip: crate::geoip::GeoIpPolicy,
    #[serde(skip_serializing_if = "crate::security::SecurityPolicy::is_default")]
    pub security: crate::security::SecurityPolicy,
}

#[derive(Debug, Deserialize)]
pub struct CreateIngressRouteBody {
    pub path: Option<String>,
    pub path_type: Option<String>,
    pub service_name: String,
    pub service_port: Option<i32>,
    pub service_port_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateIngressBody {
    pub name: Option<String>,
    pub host: String,
    pub routes: Option<Vec<CreateIngressRouteBody>>,
    pub path: Option<String>,
    pub path_type: Option<String>,
    pub tls_secret_namespace: Option<String>,
    pub tls_secret_name: Option<String>,
    pub service_namespace: String,
    pub service_name: String,
    /// Port number (preferred) or leave unset to use port name in service_port_name.
    pub service_port: Option<i32>,
    pub service_port_name: Option<String>,
    pub ingress_namespace: Option<String>,
    pub ingress_class_name: Option<String>,
    /// Existing Gateway to attach the HTTPRoute to (Gateway API sites).
    pub gateway_name: Option<String>,
    pub gateway_namespace: Option<String>,
    #[serde(default)]
    pub geoip: crate::geoip::GeoIpPolicy,
    #[serde(default)]
    pub security: crate::security::SecurityPolicy,
}

#[derive(Serialize)]
pub struct GatewayListenerRow {
    pub name: String,
    pub protocol: String,
    pub port: i32,
    pub hostname: Option<String>,
}

#[derive(Serialize)]
pub struct K8sGatewayRow {
    pub name: String,
    pub namespace: String,
    pub gateway_class: Option<String>,
    pub host: Option<String>,
    pub tls_secret_name: Option<String>,
    pub listeners: Vec<GatewayListenerRow>,
    pub created_at: Option<String>,
}

#[derive(Serialize)]
pub struct K8sHttpRouteRow {
    pub name: String,
    pub namespace: String,
    pub host: Option<String>,
    pub gateway_namespace: Option<String>,
    pub gateway_name: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Serialize)]
pub struct GatewayFormRow {
    pub namespace: String,
    pub name: String,
    pub host: String,
    pub gateway_class_name: Option<String>,
    pub tls_secret_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateGatewayBody {
    pub name: Option<String>,
    pub host: String,
    pub gateway_namespace: Option<String>,
    pub tls_secret_namespace: Option<String>,
    pub tls_secret_name: Option<String>,
    pub gateway_class_name: Option<String>,
}

fn gateway_tls_secret(gw: &Gateway) -> Option<String> {
    gw.spec
        .listeners
        .iter()
        .find(|l| l.protocol.eq_ignore_ascii_case("https"))
        .and_then(|l| l.tls.as_ref())
        .and_then(|tls| tls.certificate_refs.first())
        .map(|cert| cert.name.clone())
}

fn gateway_host(gw: &Gateway) -> Option<String> {
    gw.spec
        .listeners
        .iter()
        .find(|l| l.protocol.eq_ignore_ascii_case("https"))
        .and_then(|l| l.hostname.clone())
}

fn gateway_managed_by_app(gw: &Gateway, want_class: Option<&str>) -> bool {
    match want_class {
        Some(want) => matches!(
            gw.spec.gateway_class_name.as_deref(),
            Some(class) if class == want
        ),
        None => gw.spec.gateway_class_name.is_some(),
    }
}

fn managed_gateway_keys(gateways: &[Gateway], want_class: Option<&str>) -> HashSet<(String, String)> {
    gateways
        .iter()
        .filter(|gw| gateway_managed_by_app(gw, want_class))
        .map(|gw| {
            (
                gw.namespace().unwrap_or_else(|| "default".to_string()),
                gw.name_any(),
            )
        })
        .collect()
}

fn httproute_attached_to_managed(
    route: &HTTPRoute,
    managed: &HashSet<(String, String)>,
) -> bool {
    let route_ns = route.namespace().unwrap_or_else(|| "default".to_string());
    let route_name = route.name_any();
    if route.spec.parent_refs.is_empty() {
        return managed.contains(&(route_ns.clone(), route_name));
    }
    route.spec.parent_refs.iter().any(|parent| {
        let gw_name = &parent.name;
        let gw_ns = parent
            .namespace
            .as_deref()
            .unwrap_or(route_ns.as_str());
        managed.contains(&(gw_ns.to_string(), gw_name.clone()))
    })
}

fn httproute_row_from(route: &HTTPRoute) -> K8sHttpRouteRow {
    let host = route.spec.hostnames.first().cloned();
    let parent = route.spec.parent_refs.first();
    let gateway_name = parent.map(|p| p.name.clone());
    let gateway_namespace = parent
        .and_then(|p| p.namespace.clone())
        .or_else(|| route.namespace());
    let created_at = route
        .metadata
        .creation_timestamp
        .as_ref()
        .map(|t| format_k8s_time(&t.0));
    K8sHttpRouteRow {
        name: route.name_any(),
        namespace: route.namespace().unwrap_or_else(|| "default".to_string()),
        host,
        gateway_namespace,
        gateway_name,
        created_at,
    }
}

async fn load_all_gateways(
    client: &kube::Client,
    namespace: Option<&str>,
) -> Result<Vec<Gateway>, String> {
    let api: Api<Gateway> = if let Some(ns) = namespace {
        Api::namespaced(client.clone(), ns)
    } else {
        Api::all(client.clone())
    };
    api.list(&ListParams::default())
        .await
        .map(|list| list.items)
        .map_err(|e| e.to_string())
}

fn gateway_not_managed_response() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "error": "Gateway is not managed by this controller (check gateway class / PERTISK_GATEWAY_CLASS)"
        })),
    )
        .into_response()
}

fn httproute_not_managed_response() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "error": "HTTPRoute is not attached to a Gateway managed by this controller"
        })),
    )
        .into_response()
}

fn ensure_gateway_managed(state: &AdminState, gw: &Gateway) -> Result<(), Response> {
    if gateway_managed_by_app(gw, state.gateway_class.as_deref()) {
        Ok(())
    } else {
        Err(gateway_not_managed_response())
    }
}

async fn ensure_httproute_managed(
    state: &AdminState,
    client: &kube::Client,
    route: &HTTPRoute,
) -> Result<(), Response> {
    let gateways = match load_all_gateways(client, None).await {
        Ok(gws) => gws,
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response());
        }
    };
    let managed = managed_gateway_keys(&gateways, state.gateway_class.as_deref());
    if httproute_attached_to_managed(route, &managed) {
        Ok(())
    } else {
        Err(httproute_not_managed_response())
    }
}

fn gateway_listener_rows(gw: &Gateway) -> Vec<GatewayListenerRow> {
    gw.spec
        .listeners
        .iter()
        .map(|l| GatewayListenerRow {
            name: l.name.clone(),
            protocol: l.protocol.clone(),
            port: l.port,
            hostname: l.hostname.clone(),
        })
        .collect()
}

fn gateway_row_from(gw: Gateway) -> K8sGatewayRow {
    let created_at = gw
        .metadata
        .creation_timestamp
        .as_ref()
        .map(|t| format_k8s_time(&t.0));
    K8sGatewayRow {
        name: gw.name_any(),
        namespace: gw.namespace().unwrap_or_else(|| "default".to_string()),
        gateway_class: gw.spec.gateway_class_name.clone(),
        host: gateway_host(&gw),
        tls_secret_name: gateway_tls_secret(&gw),
        listeners: gateway_listener_rows(&gw),
        created_at,
    }
}

fn resolve_gateway_namespace(body: &CreateGatewayBody) -> Result<String, Response> {
    let ns = body
        .gateway_namespace
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            body.tls_secret_namespace
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
        })
        .unwrap_or("default");
    Ok(ns.to_string())
}

fn resolve_gateway_site_namespace(body: &CreateIngressBody) -> String {
    body.gateway_namespace
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            body.ingress_namespace
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            let ns = body.service_namespace.trim();
            if ns.is_empty() {
                None
            } else {
                Some(ns)
            }
        })
        .unwrap_or("default")
        .to_string()
}

fn backend_port(number: Option<i32>, name: Option<&str>) -> Option<ServiceBackendPort> {
    if let Some(number) = number {
        Some(ServiceBackendPort {
            number: Some(number),
            name: None,
        })
    } else {
        name.map(|name| ServiceBackendPort {
            number: None,
            name: Some(name.to_string()),
        })
    }
}

fn merge_security_annotations(
    existing: Option<std::collections::BTreeMap<String, String>>,
    geoip: &crate::geoip::GeoIpPolicy,
    security: &crate::security::SecurityPolicy,
) -> Option<std::collections::BTreeMap<String, String>> {
    let mut annotations = existing.unwrap_or_default();
    crate::geoip::apply_annotations(&mut annotations, geoip);
    crate::security::apply_annotations(&mut annotations, security);
    if annotations.is_empty() {
        None
    } else {
        Some(annotations)
    }
}

fn ingress_paths_from_body(body: &CreateIngressBody) -> Result<Vec<HTTPIngressPath>, String> {
    if let Some(routes) = body.routes.as_ref().filter(|routes| !routes.is_empty()) {
        let mut paths = Vec::with_capacity(routes.len());
        for route in routes {
            let service_name = route.service_name.trim();
            if service_name.is_empty() {
                return Err("each route requires service_name".to_string());
            }
            let service_port_name = route
                .service_port_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let service_port = backend_port(route.service_port, service_port_name)
                .ok_or_else(|| "each route requires service_port or service_port_name".to_string())?;
            let path = route.path.as_deref().unwrap_or("/").trim();
            let path_type = route
                .path_type
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("Prefix");
            paths.push(HTTPIngressPath {
                path: Some(if path.is_empty() { "/".to_string() } else { path.to_string() }),
                path_type: path_type.to_string(),
                backend: IngressBackend {
                    service: Some(IngressServiceBackend {
                        name: service_name.to_string(),
                        port: Some(service_port),
                    }),
                    resource: None,
                },
            });
        }
        return Ok(paths);
    }

    let service_name = body.service_name.trim();
    if service_name.is_empty() {
        return Err("service_name is required".to_string());
    }
    let service_port_name = body
        .service_port_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let service_port = backend_port(body.service_port, service_port_name)
        .ok_or_else(|| "service_port or service_port_name is required".to_string())?;
    let path = body.path.as_deref().unwrap_or("/").trim();
    let path_type = body
        .path_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Prefix");
    Ok(vec![HTTPIngressPath {
        path: Some(if path.is_empty() { "/".to_string() } else { path.to_string() }),
        path_type: path_type.to_string(),
        backend: IngressBackend {
            service: Some(IngressServiceBackend {
                name: service_name.to_string(),
                port: Some(service_port),
            }),
            resource: None,
        },
    }])
}

pub async fn kubernetes_ingresses_create(
    State(state): State<AdminState>,
    _headers: axum::http::HeaderMap,
    Json(body): Json<CreateIngressBody>,
) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let host = body.host.trim();
    if host.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "host is required" })),
        )
            .into_response();
    }
    let service_namespace = body.service_namespace.trim();
    if service_namespace.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "service_namespace is required" })),
        )
            .into_response();
    }
    let ingress_ns = body
        .ingress_namespace
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(service_namespace);
    let ingress_name = body
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| host.replace('.', "-").replace('*', "wildcard"));
    let ingress_class = body
        .ingress_class_name
        .clone()
        .or_else(|| state.ingress_class.clone());
    let paths = match ingress_paths_from_body(&body) {
        Ok(paths) => paths,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": error })),
            )
                .into_response();
        }
    };
    let rule = IngressRule {
        host: Some(host.to_string()),
        http: Some(HTTPIngressRuleValue {
            paths,
        }),
    };
    let mut spec = IngressSpec {
        default_backend: None,
        ingress_class_name: ingress_class,
        rules: Some(vec![rule]),
        tls: None,
    };
    if let (Some(ns), Some(name)) = (
        body.tls_secret_namespace.as_deref().map(str::trim).filter(|s| !s.is_empty()),
        body.tls_secret_name.as_deref().map(str::trim).filter(|s| !s.is_empty()),
    ) {
        if ns != ingress_ns {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "TLS secret must be in the same namespace as the Ingress (service namespace)"
                })),
            )
                .into_response();
        }
        spec.tls = Some(vec![IngressTLS {
            hosts: Some(vec![host.to_string()]),
            secret_name: Some(name.to_string()),
        }]);
    }

    let ingress = Ingress {
        metadata: kube::core::ObjectMeta {
            name: Some(ingress_name.clone()),
            namespace: Some(ingress_ns.to_string()),
            annotations: merge_security_annotations(None, &body.geoip, &body.security),
            ..Default::default()
        },
        spec: Some(spec),
        status: None,
    };

    let api: Api<Ingress> = Api::namespaced(client.clone(), ingress_ns);
    match api.create(&PostParams::default(), &ingress).await {
        Ok(_) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "message": "Ingress created",
                "name": ingress_name,
                "namespace": ingress_ns
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn kubernetes_ingress_get(
    State(state): State<AdminState>,
    AxumPath((namespace, name)): AxumPath<(String, String)>,
    _headers: axum::http::HeaderMap,
) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let api: Api<Ingress> = Api::namespaced(client.clone(), &namespace);
    let ingress = match api.get(&name).await {
        Ok(i) => i,
        Err(e) => {
            let err_str = e.to_string();
            let code = if err_str.contains("404") || err_str.to_lowercase().contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            return (
                code,
                Json(serde_json::json!({ "error": err_str })),
            )
                .into_response();
        }
    };
    let spec = match ingress.spec.as_ref() {
        Some(sp) => sp,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Ingress has no spec" })),
            )
                .into_response();
        }
    };
    let (host, routes, tls_secret) = match spec.rules.as_deref() {
        Some(rules) if !rules.is_empty() => {
            let rule = &rules[0];
            let host = rule.host.as_deref().unwrap_or("*").to_string();
            let routes = rule
                .http
                .as_ref()
                .map(|http| {
                    http.paths
                        .iter()
                        .map(|path| {
                            let (service_name, service_port, service_port_name) = match path.backend.service.as_ref() {
                                Some(service) => (
                                    service.name.clone(),
                                    service.port.as_ref().and_then(|port| port.number),
                                    service.port.as_ref().and_then(|port| port.name.clone()),
                                ),
                                None => (String::new(), None, None),
                            };
                            IngressFormRouteRow {
                                path: path.path.as_deref().unwrap_or("/").to_string(),
                                path_type: path.path_type.clone(),
                                service_name,
                                service_port,
                                service_port_name,
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .filter(|paths| !paths.is_empty())
                .unwrap_or_else(|| {
                    vec![IngressFormRouteRow {
                        path: "/".to_string(),
                        path_type: "Prefix".to_string(),
                        service_name: String::new(),
                        service_port: None,
                        service_port_name: None,
                    }]
                });
            let tls_secret = spec
                .tls
                .as_ref()
                .and_then(|t| t.first())
                .and_then(|t| t.secret_name.clone());
            (host, routes, tls_secret)
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Ingress has no rules" })),
            )
                .into_response();
        }
    };
    let first_route = routes.first();
    let first_path = first_route
        .map(|route| route.path.clone())
        .unwrap_or_else(|| "/".to_string());
    let first_path_type = first_route
        .map(|route| route.path_type.clone())
        .unwrap_or_else(|| "Prefix".to_string());
    let first_service_name = first_route
        .map(|route| route.service_name.clone())
        .unwrap_or_default();
    let first_service_port = first_route.and_then(|route| route.service_port);
    let first_service_port_name = first_route.and_then(|route| route.service_port_name.clone());
    let annotations = ingress.metadata.annotations.as_ref();
    let row = IngressFormRow {
        namespace: namespace.clone(),
        name: name.clone(),
        host,
        routes,
        path: first_path,
        path_type: first_path_type,
        tls_secret_name: tls_secret,
        service_name: first_service_name,
        service_port: first_service_port,
        service_port_name: first_service_port_name,
        ingress_class_name: spec.ingress_class_name.clone(),
        gateway_name: None,
        gateway_namespace: None,
        geoip: crate::geoip::policy_from_annotations(annotations),
        security: crate::security::policy_from_annotations(annotations),
    };
    Json(row).into_response()
}

pub async fn kubernetes_ingress_update(
    State(state): State<AdminState>,
    AxumPath((namespace, name)): AxumPath<(String, String)>,
    _headers: axum::http::HeaderMap,
    Json(body): Json<CreateIngressBody>,
) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let host = body.host.trim();
    if host.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "host is required" })),
        )
            .into_response();
    }
    let service_namespace = body.service_namespace.trim();
    if service_namespace.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "service_namespace is required" })),
        )
            .into_response();
    }
    let ingress_ns = namespace;
    let ingress_class = body
        .ingress_class_name
        .clone()
        .or_else(|| state.ingress_class.clone());
    let paths = match ingress_paths_from_body(&body) {
        Ok(paths) => paths,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": error })),
            )
                .into_response();
        }
    };
    let rule = IngressRule {
        host: Some(host.to_string()),
        http: Some(HTTPIngressRuleValue {
            paths,
        }),
    };
    let mut spec = IngressSpec {
        default_backend: None,
        ingress_class_name: ingress_class,
        rules: Some(vec![rule]),
        tls: None,
    };
    if let (Some(ns), Some(tls_name)) = (
        body.tls_secret_namespace.as_deref().map(str::trim).filter(|s| !s.is_empty()),
        body.tls_secret_name.as_deref().map(str::trim).filter(|s| !s.is_empty()),
    ) {
        if ns != ingress_ns {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "TLS secret must be in the same namespace as the Ingress"
                })),
            )
                .into_response();
        }
        spec.tls = Some(vec![IngressTLS {
            hosts: Some(vec![host.to_string()]),
            secret_name: Some(tls_name.to_string()),
        }]);
    }

    let api: Api<Ingress> = Api::namespaced(client.clone(), &ingress_ns);
    let mut current = match api.get(&name).await {
        Ok(i) => i,
        Err(e) => {
            let err_str = e.to_string();
            let code = if err_str.contains("404") || err_str.to_lowercase().contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            return (
                code,
                Json(serde_json::json!({ "error": err_str })),
            )
                .into_response();
        }
    };
    current.spec = Some(spec);
    current.metadata.annotations = merge_security_annotations(
        current.metadata.annotations.clone(),
        &body.geoip,
        &body.security,
    );
    match api.replace(&name, &PostParams::default(), &current).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "message": "Ingress updated",
                "name": name,
                "namespace": ingress_ns
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn kubernetes_ingress_delete(
    State(state): State<AdminState>,
    AxumPath((namespace, name)): AxumPath<(String, String)>,
    _headers: axum::http::HeaderMap,
) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let api: Api<Ingress> = Api::namespaced(client.clone(), &namespace);
    match api.delete(&name, &DeleteParams::default()).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "message": "Ingress deleted",
                "name": name,
                "namespace": namespace
            })),
        )
            .into_response(),
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("404") || err_str.to_lowercase().contains("not found") {
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "message": "Ingress deleted",
                        "name": name,
                        "namespace": namespace
                    })),
                )
                    .into_response()
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": err_str })),
                )
                    .into_response()
            }
        }
    }
}

pub async fn kubernetes_ingresses(State(state): State<AdminState>, Query(q): Query<NamespaceQuery>) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let api: Api<Ingress> = if let Some(ref ns) = q.namespace {
        Api::namespaced(client.clone(), ns)
    } else {
        Api::all(client.clone())
    };
    let list = match api.list(&ListParams::default()).await {
        Ok(l) => l,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let rows: Vec<K8sIngressRow> = list
        .items
        .into_iter()
        .map(|i| {
            let spec = i.spec.as_ref();
            let class = spec.and_then(|s| s.ingress_class_name.clone());
            let created_at = i
                .metadata
                .creation_timestamp
                .as_ref()
                .map(|t| format_k8s_time(&t.0));
            let hosts: Vec<String> = spec
                .and_then(|s| s.rules.as_ref())
                .map(|r| r.iter().filter_map(|x| x.host.clone()).collect())
                .unwrap_or_default();
            K8sIngressRow {
                name: i.name_any(),
                namespace: i.namespace().unwrap_or_else(|| "default".to_string()),
                class,
                hosts,
                created_at,
            }
        })
        .collect();
    Json(rows).into_response()
}

pub async fn kubernetes_nodes(State(state): State<AdminState>) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let api: Api<Node> = Api::all(client.clone());
    let list = match api.list(&ListParams::default()).await {
        Ok(l) => l,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let rows: Vec<K8sNodeRow> = list
        .items
        .into_iter()
        .map(|n| {
            let ready = n
                .status
                .as_ref()
                .and_then(|s| s.conditions.as_ref())
                .and_then(|c| c.iter().find(|x| x.type_ == "Ready"))
                .map(|r| r.status.clone())
                .unwrap_or_else(|| "Unknown".to_string());
            let created_at = n
                .metadata
                .creation_timestamp
                .as_ref()
                .map(|t| format_k8s_time(&t.0));
            let (capacity_cpu, capacity_memory) = n
                .status
                .as_ref()
                .and_then(|s| s.capacity.as_ref())
                .map(|cap| {
                    let cpu = cap.get("cpu").map(|v| v.0.clone());
                    let mem = cap.get("memory").map(|v| v.0.clone());
                    (cpu, mem)
                })
                .unwrap_or((None, None));
            K8sNodeRow {
                name: n.name_any(),
                ready,
                capacity_cpu,
                capacity_memory,
                created_at,
            }
        })
        .collect();
    Json(rows).into_response()
}

pub async fn kubernetes_events(State(state): State<AdminState>) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let api: Api<Event> = Api::all(client.clone());
    let list = match api.list(&ListParams::default()).await {
        Ok(l) => l,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let mut rows: Vec<K8sEventRow> = list
        .items
        .into_iter()
        .map(|e| {
            let created_at = e
                .last_timestamp
                .as_ref()
                .map(|t| format_k8s_time(&t.0))
                .or_else(|| e.first_timestamp.as_ref().map(|t| format_k8s_time(&t.0)))
                .or_else(|| e.metadata.creation_timestamp.as_ref().map(|t| format_k8s_time(&t.0)));

            K8sEventRow {
                namespace: e.namespace().unwrap_or_else(|| "default".to_string()),
                name: e.name_any(),
                r#type: e.type_.clone(),
                reason: e.reason.clone(),
                message: e.message.clone(),
                involved_kind: e.involved_object.kind.clone(),
                involved_name: e.involved_object.name.clone(),
                created_at,
            }
        })
        .collect();

    rows.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    rows.truncate(20);
    Json(rows).into_response()
}

pub async fn kubernetes_cluster_summary(State(state): State<AdminState>) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };

    let mut metrics = K8sClusterSummary::default();

    // Get all nodes
    let nodes_api: Api<Node> = Api::all(client.clone());
    let nodes = match nodes_api.list(&ListParams::default()).await {
        Ok(n) => n,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    metrics.node_count = nodes.items.len() as i32;

    // Aggregate node capacity and allocatable
    for node in &nodes.items {
        if let Some(status) = &node.status {
            // Capacity
            if let Some(capacity) = &status.capacity {
                if let Some(cpu) = capacity.get("cpu") {
                    if let Some(val) = parse_cpu_quantity(&cpu.0) {
                        metrics.cpu_capacity += val;
                    }
                }
                if let Some(memory) = capacity.get("memory") {
                    if let Some(val) = parse_bytes_quantity(&memory.0) {
                        metrics.memory_capacity += val;
                    }
                }
                if let Some(storage) = capacity.get("ephemeral-storage") {
                    if let Some(val) = parse_bytes_quantity(&storage.0) {
                        metrics.storage_capacity += val;
                    }
                }
                if let Some(pods) = capacity.get("pods") {
                    if let Ok(val) = pods.0.parse::<i64>() {
                        metrics.pod_capacity += val;
                    }
                }
            }
            
            // Allocatable
            if let Some(allocatable) = &status.allocatable {
                if let Some(cpu) = allocatable.get("cpu") {
                    if let Some(val) = parse_cpu_quantity(&cpu.0) {
                        metrics.cpu_allocatable += val;
                    }
                }
                if let Some(memory) = allocatable.get("memory") {
                    if let Some(val) = parse_bytes_quantity(&memory.0) {
                        metrics.memory_allocatable += val;
                    }
                }
                if let Some(storage) = allocatable.get("ephemeral-storage") {
                    if let Some(val) = parse_bytes_quantity(&storage.0) {
                        metrics.storage_allocatable += val;
                    }
                }
                if let Some(pods) = allocatable.get("pods") {
                    if let Ok(val) = pods.0.parse::<i64>() {
                        metrics.pod_allocatable += val;
                    }
                }
            }
        }
    }

    // List all pods across all namespaces
    let pods_api: Api<Pod> = Api::all(client.clone());
    let pods = match pods_api.list(&ListParams::default()).await {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    metrics.pod_count = pods.items.len() as i32;

    // Aggregate pod requests and limits (skip pods not Running or Pending)
    for pod in &pods.items {
        let phase = pod.status.as_ref()
            .and_then(|s| s.phase.as_ref())
            .map(|p| p.as_str())
            .unwrap_or("Unknown");
        
        if phase != "Running" && phase != "Pending" {
            continue;
        }

        if let Some(spec) = &pod.spec {
            for container in &spec.containers {
                if let Some(resources) = &container.resources {
                    if let Some(requests) = &resources.requests {
                        if let Some(cpu) = requests.get(&String::from("cpu")) {
                            if let Some(val) = parse_cpu_quantity(&cpu.0) {
                                metrics.cpu_requests += val;
                            }
                        }
                        if let Some(memory) = requests.get(&String::from("memory")) {
                            if let Some(val) = parse_bytes_quantity(&memory.0) {
                                metrics.memory_requests += val;
                            }
                        }
                        if let Some(storage) = requests.get(&String::from("ephemeral-storage")) {
                            if let Some(val) = parse_bytes_quantity(&storage.0) {
                                metrics.storage_requests += val;
                            }
                        }
                    }

                    if let Some(limits) = &resources.limits {
                        if let Some(cpu) = limits.get(&String::from("cpu")) {
                            if let Some(val) = parse_cpu_quantity(&cpu.0) {
                                metrics.cpu_limits += val;
                            }
                        }
                        if let Some(memory) = limits.get(&String::from("memory")) {
                            if let Some(val) = parse_bytes_quantity(&memory.0) {
                                metrics.memory_limits += val;
                            }
                        }
                    }
                }
            }
        }
    }

    // Calculate percentages based on allocatable resources
    if metrics.cpu_allocatable > 0 {
        metrics.cpu_requests_percent = (metrics.cpu_requests as f64 / metrics.cpu_allocatable as f64) * 100.0;
        metrics.cpu_limits_percent = (metrics.cpu_limits as f64 / metrics.cpu_allocatable as f64) * 100.0;
    }
    
    if metrics.memory_allocatable > 0 {
        metrics.memory_requests_percent = (metrics.memory_requests as f64 / metrics.memory_allocatable as f64) * 100.0;
        metrics.memory_limits_percent = (metrics.memory_limits as f64 / metrics.memory_allocatable as f64) * 100.0;
    }
    
    if metrics.storage_allocatable > 0 {
        metrics.storage_requests_percent = (metrics.storage_requests as f64 / metrics.storage_allocatable as f64) * 100.0;
    }

    Json(metrics).into_response()
}

fn host_to_k8s_name(host: &str) -> String {
    host.replace('.', "-").replace('*', "wildcard")
}

fn gateway_path_match_type(path_type: &str) -> &'static str {
    match path_type.trim() {
        "Exact" => "Exact",
        _ => "PathPrefix",
    }
}

fn httproute_rules_from_body(
    body: &CreateIngressBody,
    service_namespace: &str,
) -> Result<Vec<HTTPRouteRule>, String> {
    let build_rule = |path: &str, path_type: &str, service_name: &str, service_port: i32| HTTPRouteRule {
        matches: vec![HTTPRouteMatch {
            path: Some(HTTPPathMatch {
                match_type: Some(gateway_path_match_type(path_type).to_string()),
                value: Some(if path.is_empty() { "/".to_string() } else { path.to_string() }),
            }),
        }],
        backend_refs: vec![HTTPBackendRef {
            group: None,
            kind: Some("Service".to_string()),
            name: service_name.to_string(),
            namespace: Some(service_namespace.to_string()),
            port: Some(service_port),
        }],
    };

    if let Some(routes) = body.routes.as_ref().filter(|routes| !routes.is_empty()) {
        let mut rules = Vec::with_capacity(routes.len());
        for route in routes {
            let service_name = route.service_name.trim();
            if service_name.is_empty() {
                return Err("each route requires service_name".to_string());
            }
            let service_port = route
                .service_port
                .ok_or_else(|| "each route requires service_port or service_port_name".to_string())?;
            let path = route.path.as_deref().unwrap_or("/").trim();
            let path_type = route
                .path_type
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("Prefix");
            rules.push(build_rule(path, path_type, service_name, service_port));
        }
        return Ok(rules);
    }

    let service_name = body.service_name.trim();
    if service_name.is_empty() {
        return Err("service_name is required".to_string());
    }
    let service_port = body
        .service_port
        .ok_or_else(|| "service_port or service_port_name is required".to_string())?;
    let path = body.path.as_deref().unwrap_or("/").trim();
    let path_type = body
        .path_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Prefix");
    Ok(vec![build_rule(path, path_type, service_name, service_port)])
}

fn standard_allowed_routes_same_namespace() -> AllowedRoutes {
    AllowedRoutes {
        namespaces: Some(RouteNamespaces {
            from: Some("Same".to_string()),
        }),
    }
}

fn gateway_listener_tls(secret_name: &str) -> GatewayListenerTls {
    GatewayListenerTls {
        mode: Some("Terminate".to_string()),
        certificate_refs: vec![SecretObjectReference {
            group: None,
            kind: Some("Secret".to_string()),
            name: secret_name.to_string(),
            namespace: None,
        }],
    }
}

fn build_https_listener(host: &str, tls_secret_name: Option<&str>) -> GatewayListener {
    GatewayListener {
        name: "https".to_string(),
        port: 443,
        protocol: "HTTPS".to_string(),
        hostname: Some(host.to_string()),
        allowed_routes: Some(standard_allowed_routes_same_namespace()),
        tls: tls_secret_name.map(gateway_listener_tls),
    }
}

fn merge_https_listener(old: &GatewayListener, host: &str, tls_secret_name: Option<&str>) -> GatewayListener {
    let tls = match tls_secret_name {
        Some(secret_name) => {
            let mode = old
                .tls
                .as_ref()
                .and_then(|t| t.mode.clone())
                .or_else(|| Some("Terminate".to_string()));
            Some(GatewayListenerTls {
                mode,
                certificate_refs: vec![SecretObjectReference {
                    group: None,
                    kind: Some("Secret".to_string()),
                    name: secret_name.to_string(),
                    namespace: None,
                }],
            })
        }
        None => None,
    };
    GatewayListener {
        name: old.name.clone(),
        port: old.port,
        protocol: old.protocol.clone(),
        hostname: Some(host.to_string()),
        allowed_routes: old
            .allowed_routes
            .clone()
            .or_else(|| Some(standard_allowed_routes_same_namespace())),
        tls,
    }
}

fn build_gateway_spec(
    host: &str,
    gateway_class: Option<String>,
    tls_secret_name: Option<&str>,
) -> GatewaySpec {
    GatewaySpec {
        gateway_class_name: gateway_class,
        listeners: vec![build_https_listener(host, tls_secret_name)],
    }
}

fn apply_gateway_form_to_spec(
    existing: Option<&GatewaySpec>,
    host: &str,
    gateway_class: Option<String>,
    tls_secret_name: Option<&str>,
) -> GatewaySpec {
    let Some(existing) = existing else {
        return build_gateway_spec(host, gateway_class, tls_secret_name);
    };
    let class = gateway_class.or_else(|| existing.gateway_class_name.clone());
    let mut listeners = existing.listeners.clone();
    if let Some(idx) = listeners.iter().position(|l| {
        l.name == "https" || l.protocol.eq_ignore_ascii_case("https")
    }) {
        listeners[idx] = merge_https_listener(&listeners[idx], host, tls_secret_name);
    } else {
        listeners.push(build_https_listener(host, tls_secret_name));
    }
    GatewaySpec {
        gateway_class_name: class,
        listeners,
    }
}

fn build_httproute_spec(
    host: &str,
    gateway_name: &str,
    namespace: &str,
    rules: Vec<HTTPRouteRule>,
) -> HTTPRouteSpec {
    HTTPRouteSpec {
        parent_refs: vec![ParentReference {
            group: Some("gateway.networking.k8s.io".to_string()),
            kind: Some("Gateway".to_string()),
            name: gateway_name.to_string(),
            namespace: Some(namespace.to_string()),
            section_name: Some("https".to_string()),
        }],
        hostnames: vec![host.to_string()],
        rules,
    }
}

fn ingress_form_routes_from_httproute(route: &HTTPRoute) -> Vec<IngressFormRouteRow> {
    if route.spec.rules.is_empty() {
        return vec![IngressFormRouteRow {
            path: "/".to_string(),
            path_type: "Prefix".to_string(),
            service_name: String::new(),
            service_port: None,
            service_port_name: None,
        }];
    }
    route.spec.rules
        .iter()
        .map(|rule| {
            let path_match = rule.matches.first().and_then(|m| m.path.as_ref());
            let path = path_match
                .and_then(|p| p.value.clone())
                .unwrap_or_else(|| "/".to_string());
            let path_type = match path_match.and_then(|p| p.match_type.as_deref()) {
                Some("Exact") => "Exact".to_string(),
                _ => "Prefix".to_string(),
            };
            let backend = rule.backend_refs.first();
            IngressFormRouteRow {
                path,
                path_type,
                service_name: backend.map(|b| b.name.clone()).unwrap_or_default(),
                service_port: backend.and_then(|b| b.port),
                service_port_name: None,
            }
        })
        .collect()
}

pub async fn kubernetes_gateway_sites_create(
    State(state): State<AdminState>,
    _headers: axum::http::HeaderMap,
    Json(body): Json<CreateIngressBody>,
) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    if !state.gateway_api_enabled {
        return gateway_api_disabled();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let host = body.host.trim();
    if host.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "host is required" })),
        )
            .into_response();
    }
    let service_namespace = body.service_namespace.trim();
    if service_namespace.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "service_namespace is required" })),
        )
            .into_response();
    }
    let gateway_name = match body
        .gateway_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(name) => name.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "gateway_name is required" })),
            )
                .into_response();
        }
    };
    let ns = resolve_gateway_site_namespace(&body);
    let gw_api: Api<Gateway> = Api::namespaced(client.clone(), &ns);
    let gateway = match gw_api.get(&gateway_name).await {
        Ok(gw) => gw,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("Gateway \"{}\" not found in namespace \"{}\"", gateway_name, ns)
                })),
            )
                .into_response();
        }
    };
    if let Err(resp) = ensure_gateway_managed(&state, &gateway) {
        return resp;
    }
    let resource_name = body
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| host_to_k8s_name(host));
    let rules = match httproute_rules_from_body(&body, service_namespace) {
        Ok(rules) => rules,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": error })),
            )
                .into_response();
        }
    };

    let route_api: Api<HTTPRoute> = Api::namespaced(client.clone(), &ns);
    let httproute = HTTPRoute {
        metadata: kube::core::ObjectMeta {
            name: Some(resource_name.clone()),
            namespace: Some(ns.clone()),
            annotations: merge_security_annotations(None, &body.geoip, &body.security),
            ..Default::default()
        },
        spec: build_httproute_spec(
            host,
            &gateway_name,
            &ns,
            rules,
        ),
    };
    match route_api.create(&PostParams::default(), &httproute).await {
        Ok(_) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "message": "Gateway site created",
                "name": resource_name,
                "namespace": ns,
                "resource_kind": "httproute"
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to create HTTPRoute: {}", e) })),
        )
            .into_response(),
    }
}

pub async fn kubernetes_gateway_site_get(
    State(state): State<AdminState>,
    AxumPath((namespace, name)): AxumPath<(String, String)>,
    _headers: axum::http::HeaderMap,
) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    if !state.gateway_api_enabled {
        return gateway_api_disabled();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let route_api: Api<HTTPRoute> = Api::namespaced(client.clone(), &namespace);
    let route = match route_api.get(&name).await {
        Ok(r) => r,
        Err(e) => {
            let err_str = e.to_string();
            let code = if err_str.contains("404") || err_str.to_lowercase().contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            return (
                code,
                Json(serde_json::json!({ "error": err_str })),
            )
                .into_response();
        }
    };
    if let Err(resp) = ensure_httproute_managed(&state, client, &route).await {
        return resp;
    }
    let spec = &route.spec;
    let host = spec
        .hostnames
        .first()
        .cloned()
        .unwrap_or_else(|| "*".to_string());
    let routes = ingress_form_routes_from_httproute(&route);
    let gateway_name = spec
        .parent_refs
        .first()
        .map(|p| p.name.clone())
        .unwrap_or_default();
    let gateway_namespace = spec
        .parent_refs
        .first()
        .and_then(|p| p.namespace.clone())
        .unwrap_or_else(|| namespace.clone());
    let gw_api: Api<Gateway> = Api::namespaced(client.clone(), &gateway_namespace);
    let tls_secret = if gateway_name.is_empty() {
        None
    } else {
        gw_api
            .get(&gateway_name)
            .await
            .ok()
            .and_then(|gw| gateway_tls_secret(&gw))
    };
    let gateway_class = if gateway_name.is_empty() {
        None
    } else {
        gw_api
            .get(&gateway_name)
            .await
            .ok()
            .and_then(|gw| gw.spec.gateway_class_name)
    };
    let first_route = routes.first();
    let annotations = route.metadata.annotations.as_ref();
    let row = IngressFormRow {
        namespace: namespace.clone(),
        name: name.clone(),
        host,
        routes: routes.clone(),
        path: first_route
            .map(|route| route.path.clone())
            .unwrap_or_else(|| "/".to_string()),
        path_type: first_route
            .map(|route| route.path_type.clone())
            .unwrap_or_else(|| "Prefix".to_string()),
        tls_secret_name: tls_secret,
        service_name: first_route
            .map(|route| route.service_name.clone())
            .unwrap_or_default(),
        service_port: first_route.and_then(|route| route.service_port),
        service_port_name: first_route.and_then(|route| route.service_port_name.clone()),
        ingress_class_name: gateway_class,
        gateway_name: if gateway_name.is_empty() {
            None
        } else {
            Some(gateway_name)
        },
        gateway_namespace: Some(gateway_namespace),
        geoip: crate::geoip::policy_from_annotations(annotations),
        security: crate::security::policy_from_annotations(annotations),
    };
    Json(row).into_response()
}

pub async fn kubernetes_gateway_site_update(
    State(state): State<AdminState>,
    AxumPath((namespace, name)): AxumPath<(String, String)>,
    _headers: axum::http::HeaderMap,
    Json(body): Json<CreateIngressBody>,
) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    if !state.gateway_api_enabled {
        return gateway_api_disabled();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let host = body.host.trim();
    if host.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "host is required" })),
        )
            .into_response();
    }
    let service_namespace = body.service_namespace.trim();
    if service_namespace.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "service_namespace is required" })),
        )
            .into_response();
    }
    let rules = match httproute_rules_from_body(&body, service_namespace) {
        Ok(rules) => rules,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": error })),
            )
                .into_response();
        }
    };

    let route_api: Api<HTTPRoute> = Api::namespaced(client.clone(), &namespace);
    let mut httproute = match route_api.get(&name).await {
        Ok(route) => route,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to load HTTPRoute: {}", e) })),
            )
                .into_response();
        }
    };
    if let Err(resp) = ensure_httproute_managed(&state, client, &httproute).await {
        return resp;
    }
    let gateway_name = body
        .gateway_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .or_else(|| {
            httproute
                .spec
                .parent_refs
                .first()
                .map(|p| p.name.clone())
        })
        .unwrap_or_default();
    if gateway_name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "gateway_name is required" })),
        )
            .into_response();
    }
    let gateway_ns = body
        .gateway_namespace
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .or_else(|| {
            httproute
                .spec
                .parent_refs
                .first()
                .and_then(|p| p.namespace.clone())
        })
        .unwrap_or_else(|| namespace.clone());
    let gw_api: Api<Gateway> = Api::namespaced(client.clone(), &gateway_ns);
    let gateway = match gw_api.get(&gateway_name).await {
        Ok(gw) => gw,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("Gateway \"{}\" not found in namespace \"{}\"", gateway_name, gateway_ns)
                })),
            )
                .into_response();
        }
    };
    if let Err(resp) = ensure_gateway_managed(&state, &gateway) {
        return resp;
    }
    httproute.spec = build_httproute_spec(host, &gateway_name, &gateway_ns, rules);
    httproute.metadata.annotations = merge_security_annotations(
        httproute.metadata.annotations.clone(),
        &body.geoip,
        &body.security,
    );
    match route_api.replace(&name, &PostParams::default(), &httproute).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "message": "Gateway site updated",
                "name": name,
                "namespace": namespace,
                "resource_kind": "httproute"
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to update HTTPRoute: {}", e) })),
        )
            .into_response(),
    }
}

pub async fn kubernetes_gateway_site_delete(
    State(state): State<AdminState>,
    AxumPath((namespace, name)): AxumPath<(String, String)>,
    _headers: axum::http::HeaderMap,
) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    if !state.gateway_api_enabled {
        return gateway_api_disabled();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let route_api: Api<HTTPRoute> = Api::namespaced(client.clone(), &namespace);
    let route = match route_api.get(&name).await {
        Ok(route) => route,
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("404") || err_str.to_lowercase().contains("not found") {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({ "error": err_str })),
                )
                    .into_response();
            }
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": err_str })),
            )
                .into_response();
        }
    };
    if let Err(resp) = ensure_httproute_managed(&state, client, &route).await {
        return resp;
    }
    match route_api.delete(&name, &DeleteParams::default()).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "message": "Gateway site deleted",
                "name": name,
                "namespace": namespace
            })),
        )
            .into_response(),
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("404") || err_str.to_lowercase().contains("not found") {
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "message": "Gateway site deleted",
                        "name": name,
                        "namespace": namespace
                    })),
                )
                    .into_response()
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": err_str })),
                )
                    .into_response()
            }
        }
    }
}

pub async fn kubernetes_gateways(
    State(state): State<AdminState>,
    Query(q): Query<NamespaceQuery>,
    _headers: axum::http::HeaderMap,
) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    if !state.gateway_api_enabled {
        return gateway_api_disabled();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let api: Api<Gateway> = if let Some(ref ns) = q.namespace {
        Api::namespaced(client.clone(), ns)
    } else {
        Api::all(client.clone())
    };
    let want_class = state.gateway_class.as_deref();
    let list = match api.list(&ListParams::default()).await {
        Ok(l) => l,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let rows: Vec<K8sGatewayRow> = list
        .items
        .into_iter()
        .filter(|gw| gateway_managed_by_app(gw, want_class))
        .map(gateway_row_from)
        .collect();
    Json(rows).into_response()
}

pub async fn kubernetes_httproutes(
    State(state): State<AdminState>,
    Query(q): Query<NamespaceQuery>,
) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    if !state.gateway_api_enabled {
        return gateway_api_disabled();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let want_class = state.gateway_class.as_deref();
    let gateways = match load_all_gateways(client, q.namespace.as_deref()).await {
        Ok(gws) => gws,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response();
        }
    };
    let managed = managed_gateway_keys(&gateways, want_class);
    if managed.is_empty() {
        return Json(Vec::<K8sHttpRouteRow>::new()).into_response();
    }
    let route_api: Api<HTTPRoute> = if let Some(ref ns) = q.namespace {
        Api::namespaced(client.clone(), ns)
    } else {
        Api::all(client.clone())
    };
    let list = match route_api.list(&ListParams::default()).await {
        Ok(l) => l,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let rows: Vec<K8sHttpRouteRow> = list
        .items
        .iter()
        .filter(|route| httproute_attached_to_managed(route, &managed))
        .map(httproute_row_from)
        .collect();
    Json(rows).into_response()
}

pub async fn kubernetes_gateways_create(
    State(state): State<AdminState>,
    _headers: axum::http::HeaderMap,
    Json(body): Json<CreateGatewayBody>,
) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    if !state.gateway_api_enabled {
        return gateway_api_disabled();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let host = body.host.trim();
    if host.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "host is required" })),
        )
            .into_response();
    }
    let ns = match resolve_gateway_namespace(&body) {
        Ok(ns) => ns,
        Err(resp) => return resp,
    };
    let resource_name = body
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| host_to_k8s_name(host));
    let gateway_class = body
        .gateway_class_name
        .clone()
        .or_else(|| state.gateway_class.clone())
        .or_else(|| state.ingress_class.clone());
    let tls_secret_name = match (
        body.tls_secret_namespace.as_deref().map(str::trim).filter(|s| !s.is_empty()),
        body.tls_secret_name.as_deref().map(str::trim).filter(|s| !s.is_empty()),
    ) {
        (Some(tls_ns), Some(tls_name)) if tls_ns == ns => Some(tls_name.to_string()),
        (Some(_), Some(_)) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "TLS secret must be in the same namespace as the Gateway"
                })),
            )
                .into_response();
        }
        _ => None,
    };
    let gw_api: Api<Gateway> = Api::namespaced(client.clone(), &ns);
    let gateway = Gateway {
        metadata: kube::core::ObjectMeta {
            name: Some(resource_name.clone()),
            namespace: Some(ns.clone()),
            ..Default::default()
        },
        spec: build_gateway_spec(host, gateway_class, tls_secret_name.as_deref()),
        status: None,
    };
    match gw_api.create(&PostParams::default(), &gateway).await {
        Ok(_) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "message": "Gateway created",
                "name": resource_name,
                "namespace": ns
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to create Gateway: {}", e) })),
        )
            .into_response(),
    }
}

pub async fn kubernetes_gateway_get(
    State(state): State<AdminState>,
    AxumPath((namespace, name)): AxumPath<(String, String)>,
    _headers: axum::http::HeaderMap,
) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    if !state.gateway_api_enabled {
        return gateway_api_disabled();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let gw_api: Api<Gateway> = Api::namespaced(client.clone(), &namespace);
    let gw = match gw_api.get(&name).await {
        Ok(gw) => gw,
        Err(e) => {
            let err_str = e.to_string();
            let code = if err_str.contains("404") || err_str.to_lowercase().contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            return (code, Json(serde_json::json!({ "error": err_str }))).into_response();
        }
    };
    if let Err(resp) = ensure_gateway_managed(&state, &gw) {
        return resp;
    }
    let row = GatewayFormRow {
        namespace,
        name,
        host: gateway_host(&gw).unwrap_or_default(),
        gateway_class_name: gw.spec.gateway_class_name.clone(),
        tls_secret_name: gateway_tls_secret(&gw),
    };
    Json(row).into_response()
}

pub async fn kubernetes_gateway_update(
    State(state): State<AdminState>,
    AxumPath((namespace, name)): AxumPath<(String, String)>,
    _headers: axum::http::HeaderMap,
    Json(body): Json<CreateGatewayBody>,
) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    if !state.gateway_api_enabled {
        return gateway_api_disabled();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let host = body.host.trim();
    if host.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "host is required" })),
        )
            .into_response();
    }
    let gateway_class = body
        .gateway_class_name
        .clone()
        .or_else(|| state.gateway_class.clone())
        .or_else(|| state.ingress_class.clone());
    let tls_secret_name = match (
        body.tls_secret_namespace.as_deref().map(str::trim).filter(|s| !s.is_empty()),
        body.tls_secret_name.as_deref().map(str::trim).filter(|s| !s.is_empty()),
    ) {
        (Some(tls_ns), Some(tls_name)) if tls_ns == namespace => Some(tls_name.to_string()),
        (Some(_), Some(_)) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "TLS secret must be in the same namespace as the Gateway"
                })),
            )
                .into_response();
        }
        _ => None,
    };
    let gw_api: Api<Gateway> = Api::namespaced(client.clone(), &namespace);
    let mut gateway = match gw_api.get(&name).await {
        Ok(gw) => gw,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to load Gateway: {}", e) })),
            )
                .into_response();
        }
    };
    if let Err(resp) = ensure_gateway_managed(&state, &gateway) {
        return resp;
    }
    gateway.spec = apply_gateway_form_to_spec(
        Some(&gateway.spec),
        host,
        gateway_class,
        tls_secret_name.as_deref(),
    );
    match gw_api.replace(&name, &PostParams::default(), &gateway).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "message": "Gateway updated",
                "name": name,
                "namespace": namespace
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to update Gateway: {}", e) })),
        )
            .into_response(),
    }
}

pub async fn kubernetes_gateway_delete(
    State(state): State<AdminState>,
    AxumPath((namespace, name)): AxumPath<(String, String)>,
    _headers: axum::http::HeaderMap,
) -> Response {
    if !state.viewer_mode {
        return not_available();
    }
    if !state.gateway_api_enabled {
        return gateway_api_disabled();
    }
    let Some(ref client) = state.kube_client else {
        return not_available();
    };
    let gw_api: Api<Gateway> = Api::namespaced(client.clone(), &namespace);
    let gateway = match gw_api.get(&name).await {
        Ok(gw) => gw,
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("404") || err_str.to_lowercase().contains("not found") {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({ "error": err_str })),
                )
                    .into_response();
            }
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": err_str })),
            )
                .into_response();
        }
    };
    if let Err(resp) = ensure_gateway_managed(&state, &gateway) {
        return resp;
    }
    match gw_api.delete(&name, &DeleteParams::default()).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "message": "Gateway deleted",
                "name": name,
                "namespace": namespace
            })),
        )
            .into_response(),
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("404") || err_str.to_lowercase().contains("not found") {
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "message": "Gateway deleted",
                        "name": name,
                        "namespace": namespace
                    })),
                )
                    .into_response()
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": err_str })),
                )
                    .into_response()
            }
        }
    }
}

