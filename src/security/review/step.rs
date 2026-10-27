//! The Security review catalogue Step, shared by Ticket runs and Processes.

use anyhow::Result;

use super::{Outcome, run};
use crate::delivery::UntrustedEdit;
use crate::issue::IssueUrl;
use crate::pull_request::PullRequest;
use crate::session::{Kind, Sessions};
use crate::worktree::Worktree;

pub(crate) struct Step<'a> {
    pub sessions: &'a Sessions<'a>,
    pub worktree: &'a Worktree,
    pub issue: &'a IssueUrl,
    pub base: &'a str,
}

impl Step<'_> {
    /// Session failures become an outcome that holds Self-merge after
    /// publication. An untrusted issue edit stops the Process immediately.
    pub fn run(self, pull_request: &mut PullRequest, prompt: &str) -> Result<Outcome> {
        let review: Result<super::Review> = (|| {
            pull_request.ensure_actionable(false)?;
            let review = run(self.sessions, self.worktree, self.issue, self.base, prompt)?;
            pull_request.ensure_actionable(false)?;
            Ok(review)
        })();
        if review
            .as_ref()
            .err()
            .is_some_and(|error| error.is::<UntrustedEdit>())
        {
            return Err(review.err().expect("an untrusted edit"));
        }
        pull_request.security_reviewed(review.as_ref().map_or(1, |review| review.findings.len()));
        Ok(Outcome::of(
            review,
            self.sessions.last_log_of(Kind::SecurityReview),
        ))
    }
}
