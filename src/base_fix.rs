//! The Base fix (ADR-0008): a Merge run into a Run's Base branch, on an issue
//! thirdshift writes, that a Run asked to starts when its only red checks are
//! Inherited failures, and waits for.

use anyhow::{Result, bail};

use crate::child_run::{self, Ended, Kind};
use crate::ci;
use crate::github::{self, Check, CheckState};
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
enum OnInheritedFailures {
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
    on_inherited_failures: OnInheritedFailures,
    started: Option<Started>,
}

/// A Base fix a Run started.
struct Started {
    issue: IssueUrl,
    merged: bool,
}

impl BaseFix {
    /// The Base fix of a Run that is the child Run `child`, if it is one,
    /// and that, with `asked`, was given `base-fix`. A Base fix starts none
    /// of its own, whatever it was given.
    pub fn new(child: Option<&Kind>, asked: bool) -> Self {
        let on_inherited_failures = match (child, asked) {
            (Some(Kind::BaseFix { .. }), _) => OnInheritedFailures::IsBaseFix,
            (_, true) => OnInheritedFailures::StartBaseFix,
            (_, false) => OnInheritedFailures::FailTheRun,
        };
        BaseFix {
            on_inherited_failures,
            started: None,
        }
    }

    /// Whether the Run compares its red checks with the Base branch's, so
    /// that some may be Inherited failures.
    pub fn sees_inherited_failures(&self) -> bool {
        self.on_inherited_failures != OnInheritedFailures::IsBaseFix
    }

    /// Whether the Run may start a Base fix, as may then each Ticket's Run
    /// of a Spec run.
    pub fn may_start(&self) -> bool {
        self.on_inherited_failures == OnInheritedFailures::StartBaseFix
    }

    /// Fix the checks `inherited`, the only red ones on the PR `pr_url` of
    /// the Run on `issue` and all Inherited failures from `base` at
    /// `base_commit`, with a Base fix: write its issue, start it as a child
    /// `thirdshift` and wait for it to merge, after which the Run is to merge
    /// `base` in and watch CI again. Fails, with the Run's cause, if the Run
    /// was not asked to start one, if the Base fix fails, in which case the
    /// cause names its issue, or if the Run has had its one Base fix, in
    /// which case the cause names that one's issue.
    pub fn fix(
        &mut self,
        issue: &IssueUrl,
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
        if !self.may_start() {
            bail!("CI red on {checks}, which also fails on {base} at {base_at}; fix {base} first");
        }
        // The issue links the Base branch's own failed checks, not the PR's.
        let on_base: Vec<Check> = github::checks_on(issue, base_commit)?
            .into_iter()
            .filter(|check| {
                check.state == CheckState::Failed
                    && inherited.iter().any(|failed| failed.name == check.name)
            })
            .collect();
        let fix = github::create_issue(
            issue,
            &format!("CI red on {base}: {checks}"),
            &issue_body(issue, pr_url, base, base_at, &on_base),
            &LABELS,
        )?;
        let number = fix.number;
        progress::step(format_args!(
            "starting Base fix #{number} into {base}: {}",
            fix.url
        ));
        let started = self.started.insert(Started {
            issue: fix,
            merged: false,
        });
        let kind = Kind::BaseFix {
            base: base.to_string(),
        };
        let child = child_run::start(&started.issue, &kind, false)?;
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
/// `failed` on the Base branch `base` at its commit `base_at`, shortened, with
/// their URLs, and the Run that found them, on `issue` with the PR `pr_url`.
fn issue_body(
    issue: &IssueUrl,
    pr_url: &str,
    base: &str,
    base_at: &str,
    failed: &[Check],
) -> String {
    let mut checks = String::new();
    for check in failed {
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
        issue.number
    )
}
