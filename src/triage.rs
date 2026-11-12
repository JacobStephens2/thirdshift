//! A Report investigated read-only, then filed without dispatching work.

use std::io::{IsTerminal, Read};

use anyhow::{Context, Result, ensure};

use crate::failed_run::FailedRun;
use crate::git::Git;
use crate::github::{GENERATED, GitHub};
use crate::harness::Choice;
use crate::issue::{IssueInput, IssueUrl, Repo};
use crate::labels;
use crate::{command_limit, launch, logs, prompt, session, worktree};

mod correction;
mod existing;
mod images;
mod pass;
mod recommendation;

pub use pass::{Outcome, run as run_pass};
use recommendation::{Publishing, Recommendation, is_factory_label};

/// Input is resolved before Setup can consume a piped Report.
pub enum Input {
    Report(Report),
    Issue(IssueUrl),
}

impl Input {
    /// The existing issue to triage in place, if this is not a Report.
    fn issue(&self) -> Option<&IssueUrl> {
        match self {
            Self::Issue(issue) => Some(issue),
            Self::Report(_) => None,
        }
    }

    /// No supplied work means an unattended Pass. Resolve stdin before Setup.
    pub fn read(issue: Option<IssueInput>, file: Option<&str>) -> Result<Option<Self>> {
        if let Some(issue) = issue {
            return Ok(Some(Self::Issue(launch::resolve_issue(issue)?)));
        }
        if file.is_none() && std::io::stdin().is_terminal() {
            return Ok(None);
        }
        let report = Report::read(file)?;
        if file.is_none() && report.text.trim().is_empty() {
            return Ok(None);
        }
        ensure!(
            !report.text.trim().is_empty(),
            "give a Report on standard input or with --file <path>"
        );
        Ok(Some(Self::Report(report)))
    }
}

/// The free text a Triage run investigates, with where it came from: the
/// file the command named, or standard input.
pub struct Report {
    text: String,
    file: Option<String>,
}

impl Report {
    /// Read before command startup, so Setup cannot consume a piped Report.
    pub fn read(file: Option<&str>) -> Result<Report> {
        let text = if let Some(path) = file {
            std::fs::read_to_string(path)
                .with_context(|| format!("could not read Report {path}"))?
        } else {
            let mut text = String::new();
            std::io::stdin().read_to_string(&mut text)?;
            text
        };
        Ok(Report {
            text,
            file: file.map(String::from),
        })
    }

    /// The Report verbatim in a collapsed section, saying where it came from.
    fn quoted(&self) -> String {
        let fence = "`".repeat(
            self.text
                .split(|c| c != '`')
                .map(str::len)
                .max()
                .unwrap_or(0)
                .max(2)
                + 1,
        );
        let source = self
            .file
            .as_deref()
            .map_or_else(|| "standard input".into(), |path| format!("file: {path}"));
        format!(
            "<details>\n<summary>Original Report</summary>\n\nSource: {source}\n\n{fence}text\n{}\n{fence}\n\n</details>",
            self.text
        )
    }
}

/// Guidance, correction and publication share the same usable repository categories.
fn repository_categories(repo: &Repo, github: &GitHub) -> Result<Vec<String>> {
    Ok(github
        .repository_labels(&repo.slug())?
        .names()
        .filter(|name| !is_factory_label(name) && !name.contains(','))
        .map(str::to_owned)
        .collect())
}

/// The result retained by stdout, the Command record and Run notification.
#[derive(Debug, PartialEq, Eq)]
pub struct Ended {
    pub json: String,
    pub filed: bool,
    pub notification: String,
    /// Issues successfully filed or commented on, in publication order.
    pub published: Vec<IssueUrl>,
}

/// Publication stopped after these issues were successfully filed or commented on.
#[derive(Debug)]
pub struct IncompletePublication {
    pub published: Vec<IssueUrl>,
}

/// Marks private Triage failures so a notification cannot quote their cause.
/// The full error chain remains available in local output and logs.
#[derive(Debug)]
pub(crate) struct PrivateFindingTriage {
    pub notification: String,
}

impl std::fmt::Display for PrivateFindingTriage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("private Security finding triage failed; details retained locally")
    }
}

impl std::error::Error for PrivateFindingTriage {}

impl std::fmt::Display for IncompletePublication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Triage publication incomplete. Issues already filed or commented on:"
        )?;
        for issue in &self.published {
            write!(f, "\n{}", issue.url)?;
        }
        Ok(())
    }
}

/// Check the comment targets against GitHub, writing nothing. Both modes
/// check them, so `--recommend-only` refuses exactly what filing would refuse.
fn check_comment_targets(
    recommendation: &Recommendation,
    repo: &Repo,
    github: &GitHub,
) -> Result<()> {
    for comment in recommendation.comments() {
        let issue = comment.target();
        ensure!(
            issue.owner.eq_ignore_ascii_case(&repo.owner)
                && issue.repo.eq_ignore_ascii_case(&repo.name),
            "the Triage comment must target an issue in {}: {}",
            repo.slug(),
            issue.url
        );
        ensure!(
            github.issue_is_open(issue)?,
            "the Triage comment must target an open issue: {}",
            issue.url
        );
    }
    Ok(())
}

/// File a Report's issues and comments. Each write that lands is pushed to
/// `published` as it lands, so a failure partway still names it.
fn publish(
    report: &Report,
    issues: &mut [recommendation::Issue],
    comments: &mut [recommendation::Comment],
    repo: &Repo,
    github: &GitHub,
    published: &mut Vec<IssueUrl>,
) -> Result<()> {
    let original_report = report.quoted();
    let mut wontfix = Vec::new();
    for issue in issues {
        let body = format!(
            "This issue was generated by AI.\n\n{}{}\n\n{original_report}",
            issue.body(),
            issue.notes(),
        );
        let filing = issue.filing();
        let state = filing.state();
        let url =
            github.create_triaged_issue(repo, issue.title(), &body, filing.category(), state)?;
        issue.record_filed(&url, body);
        if state == labels::WONTFIX {
            wontfix.push(url.clone());
        }
        published.push(url);
    }
    if published.len() > 1 {
        for issue in published.iter() {
            let links = published
                .iter()
                .filter(|other| other.number != issue.number)
                .map(|other| other.url.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            github.comment_issue(
                issue,
                &format!("{GENERATED}\n\nOther issues from the same Report:\n{links}"),
            )?;
        }
    }
    // The Day shift's triage closes a wontfix issue; so does a Triage run.
    for issue in &wontfix {
        github.close_issue(
            issue,
            &format!("{GENERATED}\n\nClosed as wontfix, as the Triage run recommended. The issue body says why."),
        )?;
    }
    for comment in comments {
        let body = format!("{GENERATED}\n\n{}\n\n{original_report}", comment.body());
        github.comment_issue(comment.target(), &body)?;
        published.push(comment.target().clone());
        comment.record_posted(body);
    }
    Ok(())
}

/// What a Triage run may do beyond investigating: its flags, with the User
/// config's permissions already added.
pub struct Options {
    /// `--recommend-only`: check what filing would do, then write nothing to GitHub.
    pub recommend_only: bool,
    /// Keep a recommended `ready-for-agent` on the issues it files.
    pub allow_ready_for_agent: bool,
    /// Run the session in the read-only whole-machine sandbox.
    pub read_machine: bool,
}

/// A Triage run always takes a Command slot, even above the Command limit.
pub fn run(
    input: &Input,
    base: Option<&str>,
    options: Options,
    harness: &mut Choice,
) -> Result<Ended, FailedRun> {
    let repo = launch::repo()?;
    command_limit::hold_slot(&repo);
    let target = input.issue();
    logs::started(
        logs::Work::TriageRun {
            repo: &repo,
            issue: target,
        },
        harness,
    );
    let base = match target {
        Some(issue) => {
            let directory = launch::LaunchDirectory::open_for_run(issue)?;
            existing::check_trust(issue, &GitHub::new())?;
            directory.prepare_base_branch(base)?
        }
        None => launch::selected_base(base)?,
    };
    let launch = Git::new(std::env::current_dir().context("could not read the Launch directory")?);
    let claim = target
        .filter(|_| !options.recommend_only)
        .map(|issue| existing::Claim::take(&launch, issue))
        .transpose()?;
    investigate(input, options, harness, &repo, &launch, &base, claim)
}

fn investigate(
    input: &Input,
    options: Options,
    harness: &mut Choice,
    repo: &Repo,
    launch: &Git,
    base: &launch::BaseBranch,
    mut claim: Option<existing::Claim<'_>>,
) -> Result<Ended, FailedRun> {
    let target = input.issue();
    let finding_notification = target
        .map(|issue| existing::finding_notification(issue, &GitHub::new()))
        .transpose()?
        .flatten();
    harness.check_without_probe()?;
    let checkout = worktree::ReviewWorktree::for_triage(launch, &repo.name, base.origin_commit())?;
    let session_logs = session::Logs::of_triage_run(repo);
    let mut published = Vec::new();
    let investigate = |sessions: &session::Sessions<'_>| {
        let skill = crate::skills::find(checkout.path(), "thirdshift-triage")?;
        let (session_prompt, categories) = match input {
            Input::Report(report) => {
                let categories = repository_categories(repo, &GitHub::new())?;
                let session_prompt =
                    prompt::triage(&skill.path, &report.text, &categories, options.read_machine);
                (session_prompt, categories)
            }
            Input::Issue(issue) => {
                existing::check_trust(issue, &GitHub::new())?;
                images::hand_over(issue, checkout.path())?;
                let categories = repository_categories(repo, &GitHub::new())?;
                let session_prompt =
                    prompt::triage_issue(&skill.path, issue, &categories, options.read_machine);
                (session_prompt, categories)
            }
        };
        let captured = recommendation::Captured {
            categories: &categories,
            input,
            allow_ready_for_agent: options.allow_ready_for_agent,
        };
        let mut recommendation = correction::recommendation(sessions, &session_prompt, &captured)?;
        let github = GitHub::new();
        let publication = check_comment_targets(&recommendation, repo, &github).and_then(|()| {
            if let Some(target) = target {
                existing::check_target(target, &github)?;
            }
            if options.recommend_only {
                return Ok(());
            }
            match recommendation.publishing() {
                Publishing::Report {
                    report,
                    issues,
                    comments,
                } => publish(report, issues, comments, repo, &github, &mut published),
                Publishing::Existing {
                    target,
                    issue,
                    comments,
                } => existing::publish(target, issue, comments, &github, &mut published),
            }
        });
        let json = serde_json::to_string(&recommendation)?;
        let notification = finding_notification
            .clone()
            .unwrap_or_else(|| recommendation.notification());
        publication.with_context(|| format!("Triage analysis retained:\n{notification}"))?;
        Ok(Ended {
            json,
            filed: !options.recommend_only,
            notification,
            published: published.clone(),
        })
    };
    let (result, log) = if options.read_machine {
        let sandbox = crate::sandbox::triage::prepare(checkout.path(), harness.harness)?;
        session::Sessions::within_sandboxed(
            &session_logs,
            checkout.path(),
            harness,
            &sandbox,
            investigate,
        )
    } else {
        session::Sessions::within(&session_logs, checkout.path(), harness, investigate)
    };
    if result.is_ok()
        && let Some(claim) = &mut claim
    {
        claim.completed();
    }
    result.map_err(|error| {
        let mut failed = FailedRun::from(error);
        if !published.is_empty() {
            failed.error = failed.error.context(IncompletePublication { published });
        }
        if let Some(notification) = finding_notification {
            failed.error = failed.error.context(PrivateFindingTriage { notification });
        }
        failed.log = log;
        failed
    })
}
