//! Where runs failed: the window's Failed Commands by the stage they stopped
//! at and how they failed, each with the Commands to open.

use std::collections::BTreeMap;

use serde::Serialize;

use super::outcomes::{Class, Fact};
use super::scope::Window;

/// Every Failed Command in the window, died ones included. `observed`
/// Commands recorded where and how they failed; `missing` ones, older or
/// died, fall back to their last recorded stage and `died` or `failed`.
#[derive(Default, Serialize)]
pub(crate) struct WhereRunsFailed {
    pub(crate) failed: usize,
    pub(crate) observed: usize,
    pub(crate) missing: usize,
    pub(crate) rows: Vec<Stopped>,
}

/// The Failed Commands that stopped at one stage in one way.
#[derive(Serialize)]
pub(crate) struct Stopped {
    /// Missing when the Command recorded no stage at all.
    pub(crate) stopped_at: Option<String>,
    /// `held`, `waiting`, `crashed` or `stage_failed` when recorded; else
    /// `died` or `failed`.
    pub(crate) category: &'static str,
    /// Whether the Commands recorded this stage and category themselves.
    pub(crate) recorded: bool,
    pub(crate) failed: usize,
    pub(crate) commands: Vec<FailedCommand>,
}

impl Stopped {
    pub(crate) fn selectors(&self) -> impl Iterator<Item = &String> {
        self.commands.iter().map(|command| &command.selector)
    }
}

#[derive(Serialize)]
pub(crate) struct FailedCommand {
    pub(crate) name: String,
    pub(crate) kind: &'static str,
    pub(crate) account: Option<String>,
    pub(crate) started_at: String,
    /// `thirdshift show <selector>`, and the Console's Command details.
    pub(crate) selector: String,
}

pub(super) fn read(facts: &[Fact<'_>], window: &Window) -> WhereRunsFailed {
    let mut rows = BTreeMap::<_, Vec<FailedCommand>>::new();
    let mut breakdown = WhereRunsFailed::default();
    for fact in facts
        .iter()
        .filter(|fact| fact.counted.class == Class::Failed && window.shows(fact.counted.bucket))
    {
        let record = &fact.command.record;
        let key = match record.failure_category {
            Some(category) => (record.stopped_at.clone(), category.name(), true),
            None => (
                record.stages.last().map(|stage| stage.name.clone()),
                if fact.command.state == "died" {
                    "died"
                } else {
                    "failed"
                },
                false,
            ),
        };
        breakdown.failed += 1;
        if key.2 {
            breakdown.observed += 1;
        } else {
            breakdown.missing += 1;
        }
        rows.entry(key).or_default().push(FailedCommand {
            name: record.name.clone(),
            kind: fact.counted.kind,
            account: fact.counted.account.clone(),
            started_at: record.started_at.clone(),
            selector: fact.counted.selector.clone(),
        });
    }
    breakdown.rows = rows
        .into_iter()
        .map(|((stopped_at, category, recorded), commands)| Stopped {
            stopped_at,
            category,
            recorded,
            failed: commands.len(),
            commands,
        })
        .collect();
    // The largest cluster first; ties keep stage and category order.
    breakdown
        .rows
        .sort_by_key(|row| std::cmp::Reverse(row.failed));
    breakdown
}
