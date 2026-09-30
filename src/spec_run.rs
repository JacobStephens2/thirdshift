//! A Spec run: each of a Spec's Tickets, in dependency order, taken by a
//! Ticket's Run, a Merge run into the Spec branch in a child `thirdshift`
//! (ADR-0006), then the Spec review and the Spec PR from the Spec branch into
//! the Base branch.

use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::args;
use crate::failed_run::{self, FailedRun};
use crate::github::{self, Ticket};
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::plugin::Plugin;
use crate::progress;
use crate::prompt;
use crate::run::{self, Goal, Reached};
use crate::session::{Logs, Sessions};
use crate::worktree::Worktree;

/// The triage labels that make an open Ticket an Unready Ticket.
const UNREADY_LABELS: [&str; 4] = ["ready-for-human", "needs-info", "wontfix", "needs-triage"];

/// The Spec review session's kind, in its progress lines and log name.
const SPEC_REVIEW: &str = "spec-review";

/// Take the Spec `spec`, whose Tickets were last read as `tickets`, from
/// its Spec branch, checked out in `worktree`, to a Spec PR into `base`,
/// ready for review. The Spec branch is pushed before any Ticket starts, and
/// the Tickets run one at a time, the graph read again after each. A Ticket
/// that fails ends the Spec run. Once every Ticket has landed, the Spec PR is
/// opened as a draft and the Spec review, logged in `logs`, reviews the Spec
/// branch before the Spec PR is marked ready; a Spec review that fails goes
/// through the Failed run path. The worktree is cleaned up when this returns,
/// or kept by the Failed run path if its work did not reach origin.
pub fn run(
    spec: &IssueUrl,
    tickets: Vec<Ticket>,
    worktree: Worktree,
    base: &str,
    logs: &Logs,
) -> Result<Reached, FailedRun> {
    land_tickets(spec, tickets, &worktree)?;
    let pr_url = open_spec_pr(spec, worktree.branch(), base)?;
    let mut log = logs.path(SPEC_REVIEW);
    if let Err(error) = review(spec, &worktree, base, &pr_url, logs, &mut log) {
        return Err(failed_run::fail(spec, worktree, base, &log, error));
    }
    Ok(Reached {
        pr_url,
        goal: Goal::ReadyForReview,
        log: Some(log),
    })
}

/// Push the Spec branch, then run each ready Ticket, lowest number first,
/// until none is ready. Fails if a Ticket fails, or if any is left open.
fn land_tickets(spec: &IssueUrl, mut tickets: Vec<Ticket>, worktree: &Worktree) -> Result<()> {
    worktree.push()?;
    let mut started = HashSet::new();
    while let Some(ticket) = next_ready(&tickets, &started) {
        if interrupt::requested() {
            bail!("interrupted");
        }
        started.insert(ticket);
        run_ticket(spec, ticket, worktree.branch())?;
        tickets = github::tickets(spec)?;
    }
    let open: Vec<String> = tickets
        .iter()
        .filter(|ticket| ticket.is_open)
        .map(|ticket| format!("#{}", ticket.number))
        .collect();
    if !open.is_empty() {
        bail!("no Ticket is ready, and {} still open", open.join(", "));
    }
    Ok(())
}

/// The lowest-numbered Ticket that is open, not an Unready Ticket, has no
/// sub-issues, has every blocker closed and hasn't been `started` yet.
fn next_ready(tickets: &[Ticket], started: &HashSet<u64>) -> Option<u64> {
    tickets
        .iter()
        .filter(|ticket| {
            ticket.is_open
                && !ticket.has_sub_issues
                && ticket.open_blockers.is_empty()
                && !started.contains(&ticket.number)
                && !ticket
                    .labels
                    .iter()
                    .any(|label| UNREADY_LABELS.contains(&label.as_str()))
        })
        .map(|ticket| ticket.number)
        .min()
}

/// Run Ticket `number` of `spec` as a child `thirdshift`, a Merge run into
/// `spec_branch` from the same Launch directory, relaying its stderr with a
/// `#<number>: ` prefix. Fails unless it exits 0.
fn run_ticket(spec: &IssueUrl, number: u64, spec_branch: &str) -> Result<()> {
    progress::step(format_args!("starting #{number}"));
    let ticket = spec.sibling(number);
    let mut child = Command::new(std::env::current_exe().context("no thirdshift executable")?)
        .args([args::SPEC_BRANCH, spec_branch, &ticket.url])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not start the Run for #{number}"))?;
    let stderr = child.stderr.take().context("no stderr from the Run")?;
    for line in BufReader::new(stderr).lines() {
        let line = line.with_context(|| format!("could not read the Run for #{number}"))?;
        let line = line.strip_prefix("thirdshift: ").unwrap_or(&line);
        progress::step(format_args!("#{number}: {line}"));
    }
    let status = child
        .wait()
        .with_context(|| format!("could not wait for the Run for #{number}"))?;
    if !status.success() {
        bail!("#{number} failed");
    }
    progress::step(format_args!("#{number} landed"));
    Ok(())
}

/// The Spec PR from `spec_branch` into `base`: the open one if there is one,
/// else a new draft titled from the Spec that closes it.
fn open_spec_pr(spec: &IssueUrl, spec_branch: &str, base: &str) -> Result<String> {
    if let Some(pr) = github::pull_request_for(spec, spec_branch)?.filter(|pr| pr.is_open()) {
        return Ok(pr.url);
    }
    progress::step(format_args!("opening the Spec PR into {base} as a draft"));
    let title = github::issue_title(spec)?;
    let body = format!(
        "The work on Spec #{number}, gathered from its Tickets on {spec_branch}.\n\n\
         Closes #{number}\n",
        number = spec.number
    );
    github::create_draft_pr(spec, spec_branch, base, &title, &body)
}

/// Bring the Spec branch in `worktree` up to date with the Tickets landed on
/// origin, and run the Spec review on it, pointing `log` at its session's
/// log. Then push the Spec branch, for any commit the session left unpushed,
/// and mark the Spec PR `pr_url` into `base` ready.
fn review(
    spec: &IssueUrl,
    worktree: &Worktree,
    base: &str,
    pr_url: &str,
    logs: &Logs,
    log: &mut PathBuf,
) -> Result<()> {
    let spec_branch = worktree.branch();
    worktree.fast_forward_to_origin()?;
    let plugin = Plugin::write()?;
    let sessions = Sessions {
        logs,
        worktree: worktree.path(),
        plugin_dir: plugin.path(),
    };
    let prompt = prompt::spec_review(spec, base, spec_branch, pr_url);
    sessions.run(SPEC_REVIEW, &prompt, log)?;
    worktree.push()?;
    run::mark_pr_ready(spec, spec_branch, base)?;
    Ok(())
}
