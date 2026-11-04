//! A Report investigated read-only, then filed without dispatching work.

use std::io::{IsTerminal, Read};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::failed_run::FailedRun;
use crate::git::Git;
use crate::github::GitHub;
use crate::harness::Choice;
use crate::labels::{self, Label};
use crate::{command_limit, launch, logs, prompt, session, worktree};

/// Read before command startup, so Setup cannot consume a piped Report.
pub fn read_report(file: Option<&str>) -> Result<String> {
    let report = if let Some(path) = file {
        std::fs::read_to_string(path).with_context(|| format!("could not read Report {path}"))?
    } else {
        ensure!(
            !std::io::stdin().is_terminal(),
            "give a Report on standard input or with --file <path>"
        );
        let mut report = String::new();
        std::io::stdin().read_to_string(&mut report)?;
        report
    };
    ensure!(
        !report.trim().is_empty(),
        "give a Report on standard input or with --file <path>"
    );
    Ok(report)
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
}

/// Keep the session's prose out of the machine-readable stdout contract.
fn recommendation(message: Option<&str>) -> Result<Recommendation> {
    let json = message
        .and_then(|message| {
            message
                .lines()
                .rev()
                .find_map(|line| line.strip_prefix("Triage recommendation: "))
        })
        .context("the Triage session ended without its final Triage recommendation: JSON line")?;
    let recommendation: Recommendation = serde_json::from_str(json)
        .context("the Triage recommendation is not valid recommendation JSON")?;
    ensure!(
        !recommendation.category.trim().is_empty()
            && state_label(&recommendation.state).is_some()
            && !recommendation.investigation.trim().is_empty()
            && recommendation.comments.iter().all(|comment| {
                !comment.body.trim().is_empty()
                    && crate::issue::IssueUrl::parse(&comment.url).is_ok()
            })
            && recommendation.issues.iter().all(|issue| {
                !issue.title.trim().is_empty()
                    && !issue.body.trim().is_empty()
                    && issue
                        .state
                        .as_deref()
                        .is_none_or(|state| state_label(state).is_some())
                    && issue
                        .confirmation_blocked
                        .as_deref()
                        .is_none_or(|reason| !reason.trim().is_empty())
            }),
        "the Triage recommendation needs a category, triage state, investigation and complete proposed issues or comments"
    );
    for issue in &recommendation.issues {
        let state = issue.state.as_deref().unwrap_or(&recommendation.state);
        let needs_answer = issue.confirmation_blocked.is_some()
            || labels::NEEDS_INFO.is_named(state)
            || labels::READY_FOR_HUMAN.is_named(state);
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
    }
    Ok(recommendation)
}

fn state_label(state: &str) -> Option<Label> {
    std::iter::once(labels::READY_FOR_AGENT)
        .chain(labels::UNREADY)
        .find(|label| label.is_named(state))
}

/// A label thirdshift treats as workflow state. The session may name a
/// repository category; these stay factory-owned, whatever their case.
fn factory_label(name: &str) -> bool {
    state_label(name).is_some()
        || crate::claim::IN_PROGRESS.is_named(name)
        || crate::claim::AGENT_FAILED.is_named(name)
        || crate::base_fix::BASE_FIX.is_named(name)
        || labels::FLAKY_TEST.is_named(name)
        || crate::security::fixing::SECURITY_FIX.is_named(name)
        || crate::architect::is_mark(name)
        || crate::github::is_security_finding_label(name)
}

/// The result retained by stdout, the Command record and Run notification.
#[derive(Debug, PartialEq, Eq)]
pub struct Ended {
    pub json: String,
    pub filed: bool,
    pub notification: String,
}

fn quoted_report(report: &str, source: Option<&str>) -> String {
    let fence = "`".repeat(
        report
            .split(|c| c != '`')
            .map(str::len)
            .max()
            .unwrap_or(0)
            .max(2)
            + 1,
    );
    let source = source.map_or_else(|| "standard input".into(), |path| format!("file: {path}"));
    format!(
        "<details>\n<summary>Original Report</summary>\n\nSource: {source}\n\n{fence}text\n{report}\n{fence}\n\n</details>"
    )
}

fn publish(
    recommendation: &mut Recommendation,
    repo: &crate::issue::Repo,
    report: &str,
    source: Option<&str>,
    allow_ready_for_agent: bool,
    published: &mut Vec<crate::issue::IssueUrl>,
) -> Result<()> {
    let github = GitHub::new();
    let targets = recommendation
        .comments
        .iter()
        .map(|comment| {
            let issue = crate::issue::IssueUrl::parse(&comment.url)?;
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
    let categories = github.repository_labels(&repo.slug())?;
    let original_report = quoted_report(report, source);
    for issue in &mut recommendation.issues {
        let category = categories
            .names()
            .find(|name| {
                name.eq_ignore_ascii_case(
                    issue
                        .category
                        .as_deref()
                        .unwrap_or(&recommendation.category),
                )
            })
            .filter(|name| !factory_label(name) && !name.contains(','))
            .context(
                "the Triage category must be a repository category label, not a triage state or another factory label",
            )?;
        let recommended = state_label(issue.state.as_deref().unwrap_or(&recommendation.state))
            .context("unknown Triage state")?;
        let state = if issue.confirmation_blocked.is_some() {
            labels::READY_FOR_HUMAN
        } else if recommended == labels::READY_FOR_AGENT && !allow_ready_for_agent {
            labels::NEEDS_TRIAGE
        } else {
            recommended
        };
        let blocked = issue
            .confirmation_blocked
            .as_deref()
            .map_or_else(String::new, |reason| {
                format!("\n\nConfirmation needs the Day shift: {reason}")
            });
        let note = if recommended == labels::READY_FOR_AGENT && state == labels::NEEDS_TRIAGE {
            "\n\nready-for-agent was recommended; needs-triage is applied until the Day shift permits readiness."
        } else {
            ""
        };
        issue.body = format!(
            "This issue was generated by AI.\n\n{}{note}{blocked}{}\n\n{original_report}",
            issue.body,
            issue
                .questionnaire
                .as_deref()
                .map_or_else(String::new, |text| format!(
                    "\n\n## Questionnaire\n\n{text}"
                )),
        );
        issue.category = Some(category.into());
        issue.state = Some(state.name().into());
    }
    for issue in &mut recommendation.issues {
        let url = github.create_triaged_issue(
            repo,
            &issue.title,
            &issue.body,
            issue.category.as_deref().expect("validated category"),
            state_label(issue.state.as_deref().expect("validated state")).expect("validated state"),
        )?;
        issue.url = Some(url.url.clone());
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
            github.comment_issue(issue, &format!("This comment was generated by AI.\n\nOther issues from the same Report:\n{links}"))?;
        }
    }
    for (comment, target) in recommendation.comments.iter_mut().zip(targets) {
        comment.body = format!(
            "This comment was generated by AI.\n\n{}\n\n{original_report}",
            comment.body
        );
        github.comment_issue(&target, &comment.body)?;
        published.push(target);
    }
    Ok(())
}

/// A Triage run always takes a Command slot, even above the Command limit.
pub fn run(
    report: &str,
    source: Option<&str>,
    recommend_only: bool,
    allow_ready_for_agent: bool,
    harness: &mut Choice,
) -> Result<Ended, FailedRun> {
    let repo = launch::repo()?;
    command_limit::hold_slot(&repo);
    logs::started(logs::Work::TriageRun(&repo), harness);
    harness.check_without_probe()?;
    let base = launch::checked_out_base()?;
    let launch = Git::new(std::env::current_dir().context("could not read the Launch directory")?);
    let checkout = worktree::ReviewWorktree::for_triage(&launch, &repo.name, base.origin_commit())?;
    let session_logs = session::Logs::of_triage_run(&repo);
    let mut published = Vec::new();
    let (result, log) =
        session::Sessions::within(&session_logs, checkout.path(), harness, |sessions| {
            let skill = crate::skills::find(checkout.path(), "thirdshift-triage")?;
            let message = sessions.run_to_final_message(
                session::Purpose::Ordinary,
                session::Kind::Triage,
                &prompt::triage(&skill.path, report),
            )?;
            let mut recommendation = recommendation(message.as_deref())?;
            if !recommend_only {
                publish(
                    &mut recommendation,
                    &repo,
                    report,
                    source,
                    allow_ready_for_agent,
                    &mut published,
                )?;
            }
            let json = serde_json::to_string(&recommendation)?;
            let mut notification = format!("Recommendation:\n{json}\n");
            for issue in &recommendation.issues {
                if let Some(questionnaire) = &issue.questionnaire {
                    notification.push_str(&format!("\nQuestionnaire: {}\n", issue.title));
                    if let Some(url) = &issue.url {
                        notification.push_str(&format!("Issue: {url}\n"));
                    }
                    notification.push_str(&format!("\n{questionnaire}\n"));
                }
            }
            Ok(Ended {
                json,
                filed: !recommend_only,
                notification,
            })
        });
    result.map_err(|error| {
        let mut failed = FailedRun::from(error);
        if !published.is_empty() {
            let urls = published
                .iter()
                .map(|issue| issue.url.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            failed.error = failed.error.context(format!(
                "Triage publication incomplete. Issues already filed or commented on:\n{urls}"
            ));
        }
        failed.log = log;
        failed
    })
}
