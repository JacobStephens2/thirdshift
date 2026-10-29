//! Atomic, per-issue exclusion between Pickup runs on different machines,
//! and between a Pickup run and the Architect run or Security run that marks
//! its own issue ready and dispatches it. Label edits and ordinary Issue
//! branch pushes cannot arbitrate that race. A separate origin ref is
//! created only if absent, then held through dispatch.

use anyhow::{Context, Result};

use crate::git::Git;
use crate::issue::IssueUrl;
use crate::progress;

pub struct Reservation {
    launch: Git,
    reference: String,
    token: String,
}

impl Reservation {
    /// Create a unique commit and atomically reserve `issue` with an empty
    /// lease. Existing reservations are never adopted, expired or overwritten.
    /// Errors with an uncertain remote outcome retain the ref for recovery.
    pub fn take(launch: &Git, issue: &IssueUrl) -> Result<Option<Self>> {
        Self::take_ref(launch, reference(issue), &issue.url)
    }

    /// Reserve `issue`, which this run published and is about to mark
    /// ready and dispatch itself, as a Pickup run reserves its Ready issue,
    /// so that an overlapping Pickup run passes it over. Fails if another
    /// run already holds it.
    pub fn of_published(launch: &Git, issue: &IssueUrl) -> Result<Self> {
        Self::take(launch, issue)?.with_context(|| {
            format!(
                "#{} is already reserved on origin, at {}",
                issue.number,
                reference(issue)
            )
        })
    }

    /// Serialize CI-red Ticket publication across all Base branches and hosts.
    pub fn ci_red(launch: &Git) -> Result<Option<Self>> {
        Self::take_ref(
            launch,
            "refs/heads/thirdshift/pickup/ci-red".into(),
            "CI-red Ticket filing",
        )
    }

    fn take_ref(launch: &Git, reference: String, subject: &str) -> Result<Option<Self>> {
        let nonce = tempfile::Builder::new().rand_bytes(32).tempfile()?;
        // A reservation carries no source tree or parent: naming a Base branch
        // permits a Launch directory with unrelated, unpublished local work.
        let tree = launch.run(&["mktree"])?;
        let message = format!(
            "thirdshift Pickup reservation for {}\n\nToken: {}\nHost: {}\nLaunch: {}\nProcess: {}\nStarted: {}",
            subject,
            nonce.path().display(),
            crate::host::name().unwrap_or_else(|| "unknown".into()),
            launch.dir().display(),
            std::process::id(),
            chrono::Utc::now(),
        );
        let token = launch.run(&[
            "-c",
            "commit.gpgSign=false",
            "commit-tree",
            &tree,
            "-m",
            &message,
        ])?;
        let pushed = launch.run(&[
            "push",
            "--no-verify",
            &format!("--force-with-lease={reference}:"),
            "origin",
            &format!("{token}:{reference}"),
        ]);
        if let Err(error) = pushed {
            let remote = launch.run(&["ls-remote", "--refs", "origin", &reference])
                .with_context(|| format!(
                    "{error:#}; could not verify reservation {reference}; check origin by hand (our token {token})"
                ))?;
            if let Some((other, _)) = remote.split_once('\t')
                && other != token
            {
                return Ok(None);
            }
            return Err(error).with_context(|| format!(
                "could not reserve {reference}; check origin by hand before retrying (our token {token})"
            ));
        }
        Ok(Some(Self {
            launch: launch.completion(),
            reference,
            token,
        }))
    }
}

/// The ref that reserves `issue` on origin.
fn reference(issue: &IssueUrl) -> String {
    format!("refs/heads/thirdshift/pickup/{}", issue.number)
}

impl Drop for Reservation {
    fn drop(&mut self) {
        // Finish after interruption, and never delete another owner's token.
        if let Err(error) = self.launch.run(&[
            "push",
            "--no-verify",
            &format!("--force-with-lease={}:{}", self.reference, self.token),
            "origin",
            &format!(":{}", self.reference),
        ]) {
            progress::warn(
                &error,
                format_args!(
                    "could not release Pickup reservation {}; check origin by hand (our token {})",
                    self.reference, self.token
                ),
            );
        }
    }
}
