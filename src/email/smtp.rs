//! SMTP send via lettre.

use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use lettre::message::{header::ContentType, Mailbox, Message, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

use crate::db::SmtpSettingsRow;

use super::templates::{self, EmailContent};

/// Cap SMTP connect/handshake/send so the management API never hangs indefinitely.
const SMTP_TIMEOUT: Duration = Duration::from_secs(15);

pub async fn send_email(
    settings: &SmtpSettingsRow,
    to: &str,
    subject: &str,
    content: EmailContent,
    require_enabled: bool,
) -> Result<()> {
    if require_enabled && !settings.enabled {
        bail!("SMTP is disabled");
    }
    if settings.host.trim().is_empty() {
        bail!("SMTP host is not configured");
    }
    if settings.from_email.trim().is_empty() {
        bail!("From email is not configured");
    }
    let to = to.trim();
    if to.is_empty() {
        bail!("recipient address is empty");
    }

    let from_name = if settings.from_name.trim().is_empty() {
        "Pertisk Proxy".to_string()
    } else {
        settings.from_name.clone()
    };
    let plain = templates::render_plain(&from_name, &content);
    let html = templates::render_html(&from_name, &content);

    let from_mailbox = Mailbox::new(
        if settings.from_name.trim().is_empty() {
            None
        } else {
            Some(settings.from_name.clone())
        },
        settings
            .from_email
            .parse()
            .map_err(|e| anyhow!("invalid from email: {e}"))?,
    );
    let to_mailbox = to
        .parse()
        .map_err(|e| anyhow!("invalid recipient {to}: {e}"))?;

    let email = Message::builder()
        .from(from_mailbox)
        .to(to_mailbox)
        .subject(subject)
        .multipart(
            MultiPart::alternative()
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_PLAIN)
                        .body(plain),
                )
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_HTML)
                        .body(html),
                ),
        )?;

    let password = settings.password.trim();
    let password = if password.is_empty() {
        None
    } else {
        Some(password)
    };

    if !settings.username.trim().is_empty() && password.is_none() {
        bail!("SMTP username is set but password is missing");
    }

    tracing::info!(
        host = %settings.host,
        port = settings.port,
        use_tls = settings.use_tls,
        to = %to,
        "sending email"
    );

    let mailer = build_mailer(settings, password)?;
    match tokio::time::timeout(SMTP_TIMEOUT, mailer.send(email)).await {
        Ok(Ok(_)) => {
            tracing::info!(to = %to, "email sent");
            Ok(())
        }
        Ok(Err(err)) => Err(anyhow!("SMTP send to {to} failed: {err}")),
        Err(_) => Err(anyhow!(
            "SMTP send to {to} timed out after {}s (check host, port, TLS, and firewall)",
            SMTP_TIMEOUT.as_secs()
        )),
    }
}

fn build_mailer(
    settings: &SmtpSettingsRow,
    password: Option<&str>,
) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
    let host = settings.host.trim();
    let port = settings.port as u16;

    let mut builder = if !settings.use_tls {
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)
    } else if port == 465 {
        AsyncSmtpTransport::<Tokio1Executor>::relay(host)?
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)?
    };
    builder = builder.port(port).timeout(Some(SMTP_TIMEOUT));

    if !settings.username.trim().is_empty() {
        let pass = password.ok_or_else(|| {
            anyhow!("SMTP password is required when username is configured")
        })?;
        builder = builder.credentials(Credentials::new(
            settings.username.trim().to_string(),
            pass.to_string(),
        ));
    }

    Ok(builder.build())
}
