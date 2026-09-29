//! The Run notification: one email when a Run ends, whatever its outcome,
//! through the same checks and the same send as `email-test`.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::config::EmailSettings;
use crate::email::Resend;
use crate::failed_run::FailedRun;
use crate::github;
use crate::host;
use crate::issue::IssueUrl;
use crate::progress;
use crate::run::{Goal, Reached};

/// What a Run asks about its Run notification, by its command or, without
/// `email` or `no-email`, by the User config.
#[derive(Debug, PartialEq, Eq)]
pub enum NotificationAsk {
    /// Send one, to this address, else to `email.to`.
    Send(Option<String>),
    /// Send none.
    Skip,
}

/// A Run notification, checked and waiting for the Run to end.
pub struct RunNotification {
    resend: Resend,
    issue: IssueUrl,
    /// The issue's title, if GitHub gave it when the Run started.
    title: Option<String>,
    started: Instant,
}

impl RunNotification {
    /// The Run notification for a Run on `issue` starting now, sent to `to`,
    /// else to `email.to`. Fails, before any work, with the same checks as
    /// `email-test`: an address is known and `RESEND_API_KEY` is set. Once
    /// they pass, it reads the issue's title now, while someone may be
    /// watching, rather than after the Run, when a hung `gh` could keep the
    /// notification from ever going.
    pub fn new(to: Option<String>, settings: &EmailSettings, issue: &IssueUrl) -> Result<Self> {
        let started = Instant::now();
        let resend = Resend::new(to, settings)?;
        Ok(RunNotification {
            resend,
            issue: issue.clone(),
            // Left out of the subject if GitHub can't be asked; the Run's own
            // preflight reports why.
            title: github::issue_title(issue).ok(),
            started,
        })
    }

    /// Send the notification for the Run that `ended`, whose goal was `goal`.
    /// A failed send is only a warning: it never changes the Run's outcome.
    pub fn send(self, ended: &Result<Reached, FailedRun>, goal: Goal) {
        let (outcome, pr_url, cause, log) = match ended {
            Ok(reached) => (
                goal.outcome(),
                Some(reached.pr_url.as_str()),
                None,
                Some(reached.log.as_path()),
            ),
            Err(failed) => {
                let (outcome, cause) = if failed.interrupted {
                    ("interrupted", None)
                } else {
                    ("failed", Some(format!("{:#}", failed.error)))
                };
                (
                    outcome,
                    failed.pr_url.as_deref(),
                    cause,
                    failed.log.as_deref(),
                )
            }
        };
        let host = host::name();
        let body = Body {
            pr_url,
            cause: cause.as_deref(),
            log,
            host: host.as_deref().unwrap_or("unknown host"),
            took: self.started.elapsed(),
        };
        let subject = subject(&self.issue, self.title.as_deref(), outcome);
        if let Err(error) = self.resend.send(&subject, &body.text()) {
            progress::step(format_args!(
                "warning: could not send the Run notification: {error:#}"
            ));
        }
    }
}

/// `[thirdshift] <owner>/<repo>#<n> <title>: <outcome>`, without the title
/// if it isn't known.
fn subject(issue: &IssueUrl, title: Option<&str>, outcome: &str) -> String {
    let title = title.map(|title| format!(" {title}")).unwrap_or_default();
    format!(
        "[thirdshift] {}#{}{title}: {outcome}",
        issue.repo_slug(),
        issue.number
    )
}

/// What the notification's plain-text body says.
struct Body<'a> {
    pr_url: Option<&'a str>,
    cause: Option<&'a str>,
    log: Option<&'a Path>,
    host: &'a str,
    took: Duration,
}

impl Body<'_> {
    fn text(&self) -> String {
        let mut text = String::new();
        if let Some(pr_url) = self.pr_url {
            text += &format!("Pull request: {pr_url}\n");
        }
        if let Some(cause) = self.cause {
            text += &format!("Cause:        {cause}\n");
        }
        if let Some(log) = self.log {
            text += &format!("Session log:  {}\n", log.display());
        }
        text += &format!("Host:         {}\n", self.host);
        text += &format!("Took:         {}\n", took(self.took));
        text
    }
}

/// `duration` to the second, as in `1h 2m 3s`, `2m 3s` or `3s`.
fn took(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue() -> IssueUrl {
        IssueUrl::parse("https://github.com/acme/widgets/issues/123").unwrap()
    }

    #[test]
    fn the_subject_names_the_issue_its_title_if_known_and_the_outcome() {
        assert_eq!(
            subject(&issue(), Some("Add export button"), "ready for review"),
            "[thirdshift] acme/widgets#123 Add export button: ready for review"
        );
        assert_eq!(
            subject(&issue(), None, "failed"),
            "[thirdshift] acme/widgets#123: failed"
        );
    }

    #[test]
    fn the_body_leaves_out_what_the_run_does_not_have() {
        let body = Body {
            pr_url: None,
            cause: Some("origin mismatch"),
            log: None,
            host: "droplet-1",
            took: Duration::from_secs(4),
        };
        assert_eq!(
            body.text(),
            "Cause:        origin mismatch\n\
             Host:         droplet-1\n\
             Took:         4s\n"
        );
        let body = Body {
            pr_url: Some("https://github.com/acme/widgets/pull/1"),
            cause: None,
            log: Some(Path::new("/home/me/.thirdshift/logs/x.jsonl")),
            ..body
        };
        assert_eq!(
            body.text(),
            "Pull request: https://github.com/acme/widgets/pull/1\n\
             Session log:  /home/me/.thirdshift/logs/x.jsonl\n\
             Host:         droplet-1\n\
             Took:         4s\n"
        );
    }

    #[test]
    fn took_is_to_the_second() {
        for (seconds, text) in [
            (0, "0s"),
            (59, "59s"),
            (60, "1m 0s"),
            (3599, "59m 59s"),
            (3723, "1h 2m 3s"),
        ] {
            assert_eq!(took(Duration::from_secs(seconds)), text);
        }
    }
}
