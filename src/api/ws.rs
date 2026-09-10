//! Live admin updates: WebSocket (`/api/ws`) and SSE (`/api/live`).
//!
//! Ingress mode terminates TLS on the data plane with HTTP/2 preferred. Browsers then often use
//! WebSocket-over-H2 (Extended CONNECT), which Pingora does not upgrade. SSE is a normal streaming
//! GET and works through that path; keep `/api/ws` for direct management access.

use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
};
use futures::{SinkExt, Stream, StreamExt};
use serde::Deserialize;
use tokio::sync::{mpsc, Mutex};
use tracing::debug;

use super::{
    build_certificates, build_config, build_logs_entries, build_management_info,
    build_metrics_response, build_routes_response, extract_bearer, fetch_tunnel_status_value,
    is_token_authorized, AdminState, LogsQuery,
};

const MAX_LIVE_CONNECTIONS: usize = 64;
static LIVE_CONNECTIONS: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Deserialize)]
pub struct WsConnectQuery {
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LiveSseQuery {
    token: Option<String>,
    /// Comma-separated channel names.
    channels: Option<String>,
    #[serde(rename = "logs_type")]
    logs_type: Option<String>,
    logs_host: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Channel {
    Management,
    Metrics,
    Logs,
    Config,
    Certificates,
    Routes,
    Tunnel,
    Pods,
}

impl Channel {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "management" => Some(Self::Management),
            "metrics" => Some(Self::Metrics),
            "logs" => Some(Self::Logs),
            "config" => Some(Self::Config),
            "certificates" => Some(Self::Certificates),
            "routes" => Some(Self::Routes),
            "tunnel" => Some(Self::Tunnel),
            "pods" => Some(Self::Pods),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Management => "management",
            Self::Metrics => "metrics",
            Self::Logs => "logs",
            Self::Config => "config",
            Self::Certificates => "certificates",
            Self::Routes => "routes",
            Self::Tunnel => "tunnel",
            Self::Pods => "pods",
        }
    }

    fn interval(self) -> Duration {
        match self {
            Self::Management | Self::Metrics | Self::Logs => Duration::from_secs(3),
            Self::Config | Self::Certificates | Self::Routes => Duration::from_secs(5),
            Self::Tunnel | Self::Pods => Duration::from_secs(10),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum ClientMsg {
    Subscribe {
        channels: Vec<String>,
    },
    Unsubscribe {
        channels: Vec<String>,
    },
    LogsFilter {
        #[serde(rename = "type")]
        log_type: Option<String>,
        host: Option<String>,
    },
}

struct ConnState {
    channels: HashSet<Channel>,
    logs_filter: LogsQuery,
    last_payload: HashMap<Channel, String>,
    last_sent_at: HashMap<Channel, std::time::Instant>,
}

impl Default for ConnState {
    fn default() -> Self {
        Self {
            channels: HashSet::new(),
            logs_filter: LogsQuery::default(),
            last_payload: HashMap::new(),
            last_sent_at: HashMap::new(),
        }
    }
}

fn parse_channels(raw: Option<&str>) -> HashSet<Channel> {
    raw.unwrap_or("")
        .split(',')
        .filter_map(|c| Channel::parse(c.trim()))
        .collect()
}

fn acquire_live_slot() -> Result<(), Response> {
    let prev = LIVE_CONNECTIONS.fetch_add(1, Ordering::SeqCst);
    if prev >= MAX_LIVE_CONNECTIONS {
        LIVE_CONNECTIONS.fetch_sub(1, Ordering::SeqCst);
        return Err((StatusCode::SERVICE_UNAVAILABLE, "too many live connections").into_response());
    }
    Ok(())
}

fn release_live_slot() {
    LIVE_CONNECTIONS.fetch_sub(1, Ordering::SeqCst);
}

struct LiveSlotGuard;
impl Drop for LiveSlotGuard {
    fn drop(&mut self) {
        release_live_slot();
    }
}

async fn authorize_live(state: &AdminState, token: Option<&str>, headers: &HeaderMap) -> bool {
    let token = token
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .or_else(|| extract_bearer(headers));
    is_token_authorized(state, token).await
}

pub async fn ws_handler(
    State(state): State<AdminState>,
    Query(q): Query<WsConnectQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !authorize_live(&state, q.token.as_deref(), &headers).await {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    if let Err(resp) = acquire_live_slot() {
        return resp;
    }

    ws.on_upgrade(move |socket| async move {
        handle_socket(state, socket).await;
        release_live_slot();
    })
}

/// SSE live feed used by the admin UI (works through ingress HTTP/2).
pub async fn live_sse_handler(
    State(state): State<AdminState>,
    Query(q): Query<LiveSseQuery>,
    headers: HeaderMap,
) -> Response {
    if !authorize_live(&state, q.token.as_deref(), &headers).await {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    if let Err(resp) = acquire_live_slot() {
        return resp;
    }

    let channels = parse_channels(q.channels.as_deref());
    if channels.is_empty() {
        release_live_slot();
        return (StatusCode::BAD_REQUEST, "channels required").into_response();
    }

    let logs_filter = LogsQuery {
        log_type: q.logs_type,
        host: q.logs_host,
    };

    Sse::new(sse_stream(state, channels, logs_filter))
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("ping"),
        )
        .into_response()
}

fn sse_stream(
    state: AdminState,
    channels: HashSet<Channel>,
    logs_filter: LogsQuery,
) -> impl Stream<Item = Result<Event, Infallible>> {
    let (tx, rx) = mpsc::channel::<Event>(8);
    tokio::spawn(async move {
        let _guard = LiveSlotGuard;
        let mut last_payload: HashMap<Channel, String> = HashMap::new();
        let mut last_sent_at: HashMap<Channel, std::time::Instant> = HashMap::new();
        let channel_list: Vec<Channel> = channels.into_iter().collect();

        for ch in &channel_list {
            if let Some(event) =
                channel_event(&state, *ch, &logs_filter, &mut last_payload, true).await
            {
                last_sent_at.insert(*ch, std::time::Instant::now());
                if tx.send(event).await.is_err() {
                    return;
                }
            }
        }

        let mut interval = tokio::time::interval(Duration::from_millis(500));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let now = std::time::Instant::now();
            for ch in &channel_list {
                let due = last_sent_at
                    .get(ch)
                    .map(|t| now.duration_since(*t) >= ch.interval())
                    .unwrap_or(true);
                if !due {
                    continue;
                }
                match channel_event(&state, *ch, &logs_filter, &mut last_payload, false).await {
                    Some(event) => {
                        last_sent_at.insert(*ch, std::time::Instant::now());
                        if tx.send(event).await.is_err() {
                            return;
                        }
                    }
                    None => {
                        last_sent_at.insert(*ch, std::time::Instant::now());
                    }
                }
            }
        }
    });

    futures::stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Some(event) => Some((Ok::<Event, Infallible>(event), rx)),
            None => None,
        }
    })
}

async fn channel_event(
    state: &AdminState,
    channel: Channel,
    logs_filter: &LogsQuery,
    last_payload: &mut HashMap<Channel, String>,
    force: bool,
) -> Option<Event> {
    let payload = match snapshot_channel(state, channel, logs_filter).await {
        Ok(data) => serde_json::json!({ "channel": channel.as_str(), "data": data }).to_string(),
        Err(error) => {
            serde_json::json!({ "channel": channel.as_str(), "error": error }).to_string()
        }
    };
    if !force && last_payload.get(&channel).is_some_and(|prev| prev == &payload) {
        return None;
    }
    last_payload.insert(channel, payload.clone());
    Some(Event::default().data(payload))
}

async fn handle_socket(state: AdminState, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let conn = Arc::new(Mutex::new(ConnState::default()));
    let (tx, mut rx) = mpsc::channel::<Message>(32);

    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    let ticker_state = state.clone();
    let ticker_conn = Arc::clone(&conn);
    let ticker_tx = tx.clone();
    let ticker = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if ticker_tx.is_closed() {
                break;
            }
            if push_due_channels(&ticker_state, &ticker_conn, &ticker_tx)
                .await
                .is_err()
            {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            Message::Text(text) => {
                if let Err(err) = handle_client_text(&state, &conn, &tx, &text).await {
                    let _ = tx
                        .send(Message::Text(
                            serde_json::json!({ "error": err }).to_string().into(),
                        ))
                        .await;
                }
            }
            Message::Ping(payload) => {
                if tx.send(Message::Pong(payload)).await.is_err() {
                    break;
                }
            }
            Message::Close(_) => break,
            Message::Pong(_) | Message::Binary(_) => {}
        }
    }

    ticker.abort();
    drop(tx);
    let _ = writer.await;
    debug!("admin websocket closed");
}

async fn handle_client_text(
    state: &AdminState,
    conn: &Arc<Mutex<ConnState>>,
    tx: &mpsc::Sender<Message>,
    text: &str,
) -> Result<(), String> {
    let msg: ClientMsg = serde_json::from_str(text).map_err(|e| format!("invalid message: {e}"))?;
    match msg {
        ClientMsg::Subscribe { channels } => {
            let parsed: Vec<Channel> = channels
                .iter()
                .filter_map(|c| Channel::parse(c.trim()))
                .collect();
            {
                let mut guard = conn.lock().await;
                for ch in &parsed {
                    guard.channels.insert(*ch);
                    guard.last_payload.remove(ch);
                    guard.last_sent_at.remove(ch);
                }
            }
            for ch in parsed {
                push_channel(state, conn, tx, ch, true).await?;
            }
        }
        ClientMsg::Unsubscribe { channels } => {
            let mut guard = conn.lock().await;
            for name in channels {
                if let Some(ch) = Channel::parse(name.trim()) {
                    guard.channels.remove(&ch);
                    guard.last_payload.remove(&ch);
                    guard.last_sent_at.remove(&ch);
                }
            }
        }
        ClientMsg::LogsFilter { log_type, host } => {
            {
                let mut guard = conn.lock().await;
                guard.logs_filter = LogsQuery { log_type, host };
                guard.last_payload.remove(&Channel::Logs);
                guard.last_sent_at.remove(&Channel::Logs);
            }
            let subscribed = conn.lock().await.channels.contains(&Channel::Logs);
            if subscribed {
                push_channel(state, conn, tx, Channel::Logs, true).await?;
            }
        }
    }
    Ok(())
}

async fn push_due_channels(
    state: &AdminState,
    conn: &Arc<Mutex<ConnState>>,
    tx: &mpsc::Sender<Message>,
) -> Result<(), ()> {
    let due: Vec<Channel> = {
        let guard = conn.lock().await;
        let now = std::time::Instant::now();
        guard
            .channels
            .iter()
            .copied()
            .filter(|ch| {
                guard
                    .last_sent_at
                    .get(ch)
                    .map(|t| now.duration_since(*t) >= ch.interval())
                    .unwrap_or(true)
            })
            .collect()
    };
    for ch in due {
        if push_channel(state, conn, tx, ch, false).await.is_err() {
            return Err(());
        }
    }
    Ok(())
}

async fn push_channel(
    state: &AdminState,
    conn: &Arc<Mutex<ConnState>>,
    tx: &mpsc::Sender<Message>,
    channel: Channel,
    force: bool,
) -> Result<(), String> {
    let logs_filter = {
        let guard = conn.lock().await;
        if !guard.channels.contains(&channel) {
            return Ok(());
        }
        guard.logs_filter.clone()
    };

    let data = match snapshot_channel(state, channel, &logs_filter).await {
        Ok(v) => v,
        Err(error) => {
            let msg = serde_json::json!({ "channel": channel.as_str(), "error": error });
            tx.send(Message::Text(msg.to_string().into()))
                .await
                .map_err(|_| "websocket closed".to_string())?;
            let mut guard = conn.lock().await;
            guard.last_sent_at.insert(channel, std::time::Instant::now());
            return Ok(());
        }
    };

    let payload = serde_json::json!({
        "channel": channel.as_str(),
        "data": data,
    })
    .to_string();

    {
        let mut guard = conn.lock().await;
        if !force && guard.last_payload.get(&channel).is_some_and(|prev| prev == &payload) {
            guard.last_sent_at.insert(channel, std::time::Instant::now());
            return Ok(());
        }
        guard.last_payload.insert(channel, payload.clone());
        guard.last_sent_at.insert(channel, std::time::Instant::now());
    }

    tx.send(Message::Text(payload.into()))
        .await
        .map_err(|_| "websocket closed".to_string())?;
    Ok(())
}

async fn snapshot_channel(
    state: &AdminState,
    channel: Channel,
    logs_filter: &LogsQuery,
) -> Result<serde_json::Value, String> {
    match channel {
        Channel::Management => serde_json::to_value(build_management_info(state).await)
            .map_err(|e| e.to_string()),
        Channel::Metrics => {
            serde_json::to_value(build_metrics_response(state)).map_err(|e| e.to_string())
        }
        Channel::Logs => serde_json::to_value(build_logs_entries(state, logs_filter).await)
            .map_err(|e| e.to_string()),
        Channel::Config => {
            serde_json::to_value(build_config(state).await).map_err(|e| e.to_string())
        }
        Channel::Certificates => match build_certificates(state).await {
            Ok(rows) => serde_json::to_value(rows).map_err(|e| e.to_string()),
            Err(e) => Err(e),
        },
        Channel::Routes => {
            serde_json::to_value(build_routes_response(state)).map_err(|e| e.to_string())
        }
        Channel::Tunnel => fetch_tunnel_status_value().await,
        Channel::Pods => {
            #[cfg(feature = "ingress")]
            {
                match super::kubernetes::list_pods_rows(state, None).await {
                    Ok(None) => Ok(serde_json::json!([])),
                    Ok(Some(rows)) => serde_json::to_value(rows).map_err(|e| e.to_string()),
                    Err(e) => Err(e),
                }
            }
            #[cfg(not(feature = "ingress"))]
            {
                Ok(serde_json::json!([]))
            }
        }
    }
}
