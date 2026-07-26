//! HTML/plain email templates for SMTP notifications.

use std::fmt::Write;

#[derive(Debug, Clone)]
pub struct EmailContent {
    pub headline: String,
    pub paragraphs: Vec<String>,
    pub details: Vec<(String, String)>,
}

impl EmailContent {
    pub fn simple(headline: impl Into<String>, paragraphs: Vec<String>) -> Self {
        Self {
            headline: headline.into(),
            paragraphs,
            details: Vec::new(),
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
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoginFailureDetails {
    pub username: String,
    pub ip_address: String,
    pub attempted_at: String,
    pub user_agent: String,
}

pub fn login_failure_content(details: &LoginFailureDetails) -> EmailContent {
    let ua = truncate_ua(&details.user_agent);
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
        vec![
            ("Username".into(), details.username.clone()),
            ("IP address".into(), details.ip_address.clone()),
            ("Time".into(), details.attempted_at.clone()),
            ("User agent".into(), ua),
        ],
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

pub fn sample_content(kind: &str) -> (String, EmailContent) {
    match kind {
        "login_failure" => (
            "Failed management login".into(),
            login_failure_content(&LoginFailureDetails {
                username: "admin".into(),
                ip_address: "203.0.113.42".into(),
                attempted_at: "2026-07-23 04:12:08 UTC".into(),
                user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36"
                    .into(),
            }),
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
    let _ = writeln!(body, "---");
    let _ = write!(body, "{from_name}");
    body
}

pub fn render_html(from_name: &str, content: &EmailContent) -> String {
    let headline = html_escape(&content.headline);
    let from_name_esc = html_escape(from_name);

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
              <p style="margin:0;font-size:18px;font-weight:700;color:#18181b;letter-spacing:-0.02em;">{from_name_esc}</p>
            </td>
          </tr>
          <tr>
            <td style="background:#ffffff;border:1px solid #e4e4e7;border-radius:14px;padding:28px 28px 24px;">
              <h1 style="margin:0 0 18px;font-size:22px;line-height:1.3;font-weight:700;color:#18181b;">{headline}</h1>
              {paragraphs}
              {details}
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
        let content = login_failure_content(&LoginFailureDetails {
            username: "alice".into(),
            ip_address: "1.2.3.4".into(),
            attempted_at: "now".into(),
            user_agent: "curl/8".into(),
        });
        let html = render_html("Pertisk Proxy", &content);
        assert!(html.contains("alice"));
        assert!(html.contains("1.2.3.4"));
        assert!(html.contains("Failed management login"));
    }
}
