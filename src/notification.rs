//! The Run notification: saved beside the Command log when a Run, a Spec
//! run or a Pass ends, unless the Pass was skipped, and optionally emailed
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

/// Whether a Run or a Pass emails its Run notification, by its command or,
/// without `email` or `no-email`, by the User config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationAsk {
    /// Send one, to this address, else to `email.to`.
    Send(Option<String>),
    /// Save it without emailing it.
    Skip,
}

/// What a Run notification is about, as a command starts.
pub enum About<'a> {
    /// A Run or a Spec run on this issue.
    Issue(&'a IssueUrl),
    /// An Architect run, from the Launch directory.
    ArchitectRun,
    /// A Pickup run, which is about the issue it takes, once it takes one.
    PickupRun,
    /// A Security run, from the Launch directory.
    SecurityRun,
}

/// What a Run notification's subject names.
enum Subject {
    /// The issue a Run or a Spec run is on, with its title if it is known:
    /// as GitHub gave it when the Run started, or as the Pickup run that
    /// took the issue listed it.
    Issue {
        issue: IssueUrl,
        title: Option<String>,
        picked_up: bool,
        is_spec: bool,
    },
    /// An Architect run, with the repository the Launch directory's `origin`
    /// names, if it names one.
    ArchitectRun(Option<Repo>),
    /// A Security run, with its repository if origin names one.
    SecurityRun(Option<Repo>),
    /// A Pass whose notification subject has not been established yet.
    Pending,
}

impl Subject {
    /// The Command a notification about an Architect or Security run is
    /// about, by name, with its repository if known.
    fn command(&self) -> Option<(&'static str, Option<&Repo>)> {
        match self {
            Subject::ArchitectRun(repo) => Some(("Architect run", repo.as_ref())),
            Subject::SecurityRun(repo) => Some(("Security run", repo.as_ref())),
            Subject::Issue { .. } | Subject::Pending => None,
        }
    }
}

/// A Run notification waiting for its command to end, with email checked
/// when requested.
pub struct RunNotification {
    resend: Option<Resend>,
    started: Instant,
    started_at: chrono::DateTime<chrono::Utc>,
    subject: Subject,
    /// The command's configured Harness, Model and Effort, once known.
    configured: Option<String>,
}

impl RunNotification {
    /// The Run notification for a command about `about` starting now. If
    /// `ask` requests email, checks it before any work, as `email-test` does:
    /// an address is known and a Resend API key is found. Reads a Run's
    /// issue title and kind now, while someone
    /// may be watching, rather than after the Run, when a hung `gh` could
    /// keep the notification from ever going; and an Architect run's
    /// repository.
    pub fn new(ask: NotificationAsk, settings: &EmailSettings, about: About) -> Result<Self> {
        let started = Instant::now();
        let started_at = chrono::Utc::now();
        let resend = match ask {
            NotificationAsk::Send(to) => Some(Resend::new(to, settings)?),
            NotificationAsk::Skip => None,
        };
        let subject = match about {
            About::Issue(issue) => Subject::Issue {
                issue: issue.clone(),
                picked_up: false,
                // Keep the kind even if launch or Harness preflight fails.
                // A failed read leaves the Run's own preflight to report why.
                is_spec: GitHub::new()
                    .tickets(issue)
                    .is_ok_and(|tickets| !tickets.is_empty()),
                // Left out of the subject if GitHub can't be asked; the Run's
                // own preflight reports why.
                title: GitHub::new().issue_title(issue).ok(),
            },
            // Left out of the subject if origin names none; the Architect
            // run's own preflight reports why.
            About::ArchitectRun => Subject::ArchitectRun(launch::repo().ok()),
            About::PickupRun => Subject::Pending,
            About::SecurityRun => Subject::SecurityRun(launch::repo().ok()),
        };
        Ok(RunNotification {
            resend,
            started,
            started_at,
            subject,
            configured: None,
        })
    }

    /// Take note that the Pickup run took `issue`, titled `title`, as its
    /// search listed it. Its notification keeps the issue subject and names
    /// the Pickup run and the kind of run it dispatched in the body.
    pub fn took(&mut self, issue: &IssueUrl, title: String, is_spec: bool) {
        self.subject = Subject::Issue {
            issue: issue.clone(),
            title: Some(title),
            picked_up: true,
            is_spec,
        };
    }

    /// Name the configured Harness, Model and Effort without claiming
    /// that the Harness confirmed which Model executed.
    pub fn built_with(&mut self, harness: &Choice) {
        self.configured = Some(format!("{harness} (configured)"));
    }

    /// Save the one notification for the command that ended as `account`
    /// tells, and email it if asked. A Pickup run that took no issue has
    /// nothing to tell, as a skipped pass, which has no account, does not
    /// either. A failed write or send is only a warning: it never changes
    /// the command's outcome.
    pub fn finish(self, account: &Account) {
        let Some(subject) = subject_line(&self.subject, account.outcome) else {
            return;
        };
        let host = host::name();
        let command_log = logs::command_log_path();
        let mut text = body(
            &self.subject,
            account,
            self.configured.as_deref(),
            command_log.as_deref(),
            host.as_deref().unwrap_or("unknown host"),
            self.started.elapsed(),
        );
        if account.pre_audit_failure.is_some() {
            text += &format!(
                "Started (UTC): {}\nDiagnostics:  Check this command's stderr on {} at that time (or the scheduler's captured stderr, if scheduled).\n",
                self.started_at.format("%Y-%m-%d %H:%M:%S UTC"),
                host.as_deref().unwrap_or("unknown host"),
            );
        }
        let lines = crate::session::recording::Recording::capture().notification_lines();
        if !lines.is_empty() {
            text += "\nSession models:\n";
            text += "Listed sessions launched the harness. Requested models are not confirmed by harness evidence.\n";
            for line in lines {
                text += &format!("{line}\n");
            }
        }
        if let Some(log) = command_log {
            let path = log.with_extension("notification.txt");
            if let Err(error) = std::fs::write(&path, format!("Subject: {subject}\n\n{text}")) {
                progress::step(format_args!(
                    "warning: could not save the Run notification to {}: {error}",
                    path.display()
                ));
            }
        }
        if let Some(resend) = self.resend
            && let Err(error) = resend.send(&subject, &text)
        {
            progress::step(format_args!(
                "warning: could not send the Run notification: {error:#}"
            ));
        }
    }
}

/// The subject of a notification about `subject` for a command whose
/// outcome is `outcome`: none about a Pickup run that took no issue.
fn subject_line(subject: &Subject, outcome: &str) -> Option<String> {
    if let Subject::Issue { issue, title, .. } = subject {
        return Some(issue_subject(issue, title.as_deref(), outcome));
    }
    subject
        .command()
        .map(|(command, repo)| command_subject(command, repo, outcome))
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

/// `[thirdshift] <owner>/<repo> <command>: <outcome>`, without the
/// repository if it isn't known.
fn command_subject(command: &str, repo: Option<&Repo>, outcome: &str) -> String {
    let repo = repo
        .map(|repo| format!(" {}", repo.slug()))
        .unwrap_or_default();
    format!("[thirdshift]{repo} {command}: {outcome}")
}

/// The plain-text body of the notification for a command that ended as
/// `account` tells, about `subject`, with its `configured` Harness, Model and Effort, if known,
/// naming `command_log`, if the command keeps one,
/// and `host`, for a command that `took` so long. It starts with the command's
/// outcome, as the subject gives it, then the rest of what the subject
/// tells, so the body alone carries it: the original Issue URL and the
/// issue's title, if known, when the subject names an issue, else the
/// command and its repository, if known. An Architect run's then tells how its
/// review ended, and the outcome of the run it dispatched, if any. The cause
/// is left out of an interrupted command's, whose outcome says so.
fn body(
    subject: &Subject,
    account: &Account,
    configured: Option<&str>,
    command_log: Option<&Path>,
    host: &str,
    took: Duration,
) -> String {
    let mut text = format!("Result:       {}\n", account.outcome);
    if let Subject::Issue {
        issue,
        title,
        picked_up,
        is_spec,
    } = subject
    {
        text += &format!("Issue:        {}\n", issue.url);
        if let Some(title) = title {
            text += &format!("Title:        {title}\n");
        }
        let command = match (*picked_up, *is_spec) {
            (false, false) => "Run",
            (false, true) => "Spec run",
            (true, false) => "Pickup run, which dispatched a Run",
            (true, true) => "Pickup run, which dispatched a Spec run",
        };
        text += &command_lines(command, None);
    }
    if let Some((command, repo)) = subject.command() {
        text += &command_lines(command, repo);
    }
    if let Some(findings) = account.security_findings {
        let audit = match &account.ended {
            Err(_) if account.pre_audit_failure.is_some() => "not started",
            Err(_) if account.failed_reproduction.is_some() => "audit complete",
            Ok(line) => line.as_str(),
            Err(_) => account.outcome,
        };
        text += &format!("Audit:        {audit}\n");
        if let Some(number) = account.failed_reproduction {
            let status = if account.interrupted {
                "interrupted"
            } else if account
                .ended
                .as_ref()
                .err()
                .and_then(|cause| cause.safeguard_refusal())
                .is_some()
            {
                "refused"
            } else {
                "failed"
            };
            text += &format!("Reproduction: {number} {status}\n");
        }
        if !findings.is_empty() {
            text += "\nSecurity findings:\n";
            for finding in findings {
                let severity = finding
                    .severity
                    .as_ref()
                    .map(|severity| format!("{severity}: "))
                    .unwrap_or_default();
                text += &format!("- {severity}{}: {}\n", finding.title, finding.url);
            }
            text += "\n";
        }
    }
    if let Some(review) = &account.review {
        text += &format!("Review:       {}\n", review.line);
        if let Some(title) = review.title {
            text += &format!("Description:  {title}\n");
        }
        if let Some(strength) = review.strength {
            text += &format!("Strength:     {strength}\n");
        }
        if review.dispatched.is_some() {
            text += &format!("Dispatched:   {}\n", account.outcome);
        }
    }
    if let Some(pr_url) = account.pr_url {
        text += &format!("Pull request: {pr_url}\n");
    } else if account.outcome == "bug not reproduced" {
        for url in &account.urls {
            text += &format!("Triage notes: {url}\n");
        }
    }
    // Security failure causes can quote private verifier work or Harness
    // diagnostics. Only fixed operational and safeguard categories are safe to send.
    if let Err(cause) = &account.ended
        && !account.interrupted
    {
        let cause = if account.security_findings.is_some() {
            account
                .pre_audit_failure
                .or_else(|| cause.safeguard_refusal())
        } else {
            Some(cause.full())
        };
        if let Some(cause) = cause {
            text += &format!("Cause:        {cause}\n");
        }
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
    if let Some(configured) = configured {
        if account.pre_audit_failure.is_some() {
            text += &format!("Configured:   {configured}\n");
        } else {
            text += &format!("Built with {configured}\n");
        }
    }
    text += "Sent by:      thirdshift\n";
    text += &format!("Host:         {host}\n");
    text += &format!("Took:         {}\n", progress::duration(took));
    if !account.ticket_lines.is_empty() {
        text += "\nTickets:\n";
        for line in account.ticket_lines {
            text += &format!("{line}\n");
        }
    }
    if let Some(recovery) = account.recovery {
        for line in recovery.lines() {
            text += &format!("{line}\n");
        }
    }
    text
}

/// The body lines naming the `command` a notification is about, and its
/// `repo`, if known, as its subject does.
fn command_lines(command: &str, repo: Option<&Repo>) -> String {
    let mut text = format!("Command:      {command}\n");
    if let Some(repo) = repo {
        text += &format!("Repository:   {}\n", repo.slug());
    }
    text
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
            picked_up: false,
            is_spec: false,
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
            security_findings: None,
            security_findings_waiting: false,
            failed_reproduction: None,
            pre_audit_failure: None,
            recovery: None,
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
            security_findings: None,
            security_findings_waiting: false,
            failed_reproduction: None,
            pre_audit_failure: None,
            recovery: None,
            urls: Vec::new(),
        }
    }

    /// The subject and body of the notification about `subject` for a
    /// command that ended as `account` tells, if it sends one.
    fn told(subject: &Subject, account: &Account) -> Option<(String, String)> {
        subject_line(subject, account.outcome).map(|line| (line, body_of(subject, account)))
    }

    /// The body for a command that ended as `account` tells, run on
    /// `droplet-1` for 4s with no Command log.
    fn body_of(subject: &Subject, account: &Account) -> String {
        body(
            subject,
            account,
            None,
            None,
            "droplet-1",
            Duration::from_secs(4),
        )
    }

    #[test]
    fn a_runs_notification_tells_how_the_run_ended() {
        assert_eq!(
            told(&about_issue(), &merged()),
            Some((
                "[thirdshift] acme/widgets#123 Add export button: merged".to_string(),
                format!(
                    "Result:       merged\n\
                     Issue:        https://github.com/acme/widgets/issues/123\n\
                     Title:        Add export button\n\
                     Command:      Run\n\
                     Pull request: {PR}\n\
                     Session log:  {LOG}\n\
                     Sent by:      thirdshift\n\
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
                    "Result:       failed\n\
                     Issue:        https://github.com/acme/widgets/issues/123\n\
                     Title:        Add export button\n\
                     Command:      Run\n\
                     Cause:        claude exited 1\n\
                     Base fix:     {PLAN} not merged\n\
                     Session log:  {LOG}\n\
                     Sent by:      thirdshift\n\
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
                "Result:       interrupted\n\
                 Issue:        https://github.com/acme/widgets/issues/123\n\
                 Title:        Add export button\n\
                 Command:      Run\n\
                 Sent by:      thirdshift\n\
                 Host:         droplet-1\n\
                 Took:         4s\n"
                    .to_string()
            ))
        );
    }

    #[test]
    fn an_architect_runs_notification_with_its_plan_dispatched_tells_how_both_ended() {
        let account = Account {
            review: Some(Review {
                title: None,
                strength: Some("Strong"),
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
                    "Result:       merged\n\
                     Command:      Architect run\n\
                     Repository:   acme/widgets\n\
                     Review:       plan published: {PLAN}\n\
                     Strength:     Strong\n\
                     Dispatched:   merged\n\
                     Pull request: {PR}\n\
                     Session log:  {LOG}\n\
                     Sent by:      thirdshift\n\
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
                title: None,
                strength: Some("not reported"),
                line: format!("idea filed: {PLAN}"),
                dispatched: None,
            }),
            recovery: None,
            urls: vec![PLAN],
            ..merged()
        };
        assert_eq!(
            told(&about_architect_run(), &account),
            Some((
                "[thirdshift] acme/widgets Architect run: idea filed".to_string(),
                format!(
                    "Result:       idea filed\n\
                     Command:      Architect run\n\
                     Repository:   acme/widgets\n\
                     Review:       idea filed: {PLAN}\n\
                     Strength:     not reported\n\
                     Sent by:      thirdshift\n\
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
                title: None,
                strength: None,
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
                    "Result:       review failed\n\
                     Command:      Architect run\n\
                     Review:       failed\n\
                     Cause:        claude exited 1\n\
                     Session log:  {LOG}\n\
                     Sent by:      thirdshift\n\
                     Host:         droplet-1\n\
                     Took:         4s\n"
                )
            ))
        );
    }

    #[test]
    fn the_body_tells_everything_the_subject_does() {
        let repo = || Repo::of_origin("https://github.com/acme/widgets.git");
        let subjects = [
            about_issue(),
            Subject::Issue {
                issue: issue(),
                title: None,
                picked_up: false,
                is_spec: false,
            },
            about_architect_run(),
            Subject::ArchitectRun(None),
            Subject::SecurityRun(repo()),
            Subject::SecurityRun(None),
        ];
        for subject in &subjects {
            let (line, body) = told(subject, &merged()).unwrap();
            // Past the tag, the subject is `<about>: <outcome>`; each word of
            // what it is about is in the body.
            let about = line
                .strip_prefix("[thirdshift]")
                .and_then(|rest| rest.strip_suffix(": merged"))
                .unwrap();
            for word in about.split([' ', '#']).filter(|word| !word.is_empty()) {
                assert!(body.contains(word), "{word:?} of {line:?} not in:\n{body}");
            }
            assert!(body.starts_with("Result:       merged\n"), "{body}");
        }
    }

    #[test]
    fn the_body_names_thirdshift_as_the_subject_does() {
        for subject in [
            about_issue(),
            about_architect_run(),
            Subject::SecurityRun(None),
        ] {
            let (line, body) = told(&subject, &merged()).unwrap();
            assert!(line.starts_with("[thirdshift]"));
            assert!(body.contains("thirdshift"), "{body}");
        }
    }

    #[test]
    fn a_security_runs_body_names_the_command_and_repository() {
        let account = Account {
            security_findings: Some(&[]),
            ..merged()
        };
        assert!(
            body_of(
                &Subject::SecurityRun(Repo::of_origin("https://github.com/acme/widgets.git")),
                &account
            )
            .starts_with(
                "Result:       merged\n\
                     Command:      Security run\n\
                     Repository:   acme/widgets\n\
                     Audit:"
            )
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
            body_of(&Subject::Pending, &failed("origin mismatch")),
            "Result:       failed\n\
             Cause:        origin mismatch\n\
             Sent by:      thirdshift\n\
             Host:         droplet-1\n\
             Took:         4s\n"
        );
        let account = Account {
            log: Some(Path::new("/home/me/.thirdshift/logs/sessions/x.jsonl")),
            ..merged()
        };
        assert_eq!(
            body(
                &Subject::Pending,
                &account,
                Some("claude · opus · high"),
                Some(Path::new("/home/me/.thirdshift/logs/commands/issue/x.log")),
                "droplet-1",
                Duration::from_secs(4)
            ),
            "Result:       merged\n\
             Pull request: https://github.com/acme/widgets/pull/1\n\
             Session log:  /home/me/.thirdshift/logs/sessions/x.jsonl\n\
             Command log:  /home/me/.thirdshift/logs/commands/issue/x.log\n\
             Built with claude · opus · high\n\
             Sent by:      thirdshift\n\
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
            body_of(&Subject::Pending, &account),
            "Result:       failed\n\
             Pull request: https://github.com/acme/widgets/pull/1\n\
             Cause:        claude exited 1\n\
             Base fix:     https://github.com/acme/widgets/issues/8 merged\n\
             Sent by:      thirdshift\n\
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
            body_of(&Subject::Pending, &account),
            "Result:       failed\n\
             Pull request: https://github.com/acme/widgets/pull/1\n\
             Cause:        CI red on test, which also fails on main at 362b9ca; fix main first\n\
             Base check:   test: https://ci.example/main/test\n\
             Or set:       base.fix = true\n\
             Session log:  /home/me/.thirdshift/logs/x.jsonl\n\
             Sent by:      thirdshift\n\
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
            body_of(&Subject::Pending, &account),
            "Result:       failed\n\
             Cause:        Tickets not done: #21, #22\n\
             Sent by:      thirdshift\n\
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
            command_subject("Architect run", Some(&repo), "plan published"),
            "[thirdshift] acme/widgets Architect run: plan published"
        );
        assert_eq!(
            command_subject("Architect run", None, "review failed"),
            "[thirdshift] Architect run: review failed"
        );
    }
}
