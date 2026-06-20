use tracing::Level;
use tracing_subscriber::{fmt, EnvFilter};

pub fn init() {
    let filter = env_filter(parse_log_level_from_env());
    fmt().with_env_filter(filter).init();
}

fn env_filter(default_level: Level) -> EnvFilter {
    if let Ok(raw) = std::env::var("PERTISK_LOG_LEVEL") {
        if !raw.trim().is_empty() {
            let level = parse_log_level(&raw);
            if let Ok(filter) = EnvFilter::try_new(&build_filter_spec(level)) {
                return filter;
            }
        }
    }

    EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(build_filter_spec(default_level)))
}

/// Suppresses Pingora TLS handshake noise from LAN probes (e.g. router at 192.168.1.1).
/// Set `PERTISK_LOG_TLS_HANDSHAKE=1` to show those errors.
fn build_filter_spec(level: Level) -> String {
    let base = level_to_filter_str(level);
    if env_bool("PERTISK_LOG_TLS_HANDSHAKE", false) {
        return base.to_string();
    }
    if matches!(level, Level::TRACE | Level::DEBUG) {
        return base.to_string();
    }
    format!("{base},pingora_core::services::listening=off,pingora_proxy=warn")
}

fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes"
        ),
        Err(_) => default,
    }
}

pub fn parse_log_level_from_env() -> Level {
    let Some(raw) = std::env::var("PERTISK_LOG_LEVEL")
        .ok()
        .filter(|s| !s.trim().is_empty())
    else {
        return Level::INFO;
    };
    parse_log_level(&raw)
}

fn parse_log_level(raw: &str) -> Level {
    match raw.trim().to_ascii_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" | "warning" => Level::WARN,
        "error" => Level::ERROR,
        other => {
            eprintln!("warn: invalid PERTISK_LOG_LEVEL={other:?}, using info");
            Level::INFO
        }
    }
}

fn level_to_filter_str(level: Level) -> &'static str {
    match level {
        Level::TRACE => "trace",
        Level::DEBUG => "debug",
        Level::INFO => "info",
        Level::WARN => "warn",
        Level::ERROR => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_log_level_values() {
        assert_eq!(parse_log_level("debug"), Level::DEBUG);
        assert_eq!(parse_log_level("WARN"), Level::WARN);
        assert_eq!(parse_log_level("invalid"), Level::INFO);
    }

    #[test]
    fn build_filter_spec_quiets_pingora_at_info() {
        assert_eq!(
            build_filter_spec(Level::INFO),
            "info,pingora_core::services::listening=off,pingora_proxy=warn"
        );
        assert_eq!(build_filter_spec(Level::DEBUG), "debug");
    }
}
