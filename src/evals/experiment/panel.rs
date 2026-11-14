//! The Grader panel: the Bar's model seats, each reading every pair of
//! process Trials blind in both presentation orders. Every seat's calls and
//! the panel's vote are reported beside the Verdict, which they decide only
//! when the Bar counts the panel.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use rand::{SeedableRng, seq::SliceRandom};
use rand_pcg::Pcg64;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::deciders::Role;
use super::trial_results::files::{Files, Native};
use super::{Arm, Description, Task, arms, model_grader, pairs, store, trial_results};
use crate::evals::{format, repository};
use crate::harness::{Choice, ChosenBy, Harness};
use crate::sandbox::code_grader::Check;
use crate::session::usage;
use crate::trial::accounting::Prices;
use crate::trial::blind_diff;
use crate::{interrupt, progress, prompt};

mod retained_calls;

/// The panel the Bar fixes before the first Trial.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Panel {
    pub seats: Vec<Seat>,
    pub vote: Vote,
    /// A seat from either Arm's family doesn't vote while a seat from
    /// another family sits on the panel.
    pub arm_family_rule: bool,
    /// USD each run of the Experiment may spend on panel sessions, priced
    /// from the frozen table, before grading pauses. Never the Arms' cost.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "super::efficiency::optional_decimal"
    )]
    pub spending_limit: Option<f64>,
    /// Beside the Verdict or counted in it.
    #[serde(default)]
    pub role: Role,
    /// Families the voting panel must span to stand in for people.
    #[serde(default = "default_families")]
    pub families: usize,
    /// The least people–panel agreement on the people's sample, as a share
    /// of merge calls, that lets the panel stand in for people.
    #[serde(
        default = "default_agreement",
        deserialize_with = "crate::verdict::decimal"
    )]
    pub agreement: f64,
}

fn default_families() -> usize {
    2
}

fn default_agreement() -> f64 {
    0.8
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
    /// The panel's flags, removed from `flags` as they are read.
    pub(super) fn parse(
        seats: Vec<Seat>,
        flags: &mut BTreeMap<&str, &str>,
    ) -> Result<Option<Self>> {
        let role = flags
            .remove("--panel")
            .map(|text| Role::parse("--panel", text))
            .transpose()?;
        let vote = flags.remove("--panel-vote");
        let arm_family_rule = flags.remove("--panel-arm-family-rule");
        let spending_limit = flags.remove("--panel-spending-limit");
        let families = flags.remove("--panel-families");
        let agreement = flags.remove("--panel-agreement");
        if seats.is_empty() || role == Some(Role::Off) {
            ensure!(seats.is_empty(), "--panel off takes no --panel-seat");
            ensure!(
                role.is_none_or(|role| role == Role::Off)
                    && vote.is_none()
                    && arm_family_rule.is_none()
                    && spending_limit.is_none()
                    && families.is_none()
                    && agreement.is_none(),
                "--panel, --panel-vote, --panel-arm-family-rule, --panel-spending-limit, --panel-families and --panel-agreement need at least one --panel-seat"
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
        let families = match families {
            None => default_families(),
            Some(text) => text
                .parse()
                .ok()
                .filter(|families| *families > 0)
                .with_context(|| {
                    format!("invalid --panel-families {text}: expected a positive number")
                })?,
        };
        let agreement = match agreement {
            None => default_agreement(),
            Some(text) => text
                .parse()
                .ok()
                .filter(|level: &f64| (0.0..=1.0).contains(level))
                .with_context(|| {
                    format!("invalid --panel-agreement {text}: expected a share from 0 to 1")
                })?,
        };
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
            spending_limit: spending_limit
                .map(|value| {
                    super::options::positive("--panel-spending-limit", value, "amount in USD")
                })
                .transpose()?,
            role: role.unwrap_or(Role::Beside),
            families,
            agreement,
        }))
    }

    /// Each seat's family and whether it votes. With the arm-family rule, a
    /// seat from either Arm's family doesn't vote while another family sits.
    fn voting(&self, arm_families: &BTreeSet<String>) -> Vec<(String, bool)> {
        let families: Vec<String> = self.seats.iter().map(Seat::family).collect();
        let other_family = families.iter().any(|family| !arm_families.contains(family));
        families
            .into_iter()
            .map(|family| {
                let votes =
                    !(self.arm_family_rule && other_family && arm_families.contains(&family));
                (family, votes)
            })
            .collect()
    }
}

/// With both the panel and the people counted, the panel stands in for
/// people on the pairs they don't read: refuse a voting panel that spans
/// fewer families than the Bar needs.
pub(super) fn check_stand_in(description: &Description) -> Result<()> {
    let bar = &description.bar;
    let Some(panel) = &bar.panel else {
        return Ok(());
    };
    if panel.role != Role::Counted || super::deciders::people_role(bar) != Role::Counted {
        return Ok(());
    }
    let voting: BTreeSet<String> = panel
        .voting(&arm_families(description))
        .into_iter()
        .filter_map(|(family, votes)| votes.then_some(family))
        .collect();
    ensure!(
        voting.len() >= panel.families,
        "the voting Grader panel spans {} famil{} ({}), but standing in for people needs {}",
        voting.len(),
        if voting.len() == 1 { "y" } else { "ies" },
        voting.into_iter().collect::<Vec<_>>().join(", "),
        panel.families
    );
    Ok(())
}

/// A Model's family is its provider: the Harness's own, or the provider an
/// OpenCode Model names.
pub(super) fn family(harness: Harness, model: Option<&str>) -> String {
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
pub(super) fn arm_families(description: &Description) -> BTreeSet<String> {
    let mut families = BTreeSet::new();
    for arm in [Arm::Baseline, Arm::Candidate] {
        let setup = description.setup(arm);
        if let Some(harness) = setup.harness.as_deref().and_then(Harness::named) {
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
    // Process Steps can add Arm families once the Arms are prepared.
    check_stand_in(&experiment.description)?;
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
            .filter_map(Option::as_ref)
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

/// The panel's spend, priced from the Experiment's frozen table and kept
/// apart from the Arms' accounting.
#[derive(Serialize)]
pub(crate) struct Spend {
    pub sessions: usize,
    /// Null while any session can't be priced.
    pub cost_usd: Option<f64>,
    pub seats: Vec<SeatSpend>,
}

#[derive(Serialize)]
pub(crate) struct SeatSpend {
    pub harness: String,
    pub model: String,
    pub effort: Option<String>,
    pub sessions: usize,
    pub cost_usd: Option<f64>,
}

/// The seat, from 0, whose sessions a `graders/` workspace holds.
fn workspace_seat(name: &str) -> Option<usize> {
    let (_, rest) = name.rsplit_once("--seat-")?;
    let (seat, order) = rest.split_once("--order-")?;
    order.parse::<usize>().ok()?;
    seat.parse::<usize>().ok()?.checked_sub(1)
}

/// The `session-*` folders a Grader workspace retains, one per session.
fn sessions(files: &impl Files, workspace: &Path) -> Result<Vec<std::path::PathBuf>> {
    Ok(trial_results::files::directory(files, workspace)?
        .into_iter()
        .filter(|entry| {
            entry.kind.is_dir()
                && entry
                    .path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("session-"))
        })
        .map(|entry| entry.path)
        .collect())
}

/// One panel session's cost, from the usage its spans retain.
fn session_cost(files: &impl Files, session: &Path, prices: Option<&Prices>) -> Result<f64> {
    let prices = prices.context("the Experiment has no frozen price table")?;
    let spans = files
        .read(&session.join("stdout.spans.jsonl"))
        .context("missing usage")?;
    prices.cost(&usage::from_spans(std::io::Cursor::new(spans))?)
}

/// Every retained panel session, retries included, priced by seat.
pub(super) fn spend(
    files: &impl Files,
    root: &Path,
    description: &Description,
    prices: Option<&Prices>,
) -> Result<Option<Spend>> {
    let Some(panel) = &description.bar.panel else {
        return Ok(None);
    };
    let mut seats: Vec<SeatSpend> = panel
        .seats
        .iter()
        .map(|seat| SeatSpend {
            harness: seat.harness.clone(),
            model: seat.model.clone(),
            effort: seat.effort.clone(),
            sessions: 0,
            cost_usd: Some(0.0),
        })
        .collect();
    for workspace in trial_results::files::directory(files, &root.join("graders"))? {
        let seat = workspace
            .path
            .file_name()
            .and_then(|name| workspace_seat(&name.to_string_lossy()))
            .and_then(|seat| seats.get_mut(seat));
        let Some(seat) = seat.filter(|_| workspace.kind.is_dir()) else {
            continue;
        };
        for session in sessions(files, &workspace.path)? {
            seat.sessions += 1;
            let cost = session_cost(files, &session, prices).ok();
            seat.cost_usd = seat.cost_usd.zip(cost).map(|(sum, cost)| sum + cost);
        }
    }
    Ok(Some(Spend {
        sessions: seats.iter().map(|seat| seat.sessions).sum(),
        cost_usd: seats
            .iter()
            .try_fold(0.0, |sum, seat| seat.cost_usd.map(|cost| sum + cost)),
        seats,
    }))
}

impl SeatSpend {
    /// The mean of this seat's sessions, once there are some and all are priced.
    pub(super) fn mean_usd(&self) -> Option<f64> {
        (self.sessions > 0)
            .then_some(self.cost_usd)
            .flatten()
            .map(|cost| cost / self.sessions as f64)
    }
}

impl Spend {
    pub(super) fn render(&self) -> String {
        let cost =
            |amount: Option<f64>| amount.map_or("unknown".into(), |amount| format!("${amount:.6}"));
        let mut text = format!(
            "\nGrader panel spend, apart from the Arms: {} over {}",
            cost(self.cost_usd),
            super::cost::counted(self.sessions, "session")
        );
        for seat in &self.seats {
            text.push_str(&format!(
                "\nGrader panel {}/{}: {}, {}",
                seat.harness,
                seat.model,
                super::cost::counted(seat.sessions, "session"),
                cost(seat.cost_usd)
            ));
        }
        text
    }
}

/// The panel sessions still to run, by seat, and the pairs they read.
pub(super) fn remaining(experiment: &store::Stored) -> Result<Vec<(usize, usize)>> {
    Ok(retained_calls::History::capture(
        &Native,
        &experiment.directory,
        &experiment.description,
        &experiment.tasks,
    )?
    .remaining())
}

/// Grade every finished pair of process Trials with every seat, in both
/// orders. Saved calls are never asked again, so a resume continues. Once
/// this run's panel sessions reach the Bar's spending limit, grading pauses;
/// each run gets the limit afresh.
pub(super) fn run(experiment: &store::Stored) -> Result<()> {
    let bar = &experiment.description.bar;
    let Some(panel) = &bar.panel else {
        return Ok(());
    };
    let mut history = retained_calls::History::capture(
        &Native,
        &experiment.directory,
        &experiment.description,
        &experiment.tasks,
    )?;
    let seats: Vec<arms::FixedHarness> =
        serde_json::from_slice(&fs::read(experiment.directory.join("panel.json"))?)?;
    ensure!(seats.len() == panel.seats.len(), "panel seats changed");
    let timeout = Duration::from_secs(bar.timeout_seconds);
    let mut caps = None;
    let mut spent = 0.0;
    let mut prepared: Option<(String, Sides)> = None;
    for work in history.executable() {
        let task = work.task;
        let index = work.trial;
        let seat_index = work.seat;
        let order = work.order;
        let seat = &panel.seats[seat_index];
        let harness = &seats[seat_index];
        let key = work.key();
        interrupt::check()?;
        if let Some(limit) = panel.spending_limit {
            ensure!(
                spent < limit,
                "Grader panel reached its ${limit:.6} spending limit for this run (${spent:.6} spent); resume the Experiment to grade the remaining pairs"
            );
        }
        if prepared
            .as_ref()
            .is_none_or(|(previous, _)| previous != &key)
        {
            let sides = Sides::read(experiment, task, &key, &work.identities(), timeout)?;
            prepared = Some((key.clone(), sides));
        }
        let sides = &prepared.as_ref().expect("prepared panel pair").1;
        let caps = match &mut caps {
            Some(caps) => caps,
            None => caps.insert(crate::config::UserConfig::load()?.trial_caps.with_env()?),
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
        let earlier = sessions(&Native, &workspace)?;
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
        history.publish(&work, call)?;
        // Priced after the call is saved, so a pause keeps it.
        if panel.spending_limit.is_some() {
            for session in sessions(&Native, &workspace)? {
                if !earlier.contains(&session) {
                    spent += session_cost(&Native, &session, experiment.pricing.as_ref())
                        .context("can't price a Grader panel session against the spending limit")?;
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
            .contains_key(prompt::ISSUE_FILE)
            .then(|| super::pairwise::ticket(task))
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
    let history = retained_calls::History::capture(files, root, description, tasks)?;
    let mut reports = Vec::new();
    for (task, index, calls) in history.complete() {
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
    let graders: Vec<SeatCalls> = panel
        .seats
        .iter()
        .zip(panel.voting(arm_families))
        .zip(calls)
        .map(|((seat, (family, votes)), orders)| {
            let agreed = |merge: fn(&Call) -> bool| match (merge(&orders[0]), merge(&orders[1])) {
                (true, true) => Merge::Ready,
                (false, false) => Merge::NotReady,
                _ => Merge::Tie,
            };
            SeatCalls {
                harness: seat.harness.clone(),
                model: seat.model.clone(),
                effort: seat.effort.clone(),
                votes,
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
