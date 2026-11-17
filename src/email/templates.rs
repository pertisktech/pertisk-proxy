//! HTML/plain email templates for SMTP notifications.

use std::fmt::Write;

/// Content-ID used when embedding the brand logo in multipart/related emails.
pub const LOGO_CONTENT_ID: &str = "pertisk-logo";

/// Compact brand mark for email headers (`admin/public/logo.png`, resized).
pub const LOGO_PNG: &[u8] = include_bytes!("assets/logo.png");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogoSrc {
    /// `cid:…` for SMTP multipart/related messages.
    Cid,
    /// Data URI for admin UI HTML previews (no MIME parts).
    DataUri,
}

impl LogoSrc {
    fn as_img_src(self) -> String {
        match self {
            Self::Cid => format!("cid:{LOGO_CONTENT_ID}"),
            Self::DataUri => format!("data:image/png;base64,{}", base64_encode(LOGO_PNG)),
        }
    }
}

fn base64_encode(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let a = u32::from(chunk[0]);
        let b = u32::from(chunk.get(1).copied().unwrap_or(0));
        let c = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (a << 16) | (b << 8) | c;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Primary call-to-action button/link in HTML emails (plain text shows the URL).
#[derive(Debug, Clone)]
pub struct EmailAction {
    pub label: String,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct EmailContent {
    pub headline: String,
    pub paragraphs: Vec<String>,
    pub details: Vec<(String, String)>,
    pub action: Option<EmailAction>,
}

impl EmailContent {
    pub fn simple(headline: impl Into<String>, paragraphs: Vec<String>) -> Self {
        Self {
            headline: headline.into(),
            paragraphs,
            details: Vec::new(),
            action: None,
        }
    }

    pub fn with_details(
        headline: impl Into<String>,
        paragraphs: Vec<String>,
        details: Vec<(String, String)>,
    ) -> Self {
        Self {
            headline: headline.into(),
            paragraphs,
            details,
            action: None,
        }
    }

    pub fn with_action(mut self, label: impl Into<String>, url: impl Into<String>) -> Self {
        if let Some(url) = normalize_http_url(&url.into()) {
            self.action = Some(EmailAction {
                label: label.into(),
                url,
            });
        }
        self
    }
}

/// Attach a management-UI CTA when a dashboard URL can be resolved.
///
/// Resolution order: `dashboard_url` setting → `PERTISK_DASHBOARD_URL` env.
/// Bare hosts like `admin.example.com` are normalized to `https://…`.
pub fn with_dashboard_link(content: EmailContent, dashboard_url: &str) -> EmailContent {
    match resolve_dashboard_url(dashboard_url) {
        Some(url) => content.with_action("Open management UI", url),
        None => content,
    }
}

/// Resolve a usable management UI URL for email CTAs.
pub fn resolve_dashboard_url(configured: &str) -> Option<String> {
    if let Some(url) = normalize_http_url(configured) {
        return Some(url);
    }
    std::env::var("PERTISK_DASHBOARD_URL")
        .ok()
        .as_deref()
        .and_then(normalize_http_url)
}

/// Normalize a user/env URL to `http(s)://…`, or `None` if unsafe/empty.
pub fn normalize_http_url(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let candidate = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    if is_safe_http_url(&candidate) {
        Some(candidate)
    } else {
        None
    }
}

fn is_safe_http_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://"))
        && !url.chars().any(|c| c.is_control() || c == '"' || c == '\'' || c == '<')
}

#[derive(Debug, Clone)]
pub struct AuthEventDetails {
    pub username: String,
    pub ip_address: String,
    pub occurred_at: String,
    pub user_agent: String,
}

fn auth_detail_rows(details: &AuthEventDetails) -> Vec<(String, String)> {
    vec![
        ("Username".into(), details.username.clone()),
        ("IP address".into(), details.ip_address.clone()),
        ("Time".into(), details.occurred_at.clone()),
        ("User agent".into(), truncate_ua(&details.user_agent)),
    ]
}

pub fn login_failure_content(details: &AuthEventDetails) -> EmailContent {
    EmailContent::with_details(
        "Failed management login",
        vec![
            format!(
                "A failed login attempt was made for username “{}”.",
                details.username
            ),
            "If this was not you, review management access and consider changing the admin password."
                .into(),
        ],
        auth_detail_rows(details),
    )
}

pub fn login_content(details: &AuthEventDetails) -> EmailContent {
    EmailContent::with_details(
        "Successful management login",
        vec![
            format!(
                "User “{}” signed in to the management UI.",
                details.username
            ),
            "If this was not you, change the admin password immediately.".into(),
        ],
        auth_detail_rows(details),
    )
}

pub fn password_change_content(details: &AuthEventDetails) -> EmailContent {
    EmailContent::with_details(
        "Management password changed",
        vec![
            format!("The password for user “{}” was changed.", details.username),
            "If you did not make this change, secure the account and review access immediately."
                .into(),
        ],
        auth_detail_rows(details),
    )
}

pub fn test_content() -> EmailContent {
    EmailContent::simple(
        "SMTP is configured",
        vec![
            "This is a test email from Pertisk Proxy.".into(),
            "If you received this message, SMTP delivery is working.".into(),
        ],
    )
}

pub fn sample_details() -> AuthEventDetails {
    AuthEventDetails {
        username: "admin".into(),
        ip_address: "203.0.113.42".into(),
        occurred_at: "2026-07-23 04:12:08 UTC".into(),
        user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36".into(),
    }
}

pub fn sample_content(kind: &str) -> (String, EmailContent) {
    match kind {
        "login" => (
            "Successful management login".into(),
            login_content(&sample_details()),
        ),
        "login_failure" => (
            "Failed management login".into(),
            login_failure_content(&sample_details()),
        ),
        "password_change" => (
            "Management password changed".into(),
            password_change_content(&sample_details()),
        ),
        _ => ("Pertisk Proxy SMTP test".into(), test_content()),
    }
}

pub fn render_plain(from_name: &str, content: &EmailContent) -> String {
    let mut body = String::new();
    let _ = writeln!(body, "{}\n", content.headline);
    for paragraph in &content.paragraphs {
        let _ = writeln!(body, "{paragraph}\n");
    }
    if !content.details.is_empty() {
        for (label, value) in &content.details {
            let _ = writeln!(body, "{label}:\t{value}");
        }
        let _ = writeln!(body);
    }
    if let Some(action) = &content.action {
        let _ = writeln!(body, "{}:\n{}", action.label, action.url);
        let _ = writeln!(body);
    }
    let _ = writeln!(body, "---");
    let _ = write!(body, "{from_name}");
    body
}

pub fn render_html(from_name: &str, content: &EmailContent) -> String {
    render_html_with_logo(from_name, content, LogoSrc::Cid)
}

/// HTML suitable for the admin Settings preview iframe (embedded data-URI logo).
pub fn render_html_preview(from_name: &str, content: &EmailContent) -> String {
    render_html_with_logo(from_name, content, LogoSrc::DataUri)
}

fn render_html_with_logo(from_name: &str, content: &EmailContent, logo: LogoSrc) -> String {
    let headline = html_escape(&content.headline);
    let from_name_esc = html_escape(from_name);
    let logo_src = logo.as_img_src();

    let mut paragraphs = String::new();
    for paragraph in &content.paragraphs {
        paragraphs.push_str(&format!(
            r#"<p style="margin:0 0 16px;font-size:15px;line-height:1.6;color:#3f3f46;">{}</p>"#,
            html_escape(paragraph)
        ));
    }

    let mut details = String::new();
    if !content.details.is_empty() {
        details.push_str(
            r#"<table role="presentation" width="100%" cellspacing="0" cellpadding="0" style="margin:0 0 20px;background:#fafafa;border:1px solid #e4e4e7;border-radius:12px;"><tr><td style="padding:16px 18px;"><table role="presentation" width="100%" cellspacing="0" cellpadding="0">"#,
        );
        for (label, value) in &content.details {
            details.push_str(&format!(
                r#"<tr>
  <td style="padding:8px 0;font-size:14px;color:#71717a;vertical-align:top;width:120px;">{}:</td>
  <td style="padding:8px 0;font-size:14px;color:#18181b;vertical-align:top;word-break:break-word;">{}</td>
</tr>"#,
                html_escape(label),
                html_escape(value)
            ));
        }
        details.push_str("</table></td></tr></table>");
    }

    let mut action = String::new();
    if let Some(cta) = &content.action {
        let label = html_escape(&cta.label);
        let href = html_escape(&cta.url);
        action.push_str(&format!(
            r#"<table role="presentation" cellspacing="0" cellpadding="0" style="margin:8px 0 4px;"><tr>
  <td style="border-radius:10px;background:#18181b;">
    <a href="{href}" style="display:inline-block;padding:12px 20px;font-size:14px;font-weight:600;line-height:1.2;color:#ffffff;text-decoration:none;border-radius:10px;" target="_blank" rel="noopener noreferrer">{label}</a>
  </td>
</tr></table>
<p style="margin:12px 0 0;font-size:12px;line-height:1.5;color:#71717a;word-break:break-all;">
  Or open: <a href="{href}" style="color:#18181b;text-decoration:underline;">{href}</a>
</p>"#
        ));
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <meta name="color-scheme" content="light" />
  <title>{headline}</title>
</head>
<body style="margin:0;padding:0;background:#f4f4f5;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;">
  <table role="presentation" width="100%" cellspacing="0" cellpadding="0" style="background:#f4f4f5;padding:32px 16px;">
    <tr>
      <td align="center">
        <table role="presentation" width="100%" cellspacing="0" cellpadding="0" style="max-width:560px;">
          <tr>
            <td style="padding:0 0 20px;text-align:center;">
              <img src="{logo_src}" width="48" height="48" alt="{from_name_esc}" style="display:block;margin:0 auto 10px;border:0;outline:none;text-decoration:none;border-radius:10px;" />
              <p style="margin:0;font-size:18px;font-weight:700;color:#18181b;letter-spacing:-0.02em;">{from_name_esc}</p>
            </td>
          </tr>
          <tr>
            <td style="background:#ffffff;border:1px solid #e4e4e7;border-radius:14px;padding:28px 28px 24px;">
              <h1 style="margin:0 0 18px;font-size:22px;line-height:1.3;font-weight:700;color:#18181b;">{headline}</h1>
              {paragraphs}
              {details}
              {action}
            </td>
          </tr>
          <tr>
            <td style="padding:20px 8px 0;text-align:center;font-size:12px;line-height:1.5;color:#71717a;">
              <p style="margin:0;">{from_name_esc}</p>
            </td>
          </tr>
        </table>
      </td>
    </tr>
  </table>
</body>
</html>"#
    )
}

fn truncate_ua(ua: &str) -> String {
    let ua = ua.trim();
    if ua.is_empty() {
        return "—".into();
    }
    const MAX: usize = 160;
    if ua.chars().count() <= MAX {
        return ua.to_string();
    }
    let truncated: String = ua.chars().take(MAX).collect();
    format!("{truncated}…")
}

fn html_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_failure_html_includes_username() {
        let content = login_failure_content(&AuthEventDetails {
            username: "alice".into(),
            ip_address: "1.2.3.4".into(),
            occurred_at: "now".into(),
            user_agent: "curl/8".into(),
        });
        let html = render_html("Pertisk Proxy", &content);
        assert!(html.contains("alice"));
        assert!(html.contains("1.2.3.4"));
        assert!(html.contains("Failed management login"));
        assert!(html.contains(&format!("cid:{LOGO_CONTENT_ID}")));
        let preview = render_html_preview("Pertisk Proxy", &content);
        assert!(preview.contains("data:image/png;base64,"));
        assert!(!LOGO_PNG.is_empty());
    }

    #[test]
    fn action_link_renders_in_html_and_plain() {
        let content = test_content().with_action("Open management UI", "https://admin.example/");
        let html = render_html("Pertisk Proxy", &content);
        assert!(html.contains("href=\"https://admin.example/\""));
        assert!(html.contains("Open management UI"));
        let plain = render_plain("Pertisk Proxy", &content);
        assert!(plain.contains("https://admin.example/"));
        assert!(plain.contains("Open management UI"));
    }

    #[test]
    fn unsafe_action_urls_are_rejected() {
        let content = test_content().with_action("Nope", "javascript:alert(1)");
        assert!(content.action.is_none());
        let linked = with_dashboard_link(test_content(), "  ");
        assert!(linked.action.is_none());
        let linked = with_dashboard_link(test_content(), "https://ok.example/login");
        assert_eq!(
            linked.action.as_ref().map(|a| a.url.as_str()),
            Some("https://ok.example/login")
        );
    }

    #[test]
    fn bare_host_is_normalized_to_https() {
        let linked = with_dashboard_link(test_content(), "admin.example.com/login");
        assert_eq!(
            linked.action.as_ref().map(|a| a.url.as_str()),
            Some("https://admin.example.com/login")
        );
        assert_eq!(
            normalize_http_url("http://plain.example"),
            Some("http://plain.example".into())
        );
    }
}
