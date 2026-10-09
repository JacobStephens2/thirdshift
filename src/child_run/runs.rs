//! Owned concurrent child Runs.

use std::collections::{BTreeMap, VecDeque};
use std::thread;

use anyhow::{Result, bail};

use super::{Ended, Handle, Kind, POLL};
use crate::base_fix::BaseFixAsk;
use crate::harness::Choice;
use crate::issue::IssueUrl;
use crate::security::Options;

/// Own child Runs until their numbered endings are delivered or cleanup finishes.
#[derive(Default)]
pub struct Runs {
    running: BTreeMap<u64, Handle>,
    endings: VecDeque<(u64, Result<Ended>)>,
}

impl Runs {
    /// Start and own a child with the same launch facts as the standalone Run.
    /// An issue number stays outstanding until its ending is delivered;
    /// refusing a duplicate or failing to launch keeps earlier Runs owned.
    pub fn start(
        &mut self,
        issue: &IssueUrl,
        kind: Kind,
        base_fix: BaseFixAsk,
        security: Options,
        harness: &Choice,
    ) -> Result<()> {
        self.start_using(issue.number, || {
            super::start(issue, kind, base_fix, security, harness)
        })
    }

    /// Launch and retain ownership before returning to the caller.
    pub(super) fn start_using(
        &mut self,
        number: u64,
        launch: impl FnOnce() -> Result<Handle>,
    ) -> Result<()> {
        if self.running.contains_key(&number)
            || self.endings.iter().any(|(queued, _)| *queued == number)
        {
            bail!("the Run for #{number} is already outstanding");
        }
        let child = launch()?;
        self.running.insert(number, child);
        Ok(())
    }

    /// Poll every active child, then deliver the oldest numbered ending.
    /// Endings discovered together follow ascending issue-number order.
    /// Retirement follows reaping and reader joins; each ending is delivered
    /// once. Empty returns None immediately; otherwise wait at the child
    /// supervision cadence, including during Command interruption.
    pub fn next_ending(&mut self) -> Option<(u64, Result<Ended>)> {
        loop {
            // Sweep all children even with endings queued, so interruption
            // reaches every active child before any result is delivered.
            self.running.retain(|&number, child| {
                if let Some(ending) = child.try_wait() {
                    self.endings.push_back((number, ending));
                    false
                } else {
                    true
                }
            });
            if let Some(ending) = self.endings.pop_front() {
                return Some(ending);
            }
            if self.running.is_empty() {
                return None;
            }
            // GitHub's polling helper returns early on interruption; child
            // supervision keeps its cadence while graceful cleanup runs.
            thread::sleep(POLL);
        }
    }
}

impl Drop for Runs {
    fn drop(&mut self) {
        // Sibling salvage may rendezvous: notify every child before waiting
        // for any of them. Handle::stop is nonblocking and once-only.
        for child in self.running.values_mut() {
            child.stop();
        }
        // Reap and join through normal completion, without a deadline or a
        // synthetic Command interruption. Undelivered endings are discarded.
        while self.next_ending().is_some() {}
    }
}
