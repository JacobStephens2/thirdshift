//! A pull request stays reserved through Review-run cleanup, even on interruption.

use anyhow::Result;

use super::PullRequestUrl;
use crate::claim::IN_PROGRESS;
use crate::git::Git;
use crate::github::GitHub;
use crate::labels::Edit;

pub(crate) struct Reservation {
    pr: PullRequestUrl,
    // Drop the label before the origin ref allows another owner in.
    _origin: crate::reservation::Reservation,
}

impl Reservation {
    pub fn take(launch: &Git, github: &GitHub, pr: &PullRequestUrl) -> Result<Option<Self>> {
        let Some(origin) = crate::reservation::Reservation::take(launch, &pr.issue)? else {
            return Ok(None);
        };
        let labels = github.review_labels(pr)?;
        if labels.has(IN_PROGRESS) {
            return Ok(None);
        }
        let held = Self {
            pr: pr.clone(),
            _origin: origin,
        };
        // Arm cleanup before the write: a failed response may still have
        // applied the label remotely.
        Edit::of(&pr.issue, labels, &[], &[IN_PROGRESS]).apply(github)?;
        Ok(Some(held))
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        let github = GitHub::new().completion();
        let ended = (|| {
            let labels = github.review_labels(&self.pr)?;
            Edit::of(&self.pr.issue, labels, &[IN_PROGRESS], &[]).apply(&github)
        })();
        if let Err(error) = ended {
            crate::progress::warn(
                &error,
                format_args!(
                    "could not remove in-progress from {}; remove it by hand",
                    self.pr.url
                ),
            );
        }
    }
}
