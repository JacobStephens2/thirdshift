//! Email through Resend's HTTP API (ADR 0005): one HTTPS POST per email, with
//! the key from `RESEND_API_KEY`. `thirdshift email-test` sends through it,
//! and Run notifications are to use the same checks and the same send.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::{Local, SecondsFormat};
use serde_json::{Value, json};

use crate::config::EmailSettings;
use crate::host;

/// Resend's shared sender, used when `email.from` isn't set. Resend only lets
/// it send to the address of the user's own Resend account.
const DEFAULT_FROM: &str = "onboarding@resend.dev";

const RESEND_URL: &str = "https://api.resend.com";

/// How long a send may take before thirdshift gives up on it.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Resend, ready to send to one address. Making one checks everything that
/// can be checked locally; nothing is sent to validate the key.
pub struct Resend {
    key: String,
    /// The API's base URL: Resend's own, unless `THIRDSHIFT_RESEND_URL`
    /// overrides it, which only the tests do.
    url: String,
    from: String,
    to: String,
}

impl Resend {
    /// Resend, sending to `to`, else to `email.to`, from `email.from`, else
    /// from Resend's shared sender. Fails naming each thing that is missing:
    /// the address, `RESEND_API_KEY`, or both.
    pub fn new(to: Option<String>, settings: &EmailSettings) -> Result<Self> {
        let to = to
            .or_else(|| settings.to.clone())
            .filter(|to| !to.is_empty());
        let key = std::env::var("RESEND_API_KEY")
            .ok()
            .filter(|key| !key.is_empty());
        let (Some(to), Some(key)) = (to.clone(), key.clone()) else {
            let mut missing = Vec::new();
            if to.is_none() {
                missing.push("no email address: give one, or set email.to in the User config");
            }
            if key.is_none() {
                missing.push("RESEND_API_KEY is unset or empty: set it to a Resend API key");
            }
            bail!("{}; nothing sent", missing.join("; "));
        };
        let url = std::env::var("THIRDSHIFT_RESEND_URL").unwrap_or_else(|_| RESEND_URL.into());
        Ok(Resend {
            key,
            url: url.trim_end_matches('/').to_string(),
            from: settings.from.clone().unwrap_or_else(|| DEFAULT_FROM.into()),
            to,
        })
    }

    /// The sender every email goes out as.
    pub fn from(&self) -> &str {
        &self.from
    }

    /// Send one plain-text email. Resend accepting it is all this can tell:
    /// on a refusal, the error is Resend's own text, word for word.
    pub fn send(&self, subject: &str, text: &str) -> Result<()> {
        let client = reqwest::blocking::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .context("can't set up the HTTPS client")?;
        let response = client
            .post(format!("{}/emails", self.url))
            .bearer_auth(&self.key)
            .json(&json!({
                "from": self.from,
                "to": self.to,
                "subject": subject,
                "text": text,
            }))
            .send()
            .context("can't reach Resend")?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let body = response.text().unwrap_or_default();
        bail!("Resend refused the email ({status}): {}", error_text(&body))
    }
}

/// The error text in a reply Resend refused with: its `message`, or the whole
/// reply if it has none.
fn error_text(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|reply| reply["message"].as_str().map(String::from))
        .unwrap_or_else(|| body.trim().to_string())
}

/// `thirdshift email-test`: send a test email to `to`, else to `email.to`.
pub fn email_test(to: Option<String>, settings: &EmailSettings) -> Result<()> {
    let resend = Resend::new(to, settings)?;
    let host = host::name();
    let host = host.as_deref().unwrap_or("unknown host");
    let time = Local::now().to_rfc3339_opts(SecondsFormat::Secs, false);
    resend.send(
        &format!("thirdshift test email from {host}"),
        &test_text(host, &time, resend.from()),
    )
}

/// The body of the test email.
fn test_text(host: &str, time: &str, from: &str) -> String {
    format!(
        "This is a test email from `thirdshift email-test`. \
         Resend accepted it, and it reached you.\n\n\
         Host:   {host}\n\
         Time:   {time}\n\
         Sender: {from}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_text_is_the_message_or_the_whole_reply() {
        assert_eq!(
            error_text(
                r#"{"statusCode":401,"message":"API key is invalid","name":"validation_error"}"#
            ),
            "API key is invalid"
        );
        assert_eq!(error_text("Bad Gateway\n"), "Bad Gateway");
        assert_eq!(error_text(r#"{"error":"x"}"#), r#"{"error":"x"}"#);
    }

    #[test]
    fn the_test_text_shows_host_time_and_sender() {
        let text = test_text("droplet-1", "2026-09-29T08:00:00-04:00", "a@b.dev");
        for line in [
            "Host:   droplet-1",
            "Time:   2026-09-29T08:00:00-04:00",
            "Sender: a@b.dev",
        ] {
            assert!(text.contains(line), "{text}");
        }
    }
}
