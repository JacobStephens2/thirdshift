//! The Grader panel: the Bar's model seats, each reading every pair of
//! process Trials blind in both presentation orders. Every seat's calls and
//! the panel's vote are reported beside the Verdict, never in it.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use rand::{SeedableRng, seq::SliceRandom};
use rand_pcg::Pcg64;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::trial_results::files::Files;
use super::{Arm, Description, Task, arms, model_grader, pairs, store, trial_results};
use crate::evals::{format, repository};
use crate::harness::{Choice, ChosenBy, Harness};
use crate::sandbox::code_grader::Check;
use crate::trial::blind_diff;
use crate::{interrupt, progress, prompt};

/// The panel the Bar fixes before the first Trial.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Panel {
    pub seats: Vec<Seat>,
    pub vote: Vote,
    /// A seat from either Arm's family doesn't vote while a seat from
    /// another family sits on the panel.
    pub arm_family_rule: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Seat {
    pub harness: String,
    pub model: String,
    pub effort: Option<String>,
}

/// What counts as one vote.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Vote {
    /// Each family takes its own majority first.
    Family,
    Model,
}

impl Seat {
    /// `harness=<name>,model=<model>[,effort=<effort>]`, as `--panel-seat`
    /// takes it. Model names can hold `/` and `:`, so fields are named.
    pub(super) fn parse(text: &str) -> Result<Self> {
        let usage = || {
            format!(
                "invalid --panel-seat {text}: expected harness=<name>,model=<model>[,effort=<effort>]"
            )
        };
        let (mut harness, mut model, mut effort) = (None, None, None);
        for field in text.split(',') {
            let (key, value) = field.split_once('=').with_context(usage)?;
            ensure!(!value.trim().is_empty(), usage());
            let slot = match key {
                "harness" => &mut harness,
                "model" => &mut model,
                "effort" => &mut effort,
                _ => bail!(usage()),
            };
            ensure!(
                slot.replace(value.to_owned()).is_none(),
                "repeated {key} in --panel-seat {text}"
            );
        }
        let harness = harness.with_context(usage)?;
        ensure!(
            Harness::named(&harness).is_some(),
            "unknown panel seat Harness {harness}"
        );
        Ok(Self {
            harness,
            model: model.with_context(usage)?,
            effort,
        })
    }

    fn harness(&self) -> Harness {
        Harness::named(&self.harness).expect("validated panel seat Harness")
    }

    pub(crate) fn family(&self) -> String {
        family(self.harness(), Some(&self.model))
    }
}

impl Panel {
    pub(super) fn parse(
        seats: Vec<Seat>,
        vote: Option<&str>,
        arm_family_rule: Option<&str>,
    ) -> Result<Option<Self>> {
        if seats.is_empty() {
            ensure!(
                vote.is_none() && arm_family_rule.is_none(),
                "--panel-vote and --panel-arm-family-rule need at least one --panel-seat"
            );
            return Ok(None);
        }
        for (index, seat) in seats.iter().enumerate() {
            ensure!(
                !seats[..index].contains(seat),
                "repeated panel seat {}/{}",
                seat.harness,
                seat.model
            );
        }
        Ok(Some(Self {
            seats,
            vote: match vote {
                None | Some("family") => Vote::Family,
                Some("model") => Vote::Model,
                Some(other) => bail!("invalid --panel-vote {other}: expected family or model"),
            },
            arm_family_rule: match arm_family_rule {
                None | Some("on") => true,
                Some("off") => false,
                Some(other) => bail!("invalid --panel-arm-family-rule {other}: expected on or off"),
            },
        }))
    }
}

/// A Model's family is its provider: the Harness's own, or the provider an
/// OpenCode Model names.
fn family(harness: Harness, model: Option<&str>) -> String {
    match harness {
        Harness::Claude => "anthropic",
        Harness::Codex => "openai",
        Harness::Grok => "xai",
        Harness::Muse => "meta",
        Harness::Agy => "google",
        Harness::Kimi => "moonshot",
        Harness::OpenCode => crate::proxy::opencode_provider(model).unwrap_or("opencode"),
    }
    .to_owned()
}

/// Every family either Arm's sessions run on, Process Steps included.
fn arm_families(description: &Description) -> BTreeSet<String> {
    let mut families = BTreeSet::new();
    for arm in [Arm::Baseline, Arm::Candidate] {
        let setup = description.setup(arm);
        if let Some(harness) = Harness::named(&setup.harness) {
            families.insert(family(harness, setup.model.as_deref()));
        }
    }
    if let Some(prepared) = &description.prepared {
        for arm in [&prepared.baseline, &prepared.candidate] {
            for step in arm.process.iter().flat_map(|process| &process.steps) {
                if let Some(harness) = Harness::named(&step.harness.name) {
                    families.insert(family(harness, step.model.as_deref()));
                }
            }
        }
    }
    families
}

/// Fix each seat's executable before Trials can change PATH, reusing an
/// Arm's pinned Harness where they share one.
pub(super) fn prepare(experiment: &store::Stored) -> Result<()> {
    let Some(panel) = &experiment.description.bar.panel else {
        return Ok(());
    };
    let path = experiment.directory.join("panel.json");
    if path.exists() {
        return Ok(());
    }
    let prepared = experiment
        .description
        .prepared
        .as_ref()
        .context("Arms not prepared")?;
    let mut fixed: Vec<arms::FixedHarness> = Vec::new();
    for seat in &panel.seats {
        let pinned = [&prepared.baseline.harness, &prepared.candidate.harness]
            .into_iter()
            .chain(&fixed)
            .find(|harness| harness.name == seat.harness)
            .cloned();
        fixed.push(match pinned {
            Some(harness) => harness,
            None => arms::fixed_harness(seat.harness())?,
        });
    }
    store::write(&path, &fixed)
}

/// One criterion of the rubric each Grader fills in for each side.
#[derive(Serialize)]
struct Criterion {
    name: &'static str,
    question: &'static str,
}

const RUBRIC: [Criterion; 4] = [
    Criterion {
        name: "criteria",
        question: "Does the change do what the Ticket asks and meet the success criteria?",
    },
    Criterion {
        name: "tests",
        question: "Are its new or changed tests proportionate: do they check the change, no less and no more than it needs?",
    },
    Criterion {
        name: "scope",
        question: "Does it stay within what the Ticket asks, with no unrelated changes?",
    },
    Criterion {
        name: "code",
        question: "Would a careful maintainer accept its code as written: clear, consistent with the code around it and no more complex than needed?",
    },
];

const LABELS: [&str; 2] = ["A", "B"];

#[derive(Serialize)]
struct Packet<'a> {
    order: usize,
    ticket: Option<&'a str>,
    criteria: &'a str,
    rubric: &'a [Criterion],
    sides: Vec<PacketSide<'a>>,
}

#[derive(Serialize)]
struct PacketSide<'a> {
    label: &'static str,
    outcome: crate::trial::Outcome,
    /// Null when the Trial's repository is no longer retained.
    diff: Option<&'a str>,
    /// Null when the task names no new tests to run.
    evidence: Option<Evidence<'a>>,
}

/// Pair evidence without its error text, which can name retained paths.
#[derive(Serialize)]
struct Evidence<'a> {
    test_files: &'a [String],
    fails_on_base: Option<bool>,
    passes_on_other: Option<bool>,
    disagreement: bool,
    base: &'a [Check],
    cross: &'a [Check],
}

impl<'a> From<&'a pairs::Side> for Evidence<'a> {
    fn from(side: &'a pairs::Side) -> Self {
        Self {
            test_files: &side.test_files,
            fails_on_base: side.fails_on_base,
            passes_on_other: side.passes_on_other,
            disagreement: side.disagreement,
            base: &side.base,
            cross: &side.cross,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    sides: Vec<ResponseSide>,
    reason: String,
    preference: Picked,
}

/// A preference as the Grader writes it, by label.
#[derive(Clone, Copy, Deserialize)]
enum Picked {
    A,
    B,
    #[serde(rename = "tie")]
    Tie,
    #[serde(rename = "both_bad")]
    BothBad,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseSide {
    label: String,
    rubric: Vec<Rating>,
    merge_reason: String,
    ready_to_merge: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rating {
    pub criterion: String,
    pub met: bool,
    pub evidence: String,
}

/// One Grader's validated reading of one pair in one order, by Arm.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Call {
    pub shown_as_a: Arm,
    pub baseline: SideCall,
    pub candidate: SideCall,
    pub reason: String,
    pub preference: Preference,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SideCall {
    pub rubric: Vec<Rating>,
    pub merge_reason: String,
    pub ready_to_merge: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Preference {
    Baseline,
    Candidate,
    Tie,
    BothBad,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Merge {
    Ready,
    NotReady,
    Tie,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Merges {
    pub baseline: Merge,
    pub candidate: Merge,
}

impl Response {
    /// Check the response against its packet and map labels back to Arms.
    fn call(self, shown: [Arm; 2]) -> Result<Call> {
        ensure!(
            !self.reason.trim().is_empty(),
            "Grader reason must not be empty"
        );
        ensure!(self.sides.len() == 2, "Grader must call both sides");
        let mut sides = BTreeMap::new();
        for side in self.sides {
            let index = LABELS
                .iter()
                .position(|label| *label == side.label)
                .context("unknown Grader label")?;
            ensure!(
                !side.merge_reason.trim().is_empty(),
                "Grader merge reason must not be empty"
            );
            ensure!(
                side.rubric.len() == RUBRIC.len()
                    && RUBRIC.iter().all(|criterion| {
                        side.rubric
                            .iter()
                            .filter(|rating| rating.criterion == criterion.name)
                            .count()
                            == 1
                    }),
                "Grader must rate each rubric criterion once"
            );
            ensure!(
                side.rubric
                    .iter()
                    .all(|rating| !rating.evidence.trim().is_empty()),
                "Grader rubric evidence must not be empty"
            );
            ensure!(
                sides
                    .insert(
                        shown[index].name(),
                        SideCall {
                            rubric: side.rubric,
                            merge_reason: side.merge_reason,
                            ready_to_merge: side.ready_to_merge,
                        },
                    )
                    .is_none(),
                "duplicate Grader label"
            );
        }
        let preference = match self.preference {
            Picked::A | Picked::B => match shown[usize::from(matches!(self.preference, Picked::B))]
            {
                Arm::Baseline => Preference::Baseline,
                Arm::Candidate => Preference::Candidate,
            },
            Picked::Tie => Preference::Tie,
            Picked::BothBad => Preference::BothBad,
        };
        Ok(Call {
            shown_as_a: shown[0],
            baseline: sides.remove("baseline").context("missing Grader side")?,
            candidate: sides.remove("candidate").context("missing Grader side")?,
            reason: self.reason,
            preference,
        })
    }
}

fn call_path(root: &Path, pair: &str, seat: usize, order: usize) -> std::path::PathBuf {
    root.join("panel")
        .join(pair)
        .join(format!("seat-{}-order-{order}.json", seat + 1))
}

/// Grade every finished pair of process Trials with every seat, in both
/// orders. Saved calls are never asked again, so a resume continues.
pub(super) fn run(experiment: &store::Stored) -> Result<()> {
    let bar = &experiment.description.bar;
    let Some(panel) = &bar.panel else {
        return Ok(());
    };
    let seats: Vec<arms::FixedHarness> =
        serde_json::from_slice(&fs::read(experiment.directory.join("panel.json"))?)?;
    ensure!(seats.len() == panel.seats.len(), "panel seats changed");
    let timeout = Duration::from_secs(bar.timeout_seconds);
    let mut caps = None;
    for task in &experiment.tasks {
        if task.manifest.session != format::Session::Process {
            continue;
        }
        for index in 0..bar.trials_per_task[&task.suite] {
            let identities = [Arm::Baseline, Arm::Candidate].map(|arm| trial_results::Identity {
                suite: &task.suite,
                task: &task.name,
                arm,
                trial: index,
            });
            if identities
                .iter()
                .any(|identity| !trial_results::finished(&experiment.directory, identity))
            {
                continue;
            }
            let key = trial_results::pair_key(&task.suite, &task.name, index);
            let mut sides = None;
            for (seat_index, (seat, harness)) in panel.seats.iter().zip(&seats).enumerate() {
                for order in 1..=2 {
                    let path = call_path(&experiment.directory, &key, seat_index, order);
                    if path.exists() {
                        continue;
                    }
                    interrupt::check()?;
                    let sides = match &mut sides {
                        Some(sides) => sides,
                        None => {
                            sides.insert(Sides::read(experiment, task, &key, &identities, timeout)?)
                        }
                    };
                    let caps = match &mut caps {
                        Some(caps) => caps,
                        None => {
                            caps.insert(crate::config::UserConfig::load()?.trial_caps.with_env()?)
                        }
                    };
                    let shown = if order == 1 {
                        sides.first_order
                    } else {
                        [sides.first_order[1], sides.first_order[0]]
                    };
                    progress::step(format!(
                        "Grader panel {}/{} #{}: seat {} ({}/{}), order {order}",
                        task.suite,
                        task.name,
                        index + 1,
                        seat_index + 1,
                        seat.harness,
                        seat.model
                    ));
                    let workspace = experiment
                        .directory
                        .join("graders")
                        .join(format!("{key}--seat-{}--order-{order}", seat_index + 1));
                    fs::create_dir_all(&workspace)?;
                    let response: Response = model_grader::session(
                        model_grader::Session {
                            kind: "panel Grader",
                            choice: &Choice {
                                harness: seat.harness(),
                                model: Some(seat.model.clone()),
                                effort: seat.effort.clone(),
                                chosen_by: ChosenBy::Command,
                            },
                            binary: &harness.path,
                            directory: &workspace,
                            prompt: prompt::panel_grader(),
                            output: ("THIRDSHIFT_GRADER_CALLS", "calls.json", "calls"),
                            timeout,
                            caps,
                        },
                        &sides.packet(task, order, shown),
                    )?;
                    let call = response.call(shown)?;
                    fs::create_dir_all(path.parent().context("panel call path")?)?;
                    store::write(&path, &call)?;
                }
            }
        }
    }
    Ok(())
}

/// What the packets of one pair show, read once for every seat and order.
struct Sides {
    ticket: Option<String>,
    /// The Arms in the first order's A and B.
    first_order: [Arm; 2],
    outcomes: [crate::trial::Outcome; 2],
    diffs: [Option<String>; 2],
    evidence: Option<pairs::Pair>,
}

impl Sides {
    fn read(
        experiment: &store::Stored,
        task: &Task,
        key: &str,
        identities: &[trial_results::Identity<'_>; 2],
        timeout: Duration,
    ) -> Result<Self> {
        let ticket = task
            .manifest
            .inputs
            .get(prompt::ISSUE_FILE)
            .map(|source| fs::read_to_string(task.directory.join(source)))
            .transpose()?;
        let digest = Sha256::digest(key.as_bytes());
        let seed = experiment.description.bar.seed ^ u64::from_le_bytes(digest[..8].try_into()?);
        let mut first_order = [Arm::Baseline, Arm::Candidate];
        first_order.shuffle(&mut Pcg64::seed_from_u64(seed));
        let mut outcomes = [crate::trial::Outcome::Void; 2];
        let mut diffs = [None, None];
        for (at, identity) in identities.iter().enumerate() {
            outcomes[at] = pairs::trial_result(&experiment.directory, identity)?.outcome;
            // A diff that cannot be read, as when the Trial's repository is
            // gone, is shown as null: its error names retained paths, which
            // would identify the Arm.
            diffs[at] = pairs::last_commit(&experiment.directory, identity)
                .and_then(|commit| {
                    let bytes = repository::bytes_until(
                        &commit.repository,
                        &[
                            "diff",
                            "--no-color",
                            "--no-ext-diff",
                            "--no-textconv",
                            "--no-renames",
                            &task.manifest.base,
                            &commit.commit,
                            "--",
                        ],
                        Instant::now() + timeout,
                    )?;
                    Ok(blind_diff(&String::from_utf8_lossy(&bytes)))
                })
                .ok();
        }
        let evidence = experiment
            .directory
            .join("pairs")
            .join(format!("{key}.json"));
        let evidence = if evidence.exists() {
            Some(serde_json::from_slice(&fs::read(evidence)?)?)
        } else {
            None
        };
        Ok(Self {
            ticket,
            first_order,
            outcomes,
            diffs,
            evidence,
        })
    }

    fn packet<'a>(&'a self, task: &'a Task, order: usize, shown: [Arm; 2]) -> Packet<'a> {
        let at = |arm: Arm| usize::from(arm == Arm::Candidate);
        Packet {
            order,
            ticket: self.ticket.as_deref(),
            criteria: &task.manifest.criteria,
            rubric: &RUBRIC,
            sides: LABELS
                .iter()
                .zip(shown)
                .map(|(label, arm)| PacketSide {
                    label,
                    outcome: self.outcomes[at(arm)],
                    diff: self.diffs[at(arm)].as_deref(),
                    evidence: self.evidence.as_ref().map(|pair| {
                        Evidence::from(match arm {
                            Arm::Baseline => &pair.baseline,
                            Arm::Candidate => &pair.candidate,
                        })
                    }),
                })
                .collect(),
        }
    }
}

/// One pair's panel report: every seat's calls and the panel's vote.
#[derive(Serialize)]
pub(crate) struct PanelPair {
    pub suite: String,
    pub task: String,
    pub trial: usize,
    pub graders: Vec<SeatCalls>,
    /// The vote units that counted, each with its own call.
    pub votes: Vec<Ballot>,
    pub preference: Preference,
    /// A tied merge call is not ready.
    pub merge: Merges,
}

#[derive(Serialize)]
pub(crate) struct SeatCalls {
    pub harness: String,
    pub model: String,
    pub effort: Option<String>,
    pub family: String,
    pub votes: bool,
    /// The first and second presentation orders.
    pub orders: Vec<Call>,
    /// The call both orders agree on, or a tie when it flips.
    pub preference: Preference,
    pub merge: Merges,
}

#[derive(Serialize)]
pub(crate) struct Ballot {
    /// A family, or a Model when each Model votes.
    pub name: String,
    pub preference: Preference,
    pub merge: Merges,
}

/// Retained panel reports for every pair whose calls are all in.
pub(super) fn read(
    files: &impl Files,
    root: &Path,
    description: &Description,
    tasks: &[Task],
) -> Result<Vec<PanelPair>> {
    let Some(panel) = &description.bar.panel else {
        return Ok(Vec::new());
    };
    let families = arm_families(description);
    let mut reports = Vec::new();
    for task in tasks {
        if task.manifest.session != format::Session::Process {
            continue;
        }
        'pairs: for index in 0..description.bar.trials_per_task[&task.suite] {
            let key = trial_results::pair_key(&task.suite, &task.name, index);
            let mut calls = Vec::new();
            for seat in 0..panel.seats.len() {
                let mut orders = Vec::new();
                for order in 1..=2 {
                    match files.read(&call_path(root, &key, seat, order)) {
                        Ok(bytes) => orders.push(serde_json::from_slice(&bytes)?),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            continue 'pairs;
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
                calls.push(orders);
            }
            let (graders, votes, preference, merge) = tally(panel, &families, calls);
            reports.push(PanelPair {
                suite: task.suite.clone(),
                task: task.name.clone(),
                trial: index,
                graders,
                votes,
                preference,
                merge,
            });
        }
    }
    Ok(reports)
}

/// Gives each retained pair the panel's result on it, once the panel has read it.
pub(super) fn attach(pairs: &mut [super::pairs::Pair], panel: &[PanelPair]) {
    for pair in pairs {
        let report = panel.iter().find(|report| {
            report.suite == pair.suite && report.task == pair.task && report.trial == pair.trial
        });
        if let Some(report) = report {
            pair.panel = Some(super::pairwise::PanelCalls {
                baseline_ready: report.merge.baseline == Merge::Ready,
                candidate_ready: report.merge.candidate == Merge::Ready,
                preference: match report.preference {
                    Preference::Baseline => super::pairwise::ArmPreference::Baseline,
                    Preference::Candidate => super::pairwise::ArmPreference::Candidate,
                    Preference::Tie => super::pairwise::ArmPreference::Tie,
                    Preference::BothBad => super::pairwise::ArmPreference::BothBad,
                },
            });
        }
    }
}

/// The call more than half of `calls` make, or `tie` without one.
fn majority<T: Copy + Eq>(calls: impl IntoIterator<Item = T>, tie: T) -> T {
    let calls: Vec<T> = calls.into_iter().collect();
    calls
        .iter()
        .copied()
        .find(|call| calls.iter().filter(|other| *other == call).count() * 2 > calls.len())
        .unwrap_or(tie)
}

fn merges(calls: impl IntoIterator<Item = Merges>) -> Merges {
    let calls: Vec<Merges> = calls.into_iter().collect();
    let arm = |pick: fn(&Merges) -> Merge| majority(calls.iter().map(pick), Merge::Tie);
    Merges {
        baseline: arm(|merges| merges.baseline),
        candidate: arm(|merges| merges.candidate),
    }
}

fn tally(
    panel: &Panel,
    arm_families: &BTreeSet<String>,
    calls: Vec<Vec<Call>>,
) -> (Vec<SeatCalls>, Vec<Ballot>, Preference, Merges) {
    let families: Vec<String> = panel.seats.iter().map(Seat::family).collect();
    let other_family = families.iter().any(|family| !arm_families.contains(family));
    let graders: Vec<SeatCalls> = panel
        .seats
        .iter()
        .zip(families)
        .zip(calls)
        .map(|((seat, family), orders)| {
            let agreed = |merge: fn(&Call) -> bool| match (merge(&orders[0]), merge(&orders[1])) {
                (true, true) => Merge::Ready,
                (false, false) => Merge::NotReady,
                _ => Merge::Tie,
            };
            SeatCalls {
                harness: seat.harness.clone(),
                model: seat.model.clone(),
                effort: seat.effort.clone(),
                votes: !(panel.arm_family_rule && other_family && arm_families.contains(&family)),
                family,
                preference: if orders[0].preference == orders[1].preference {
                    orders[0].preference
                } else {
                    Preference::Tie
                },
                merge: Merges {
                    baseline: agreed(|call| call.baseline.ready_to_merge),
                    candidate: agreed(|call| call.candidate.ready_to_merge),
                },
                orders,
            }
        })
        .collect();
    let mut units: BTreeMap<String, Vec<&SeatCalls>> = BTreeMap::new();
    for grader in graders.iter().filter(|grader| grader.votes) {
        let name = match panel.vote {
            Vote::Family => grader.family.clone(),
            Vote::Model => grader.model.clone(),
        };
        units.entry(name).or_default().push(grader);
    }
    let votes: Vec<Ballot> = units
        .into_iter()
        .map(|(name, graders)| Ballot {
            name,
            preference: majority(
                graders.iter().map(|grader| grader.preference),
                Preference::Tie,
            ),
            merge: merges(graders.iter().map(|grader| grader.merge)),
        })
        .collect();
    let preference = majority(
        votes.iter().map(|ballot| ballot.preference),
        Preference::Tie,
    );
    let mut merge = merges(votes.iter().map(|ballot| ballot.merge));
    for call in [&mut merge.baseline, &mut merge.candidate] {
        if *call == Merge::Tie {
            *call = Merge::NotReady;
        }
    }
    (graders, votes, preference, merge)
}

impl std::fmt::Display for Preference {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Baseline => "prefers Baseline",
            Self::Candidate => "prefers Candidate",
            Self::Tie => "tie",
            Self::BothBad => "both bad",
        })
    }
}

impl std::fmt::Display for Merges {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = |merge: Merge| match merge {
            Merge::Ready => "ready",
            Merge::NotReady => "not ready",
            Merge::Tie => "tied",
        };
        write!(
            formatter,
            "Baseline {}, Candidate {}",
            name(self.baseline),
            name(self.candidate)
        )
    }
}

/// One-line summaries for the Experiment's text view, numbering pairs from 1.
pub(super) fn lines(reports: &[PanelPair]) -> String {
    let mut text = String::new();
    for pair in reports {
        let name = format!("Panel {}/{} #{}", pair.suite, pair.task, pair.trial + 1);
        text.push_str(&format!("\n{name} · {} · {}", pair.preference, pair.merge));
        for grader in &pair.graders {
            text.push_str(&format!(
                "\n{name} · {}/{} ({}{}): {} then {}, {} · {}",
                grader.harness,
                grader.model,
                grader.family,
                if grader.votes { "" } else { ", not voting" },
                grader.orders[0].preference,
                grader.orders[1].preference,
                grader.preference,
                grader.merge
            ));
        }
    }
    text
}

#[cfg(test)]
mod tests;
