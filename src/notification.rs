//! The Run notification: one email when a Run, a Spec run or an Architect run
//! ends, whatever its outcome, or when a Pickup run that took an issue does,
//! through the same checks and the same send as `email-test`.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::architect::Reviewed;
use crate::base_fix::Advice;
use crate::command_log;
use crate::config::EmailSettings;
use crate::email::Resend;
use crate::failed_run::FailedRun;
use crate::github;
use crate::host;
use crate::issue::{IssueUrl, Repo};
use crate::launch;
use crate::progress;
use crate::run::Ended;

/// What a Run, an Architect run or a Pickup run asks about its Run
/// notification, by its command or, without `email` or `no-email`, by the
/// User config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationAsk {
    /// Send one, to this address, else to `email.to`.
    Send(Option<String>),
    /// Send none.
    Skip,
}

/// A Run notification's checks, passed, and when they were made: what every
/// run that asks for one holds from before any work until it ends.
struct Checked {
    resend: Resend,
    started: Instant,
}

impl Checked {
    /// The checks of a Run notification sent to `to`, else to `email.to`,
    /// for a run starting now. Fails with the same checks as `email-test`: an
    /// address is known and a Resend API key is found.
    fn new(to: Option<String>, settings: &EmailSettings) -> Result<Self> {
        let started = Instant::now();
        let resend = Resend::new(to, settings)?;
        Ok(Checked { resend, started })
    }
}

/// A Run notification, checked and waiting for the Run or Spec run on its
/// issue to end.
pub struct RunNotification {
    checked: Checked,
    issue: IssueUrl,
    /// The issue's title, if it is known: as GitHub gave it when the Run
    /// started, or as the Pickup run that took the issue listed it.
    title: Option<String>,
}

impl RunNotification {
    /// The Run notification for a Run on `issue` starting now, sent to `to`,
    /// else to `email.to`. Fails, before any work, with the same checks as
    /// `email-test`: an address is known and `RESEND_API_KEY` is set. Once
    /// they pass, it reads the issue's title now, while someone may be
    /// watching, rather than after the Run, when a hung `gh` could keep the
    /// notification from ever going.
    pub fn new(to: Option<String>, settings: &EmailSettings, issue: &IssueUrl) -> Result<Self> {
        let checked = Checked::new(to, settings)?;
        Ok(RunNotification {
            checked,
            issue: issue.clone(),
            // Left out of the subject if GitHub can't be asked; the Run's own
            // preflight reports why.
            title: github::issue_title(issue).ok(),
        })
    }

    /// Send the notification for the Run that `ended`, with what became of
    /// the Base fix it started or waited on, if any. A failed send is only a
    /// warning: it never changes the Run's outcome.
    pub fn send(self, ended: &Ended) {
        let ending = Ending::of(ended);
        let subject = subject(&self.issue, self.title.as_deref(), ending.outcome);
        send(&self.checked, &subject, None, &ending);
    }
}

/// A Pickup run's Run notification, checked and waiting for the issue the
/// Pickup run takes. One that is skipped takes none, and so sends none.
pub struct PickupNotification(Checked);

impl PickupNotification {
    /// The Run notification for a Pickup run starting now, sent to `to`, else
    /// to `email.to`. Fails, before any work, with the checks
    /// [`RunNotification::new`] makes.
    pub fn new(to: Option<String>, settings: &EmailSettings) -> Result<Self> {
        Checked::new(to, settings).map(PickupNotification)
    }

    /// The Run notification of the Spec run or Run that `issue`, the Ready
    /// issue the Pickup run took, titled `title`, is dispatched as: the one
    /// that run would send started by hand, which the Pickup run sends in
    /// its place.
    pub fn of_taken(self, issue: &IssueUrl, title: String) -> RunNotification {
        RunNotification {
            checked: self.0,
            issue: issue.clone(),
            title: Some(title),
        }
    }
}

/// An Architect run's Run notification, checked and waiting for the Architect
/// run to end.
pub struct ArchitectNotification {
    checked: Checked,
    /// The repository the Launch directory's `origin` names, if it names one.
    repo: Option<Repo>,
}

impl ArchitectNotification {
    /// The Run notification for an Architect run starting now from the Launch
    /// directory, sent to `to`, else to `email.to`. Fails, before any work,
    /// with the checks [`RunNotification::new`] makes.
    pub fn new(to: Option<String>, settings: &EmailSettings) -> Result<Self> {
        let checked = Checked::new(to, settings)?;
        Ok(ArchitectNotification {
            checked,
            // Left out of the subject if origin names none; the Architect
            // run's own preflight reports why.
            repo: launch::repo().ok(),
        })
    }

    /// Send the one notification for the Architect run whose Architecture
    /// review `ended` so, and, if its plan was dispatched, whose Spec run or
    /// Run `dispatched`, with what became of the Base fix that took, if any.
    /// A skipped Architect run has no review, and sends none. A failed send
    /// is only a warning: it never changes the Architect run's outcome.
    pub fn send(self, ended: Result<&Reviewed, &FailedRun>, dispatched: Option<&Ended>) {
        let ending = match (ended, dispatched) {
            (_, Some(dispatched)) => Ending::of(dispatched),
            (Ok(reviewed), None) => Ending {
                outcome: reviewed.review(),
                pr_url: None,
                cause: None,
                advice: &[],
                base_fix: None,
                log: None,
                tickets: &[],
            },
            (Err(failed), None) => Ending {
                outcome: failure_outcome(failed, "review failed"),
                ..Ending::of_failure(failed)
            },
        };
        let review = match ended {
            Ok(reviewed) => format!("{}: {}", reviewed.review(), reviewed.url()),
            Err(failed) => failure_outcome(failed, "failed").to_string(),
        };
        let lines = ArchitectLines {
            review,
            dispatched: dispatched.map(|_| ending.outcome),
        };
        let subject = architect_subject(self.repo.as_ref(), ending.outcome);
        send(&self.checked, &subject, Some(lines), &ending);
    }
}

/// How a Run, a Spec run or an Architecture review ended, as a Run
/// notification tells it.
struct Ending<'a> {
    outcome: &'static str,
    pr_url: Option<&'a str>,
    cause: Option<String>,
    /// What the Run says after its cause, if Inherited failures failed it
    /// with no Base fix taken.
    advice: &'a [Advice],
    /// What became of the Base fix the Run started or waited on, if any.
    base_fix: Option<&'a str>,
    log: Option<&'a Path>,
    /// In a Spec run, a line on each Ticket, as in its summary on stderr.
    tickets: &'a [String],
}

impl<'a> Ending<'a> {
    fn of(ended: &'a Ended) -> Self {
        let base_fix = ended.base_fix.as_deref();
        match &ended.outcome {
            Ok(reached) => Ending {
                outcome: reached.goal.outcome(),
                pr_url: Some(&reached.pr_url),
                cause: None,
                advice: &[],
                base_fix,
                log: reached.log.as_deref(),
                tickets: &reached.ticket_lines,
            },
            Err(failed) => Ending {
                advice: &ended.advice,
                base_fix,
                ..Ending::of_failure(failed)
            },
        }
    }

    fn of_failure(failed: &'a FailedRun) -> Self {
        Ending {
            outcome: failure_outcome(failed, "failed"),
            pr_url: failed.pr_url.as_deref(),
            cause: (!failed.interrupted).then(|| format!("{:#}", failed.error)),
            advice: &[],
            base_fix: None,
            log: failed.log.as_deref(),
            tickets: &failed.ticket_lines,
        }
    }
}

/// `interrupted`, or `failure` for a failure that was not an interrupt.
fn failure_outcome(failed: &FailedRun, failure: &'static str) -> &'static str {
    if failed.interrupted {
        "interrupted"
    } else {
        failure
    }
}

/// Send the notification with `subject` for what started when `checked` was
/// and ended as `ending`, in an Architect run with its `architect` lines
/// first. A failed send is only a warning.
fn send(checked: &Checked, subject: &str, architect: Option<ArchitectLines>, ending: &Ending) {
    let host = host::name();
    let command_log = command_log::path();
    let body = Body {
        architect,
        pr_url: ending.pr_url,
        cause: ending.cause.as_deref(),
        advice: ending.advice,
        base_fix: ending.base_fix,
        log: ending.log,
        command_log: command_log.as_deref(),
        host: host.as_deref().unwrap_or("unknown host"),
        took: checked.started.elapsed(),
        tickets: ending.tickets,
    };
    if let Err(error) = checked.resend.send(subject, &body.text()) {
        progress::step(format_args!(
            "warning: could not send the Run notification: {error:#}"
        ));
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

/// `[thirdshift] <owner>/<repo> Architect run: <outcome>`, without the
/// repository if it isn't known.
fn architect_subject(repo: Option<&Repo>, outcome: &str) -> String {
    let repo = repo
        .map(|repo| format!(" {}", repo.slug()))
        .unwrap_or_default();
    format!("[thirdshift]{repo} Architect run: {outcome}")
}

/// What an Architect run's notification says before what a Run's does.
struct ArchitectLines {
    /// How the Architecture review ended, with the issue it ended on.
    review: String,
    /// How the Spec run or Run the plan was dispatched as ended, if it was.
    dispatched: Option<&'static str>,
}

/// What the notification's plain-text body says.
struct Body<'a> {
    architect: Option<ArchitectLines>,
    pr_url: Option<&'a str>,
    cause: Option<&'a str>,
    /// What the Run says after its cause, if Inherited failures failed it
    /// with no Base fix taken.
    advice: &'a [Advice],
    /// What became of the Base fix the Run started or waited on, if any.
    base_fix: Option<&'a str>,
    log: Option<&'a Path>,
    /// The command's Command log, if it keeps one.
    command_log: Option<&'a Path>,
    host: &'a str,
    took: Duration,
    /// In a Spec run, a line on each Ticket, as in its summary on stderr.
    tickets: &'a [String],
}

impl Body<'_> {
    fn text(&self) -> String {
        let mut text = String::new();
        if let Some(ArchitectLines { review, dispatched }) = &self.architect {
            text += &format!("Review:       {review}\n");
            if let Some(dispatched) = dispatched {
                text += &format!("Dispatched:   {dispatched}\n");
            }
        }
        if let Some(pr_url) = self.pr_url {
            text += &format!("Pull request: {pr_url}\n");
        }
        if let Some(cause) = self.cause {
            text += &format!("Cause:        {cause}\n");
        }
        for line in self.advice {
            text += &format!("{:<14}{}\n", format!("{}:", line.label), line.value);
        }
        if let Some(base_fix) = self.base_fix {
            text += &format!("Base fix:     {base_fix}\n");
        }
        if let Some(log) = self.log {
            text += &format!("Session log:  {}\n", log.display());
        }
        if let Some(log) = self.command_log {
            text += &format!("Command log:  {}\n", log.display());
        }
        text += &format!("Host:         {}\n", self.host);
        text += &format!("Took:         {}\n", took(self.took));
        if !self.tickets.is_empty() {
            text += "\nTickets:\n";
            for line in self.tickets {
                text += &format!("{line}\n");
            }
        }
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
            architect: None,
            pr_url: None,
            cause: Some("origin mismatch"),
            advice: &[],
            base_fix: None,
            log: None,
            command_log: None,
            host: "droplet-1",
            took: Duration::from_secs(4),
            tickets: &[],
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
            log: Some(Path::new("/home/me/.thirdshift/logs/sessions/x.jsonl")),
            command_log: Some(Path::new("/home/me/.thirdshift/logs/commands/issue/x.log")),
            ..body
        };
        assert_eq!(
            body.text(),
            "Pull request: https://github.com/acme/widgets/pull/1\n\
             Session log:  /home/me/.thirdshift/logs/sessions/x.jsonl\n\
             Command log:  /home/me/.thirdshift/logs/commands/issue/x.log\n\
             Host:         droplet-1\n\
             Took:         4s\n"
        );
    }

    #[test]
    fn the_body_reports_the_base_fix_after_the_cause() {
        let body = Body {
            architect: None,
            pr_url: Some("https://github.com/acme/widgets/pull/1"),
            cause: Some("claude exited 1"),
            advice: &[],
            base_fix: Some("https://github.com/acme/widgets/issues/8 merged"),
            log: None,
            command_log: None,
            host: "droplet-1",
            took: Duration::from_secs(4),
            tickets: &[],
        };
        assert_eq!(
            body.text(),
            "Pull request: https://github.com/acme/widgets/pull/1\n\
             Cause:        claude exited 1\n\
             Base fix:     https://github.com/acme/widgets/issues/8 merged\n\
             Host:         droplet-1\n\
             Took:         4s\n"
        );
    }

    #[test]
    fn the_body_gives_the_advice_after_the_cause_laid_out_like_its_other_lines() {
        let advice = [
            Advice {
                label: "Base check",
                value: "test: https://ci.example/main/test".to_string(),
            },
            Advice {
                label: "Or set",
                value: "base.fix = true".to_string(),
            },
        ];
        let body = Body {
            architect: None,
            pr_url: Some("https://github.com/acme/widgets/pull/1"),
            cause: Some("CI red on test, which also fails on main at 362b9ca; fix main first"),
            advice: &advice,
            base_fix: None,
            log: Some(Path::new("/home/me/.thirdshift/logs/x.jsonl")),
            command_log: None,
            host: "droplet-1",
            took: Duration::from_secs(4),
            tickets: &[],
        };
        assert_eq!(
            body.text(),
            "Pull request: https://github.com/acme/widgets/pull/1\n\
             Cause:        CI red on test, which also fails on main at 362b9ca; fix main first\n\
             Base check:   test: https://ci.example/main/test\n\
             Or set:       base.fix = true\n\
             Session log:  /home/me/.thirdshift/logs/x.jsonl\n\
             Host:         droplet-1\n\
             Took:         4s\n"
        );
    }

    #[test]
    fn a_spec_runs_body_ends_with_a_line_per_ticket() {
        let tickets = [
            "#21 failed: claude exited 1".to_string(),
            "#22 blocked by #21".to_string(),
        ];
        let body = Body {
            architect: None,
            pr_url: None,
            cause: Some("Tickets not done: #21, #22"),
            advice: &[],
            base_fix: None,
            log: None,
            command_log: None,
            host: "droplet-1",
            took: Duration::from_secs(4),
            tickets: &tickets,
        };
        assert_eq!(
            body.text(),
            "Cause:        Tickets not done: #21, #22\n\
             Host:         droplet-1\n\
             Took:         4s\n\
             \n\
             Tickets:\n\
             #21 failed: claude exited 1\n\
             #22 blocked by #21\n"
        );
    }

    #[test]
    fn an_architect_runs_subject_names_the_repository_if_known_and_the_outcome() {
        let repo = Repo::of_origin("https://github.com/acme/widgets.git").unwrap();
        assert_eq!(
            architect_subject(Some(&repo), "plan published"),
            "[thirdshift] acme/widgets Architect run: plan published"
        );
        assert_eq!(
            architect_subject(None, "review failed"),
            "[thirdshift] Architect run: review failed"
        );
    }

    #[test]
    fn an_architect_runs_body_starts_with_the_review_and_the_dispatched_runs_outcome() {
        let body = Body {
            architect: Some(ArchitectLines {
                review: "plan published: https://github.com/acme/widgets/issues/8".to_string(),
                dispatched: Some("merged"),
            }),
            pr_url: Some("https://github.com/acme/widgets/pull/1"),
            cause: None,
            advice: &[],
            base_fix: None,
            log: None,
            command_log: None,
            host: "droplet-1",
            took: Duration::from_secs(4),
            tickets: &[],
        };
        assert_eq!(
            body.text(),
            "Review:       plan published: https://github.com/acme/widgets/issues/8\n\
             Dispatched:   merged\n\
             Pull request: https://github.com/acme/widgets/pull/1\n\
             Host:         droplet-1\n\
             Took:         4s\n"
        );
        let body = Body {
            architect: Some(ArchitectLines {
                review: "idea filed: https://github.com/acme/widgets/issues/8".to_string(),
                dispatched: None,
            }),
            pr_url: None,
            ..body
        };
        assert_eq!(
            body.text(),
            "Review:       idea filed: https://github.com/acme/widgets/issues/8\n\
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
