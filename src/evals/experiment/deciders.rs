//! Who decides a Verdict, as ADR 0028 fixes it in the Bar: the Grader panel
//! and the people each off, beside or counted, the people's seeded sample of
//! pairs per reader, and the merge call that counts for each pair.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use rand::{SeedableRng, seq::SliceRandom};
use rand_pcg::Pcg64;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::pairwise::{Agreement, Review};
use super::panel::{Merge, PanelPair};
use super::trial_results::files::Files;
use super::{Bar, Description, Task, trial_results};
use crate::evals::format;

/// Where a Grader's calls go.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Role {
    Off,
    #[default]
    Beside,
    Counted,
}

impl Role {
    pub(super) fn parse(flag: &str, text: &str) -> Result<Self> {
        Ok(match text {
            "off" => Self::Off,
            "beside" => Self::Beside,
            "counted" => Self::Counted,
            _ => bail!("invalid {flag} {text}: expected off, beside or counted"),
        })
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Beside => "beside",
            Self::Counted => "counted",
        }
    }
}

/// The people's setting the Bar fixes before the first Trial.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PeopleSetting {
    pub role: Role,
    /// Each reader as the Console or the CLI names them.
    pub readers: Vec<String>,
    /// Pairs per reader; absent when every reader reads every pair.
    pub sample: Option<usize>,
    /// Pairs every reader's sample shares.
    pub overlap: usize,
}

impl PeopleSetting {
    /// The `--people` flags. No flag, or people beside with no readers,
    /// keeps the setting every Experiment had before: anyone reads any pair,
    /// beside the Verdict.
    pub(super) fn parse(
        flags: &mut BTreeMap<&str, &str>,
        readers: Vec<String>,
    ) -> Result<Option<Self>> {
        let role = flags
            .remove("--people")
            .map(|text| Role::parse("--people", text))
            .transpose()?;
        let mut number = |flag: &str| {
            flags
                .remove(flag)
                .map(|value| {
                    value
                        .parse::<usize>()
                        .with_context(|| format!("invalid {flag}: {value}"))
                })
                .transpose()
        };
        let sample = number("--people-sample")?;
        let overlap = number("--people-overlap")?;
        for (index, reader) in readers.iter().enumerate() {
            ensure!(
                !reader.trim().is_empty(),
                "--people-reader must not be empty"
            );
            ensure!(
                !readers[..index].contains(reader),
                "repeated --people-reader {reader}"
            );
        }
        let role = role.unwrap_or_default();
        ensure!(
            role != Role::Off || (readers.is_empty() && sample.is_none() && overlap.is_none()),
            "--people off takes no --people-reader, --people-sample or --people-overlap"
        );
        ensure!(
            role != Role::Counted || !readers.is_empty(),
            "--people counted needs at least one --people-reader"
        );
        ensure!(
            sample.is_none() || !readers.is_empty(),
            "--people-sample needs at least one --people-reader"
        );
        ensure!(sample != Some(0), "--people-sample must be positive");
        ensure!(
            overlap.is_none() || sample.is_some(),
            "--people-overlap needs --people-sample"
        );
        let overlap = overlap.unwrap_or(0);
        ensure!(
            sample.is_none_or(|sample| overlap <= sample),
            "--people-overlap cannot exceed --people-sample"
        );
        if role == Role::Beside && readers.is_empty() {
            return Ok(None);
        }
        Ok(Some(Self {
            role,
            readers,
            sample,
            overlap,
        }))
    }
}

pub(crate) fn panel_role(bar: &Bar) -> Role {
    bar.panel.as_ref().map_or(Role::Off, |panel| panel.role)
}

pub(crate) fn people_role(bar: &Bar) -> Role {
    bar.people
        .as_ref()
        .map_or(Role::Beside, |people| people.role)
}

/// With people counted on a sample and the panel not counted, the Verdict
/// rests on the sampled pairs alone, each pair a unit of the statistics.
pub(crate) fn sample_only(bar: &Bar) -> bool {
    people_role(bar) == Role::Counted
        && panel_role(bar) != Role::Counted
        && bar
            .people
            .as_ref()
            .is_some_and(|people| people.sample.is_some())
}

fn counts(bar: &Bar) -> bool {
    panel_role(bar) == Role::Counted || people_role(bar) == Role::Counted
}

fn process(task: &Task, bar: &Bar) -> bool {
    task.manifest.session == format::Session::Process && bar.judges(&task.suite, &task.name)
}

/// Every judged pair of process Trials, in task order.
fn pairs(bar: &Bar, tasks: &[Task]) -> Vec<String> {
    tasks
        .iter()
        .filter(|task| process(task, bar))
        .flat_map(|task| {
            (0..bar.trials_per_task[&task.suite])
                .map(|index| trial_results::pair_key(&task.suite, &task.name, index))
        })
        .collect()
}

/// Refuse, before the first Trial, people's readers who could not read every
/// pair they may be given, or a sample larger than the Experiment.
pub(super) fn check(bar: &Bar, tasks: &[Task]) -> Result<()> {
    let Some(people) = bar
        .people
        .as_ref()
        .filter(|people| !people.readers.is_empty())
    else {
        return Ok(());
    };
    for task in tasks.iter().filter(|task| process(task, bar)) {
        ensure!(
            task.manifest
                .process_grading
                .as_ref()
                .is_some_and(|grading| grading.new_tests.is_some()),
            "people read pairs on their blind pairwise page, which needs pair evidence: {}/{} names no process_grading.new_tests",
            task.suite,
            task.name
        );
    }
    let available = pairs(bar, tasks).len();
    ensure!(
        available > 0,
        "the people's readers have no pairs to read: no judged Eval task runs a Process"
    );
    if let Some(sample) = people.sample {
        let needed = people.overlap + people.readers.len() * (sample - people.overlap);
        ensure!(
            needed <= available,
            "the people's samples need {needed} pairs ({sample} per reader, {} shared), but the Experiment has {available}",
            people.overlap
        );
    }
    Ok(())
}

/// The pairs one reader is given.
#[derive(Clone, Serialize)]
pub(crate) struct Queue {
    pub reader: String,
    /// Pair IDs, in task order.
    pub pairs: Vec<String>,
}

/// Each reader's pairs. With a sample, pairs are drawn from the Bar's seed:
/// the first `overlap` drawn go to every reader, then each reader takes the
/// next pairs alone.
pub(super) fn queues(bar: &Bar, tasks: &[Task]) -> Vec<Queue> {
    let Some(people) = &bar.people else {
        return Vec::new();
    };
    let all = pairs(bar, tasks);
    let Some(sample) = people.sample else {
        return people
            .readers
            .iter()
            .map(|reader| Queue {
                reader: reader.clone(),
                pairs: all.clone(),
            })
            .collect();
    };
    let digest = Sha256::digest(b"people's sample");
    let seed = bar.seed ^ u64::from_le_bytes(digest[..8].try_into().expect("eight bytes"));
    let mut drawn = all.clone();
    drawn.shuffle(&mut Pcg64::seed_from_u64(seed));
    let shared = people.overlap.min(drawn.len());
    let own = sample - people.overlap;
    people
        .readers
        .iter()
        .enumerate()
        .map(|(index, reader)| {
            let start = (shared + index * own).min(drawn.len());
            let end = (start + own).min(drawn.len());
            let mine: BTreeSet<&String> =
                drawn[..shared].iter().chain(&drawn[start..end]).collect();
            Queue {
                reader: reader.clone(),
                pairs: all
                    .iter()
                    .filter(|pair| mine.contains(pair))
                    .cloned()
                    .collect(),
            }
        })
        .collect()
}

/// The merge call that counts for one pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Call {
    Ready {
        baseline: bool,
        candidate: bool,
        /// Decided by people's calls rather than the panel's.
        people: bool,
    },
    /// A reader given the pair has not made their blind merge calls.
    Waiting,
    /// The counted panel has no call on the pair.
    Unread,
}

/// What decides each pair's merge call, for the Verdict.
#[derive(Default)]
pub(crate) struct Decided {
    /// Pairs whose merge call counts. With people counted on a sample and
    /// the panel not counted, a pair not listed is left out of the Verdict.
    pub calls: BTreeMap<String, Call>,
    pub sample_only: bool,
    /// Absent when neither the panel nor the people is counted.
    pub report: Option<Deciders>,
}

/// Who decided the Verdict, reported with it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Deciders {
    pub panel: Role,
    pub people: Role,
    /// Judged pairs decided by people's calls and by the panel's, pairs
    /// waiting on a reader's calls, and pairs the sample leaves out.
    pub people_pairs: usize,
    pub panel_pairs: usize,
    pub waiting_pairs: usize,
    pub left_out_pairs: usize,
    /// People–panel agreement on the people's sample, when both are
    /// counted: only the readers given each pair, once all have called it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_agreement: Option<Agreement>,
    /// Why the Verdict is inconclusive whatever the Scores show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inconclusive: Option<String>,
}

impl Deciders {
    /// One line for the CLI and the Console.
    pub(crate) fn line(&self) -> String {
        let mut line = format!(
            "Decided by: panel {}, people {} · people's calls on {} pairs, the panel's on {}",
            self.panel.name(),
            self.people.name(),
            self.people_pairs,
            self.panel_pairs
        );
        if self.left_out_pairs > 0 {
            line.push_str(&format!(
                " · {} pairs outside the sample left out",
                self.left_out_pairs
            ));
        }
        if self.waiting_pairs > 0 {
            line.push_str(&format!(
                " · {} pairs waiting on readers",
                self.waiting_pairs
            ));
        }
        if let Some(agreement) = &self.sample_agreement {
            line.push_str(&format!(
                " · people–panel agreement on the sample: {}/{} merge calls",
                agreement.agreed, agreement.compared
            ));
        }
        line
    }
}

/// Read the panel's and the people's retained calls and decide each pair.
pub(super) fn read(
    files: &impl Files,
    root: &Path,
    description: &Description,
    tasks: &[Task],
) -> Result<Decided> {
    let bar = &description.bar;
    if !counts(bar) {
        return Ok(Decided::default());
    }
    let panel = super::panel::read(files, root, description, tasks)?;
    let pairs = super::pairs::read(files, root)?;
    let people = super::pairwise::people(files, root, &pairs, bar.seed)?;
    decide(bar, tasks, &panel, &people.reviews)
}

/// The merge call each judged pair counts under the Bar. People's call on a
/// pair is ready for an Arm only if every reader given the pair calls it
/// ready in their latest blind review.
pub(super) fn decide(
    bar: &Bar,
    tasks: &[Task],
    panel: &[PanelPair],
    reviews: &[Review],
) -> Result<Decided> {
    let (panel_role, people_role) = (panel_role(bar), people_role(bar));
    if !counts(bar) {
        return Ok(Decided::default());
    }
    let model: BTreeMap<String, (bool, bool)> = panel
        .iter()
        .map(|pair| {
            (
                trial_results::pair_key(&pair.suite, &pair.task, pair.trial),
                (
                    pair.merge.baseline == Merge::Ready,
                    pair.merge.candidate == Merge::Ready,
                ),
            )
        })
        .collect();
    let mut blind = BTreeMap::new();
    for review in reviews.iter().filter(|review| review.blind) {
        blind.insert(
            (review.pair.as_str(), review.reader.who()),
            (review.baseline_ready, review.candidate_ready),
        );
    }
    let mut readers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if people_role == Role::Counted {
        for queue in queues(bar, tasks) {
            for pair in queue.pairs {
                readers.entry(pair).or_default().push(queue.reader.clone());
            }
        }
    }
    let stand_in = panel_role == Role::Counted && people_role == Role::Counted;
    let mut agreement = Agreement::default();
    let mut calls = BTreeMap::new();
    for key in pairs(bar, tasks) {
        let call = if let Some(given) = readers.get(&key) {
            let read: Option<Vec<(bool, bool)>> = given
                .iter()
                .map(|reader| blind.get(&(key.as_str(), reader.as_str())).copied())
                .collect();
            match read {
                Some(read) => {
                    if stand_in && let Some(&(baseline, candidate)) = model.get(&key) {
                        for (person_baseline, person_candidate) in &read {
                            agreement.compared += 2;
                            agreement.agreed += usize::from(*person_baseline == baseline)
                                + usize::from(*person_candidate == candidate);
                        }
                    }
                    Call::Ready {
                        baseline: read.iter().all(|call| call.0),
                        candidate: read.iter().all(|call| call.1),
                        people: true,
                    }
                }
                None => Call::Waiting,
            }
        } else if panel_role == Role::Counted {
            model
                .get(&key)
                .map_or(Call::Unread, |&(baseline, candidate)| Call::Ready {
                    baseline,
                    candidate,
                    people: false,
                })
        } else {
            // People counted on a sample, the panel not counted: the Verdict
            // rests on the sampled pairs alone.
            continue;
        };
        calls.insert(key, call);
    }
    let inconclusive = stand_in
        .then(|| {
            let level = bar.panel.as_ref().map_or(1.0, |panel| panel.agreement);
            if agreement.compared == 0 {
                Some(format!(
                    "no people–panel merge calls to compare on the sample; the panel stands in only at the Bar's agreement of {level}"
                ))
            } else {
                let share = agreement.agreed as f64 / agreement.compared as f64;
                (share < level).then(|| {
                    format!(
                        "people–panel agreement on the sample is {}/{} merge calls, below the Bar's {level}, so the panel cannot stand in for people",
                        agreement.agreed, agreement.compared
                    )
                })
            }
        })
        .flatten();
    Ok(Decided {
        calls,
        sample_only: sample_only(bar),
        report: Some(Deciders {
            panel: panel_role,
            people: people_role,
            people_pairs: 0,
            panel_pairs: 0,
            waiting_pairs: 0,
            left_out_pairs: 0,
            sample_agreement: stand_in.then_some(agreement),
            inconclusive,
        }),
    })
}

#[cfg(test)]
mod tests;
