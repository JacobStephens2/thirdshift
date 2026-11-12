//! Outcomes by `thirdshift` release, and the buckets where the release, or
//! under a narrowed view the Harness or Model, changed.

use std::collections::{BTreeMap, BTreeSet};

use chrono::NaiveDate;
use serde::Serialize;

use super::outcomes::{Fact, Tally};
use super::scope::{Filters, Window};

#[derive(Serialize)]
pub(crate) struct Versions {
    pub(crate) rows: Vec<Version>,
    pub(crate) transitions: Vec<Transition>,
}

/// The window's Commands started by one release, or by none that is known.
#[derive(Serialize)]
pub(crate) struct Version {
    pub(crate) thirdshift_version: Option<String>,
    /// Commands whose record names the release.
    pub(crate) recorded: usize,
    /// Older Commands whose release came from their Session traces.
    pub(crate) recovered: usize,
    #[serde(flatten)]
    pub(crate) tally: Tally,
}

/// A bucket whose Commands ran on a different set of values than the last
/// bucket with any.
#[derive(Serialize)]
pub(crate) struct Transition {
    pub(crate) bucket: NaiveDate,
    pub(crate) dimension: Dimension,
    pub(crate) from: BTreeSet<String>,
    pub(crate) to: BTreeSet<String>,
}

/// What changed between buckets.
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Dimension {
    ThirdshiftVersion,
    Harness,
    Model,
}

impl Dimension {
    /// The word a transition's text begins with.
    fn label(self) -> &'static str {
        match self {
            Self::ThirdshiftVersion => "thirdshift",
            Self::Harness => "Harness",
            Self::Model => "Model",
        }
    }

    fn of(self, fact: &Fact<'_>) -> Option<String> {
        let record = &fact.command.record;
        match self {
            Self::ThirdshiftVersion => record.thirdshift_version.clone(),
            Self::Harness => Some(record.harness.clone()),
            Self::Model => record.model.clone(),
        }
    }
}

impl Transition {
    pub(crate) fn text(&self) -> String {
        let join =
            |values: &BTreeSet<String>| values.iter().cloned().collect::<Vec<_>>().join(", ");
        format!(
            "{} {} → {}",
            self.dimension.label(),
            join(&self.from),
            join(&self.to)
        )
    }
}

pub(super) fn read(facts: &[Fact<'_>], window: &Window, filters: &Filters) -> Versions {
    let shown: Vec<_> = facts
        .iter()
        .filter(|fact| window.shows(fact.counted.bucket))
        .collect();
    let mut rows = BTreeMap::<_, Version>::new();
    for fact in &shown {
        let record = &fact.command.record;
        let row = rows
            .entry(record.thirdshift_version.clone())
            .or_insert_with(|| Version {
                thirdshift_version: record.thirdshift_version.clone(),
                recorded: 0,
                recovered: 0,
                tally: Tally::default(),
            });
        if record.version_recovered {
            row.recovered += 1;
        } else if record.thirdshift_version.is_some() {
            row.recorded += 1;
        }
        row.tally.add(&fact.counted);
    }
    let mut dimensions = vec![Dimension::ThirdshiftVersion];
    // Unnarrowed, Commands of every setup share each bucket, so a change in
    // the mix of Harnesses or Models is not a change of setup.
    if filters.narrowed() {
        dimensions.extend([Dimension::Harness, Dimension::Model]);
    }
    let mut transitions = Vec::new();
    for dimension in dimensions {
        let mut buckets = BTreeMap::<NaiveDate, BTreeSet<String>>::new();
        for fact in &shown {
            if let Some(value) = dimension.of(fact) {
                buckets
                    .entry(fact.counted.bucket)
                    .or_default()
                    .insert(value);
            }
        }
        let mut previous: Option<BTreeSet<String>> = None;
        for (bucket, values) in buckets {
            if let Some(from) = previous.take()
                && from != values
            {
                transitions.push(Transition {
                    bucket,
                    dimension,
                    from,
                    to: values.clone(),
                });
            }
            previous = Some(values);
        }
    }
    transitions.sort_by_key(|transition| transition.bucket);
    Versions {
        rows: rows.into_values().collect(),
        transitions,
    }
}
