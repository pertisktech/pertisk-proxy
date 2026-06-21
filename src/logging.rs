use std::sync::Arc;

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Registry};

use crate::log::{LogLevel, ProxyLog, ProxyLogEntry};

pub fn init(ui_log: Option<Arc<ProxyLog>>) {
    let filter = env_filter(parse_log_level_from_env());
    let fmt_layer = fmt::layer();

    if let Some(log) = ui_log {
        Registry::default()
            .with(filter)
            .with(fmt_layer)
            .with(UiLogLayer { log })
            .init();
    } else {
        Registry::default().with(filter).with(fmt_layer).init();
    }
}

struct UiLogLayer {
    log: Arc<ProxyLog>,
}

impl<S> Layer<S> for UiLogLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let target = event.metadata().target();
        let include = target.starts_with("pertisk_proxy")
            || (target.starts_with("pingora")
                && matches!(
                    *event.metadata().level(),
                    Level::WARN | Level::ERROR
                ));
        if !include {
            return;
        }

        let level = match *event.metadata().level() {
            Level::ERROR => LogLevel::Error,
            Level::WARN => LogLevel::Warn,
            Level::INFO => LogLevel::Info,
            Level::DEBUG => LogLevel::Debug,
            Level::TRACE => return,
        };

        let mut visitor = EventVisitor::default();
        event.record(&mut visitor);

        let message = format_event_message(event, &visitor);
        let source = short_target(target);
        self.log
            .push_sync(ProxyLogEntry::tracing_event(level, &source, message));
    }
}

#[derive(Default)]
struct EventVisitor {
    message: String,
    fields: Vec<(String, String)>,
}

impl Visit for EventVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let rendered = format!("{value:?}");
        if field.name() == "message" {
            self.message = trim_debug_quotes(&rendered);
        } else {
            self.fields.push((field.name().to_string(), rendered));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields
                .push((field.name().to_string(), format!("{value:?}")));
        }
    }
}

fn format_event_message(event: &Event<'_>, visitor: &EventVisitor) -> String {
    let mut parts = Vec::new();
    if !visitor.message.is_empty() && visitor.message != "log" {
        parts.push(visitor.message.clone());
    } else if !event.metadata().name().is_empty() && event.metadata().name() != "log" {
        parts.push(event.metadata().name().to_string());
    }

    for (key, value) in &visitor.fields {
        if should_skip_log_field(key) {
            continue;
        }
        parts.push(format!("{key}={value}"));
    }

    if parts.is_empty() {
        event.metadata().target().to_string()
    } else {
        parts.join(" ")
    }
}

fn should_skip_log_field(key: &str) -> bool {
    matches!(
        key,
        "log.target" | "log.module_path" | "log.file" | "log.line" | "log.module"
    )
}

fn trim_debug_quotes(value: &str) -> String {
    value.trim_matches('"').to_string()
}

fn short_target(target: &str) -> String {
    target
        .strip_prefix("pertisk_proxy::")
        .or_else(|| target.strip_prefix("pertisk_proxy"))
        .unwrap_or(target)
        .to_string()
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

    #[test]
    fn short_target_strips_crate_prefix() {
        assert_eq!(short_target("pertisk_proxy::tls::sni"), "tls::sni");
    }
}
