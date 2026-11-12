//! Where runs failed: the window's Failed Commands by the stage they stopped
//! at and how they failed, each with the Commands to open.

use std::collections::BTreeMap;

use serde::Serialize;

use super::outcomes::{Class, Fact};
use super::scope::Window;
use crate::logs::FailureCategory;

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

/// How a Failed Command ended: as it recorded, or, when it recorded nothing,
/// whether it died or failed.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Ending {
    Recorded(FailureCategory),
    Died,
    Failed,
}

impl Ending {
    /// `held`, `waiting`, `crashed` or `stage_failed` when recorded; else
    /// `died` or `failed`.
    pub(crate) fn category(self) -> &'static str {
        match self {
            Self::Recorded(category) => category.name(),
            Self::Died => "died",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn recorded(self) -> bool {
        matches!(self, Self::Recorded(_))
    }

    /// The Ending a `category` names, as a drilldown link carries it.
    pub(crate) fn named(category: &str) -> Option<Self> {
        FailureCategory::ALL
            .into_iter()
            .map(Self::Recorded)
            .chain([Self::Died, Self::Failed])
            .find(|ending| ending.category() == category)
    }
}

impl Serialize for Ending {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Shown {
            category: &'static str,
            /// Whether the Commands recorded this stage and category themselves.
            recorded: bool,
        }
        Shown {
            category: self.category(),
            recorded: self.recorded(),
        }
        .serialize(serializer)
    }
}

/// The Failed Commands that stopped at one stage in one way.
#[derive(Serialize)]
pub(crate) struct Stopped {
    /// Missing when the Command recorded no stage at all.
    pub(crate) stopped_at: Option<String>,
    #[serde(flatten)]
    pub(crate) ending: Ending,
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
        let (stopped_at, ending) = match record.failure_category {
            Some(category) => (record.stopped_at.clone(), Ending::Recorded(category)),
            None => (
                record.last_stage().map(str::to_owned),
                if fact.command.state == "died" {
                    Ending::Died
                } else {
                    Ending::Failed
                },
            ),
        };
        breakdown.failed += 1;
        if ending.recorded() {
            breakdown.observed += 1;
        } else {
            breakdown.missing += 1;
        }
        rows.entry((stopped_at, ending))
            .or_default()
            .push(FailedCommand {
                name: record.name.clone(),
                kind: fact.counted.kind,
                account: fact.counted.account.clone(),
                started_at: record.started_at.clone(),
                selector: fact.counted.selector.clone(),
            });
    }
    breakdown.rows = rows
        .into_iter()
        .map(|((stopped_at, ending), commands)| Stopped {
            stopped_at,
            ending,
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
