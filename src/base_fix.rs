//! The Base fix (ADR-0008): a Merge run into a Run's Base branch, on an issue
//! thirdshift writes, that a Run asked to starts when its only red checks are
//! Inherited failures, and waits for.

use anyhow::{Result, bail};

use crate::args;
use crate::child_run::{self, Ended};
use crate::ci;
use crate::github::{self, Check};
use crate::issue::IssueUrl;
use crate::progress;

/// The labels of a Base fix issue, each with the description it is added to
/// the repository with if the repository lacks it.
const LABELS: [(&str, &str); 2] = [
    ("base-fix", "A Base fix: CI is red on a Base branch"),
    ("ready-for-agent", "Ready for an agent to take on"),
];

/// What a Run does when its only red checks are Inherited failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Fail, saying to fix the Base branch first.
    FailTheRun,
    /// Start a Base fix, once, and go on when it has merged.
    StartBaseFix,
    /// The Run is itself a Base fix: every red check is its own to fix, so it
    /// sees no Inherited failures and starts no Base fix.
    IsBaseFix,
}

/// A Run's one Base fix: whether it may start one, and the one it started.
pub struct BaseFix {
    policy: Policy,
    started: Option<Started>,
}

/// A Base fix a Run started.
struct Started {
    issue: IssueUrl,
    merged: bool,
}

impl BaseFix {
    pub fn new(policy: Policy) -> Self {
        BaseFix {
            policy,
            started: None,
        }
    }

    /// Whether the Run compares its red checks with the Base branch's, so
    /// that some may be Inherited failures.
    pub fn sees_inherited_failures(&self) -> bool {
        self.policy != Policy::IsBaseFix
    }

    /// Fix the checks `inherited`, the only red ones on the PR `pr_url` of
    /// the Run on `run` and all Inherited failures from `base` at
    /// `base_commit`, with a Base fix: write its issue, start it as a child
    /// `thirdshift` and wait for it to merge, after which the Run is to merge
    /// `base` in and watch CI again. Fails, with the Run's cause, if the Run
    /// was not asked to start one, if the Base fix fails, in which case the
    /// cause names its issue, or if the Run has had its one Base fix, in
    /// which case the cause names that one's issue.
    pub fn fix(
        &mut self,
        run: &IssueUrl,
        pr_url: &str,
        base: &str,
        base_commit: &str,
        inherited: &[Check],
    ) -> Result<()> {
        let checks = ci::check_names(inherited);
        let base_at = ci::short(base_commit);
        if let Some(started) = &self.started {
            bail!(
                "CI red on {checks}, which also fails on {base} at {base_at}, \
                 even after Base fix {} merged; fix {base} first",
                started.issue.url
            );
        }
        if self.policy != Policy::StartBaseFix {
            bail!("CI red on {checks}, which also fails on {base} at {base_at}; fix {base} first");
        }
        let issue = github::create_issue(
            run,
            &format!("CI red on {base}: {checks}"),
            &issue_body(run, pr_url, base, base_at, inherited),
            &LABELS,
        )?;
        let number = issue.number;
        progress::step(format_args!(
            "starting Base fix #{number} into {base}: {}",
            issue.url
        ));
        let started = self.started.insert(Started {
            issue,
            merged: false,
        });
        let child = child_run::start(&started.issue, [args::BASE_FIX_INTO, base])?;
        progress::step(format_args!("waiting on Base fix #{number}"));
        match child_run::wait(number, child)? {
            Ended::Reached(_) => {
                started.merged = true;
                progress::step(format_args!(
                    "Base fix #{number} merged; merging {base} in again"
                ));
                Ok(())
            }
            Ended::Interrupted => bail!("interrupted"),
            Ended::Failed { cause, log } => {
                if let Some(log) = log {
                    progress::step(format_args!("Base fix #{number} session log: {log}"));
                }
                bail!("Base fix {} failed: {cause}", started.issue.url)
            }
        }
    }

    /// What became of the Base fix the Run started, if it started one, for
    /// its Run notification: the issue's URL, then `merged` or `not merged`.
    pub fn report(&self) -> Option<String> {
        let started = self.started.as_ref()?;
        let outcome = if started.merged {
            "merged"
        } else {
            "not merged"
        };
        Some(format!("{} {outcome}", started.issue.url))
    }
}

/// The body of the Base fix issue, from a fixed template: the checks
/// `inherited` with their URLs, the Base branch `base` and its commit
/// `base_at`, shortened, and the Run that found them, on `run` with the PR
/// `pr_url`.
fn issue_body(
    run: &IssueUrl,
    pr_url: &str,
    base: &str,
    base_at: &str,
    inherited: &[Check],
) -> String {
    let mut checks = String::new();
    for check in inherited {
        checks += &match &check.url {
            Some(url) => format!("- {}: {url}\n", check.name),
            None => format!("- {}\n", check.name),
        };
    }
    format!(
        "CI is red on `{base}` at {base_at}: these checks fail there, so every pull request into `{base}` inherits them.\n\
         \n\
         {checks}\
         \n\
         Found by the thirdshift Run on #{}, whose pull request is {pr_url}.\n\
         \n\
         Fix the checks on `{base}`. Do not skip, disable, or weaken tests or checks to make them pass.\n",
        run.number
    )
}
