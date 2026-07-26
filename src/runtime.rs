//! Tokio runtime tuning (mirrors pertisk-rproxy `PERTISK_PROXY_MODE` / `PERTISK_INGRESS_MODE`).

use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMode {
    Auto,
    Standard,
    Performance,
}

impl RuntimeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Standard => "standard",
            Self::Performance => "performance",
        }
    }
}

impl FromStr for RuntimeMode {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Ok(Self::Auto),
            "standard" | "default" => Ok(Self::Standard),
            "performance" | "perf" => Ok(Self::Performance),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub requested_mode: RuntimeMode,
    pub resolved_mode: RuntimeMode,
    pub worker_threads: usize,
    pub max_blocking_threads: usize,
}

pub struct RuntimeEnv {
    pub mode_var: &'static str,
    pub mode_fallback_var: Option<&'static str>,
    pub use_cpu_limit: bool,
}

fn available_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

fn resolve_mode(mode: RuntimeMode, cpus: usize) -> RuntimeMode {
    match mode {
        RuntimeMode::Auto => {
            if cpus >= 4 {
                RuntimeMode::Performance
            } else {
                RuntimeMode::Standard
            }
        }
        other => other,
    }
}

fn read_requested_mode(env: &RuntimeEnv) -> anyhow::Result<RuntimeMode> {
    let raw = std::env::var(env.mode_var)
        .ok()
        .or_else(|| {
            env.mode_fallback_var
                .and_then(|name| std::env::var(name).ok())
        })
        .unwrap_or_else(|| "auto".to_string());
    raw.parse::<RuntimeMode>().map_err(|_| {
        anyhow::anyhow!(
            "{} must be one of: auto, standard, performance (got: {})",
            env.mode_var,
            raw
        )
    })
}

fn worker_threads_from_cpu_limit() -> Option<usize> {
    std::env::var("PERTISK_CPU_LIMIT_MILLICORES")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .map(|millicores| std::cmp::max(1, millicores / 1000))
}

pub fn runtime_config_from_env(env: &RuntimeEnv) -> anyhow::Result<RuntimeConfig> {
    let requested_mode = read_requested_mode(env)?;
    let cpus = available_cpus();
    let resolved_mode = resolve_mode(requested_mode, cpus);

    let worker_threads = std::env::var("PERTISK_WORKER_THREADS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|n| *n > 0)
        .or_else(|| {
            if env.use_cpu_limit {
                worker_threads_from_cpu_limit()
            } else {
                None
            }
        })
        .unwrap_or(cpus);

    let max_blocking_threads = std::env::var("PERTISK_MAX_BLOCKING_THREADS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(match resolved_mode {
            RuntimeMode::Performance => std::cmp::max(512, cpus.saturating_mul(64)),
            _ => 512,
        });

    Ok(RuntimeConfig {
        requested_mode,
        resolved_mode,
        worker_threads,
        max_blocking_threads,
    })
}

pub fn build_runtime(cfg: &RuntimeConfig, thread_name: &str) -> anyhow::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(cfg.worker_threads)
        .max_blocking_threads(cfg.max_blocking_threads)
        .thread_name(thread_name)
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build Tokio runtime: {e}"))
}

pub fn proxy_runtime_env() -> RuntimeEnv {
    RuntimeEnv {
        mode_var: "PERTISK_PROXY_MODE",
        mode_fallback_var: None,
        use_cpu_limit: false,
    }
}

pub fn ingress_runtime_env() -> RuntimeEnv {
    RuntimeEnv {
        mode_var: "PERTISK_INGRESS_MODE",
        mode_fallback_var: Some("PERTISK_PROXY_MODE"),
        use_cpu_limit: true,
    }
}

pub fn is_performance_mode(cfg: &RuntimeConfig) -> bool {
    cfg.resolved_mode == RuntimeMode::Performance
}

/// TCP listen backlog for inbound listeners (`SOMAXCONN`-capped by the kernel).
pub fn tcp_listen_backlog(cfg: &RuntimeConfig) -> i32 {
    const DEFAULT_STANDARD: i32 = 1024;
    const DEFAULT_PERFORMANCE: i32 = 8192;

    if let Ok(raw) = std::env::var("PERTISK_TCP_LISTEN_BACKLOG") {
        if let Ok(value) = raw.trim().parse::<i32>() {
            if value > 0 {
                return value;
            }
        }
    }

    if is_performance_mode(cfg) {
        DEFAULT_PERFORMANCE
    } else {
        DEFAULT_STANDARD
    }
}

/// Pingora worker threads per service (default in Pingora is 1).
pub fn pingora_service_threads(cfg: &RuntimeConfig) -> usize {
    if let Ok(raw) = std::env::var("PERTISK_PINGORA_THREADS") {
        if let Ok(value) = raw.trim().parse::<usize>() {
            if value > 0 {
                return value;
            }
        }
    }

    // Prefer the container/pod CPU limit when present (ingress sets
    // PERTISK_CPU_LIMIT_MILLICORES from resources.limits.cpu).
    let cpus = worker_threads_from_cpu_limit().unwrap_or_else(available_cpus);
    match cfg.resolved_mode {
        RuntimeMode::Performance => cpus,
        RuntimeMode::Standard => std::cmp::max(1, cpus / 2),
        RuntimeMode::Auto => cpus,
    }
}

/// Worker threads for the dedicated HTTP/3 runtime.
///
/// QUIC already runs one listener thread per H3 bind address; on small hosts,
/// a full copy of the Pingora worker count is wasteful, while on larger hosts
/// half the available workers is a reasonable default for this auxiliary runtime.
pub fn h3_worker_threads(cfg: &RuntimeConfig) -> usize {
    std::cmp::max(1, cfg.worker_threads / 2)
}

/// Parallel accept tasks per listener fd.
pub fn pingora_listener_tasks_per_fd(cfg: &RuntimeConfig) -> usize {
    if let Ok(raw) = std::env::var("PERTISK_PINGORA_LISTENER_TASKS") {
        if let Ok(value) = raw.trim().parse::<usize>() {
            if value > 0 {
                return value;
            }
        }
    }

    match cfg.resolved_mode {
        RuntimeMode::Performance => 4,
        _ => 1,
    }
}

/// Grace period after SIGTERM before Pingora tears down runtimes (default 15s; Pingora built-in default is 300s).
pub fn grace_period_seconds() -> u64 {
    const DEFAULT: u64 = 15;
    std::env::var("PERTISK_GRACE_PERIOD_SECONDS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT)
}

/// Final runtime shutdown timeout after the grace period (default 10s).
pub fn graceful_shutdown_timeout_seconds() -> u64 {
    const DEFAULT: u64 = 10;
    std::env::var("PERTISK_GRACEFUL_SHUTDOWN_TIMEOUT_SECONDS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT)
}

/// Downstream (client-facing) TCP keepalive settings for Pingora listeners.
///
/// Defaults match common reverse-proxy practice (Aralez-style): idle 60s,
/// probe every 10s, give up after 5 probes. Set `PERTISK_TCP_KEEPALIVE=0` to disable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownstreamTcpKeepalive {
    pub enabled: bool,
    pub idle_secs: u64,
    pub interval_secs: u64,
    pub count: usize,
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
}

pub fn downstream_tcp_keepalive_config() -> DownstreamTcpKeepalive {
    const DEFAULT_IDLE: u64 = 60;
    const DEFAULT_INTERVAL: u64 = 10;
    const DEFAULT_COUNT: usize = 5;

    let master = std::env::var("PERTISK_TCP_KEEPALIVE")
        .ok()
        .map(|v| v.trim().to_ascii_lowercase());
    let enabled = match master.as_deref() {
        Some("0") | Some("false") | Some("off") | Some("no") => false,
        Some("1") | Some("true") | Some("on") | Some("yes") | None => true,
        Some(other) => other.parse::<u64>().map(|n| n != 0).unwrap_or(true),
    };

    DownstreamTcpKeepalive {
        enabled,
        idle_secs: env_u64("PERTISK_TCP_KEEPALIVE_IDLE_SECS").unwrap_or(DEFAULT_IDLE),
        interval_secs: env_u64("PERTISK_TCP_KEEPALIVE_INTERVAL_SECS").unwrap_or(DEFAULT_INTERVAL),
        count: env_usize("PERTISK_TCP_KEEPALIVE_COUNT").unwrap_or(DEFAULT_COUNT),
    }
}

/// Pingora `TcpKeepalive` for accepted downstream connections, if enabled.
pub fn downstream_tcp_keepalive() -> Option<pingora_core::protocols::TcpKeepalive> {
    use std::time::Duration;

    let cfg = downstream_tcp_keepalive_config();
    if !cfg.enabled || cfg.idle_secs == 0 || cfg.interval_secs == 0 || cfg.count == 0 {
        return None;
    }
    Some(pingora_core::protocols::TcpKeepalive {
        idle: Duration::from_secs(cfg.idle_secs),
        interval: Duration::from_secs(cfg.interval_secs),
        count: cfg.count,
        #[cfg(target_os = "linux")]
        user_timeout: Duration::ZERO,
    })
}

/// Build Pingora `ServerConf` tuned from runtime mode.
pub fn pingora_server_conf(cfg: &RuntimeConfig) -> pingora_core::server::configuration::ServerConf {
    let mut conf = pingora_core::server::configuration::ServerConf::new()
        .expect("default pingora ServerConf");

    conf.threads = pingora_service_threads(cfg);
    conf.listener_tasks_per_fd = pingora_listener_tasks_per_fd(cfg);
    conf.grace_period_seconds = Some(grace_period_seconds());
    conf.graceful_shutdown_timeout_seconds = Some(graceful_shutdown_timeout_seconds());

    if is_performance_mode(cfg) {
        conf.upstream_keepalive_pool_size = 512;
    }

    conf
}
