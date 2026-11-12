//! Existing-issue triage owns a reservation and a Claim until publication
//! succeeds or failure/Stop cleanup finishes. Report filing does neither.

use anyhow::{Context, Result, ensure};

use super::{Publication, Recommendation, is_factory_label};
use crate::claim::{AGENT_FAILED, IN_PROGRESS};
use crate::git::Git;
use crate::github::GitHub;
use crate::issue::IssueUrl;
use crate::labels::{self, Edit, Label};
use crate::trust::{self, Reads};

const STATES_AND_CLAIM: [Label; 7] = [
    labels::NEEDS_TRIAGE,
    labels::READY_FOR_HUMAN,
    labels::NEEDS_INFO,
    labels::READY_FOR_AGENT,
    labels::WONTFIX,
    AGENT_FAILED,
    IN_PROGRESS,
];

pub(super) fn check_target(issue: &IssueUrl, github: &GitHub) -> Result<()> {
    ensure!(
        github.issue_is_open(issue)?,
        "issue #{} is closed",
        issue.number
    );
    check_trust(issue, github)
}

pub(super) fn check_trust(issue: &IssueUrl, github: &GitHub) -> Result<()> {
    if let Some(reason) = trust_reason(issue, github)? {
        anyhow::bail!("issue #{} {reason}", issue.number);
    }
    Ok(())
}

/// A private finding's notification carries only its recorded title and link,
/// never the session's investigation, proposed issue text or questionnaire.
pub(super) fn finding_notification(issue: &IssueUrl, github: &GitHub) -> Result<Option<String>> {
    let current = github.issue(issue)?;
    Ok(current
        .labels
        .names()
        .any(crate::github::is_security_finding_label)
        .then(|| {
            format!(
                "Security finding: {}\nIssue: {}\nDetails remain on the private issue and in the Command log.\n",
                super::one_line(current.title.as_deref().unwrap_or("Security finding")),
                issue.url
            )
        }))
}

pub(super) fn trust_reason(issue: &IssueUrl, github: &GitHub) -> Result<Option<String>> {
    if !trust::trusted_association(&github.author_association(issue)?) {
        return Ok(Some("author is not a Trusted account".into()));
    }
    if let Some(text) = trust::untrusted_edit(github, issue, None)? {
        return Ok(Some(format!(
            "{text} was edited by an account that is not a Trusted account"
        )));
    }
    Ok(None)
}

pub(super) struct Claim<'a> {
    issue: &'a IssueUrl,
    completed: bool,
    // Field drop follows Claim::drop, so failure labels land before release.
    _origin: crate::reservation::Reservation,
}

impl<'a> Claim<'a> {
    pub fn take(launch: &Git, issue: &'a IssueUrl) -> Result<Self> {
        let origin = crate::reservation::Reservation::take(launch, issue)?
            .with_context(|| format!("#{} is already reserved on origin", issue.number))?;
        Self::reserved(issue, origin)
    }

    pub fn reserved(issue: &'a IssueUrl, origin: crate::reservation::Reservation) -> Result<Self> {
        let github = GitHub::new();
        check_target(issue, &github)?;
        let labels = github.issue_labels(issue)?;
        ensure!(
            !labels.has(IN_PROGRESS),
            "issue #{} is already Claimed",
            issue.number
        );
        // Arm cleanup before the request: a lost response may have applied it.
        let held = Self {
            issue,
            completed: false,
            _origin: origin,
        };
        Edit::of(issue, labels, &STATES_AND_CLAIM, &[IN_PROGRESS]).apply(&github)?;
        Ok(held)
    }

    pub fn completed(&mut self) {
        self.completed = true;
    }
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        let github = GitHub::new().completion();
        let on: &[Label] = match crate::logs::stop_requested().map(|request| request.ending()) {
            Some(crate::logs::stop::Ending::Requeue) => &[labels::NEEDS_TRIAGE],
            Some(crate::logs::stop::Ending::Triage) => &[labels::READY_FOR_HUMAN],
            None => &[labels::READY_FOR_HUMAN, AGENT_FAILED],
        };
        let ended = (|| {
            let labels = github.issue_labels(self.issue)?;
            Edit::of(self.issue, labels, &STATES_AND_CLAIM, on).apply(&github)
        })();
        if let Err(error) = ended {
            crate::progress::warn(
                &error,
                format_args!(
                    "could not end Triage Claim on #{}; run: {}",
                    self.issue.number,
                    labels::by_hand(self.issue, &STATES_AND_CLAIM, on)
                ),
            );
        }
    }
}

pub(super) fn check_recommendation(
    recommendation: &mut Recommendation,
    target: &IssueUrl,
) -> Result<()> {
    ensure!(
        recommendation.issues.len() == 1,
        "existing-issue Triage requires exactly one recommendation issue; propose splits in a ready-for-human issue body"
    );
    let issue = &mut recommendation.issues[0];
    ensure!(
        issue.triaged != Some(labels::NEEDS_TRIAGE),
        "existing-issue Triage must recommend ready-for-agent, ready-for-human, needs-info or wontfix"
    );
    if issue.triaged == Some(labels::READY_FOR_AGENT) {
        ensure!(
            issue
                .body
                .lines()
                .any(|line| line.trim() == "## Agent Brief"),
            "existing-issue ready-for-agent requires ## Agent Brief"
        );
    }
    ensure!(
        recommendation.comments.iter().all(|comment| {
            IssueUrl::parse(&comment.url).is_ok_and(|issue| {
                issue.number != target.number
                    || !issue.owner.eq_ignore_ascii_case(&target.owner)
                    || !issue.repo.eq_ignore_ascii_case(&target.repo)
            })
        }),
        "the target issue's single recommendation entry supplies its comment"
    );
    issue.url = Some(target.url.clone());
    Ok(())
}

pub(super) fn publish(
    recommendation: &mut Recommendation,
    publication: Publication,
    target: &IssueUrl,
    github: &GitHub,
    published: &mut Vec<IssueUrl>,
) -> Result<()> {
    let issue = &mut recommendation.issues[0];
    let filing = &publication.filings[0];
    let current = github.issue_labels(target)?;
    let categories: Vec<_> = current
        .names()
        .filter(|name| !is_factory_label(name))
        .collect();
    let category = categories
        .first()
        .copied()
        .unwrap_or(&filing.category)
        .to_string();
    let mut body = format!(
        "> *This was generated by AI during triage.*\n\n{}",
        issue.body
    );
    if filing.readiness_held {
        body.push_str("\n\nready-for-agent was recommended; ready-for-human is applied until the Day shift permits readiness.");
    }
    if let Some(reason) = &issue.confirmation_blocked {
        body.push_str(&format!("\n\nConfirmation needs the Day shift: {reason}"));
    }
    if let Some(questionnaire) = &issue.questionnaire {
        body.push_str(&format!("\n\n## Questionnaire\n\n{questionnaire}"));
    }
    github.comment_issue(target, &body)?;
    published.push(target.clone());
    let add_category = categories.is_empty();
    let mut edit = Edit::of(target, current, &STATES_AND_CLAIM, &[filing.state]);
    if add_category {
        edit = edit.with_repository_label(&filing.category);
    }
    edit.apply(github)?;
    issue.body = body;
    issue.category = Some(category);
    issue.state = Some(filing.state.name().into());
    if filing.state == labels::WONTFIX {
        github.close_triaged_issue(target)?;
    }
    for (comment, target) in recommendation.comments.iter_mut().zip(publication.targets) {
        comment.body = format!(
            "> *This was generated by AI during triage.*\n\n{}",
            comment.body
        );
        github.comment_issue(&target, &comment.body)?;
        published.push(target);
    }
    Ok(())
}
