//! The Run notification: one email when a Run, a Spec run or a Pass ends,
//! whatever its outcome, unless the Pass was skipped,
//! through the same checks and the same send as `email-test`.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::config::EmailSettings;
use crate::email::Resend;
use crate::github::GitHub;
use crate::harness::Choice;
use crate::host;
use crate::issue::{IssueUrl, Repo};
use crate::launch;
use crate::logs;
use crate::progress;
use crate::run_ending::Account;

/// What a Run or a Pass asks about its Run
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
    /// The line that says what the command's sessions ran on, once known.
    built_with: Option<String>,
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
                title: GitHub::new().issue_title(issue).ok(),
            },
            // Left out of the subject if origin names none; the Architect
            // run's own preflight reports why.
            About::ArchitectRun => Subject::ArchitectRun(launch::repo().ok()),
            About::PickupRun => Subject::Pending,
        };
        Ok(RunNotification {
            checked,
            subject,
            built_with: None,
        })
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

    /// Take note that the command's sessions run on `harness`, as the
    /// notification then says, as its pull request's body does.
    pub fn built_with(&mut self, harness: &Choice) {
        self.built_with = Some(harness.built_with());
    }

    /// Send the one notification for the command that ended as `account`
    /// tells. A Pickup run that took no issue has nothing to tell, and sends
    /// none, as a skipped pass, which has no account, does not either. A
    /// failed send is only a warning: it never changes the command's
    /// outcome.
    pub fn send(self, account: &Account) {
        let Some(subject) = subject_line(&self.subject, account.outcome) else {
            return;
        };
        let host = host::name();
        let command_log = logs::command_log_path();
        let text = body(
            account,
            self.built_with.as_deref(),
            command_log.as_deref(),
            host.as_deref().unwrap_or("unknown host"),
            self.checked.started.elapsed(),
        );
        if let Err(error) = self.checked.resend.send(&subject, &text) {
            progress::step(format_args!(
                "warning: could not send the Run notification: {error:#}"
            ));
        }
    }
}

/// The subject of a notification about `subject` for a command whose
/// outcome is `outcome`: none about a Pickup run that took no issue.
fn subject_line(subject: &Subject, outcome: &str) -> Option<String> {
    match subject {
        Subject::Issue { issue, title } => Some(issue_subject(issue, title.as_deref(), outcome)),
        Subject::ArchitectRun(repo) => Some(architect_subject(repo.as_ref(), outcome)),
        Subject::Pending => None,
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

/// The plain-text body of the notification for a command that ended as
/// `account` tells, with the line that says what its sessions were
/// `built_with`, if known, naming `command_log`, if the command keeps one,
/// and `host`, for a command that `took` so long. An Architect run's starts
/// with how its review ended, and the outcome of the run it dispatched, if
/// any. The cause is left out of an interrupted command's, whose outcome says
/// so.
fn body(
    account: &Account,
    built_with: Option<&str>,
    command_log: Option<&Path>,
    host: &str,
    took: Duration,
) -> String {
    let mut text = String::new();
    if let Some(review) = &account.review {
        text += &format!("Review:       {}\n", review.line);
        if review.dispatched.is_some() {
            text += &format!("Dispatched:   {}\n", account.outcome);
        }
    }
    if let Some(pr_url) = account.pr_url {
        text += &format!("Pull request: {pr_url}\n");
    }
    if let Err(cause) = &account.ended
        && !account.interrupted
    {
        text += &format!("Cause:        {}\n", cause.full());
    }
    for line in account.advice {
        text += &format!("{:<14}{}\n", format!("{}:", line.label), line.value);
    }
    if let Some(base_fix) = account.base_fix {
        text += &format!("Base fix:     {base_fix}\n");
    }
    if let Some(log) = account.log {
        text += &format!("Session log:  {}\n", log.display());
    }
    if let Some(log) = command_log {
        text += &format!("Command log:  {}\n", log.display());
    }
    if let Some(built_with) = built_with {
        text += &format!("{built_with}\n");
    }
    text += &format!("Host:         {host}\n");
    text += &format!("Took:         {}\n", self::took(took));
    if !account.ticket_lines.is_empty() {
        text += "\nTickets:\n";
        for line in account.ticket_lines {
            text += &format!("{line}\n");
        }
    }
    text
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
    use anyhow::anyhow;

    use super::*;
    use crate::base_fix::Advice;
    use crate::run_ending::{Cause, Review};

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

    /// A merged Run's account.
    fn merged() -> Account<'static> {
        Account {
            outcome: "merged",
            ended: Ok(format!("PR {PR} is merged")),
            interrupted: false,
            pr_url: Some(PR),
            advice: &[],
            base_fix: None,
            log: Some(Path::new(LOG)),
            ticket_lines: &[],
            review: None,
            urls: vec![PR],
        }
    }

    /// The account of a Run that failed as `cause`, with nothing else to
    /// tell.
    fn failed(cause: &str) -> Account<'static> {
        Account {
            outcome: "failed",
            ended: Err(Cause::of(&anyhow!("{cause}"))),
            interrupted: false,
            pr_url: None,
            advice: &[],
            base_fix: None,
            log: None,
            ticket_lines: &[],
            review: None,
            urls: Vec::new(),
        }
    }

    /// The subject and body of the notification about `subject` for a
    /// command that ended as `account` tells, if it sends one.
    fn told(subject: &Subject, account: &Account) -> Option<(String, String)> {
        subject_line(subject, account.outcome).map(|subject| (subject, body_of(account)))
    }

    /// The body for a command that ended as `account` tells, run on
    /// `droplet-1` for 4s with no Command log.
    fn body_of(account: &Account) -> String {
        body(account, None, None, "droplet-1", Duration::from_secs(4))
    }

    #[test]
    fn a_runs_notification_tells_how_the_run_ended() {
        assert_eq!(
            told(&about_issue(), &merged()),
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
        let base_fix = format!("{PLAN} not merged");
        let account = Account {
            base_fix: Some(&base_fix),
            log: Some(Path::new(LOG)),
            ..failed("claude exited 1")
        };
        assert_eq!(
            told(&about_issue(), &account),
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
    fn an_interrupted_runs_notification_says_so_and_leaves_out_the_cause() {
        let account = Account {
            outcome: "interrupted",
            interrupted: true,
            ..failed("interrupted")
        };
        assert_eq!(
            told(&about_issue(), &account),
            Some((
                "[thirdshift] acme/widgets#123 Add export button: interrupted".to_string(),
                "Host:         droplet-1\n\
                 Took:         4s\n"
                    .to_string()
            ))
        );
    }

    #[test]
    fn an_architect_runs_notification_with_its_plan_dispatched_tells_how_both_ended() {
        let account = Account {
            review: Some(Review {
                line: format!("plan published: {PLAN}"),
                dispatched: Some(PLAN),
            }),
            ..merged()
        };
        assert_eq!(
            told(&about_architect_run(), &account),
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
        let account = Account {
            outcome: "idea filed",
            ended: Ok(format!(
                "no Strong candidate: the Architecture review filed the idea {PLAN}"
            )),
            pr_url: None,
            log: None,
            review: Some(Review {
                line: format!("idea filed: {PLAN}"),
                dispatched: None,
            }),
            urls: vec![PLAN],
            ..merged()
        };
        assert_eq!(
            told(&about_architect_run(), &account),
            Some((
                "[thirdshift] acme/widgets Architect run: idea filed".to_string(),
                format!(
                    "Review:       idea filed: {PLAN}\n\
                     Host:         droplet-1\n\
                     Took:         4s\n"
                )
            ))
        );
    }

    #[test]
    fn an_architect_runs_notification_tells_how_its_review_failed() {
        let account = Account {
            outcome: "review failed",
            log: Some(Path::new(LOG)),
            review: Some(Review {
                line: "failed".to_string(),
                dispatched: None,
            }),
            ..failed("claude exited 1")
        };
        assert_eq!(
            told(&Subject::ArchitectRun(None), &account),
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
    fn a_pickup_run_that_took_nothing_has_no_notification() {
        assert_eq!(told(&Subject::Pending, &merged()), None);
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
        assert_eq!(
            body_of(&failed("origin mismatch")),
            "Cause:        origin mismatch\n\
             Host:         droplet-1\n\
             Took:         4s\n"
        );
        let account = Account {
            log: Some(Path::new("/home/me/.thirdshift/logs/sessions/x.jsonl")),
            ..merged()
        };
        assert_eq!(
            body(
                &account,
                Some("Built with claude · opus · high"),
                Some(Path::new("/home/me/.thirdshift/logs/commands/issue/x.log")),
                "droplet-1",
                Duration::from_secs(4)
            ),
            "Pull request: https://github.com/acme/widgets/pull/1\n\
             Session log:  /home/me/.thirdshift/logs/sessions/x.jsonl\n\
             Command log:  /home/me/.thirdshift/logs/commands/issue/x.log\n\
             Built with claude · opus · high\n\
             Host:         droplet-1\n\
             Took:         4s\n"
        );
    }

    #[test]
    fn the_body_reports_the_base_fix_after_the_cause() {
        let account = Account {
            pr_url: Some(PR),
            base_fix: Some("https://github.com/acme/widgets/issues/8 merged"),
            ..failed("claude exited 1")
        };
        assert_eq!(
            body_of(&account),
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
        let account = Account {
            pr_url: Some(PR),
            advice: &advice,
            log: Some(Path::new("/home/me/.thirdshift/logs/x.jsonl")),
            ..failed("CI red on test, which also fails on main at 362b9ca; fix main first")
        };
        assert_eq!(
            body_of(&account),
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
        let account = Account {
            ticket_lines: &tickets,
            ..failed("Tickets not done: #21, #22")
        };
        assert_eq!(
            body_of(&account),
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
