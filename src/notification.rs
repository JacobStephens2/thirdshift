//! The Run notification: one email when a Run, a Spec run or an Architect run
//! ends, whatever its outcome, or when a Pickup run that took an issue does,
//! through the same checks and the same send as `email-test`.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::base_fix::Advice;
use crate::command::Ending;
use crate::config::EmailSettings;
use crate::email::Resend;
use crate::failed_run::FailedRun;
use crate::github;
use crate::host;
use crate::issue::{IssueUrl, Repo};
use crate::launch;
use crate::logs;
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

/// What a Run notification is about, as a command starts.
pub enum About<'a> {
    /// A Run or a Spec run on this issue.
    Issue(&'a IssueUrl),
    /// An Architect run, from the Launch directory.
    ArchitectRun,
    /// A Pickup run, which is about the issue it takes, once it takes one.
    PickupRun,
}

/// What a Run notification's subject names.
enum Subject {
    /// The issue a Run or a Spec run is on, with its title if it is known:
    /// as GitHub gave it when the Run started, or as the Pickup run that
    /// took the issue listed it.
    Issue {
        issue: IssueUrl,
        title: Option<String>,
    },
    /// An Architect run, with the repository the Launch directory's `origin`
    /// names, if it names one.
    ArchitectRun(Option<Repo>),
    /// A Pickup run that has taken no issue yet, and so has nothing to tell.
    Pending,
}

/// A Run notification, checked and waiting for its command to end.
pub struct RunNotification {
    checked: Checked,
    subject: Subject,
}

impl RunNotification {
    /// The Run notification for a command about `about` starting now, sent
    /// to `to`, else to `email.to`. Fails, before any work, with the same
    /// checks as `email-test`: an address is known and a Resend API key is
    /// found. Once they pass, it reads a Run's issue title now, while someone
    /// may be watching, rather than after the Run, when a hung `gh` could
    /// keep the notification from ever going; and an Architect run's
    /// repository.
    pub fn new(to: Option<String>, settings: &EmailSettings, about: About) -> Result<Self> {
        let checked = Checked::new(to, settings)?;
        let subject = match about {
            About::Issue(issue) => Subject::Issue {
                issue: issue.clone(),
                // Left out of the subject if GitHub can't be asked; the Run's
                // own preflight reports why.
                title: github::issue_title(issue).ok(),
            },
            // Left out of the subject if origin names none; the Architect
            // run's own preflight reports why.
            About::ArchitectRun => Subject::ArchitectRun(launch::repo().ok()),
            About::PickupRun => Subject::Pending,
        };
        Ok(RunNotification { checked, subject })
    }

    /// Take note that the Pickup run took `issue`, titled `title`, as its
    /// search listed it: the notification is then the one the Spec run or
    /// Run it is dispatched as would send started by hand, which the Pickup
    /// run sends in its place.
    pub fn took(&mut self, issue: &IssueUrl, title: String) {
        self.subject = Subject::Issue {
            issue: issue.clone(),
            title: Some(title),
        };
    }

    /// Send the one notification for the command that ended as `ending`. A
    /// skipped pass, or a Pickup run that took no issue, has nothing to tell,
    /// and sends none. A failed send is only a warning: it never changes the
    /// command's outcome.
    pub fn send(self, ending: &Ending) {
        let Some(message) = message(&self.subject, ending) else {
            return;
        };
        let host = host::name();
        let command_log = logs::command_log_path();
        let text = message.body(
            command_log.as_deref(),
            host.as_deref().unwrap_or("unknown host"),
            self.checked.started.elapsed(),
        );
        if let Err(error) = self.checked.resend.send(&message.subject, &text) {
            progress::step(format_args!(
                "warning: could not send the Run notification: {error:#}"
            ));
        }
    }
}

/// What a notification about `subject` says of the command that ended as
/// `ending`: none for a skipped pass, nor about a Pickup run that took no
/// issue.
fn message<'a>(subject: &Subject, ending: &'a Ending) -> Option<Message<'a>> {
    let (told, architect) = match ending {
        Ending::Run(ended) => (Told::of(ended), None),
        Ending::Architect { review, dispatched } => {
            let told = match (review, dispatched) {
                (_, Some(dispatched)) => Told::of(dispatched),
                (Ok(reviewed), None) => Told {
                    outcome: reviewed.review(),
                    pr_url: None,
                    cause: None,
                    advice: &[],
                    base_fix: None,
                    log: None,
                    tickets: &[],
                },
                (Err(failed), None) => Told {
                    outcome: failure_outcome(failed, "review failed"),
                    ..Told::of_failure(failed)
                },
            };
            let review = match review {
                Ok(reviewed) => format!("{}: {}", reviewed.review(), reviewed.url()),
                Err(failed) => failure_outcome(failed, "failed").to_string(),
            };
            let lines = ArchitectLines {
                review,
                dispatched: dispatched.as_ref().map(|_| told.outcome),
            };
            (told, Some(lines))
        }
        Ending::Skipped(_) => return None,
    };
    let subject = match subject {
        Subject::Issue { issue, title } => issue_subject(issue, title.as_deref(), told.outcome),
        Subject::ArchitectRun(repo) => architect_subject(repo.as_ref(), told.outcome),
        Subject::Pending => return None,
    };
    Some(Message {
        subject,
        architect,
        told,
    })
}

/// A notification's subject, and what its body tells.
struct Message<'a> {
    subject: String,
    /// In an Architect run's, what it says first.
    architect: Option<ArchitectLines>,
    told: Told<'a>,
}

impl Message<'_> {
    /// The plain-text body, naming `command_log`, if the command keeps one,
    /// and `host`, for a command that `took` so long.
    fn body(&self, command_log: Option<&Path>, host: &str, took: Duration) -> String {
        let told = &self.told;
        Body {
            architect: self.architect.clone(),
            pr_url: told.pr_url,
            cause: told.cause.as_deref(),
            advice: told.advice,
            base_fix: told.base_fix,
            log: told.log,
            command_log,
            host,
            took,
            tickets: told.tickets,
        }
        .text()
    }
}

/// How a Run, a Spec run or an Architecture review ended, as a Run
/// notification tells it.
struct Told<'a> {
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

impl<'a> Told<'a> {
    fn of(ended: &'a Ended) -> Self {
        let base_fix = ended.base_fix.as_deref();
        match &ended.outcome {
            Ok(reached) => Told {
                outcome: reached.goal.outcome(),
                pr_url: Some(&reached.pr_url),
                cause: None,
                advice: &[],
                base_fix,
                log: reached.log.as_deref(),
                tickets: &reached.ticket_lines,
            },
            Err(failed) => Told {
                advice: &ended.advice,
                base_fix,
                ..Told::of_failure(failed)
            },
        }
    }

    fn of_failure(failed: &'a FailedRun) -> Self {
        Told {
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

/// `[thirdshift] <owner>/<repo>#<n> <title>: <outcome>`, without the title
/// if it isn't known.
fn issue_subject(issue: &IssueUrl, title: Option<&str>, outcome: &str) -> String {
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
#[derive(Clone)]
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
    use std::path::PathBuf;

    use anyhow::anyhow;

    use super::*;
    use crate::architect::Reviewed;
    use crate::command::Skip;
    use crate::run::{Goal, Reached};

    const PR: &str = "https://github.com/acme/widgets/pull/1";
    const PLAN: &str = "https://github.com/acme/widgets/issues/8";
    const LOG: &str = "/logs/x.jsonl";

    fn issue() -> IssueUrl {
        IssueUrl::parse("https://github.com/acme/widgets/issues/123").unwrap()
    }

    fn about_issue() -> Subject {
        Subject::Issue {
            issue: issue(),
            title: Some("Add export button".to_string()),
        }
    }

    fn about_architect_run() -> Subject {
        Subject::ArchitectRun(Repo::of_origin("https://github.com/acme/widgets.git"))
    }

    fn merged() -> Ended {
        Ended {
            outcome: Ok(Reached {
                pr_url: PR.to_string(),
                goal: Goal::Merged,
                log: Some(PathBuf::from(LOG)),
                ticket_lines: Vec::new(),
            }),
            base_fix: None,
            advice: Vec::new(),
        }
    }

    fn failed_run(cause: &str) -> FailedRun {
        FailedRun {
            error: anyhow!("{cause}"),
            pr_url: None,
            log: Some(PathBuf::from(LOG)),
            interrupted: false,
            ticket_lines: Vec::new(),
        }
    }

    fn plan() -> Reviewed {
        Reviewed::PlanReady {
            plan: IssueUrl::parse(PLAN).unwrap(),
            base: "main".to_string(),
        }
    }

    /// The subject and body of the notification about `subject` for a
    /// command that ended as `ending`, if it sends one.
    fn told(subject: &Subject, ending: &Ending) -> Option<(String, String)> {
        message(subject, ending).map(|message| {
            let body = message.body(None, "droplet-1", Duration::from_secs(4));
            (message.subject, body)
        })
    }

    #[test]
    fn a_runs_notification_tells_how_the_run_ended() {
        assert_eq!(
            told(&about_issue(), &Ending::Run(merged())),
            Some((
                "[thirdshift] acme/widgets#123 Add export button: merged".to_string(),
                format!(
                    "Pull request: {PR}\n\
                     Session log:  {LOG}\n\
                     Host:         droplet-1\n\
                     Took:         4s\n"
                )
            ))
        );
        let ended = Ended {
            outcome: Err(failed_run("claude exited 1")),
            base_fix: Some(format!("{PLAN} not merged")),
            advice: Vec::new(),
        };
        assert_eq!(
            told(&about_issue(), &Ending::Run(ended)),
            Some((
                "[thirdshift] acme/widgets#123 Add export button: failed".to_string(),
                format!(
                    "Cause:        claude exited 1\n\
                     Base fix:     {PLAN} not merged\n\
                     Session log:  {LOG}\n\
                     Host:         droplet-1\n\
                     Took:         4s\n"
                )
            ))
        );
    }

    #[test]
    fn an_architect_runs_notification_with_its_plan_dispatched_tells_how_both_ended() {
        let ending = Ending::Architect {
            review: Ok(plan()),
            dispatched: Some(merged()),
        };
        assert_eq!(
            told(&about_architect_run(), &ending),
            Some((
                "[thirdshift] acme/widgets Architect run: merged".to_string(),
                format!(
                    "Review:       plan published: {PLAN}\n\
                     Dispatched:   merged\n\
                     Pull request: {PR}\n\
                     Session log:  {LOG}\n\
                     Host:         droplet-1\n\
                     Took:         4s\n"
                )
            ))
        );
    }

    #[test]
    fn an_architect_runs_notification_with_nothing_dispatched_tells_how_its_review_ended() {
        let ending = Ending::Architect {
            review: Ok(Reviewed::IdeaFiled(IssueUrl::parse(PLAN).unwrap())),
            dispatched: None,
        };
        assert_eq!(
            told(&about_architect_run(), &ending),
            Some((
                "[thirdshift] acme/widgets Architect run: idea filed".to_string(),
                format!(
                    "Review:       idea filed: {PLAN}\n\
                     Host:         droplet-1\n\
                     Took:         4s\n"
                )
            ))
        );
        let ending = Ending::Architect {
            review: Ok(plan()),
            dispatched: None,
        };
        assert_eq!(
            told(&about_architect_run(), &ending).map(|(subject, _)| subject),
            Some("[thirdshift] acme/widgets Architect run: plan published".to_string())
        );
    }

    #[test]
    fn an_architect_runs_notification_tells_how_its_review_failed() {
        let ending = Ending::Architect {
            review: Err(failed_run("claude exited 1")),
            dispatched: None,
        };
        assert_eq!(
            told(&Subject::ArchitectRun(None), &ending),
            Some((
                "[thirdshift] Architect run: review failed".to_string(),
                format!(
                    "Review:       failed\n\
                     Cause:        claude exited 1\n\
                     Session log:  {LOG}\n\
                     Host:         droplet-1\n\
                     Took:         4s\n"
                )
            ))
        );
    }

    #[test]
    fn a_skipped_pass_or_a_pickup_run_that_took_nothing_has_no_notification() {
        let skipped = Ending::Skipped(Skip {
            reason: "no Ready issue on acme/widgets".to_string(),
            urls: Vec::new(),
        });
        for subject in [about_issue(), about_architect_run(), Subject::Pending] {
            assert_eq!(told(&subject, &skipped), None);
        }
        assert_eq!(told(&Subject::Pending, &Ending::Run(merged())), None);
    }

    #[test]
    fn the_subject_names_the_issue_its_title_if_known_and_the_outcome() {
        assert_eq!(
            issue_subject(&issue(), Some("Add export button"), "ready for review"),
            "[thirdshift] acme/widgets#123 Add export button: ready for review"
        );
        assert_eq!(
            issue_subject(&issue(), None, "failed"),
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
