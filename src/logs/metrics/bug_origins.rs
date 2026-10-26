//! Join Scoring's confirmed PRs to the already captured local Commands.

use std::collections::BTreeMap;

use anyhow::Result;

use super::super::reading::{Command, CommandIdentity, PullRequestAttribution, Snapshot};

#[derive(Default)]
pub(super) struct ConfirmedCounts(BTreeMap<CommandIdentity, usize>);

impl ConfirmedCounts {
    pub(super) fn read(snapshot: &Snapshot) -> Result<Self> {
        let attribution = PullRequestAttribution::new(snapshot);
        let mut counts = Self::default();
        for pr in crate::scoring::read_confirmed_origins()? {
            let run = match &pr.local_run {
                Some((command_record, name)) => attribution.exact(&pr.url, command_record, name),
                _ => attribution.latest(&pr.url),
            };
            if let Some(run) = run {
                *counts.0.entry(run.clone()).or_default() += 1;
            }
        }
        Ok(counts)
    }

    pub(super) fn for_run(&self, command: &Command) -> usize {
        self.0
            .get(&CommandIdentity::of(command))
            .copied()
            .unwrap_or(0)
    }
}
