//! A Report investigated read-only, then filed without dispatching work.

use std::io::{IsTerminal, Read};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::failed_run::FailedRun;
use crate::git::Git;
use crate::github::{GENERATED, GitHub};
use crate::harness::Choice;
use crate::issue::{IssueInput, IssueUrl, Repo};
use crate::labels::{self, Label};
use crate::{command_limit, launch, logs, prompt, session, worktree};

mod correction;
mod existing;
mod images;
mod pass;

pub use pass::{Outcome, run as run_pass};

/// Input is resolved before Setup can consume a piped Report.
pub enum Input {
    Report(Report),
    Issue(IssueUrl),
}

impl Input {
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

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Recommendation {
    category: String,
    state: String,
    issues: Vec<ProposedIssue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    comments: Vec<ProposedComment>,
    investigation: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProposedComment {
    url: String,
    body: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProposedIssue {
    title: String,
    body: String,
    #[serde(skip_deserializing, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    confirmation_blocked: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    questionnaire: Option<String>,
    /// The state the issue is filed with before readiness permission,
    /// resolved once by [`parse_recommendation`].
    #[serde(skip)]
    triaged: Option<Label>,
}

impl Recommendation {
    /// A readable Run notification, leaving full issue and comment bodies on stdout.
    /// Fields stay inside their records: a line break in session text must not
    /// open another `Issue:` line, and terminal controls must not be written out.
    fn notification(&self) -> String {
        let mut notification = format!(
            "Investigation:\n{}\n\nRecommendation: {} ({})\n",
            investigation_text(&self.investigation),
            one_line(&self.category),
            one_line(&self.state)
        );
        if !self.issues.is_empty() {
            notification.push_str("\nIssues:\n");
            for issue in &self.issues {
                notification.push_str(&format!(
                    "- {} ({}, {})\n",
                    one_line(&issue.title),
                    one_line(issue.category.as_deref().unwrap_or(&self.category)),
                    one_line(issue.state.as_deref().unwrap_or(&self.state))
                ));
                if let Some(url) = &issue.url {
                    notification.push_str(&format!("Issue: {}\n", one_line(url)));
                }
            }
        }
        if !self.comments.is_empty() {
            notification.push_str("\nComment targets:\n");
            for comment in &self.comments {
                notification.push_str(&format!("- {}\n", one_line(&comment.url)));
            }
        }
        for issue in &self.issues {
            if let Some(questionnaire) = &issue.questionnaire {
                notification.push_str(&format!("\nQuestionnaire: {}\n", one_line(&issue.title)));
                if let Some(url) = &issue.url {
                    notification.push_str(&format!("Issue: {}\n", one_line(url)));
                }
                notification.push_str(&format!("\n{questionnaire}\n"));
            }
        }
        notification
    }
}

/// One notification field. Line breaks and terminal controls would otherwise
/// open another record, such as a forged `Issue:` line.
fn one_line(text: &str) -> String {
    let mut collapsed = String::with_capacity(text.len());
    let mut pending_space = false;
    for ch in text.chars() {
        if ch == '\u{2028}' || ch == '\u{2029}' || ch.is_control() {
            pending_space = !collapsed.is_empty();
            continue;
        }
        if pending_space {
            collapsed.push(' ');
            pending_space = false;
        }
        collapsed.push(ch);
    }
    collapsed
}

/// The investigation, one indented line per row, so none of its text stands
/// as its own notification record.
fn investigation_text(text: &str) -> String {
    text.split('\n')
        .map(|line| format!("  {}", one_line(line)))
        .collect::<Vec<_>>()
        .join("\n")
}

const INCOMPLETE: &str = "the Triage recommendation needs a category, triage state, investigation and complete proposed issues or comments";

/// Read and check the session's final recommendation line. The session's
/// prose stays out of the machine-readable stdout contract.
fn parse_recommendation(message: Option<&str>) -> Result<Recommendation> {
    let json = message
        .and_then(|message| {
            message
                .lines()
                .rev()
                .find_map(|line| line.strip_prefix("Triage recommendation: "))
        })
        .context("the Triage session ended without its final Triage recommendation: JSON line")?;
    let mut recommendation: Recommendation = serde_json::from_str(json)
        .context("the Triage recommendation is not valid recommendation JSON")?;
    let overall = state_label(&recommendation.state);
    ensure!(
        !recommendation.category.trim().is_empty()
            && overall.is_some()
            && !recommendation.investigation.trim().is_empty()
            && recommendation.comments.iter().all(|comment| {
                !comment.body.trim().is_empty() && IssueUrl::parse(&comment.url).is_ok()
            }),
        INCOMPLETE
    );
    for issue in &mut recommendation.issues {
        ensure!(
            !issue.title.trim().is_empty()
                && !issue.body.trim().is_empty()
                && issue
                    .confirmation_blocked
                    .as_deref()
                    .is_none_or(|reason| !reason.trim().is_empty()),
            INCOMPLETE
        );
        let recommended = issue
            .state
            .as_deref()
            .map_or(overall, state_label)
            .context(INCOMPLETE)?;
        // Only the Day shift can confirm a problem that needs credentials or
        // production access, whatever the session recommended.
        let triaged = if issue.confirmation_blocked.is_some() {
            labels::READY_FOR_HUMAN
        } else {
            recommended
        };
        let needs_answer = triaged == labels::NEEDS_INFO || triaged == labels::READY_FOR_HUMAN;
        ensure!(
            if needs_answer {
                issue
                    .questionnaire
                    .as_deref()
                    .is_some_and(|text| !text.trim().is_empty())
            } else {
                issue.questionnaire.is_none()
            },
            "the Triage questionnaire is required only for needs-info or ready-for-human issues: {}",
            issue.title
        );
        issue.triaged = Some(triaged);
    }
    Ok(recommendation)
}

fn state_label(state: &str) -> Option<Label> {
    std::iter::once(labels::READY_FOR_AGENT)
        .chain(labels::UNREADY)
        .find(|label| label.is_named(state))
}

/// Whether thirdshift treats `name` as workflow state. The session may name
/// a repository category; these stay factory-owned, whatever their case.
fn is_factory_label(name: &str) -> bool {
    state_label(name).is_some()
        || crate::claim::IN_PROGRESS.is_named(name)
        || crate::claim::AGENT_FAILED.is_named(name)
        || crate::base_fix::BASE_FIX.is_named(name)
        || labels::FLAKY_TEST.is_named(name)
        || crate::security::fixing::SECURITY_FIX.is_named(name)
        || crate::architect::is_mark(name)
        || crate::github::is_security_finding_label(name)
}

/// Guidance and publication share the same usable repository categories.
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

/// How filing labels one proposed issue.
struct Filing {
    /// The repository's category label, spelled as the repository spells it.
    category: String,
    state: Label,
    /// `ready-for-agent` was recommended, but filing lacks permission for it.
    readiness_held: bool,
}

/// Everything filing asks GitHub to accept, checked before any write.
struct Publication {
    filings: Vec<Filing>,
    /// The open issues in this repository that the proposed comments go to.
    targets: Vec<IssueUrl>,
}

/// Check the recommendation against GitHub, writing nothing. Both modes
/// check it, so `--recommend-only` refuses exactly what filing would refuse.
fn check_publication(
    recommendation: &Recommendation,
    repo: &Repo,
    github: &GitHub,
    allow_ready_for_agent: bool,
) -> Result<Publication> {
    let targets = recommendation
        .comments
        .iter()
        .map(|comment| {
            let issue = IssueUrl::parse(&comment.url)?;
            ensure!(
                issue.owner.eq_ignore_ascii_case(&repo.owner)
                    && issue.repo.eq_ignore_ascii_case(&repo.name),
                "the Triage comment must target an issue in {}: {}",
                repo.slug(),
                issue.url
            );
            ensure!(
                github.issue_is_open(&issue)?,
                "the Triage comment must target an open issue: {}",
                issue.url
            );
            Ok(issue)
        })
        .collect::<Result<Vec<_>>>()?;
    if recommendation.issues.is_empty() {
        return Ok(Publication {
            filings: Vec::new(),
            targets,
        });
    }
    let categories = repository_categories(repo, github)?;
    let filings = recommendation
        .issues
        .iter()
        .map(|issue| {
            let wanted = issue.category.as_deref().unwrap_or(&recommendation.category);
            let category = categories
                .iter()
                .find(|name| name.eq_ignore_ascii_case(wanted))
                .context(
                    "the Triage category must be a repository category label, not a triage state or another factory label",
                )?;
            let triaged = issue
                .triaged
                .expect("parse_recommendation resolves every proposed issue's state");
            let readiness_held = triaged == labels::READY_FOR_AGENT && !allow_ready_for_agent;
            Ok(Filing {
                category: category.clone(),
                state: if readiness_held {
                    labels::NEEDS_TRIAGE
                } else {
                    triaged
                },
                readiness_held,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Publication { filings, targets })
}

/// File the checked issues and comments. Each write that lands is pushed to
/// `published` as it lands, so a failure partway still names it.
fn publish(
    recommendation: &mut Recommendation,
    publication: Publication,
    repo: &Repo,
    report: &Report,
    github: &GitHub,
    published: &mut Vec<IssueUrl>,
) -> Result<()> {
    let original_report = report.quoted();
    let mut wontfix = Vec::new();
    for (issue, filing) in recommendation.issues.iter_mut().zip(publication.filings) {
        let held = if filing.readiness_held {
            "\n\nready-for-agent was recommended; needs-triage is applied until the Day shift permits readiness."
        } else {
            ""
        };
        let blocked = issue
            .confirmation_blocked
            .as_deref()
            .map_or_else(String::new, |reason| {
                format!("\n\nConfirmation needs the Day shift: {reason}")
            });
        let questionnaire = issue
            .questionnaire
            .as_deref()
            .map_or_else(String::new, |text| {
                format!("\n\n## Questionnaire\n\n{text}")
            });
        issue.body = format!(
            "This issue was generated by AI.\n\n{}{held}{blocked}{questionnaire}\n\n{original_report}",
            issue.body,
        );
        let url = github.create_triaged_issue(
            repo,
            &issue.title,
            &issue.body,
            &filing.category,
            filing.state,
        )?;
        issue.url = Some(url.url.clone());
        issue.category = Some(filing.category);
        issue.state = Some(filing.state.name().into());
        if filing.state == labels::WONTFIX {
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
    for (comment, target) in recommendation.comments.iter_mut().zip(publication.targets) {
        comment.body = format!("{GENERATED}\n\n{}\n\n{original_report}", comment.body);
        github.comment_issue(&target, &comment.body)?;
        published.push(target);
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
    let target = match input {
        Input::Issue(issue) => Some(issue),
        Input::Report(_) => None,
    };
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
    let target = match input {
        Input::Issue(issue) => Some(issue),
        Input::Report(_) => None,
    };
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
        let session_prompt = match input {
            Input::Report(report) => {
                prompt::triage(&skill.path, &report.text, options.read_machine)
            }
            Input::Issue(issue) => {
                existing::check_trust(issue, &GitHub::new())?;
                images::hand_over(issue, checkout.path())?;
                let categories = repository_categories(repo, &GitHub::new())?;
                prompt::triage_issue(&skill.path, issue, &categories, options.read_machine)
            }
        };
        let mut recommendation = correction::recommendation(sessions, &session_prompt, target)?;
        let github = GitHub::new();
        let publication = check_publication(
            &recommendation,
            repo,
            &github,
            options.allow_ready_for_agent,
        )
        .and_then(|mut checked| {
            if let Some(target) = target {
                existing::check_target(target, &github)?;
                if checked.filings[0].readiness_held {
                    checked.filings[0].state = labels::READY_FOR_HUMAN;
                }
            }
            if options.recommend_only {
                return Ok(());
            }
            match input {
                Input::Report(report) => publish(
                    &mut recommendation,
                    checked,
                    repo,
                    report,
                    &github,
                    &mut published,
                ),
                Input::Issue(target) => existing::publish(
                    &mut recommendation,
                    checked,
                    target,
                    &github,
                    &mut published,
                ),
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
