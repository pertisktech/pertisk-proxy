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
            if let Ok(filter) = EnvFilter::try_new(level_to_filter_str(level)) {
                return filter;
            }
        }
    }

    EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level_to_filter_str(default_level)))
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
}
