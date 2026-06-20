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
