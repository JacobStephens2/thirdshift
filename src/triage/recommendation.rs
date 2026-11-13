//! A Triage run's recommendation, interpreted once from the session's final
//! message. The initial session and its correction, `--recommend-only` and
//! both publications share the result: each proposed issue keeps its filing
//! category, state and readiness, and each proposed comment its target.
//! GitHub reads and writes stay with the callers.

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use super::{Input, Report};
use crate::issue::IssueUrl;
use crate::labels::{self, Label};

/// Everything interpretation needs besides the session's message, all
/// captured before the session.
pub(super) struct Captured<'a> {
    /// The repository's category labels, as captured for the session.
    pub categories: &'a [String],
    /// The Report to file from, or the existing issue to triage in place.
    pub input: &'a Input,
    /// Filing may keep a recommended `ready-for-agent`.
    pub allow_ready_for_agent: bool,
}

const INCOMPLETE: &str = "the Triage recommendation needs a category, triage state, investigation and complete proposed issues or comments";
const CATEGORY_ERROR: &str = "the Triage category must be a repository category label, not a triage state or another factory label";
const ONE_ISSUE: &str = "existing-issue Triage requires exactly one recommendation issue; propose splits in a ready-for-human issue body";

/// The recommendation line as the session writes it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    category: String,
    state: String,
    issues: Vec<ProposedIssue>,
    #[serde(default)]
    comments: Vec<ProposedComment>,
    investigation: String,
}

/// A proposed issue as recommended, updated only with what publication did.
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

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProposedComment {
    url: String,
    body: String,
}

/// A valid recommendation, with each proposal's resolved facts beside it.
pub(super) struct Recommendation<'a> {
    category: String,
    state: String,
    investigation: String,
    proposals: Proposals<'a>,
    comments: Vec<Comment>,
}

enum Proposals<'a> {
    Report {
        report: &'a Report,
        issues: Vec<Issue>,
    },
    Existing {
        target: &'a IssueUrl,
        issue: Box<Issue>,
    },
}

/// One proposed issue and how it is filed.
pub(super) struct Issue {
    proposed: ProposedIssue,
    filing: Filing,
}

/// How a proposed issue is labelled.
pub(super) struct Filing {
    /// The repository's category label, spelled as the repository spells it.
    category: String,
    /// The recommended state, after a confirmation block.
    triaged: Label,
    /// The state applied, after readiness permission.
    state: Label,
}

/// One proposed comment and the issue it goes to.
pub(super) struct Comment {
    proposed: ProposedComment,
    target: IssueUrl,
}

/// The proposals as one mode publishes them.
pub(super) enum Publishing<'r, 'a> {
    Report {
        report: &'a Report,
        issues: &'r mut [Issue],
        comments: &'r mut [Comment],
    },
    Existing {
        target: &'a IssueUrl,
        issue: &'r mut Issue,
        comments: &'r mut [Comment],
    },
}

impl<'a> Recommendation<'a> {
    /// Interpret the session's final `Triage recommendation:` line. A
    /// category error is reported with any other error, so one correction
    /// turn hears about both.
    pub fn interpret(message: Option<&str>, captured: &Captured<'a>) -> Result<Self> {
        let wire = read(message)?;
        let categories = wire
            .issues
            .iter()
            .map(|issue| {
                repository_category(
                    captured.categories,
                    issue.category.as_deref().unwrap_or(&wire.category),
                )
            })
            .collect::<Result<Vec<_>>>();
        let target = captured.input.issue();
        let (resolved, categories) = match (resolve(wire, target), categories) {
            (Ok(resolved), Ok(categories)) => (resolved, categories),
            (Err(error), Ok(_)) | (Ok(_), Err(error)) => return Err(error),
            (Err(error), Err(category)) => bail!("{error:#}; {category:#}"),
        };
        // An unpermitted readiness waits for the Day shift: a Report's issue
        // is triaged again, an existing issue goes to a human.
        let held = match target {
            None => labels::NEEDS_TRIAGE,
            Some(_) => labels::READY_FOR_HUMAN,
        };
        // Both are built from the same proposed issues, one entry each.
        let mut issues =
            resolved
                .issues
                .into_iter()
                .zip(categories)
                .map(|((proposed, triaged), category)| Issue {
                    proposed,
                    filing: Filing {
                        category: category.clone(),
                        triaged,
                        state: if triaged == labels::READY_FOR_AGENT
                            && !captured.allow_ready_for_agent
                        {
                            held
                        } else {
                            triaged
                        },
                    },
                });
        let proposals = match captured.input {
            Input::Report(report) => Proposals::Report {
                report,
                issues: issues.collect(),
            },
            Input::Issue(target) => {
                let mut issue = issues
                    .next()
                    .expect("resolve gives an existing target exactly one issue");
                issue.proposed.url = Some(target.url.clone());
                Proposals::Existing {
                    target,
                    issue: Box::new(issue),
                }
            }
        };
        Ok(Recommendation {
            category: resolved.category,
            state: resolved.state,
            investigation: resolved.investigation,
            proposals,
            comments: resolved.comments,
        })
    }

    /// The proposed comments, each with its parsed target, for the GitHub
    /// checks made before any write.
    pub fn comments(&self) -> &[Comment] {
        &self.comments
    }

    /// The proposals as this recommendation's mode publishes them.
    pub fn publishing(&mut self) -> Publishing<'_, 'a> {
        match &mut self.proposals {
            Proposals::Report { report, issues } => Publishing::Report {
                report,
                issues,
                comments: &mut self.comments,
            },
            Proposals::Existing { target, issue } => Publishing::Existing {
                target,
                issue,
                comments: &mut self.comments,
            },
        }
    }

    fn issues(&self) -> &[Issue] {
        match &self.proposals {
            Proposals::Report { issues, .. } => issues,
            Proposals::Existing { issue, .. } => std::slice::from_ref(issue.as_ref()),
        }
    }

    /// A readable Run notification, leaving full issue and comment bodies on stdout.
    /// Fields stay inside their records: a line break in session text must not
    /// open another `Issue:` line, and terminal controls must not be written out.
    pub fn notification(&self) -> String {
        let mut notification = format!(
            "Investigation:\n{}\n\nRecommendation: {} ({})\n",
            investigation_text(&self.investigation),
            one_line(&self.category),
            one_line(&self.state)
        );
        let issues = self.issues();
        if !issues.is_empty() {
            notification.push_str("\nIssues:\n");
            for issue in issues.iter().map(|issue| &issue.proposed) {
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
                notification.push_str(&format!("- {}\n", one_line(&comment.proposed.url)));
            }
        }
        for issue in issues.iter().map(|issue| &issue.proposed) {
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

/// Stdout's recommendation JSON: as recommended, with what publication did.
impl Serialize for Recommendation<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Output<'r> {
            category: &'r str,
            state: &'r str,
            issues: Vec<&'r ProposedIssue>,
            #[serde(skip_serializing_if = "Vec::is_empty")]
            comments: Vec<&'r ProposedComment>,
            investigation: &'r str,
        }
        Output {
            category: &self.category,
            state: &self.state,
            issues: self.issues().iter().map(|issue| &issue.proposed).collect(),
            comments: self
                .comments
                .iter()
                .map(|comment| &comment.proposed)
                .collect(),
            investigation: &self.investigation,
        }
        .serialize(serializer)
    }
}

impl Issue {
    pub fn title(&self) -> &str {
        &self.proposed.title
    }

    /// The body the session proposed.
    pub fn body(&self) -> &str {
        &self.proposed.body
    }

    /// What publication adds after the proposed body: a held readiness, a
    /// confirmation only the Day shift can make and any questionnaire.
    pub fn notes(&self) -> String {
        let mut notes = String::new();
        if self.filing.readiness_held() {
            notes.push_str(&format!(
                "\n\nready-for-agent was recommended; {} is applied until the Day shift permits readiness.",
                self.filing.state
            ));
        }
        if let Some(reason) = &self.proposed.confirmation_blocked {
            notes.push_str(&format!("\n\nConfirmation needs the Day shift: {reason}"));
        }
        if let Some(questionnaire) = &self.proposed.questionnaire {
            notes.push_str(&format!("\n\n## Questionnaire\n\n{questionnaire}"));
        }
        notes
    }

    pub fn filing(&self) -> &Filing {
        &self.filing
    }

    /// Record a Report's issue once filed: its URL, the body it was filed
    /// with, and the category and state it was filed under.
    pub fn record_filed(&mut self, issue: &IssueUrl, body: String) {
        self.proposed.url = Some(issue.url.clone());
        self.record(body, self.filing.category.clone());
    }

    /// Record an existing issue once relabelled: the comment it was given
    /// and the category it keeps.
    pub fn record_in_place(&mut self, body: String, category: String) {
        self.record(body, category);
    }

    fn record(&mut self, body: String, category: String) {
        self.proposed.body = body;
        self.proposed.category = Some(category);
        self.proposed.state = Some(self.filing.state.name().into());
    }
}

impl Filing {
    pub fn category(&self) -> &str {
        &self.category
    }

    pub fn state(&self) -> Label {
        self.state
    }

    /// `ready-for-agent` was recommended, but filing lacks permission for it.
    fn readiness_held(&self) -> bool {
        self.triaged == labels::READY_FOR_AGENT && self.state != labels::READY_FOR_AGENT
    }
}

impl Comment {
    pub fn target(&self) -> &IssueUrl {
        &self.target
    }

    /// The body the session proposed.
    pub fn body(&self) -> &str {
        &self.proposed.body
    }

    /// Record the body a landed comment was posted with.
    pub fn record_posted(&mut self, body: String) {
        self.proposed.body = body;
    }
}

/// Read the session's final recommendation line. The session's prose stays
/// out of the machine-readable stdout contract.
fn read(message: Option<&str>) -> Result<Wire> {
    let json = message
        .and_then(|message| {
            message
                .lines()
                .rev()
                .find_map(|line| line.strip_prefix("Triage recommendation: "))
        })
        .context("the Triage session ended without its final Triage recommendation: JSON line")?;
    serde_json::from_str(json).context("the Triage recommendation is not valid recommendation JSON")
}

/// A complete recommendation, each proposed issue with its triaged state.
struct Resolved {
    category: String,
    state: String,
    investigation: String,
    issues: Vec<(ProposedIssue, Label)>,
    comments: Vec<Comment>,
}

/// Check the recommendation is complete and resolve each proposed issue's
/// state; an existing `target` adds its own rules.
fn resolve(wire: Wire, target: Option<&IssueUrl>) -> Result<Resolved> {
    let Wire {
        category,
        state,
        issues,
        comments,
        investigation,
    } = wire;
    let overall = state_label(&state)
        .filter(|_| !category.trim().is_empty() && !investigation.trim().is_empty())
        .context(INCOMPLETE)?;
    let comments = comments
        .into_iter()
        .map(|proposed| {
            let target = IssueUrl::parse(&proposed.url)
                .ok()
                .filter(|_| !proposed.body.trim().is_empty())?;
            Some(Comment { proposed, target })
        })
        .collect::<Option<Vec<_>>>()
        .context(INCOMPLETE)?;
    let issues = issues
        .into_iter()
        .map(|issue| resolve_issue(issue, overall))
        .collect::<Result<Vec<_>>>()?;
    if let Some(target) = target {
        check_existing(&issues, &comments, target)?;
    }
    Ok(Resolved {
        category,
        state,
        investigation,
        issues,
        comments,
    })
}

fn resolve_issue(mut issue: ProposedIssue, overall: Label) -> Result<(ProposedIssue, Label)> {
    issue.questionnaire = issue
        .questionnaire
        .take()
        .filter(|text| !text.trim().is_empty());
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
        .map_or(Some(overall), state_label)
        .context(INCOMPLETE)?;
    // Only the Day shift can confirm a problem that needs credentials or
    // production access, whatever the session recommended.
    let triaged = if issue.confirmation_blocked.is_some() {
        labels::READY_FOR_HUMAN
    } else {
        recommended
    };
    ensure!(
        issue.questionnaire.is_some() == (triaged == labels::NEEDS_INFO),
        "the Triage questionnaire is required only for needs-info issues: {}",
        issue.title
    );
    Ok((issue, triaged))
}

fn check_existing(
    issues: &[(ProposedIssue, Label)],
    comments: &[Comment],
    target: &IssueUrl,
) -> Result<()> {
    let [(issue, triaged)] = issues else {
        bail!(ONE_ISSUE);
    };
    ensure!(
        *triaged != labels::NEEDS_TRIAGE,
        "existing-issue Triage must recommend ready-for-agent, ready-for-human, needs-info or wontfix"
    );
    if *triaged == labels::READY_FOR_AGENT {
        ensure!(
            issue
                .body
                .lines()
                .any(|line| line.trim() == "## Agent Brief"),
            "existing-issue ready-for-agent requires ## Agent Brief"
        );
    }
    ensure!(
        comments.iter().all(|comment| {
            comment.target.number != target.number || !comment.target.in_same_repo(target)
        }),
        "the target issue's single recommendation entry supplies its comment"
    );
    Ok(())
}

fn state_label(state: &str) -> Option<Label> {
    std::iter::once(labels::READY_FOR_AGENT)
        .chain(labels::UNREADY)
        .find(|label| label.is_named(state))
}

/// Whether thirdshift treats `name` as workflow state. The session may name
/// a repository category; these stay factory-owned, whatever their case.
pub(super) fn is_factory_label(name: &str) -> bool {
    state_label(name).is_some()
        || crate::claim::IN_PROGRESS.is_named(name)
        || crate::claim::AGENT_FAILED.is_named(name)
        || crate::base_fix::BASE_FIX.is_named(name)
        || labels::FLAKY_TEST.is_named(name)
        || crate::security::fixing::SECURITY_FIX.is_named(name)
        || crate::architect::is_mark(name)
        || crate::github::is_security_finding_label(name)
}

/// The repository's own spelling of a recommended category, matched case-insensitively.
fn repository_category<'c>(categories: &'c [String], wanted: &str) -> Result<&'c String> {
    categories
        .iter()
        .find(|name| name.eq_ignore_ascii_case(wanted))
        .context(CATEGORY_ERROR)
}

/// One notification field. Line breaks and terminal controls would otherwise
/// open another record, such as a forged `Issue:` line.
pub(super) fn one_line(text: &str) -> String {
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

#[cfg(test)]
mod tests;
