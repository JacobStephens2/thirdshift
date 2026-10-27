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
        let review = match review {
            Err(error) if error.is::<UntrustedEdit>() => return Err(error),
            review => review,
        };
        let outcome = Outcome::of(review, self.sessions.last_log_of(Kind::SecurityReview));
        pull_request.security_reviewed(outcome.unaddressed_count());
        Ok(outcome)
    }
}
