//! The blind pairwise review of one pair of Trials. A reader sees the Ticket,
//! its acceptance checklist, both changes and their evidence on randomised
//! sides, with no Arm identity and no model calls. They make a sealed merge
//! call per side, then a preference and optionally the rubric. Records are
//! append-only and attributed; identity and the panel's calls are revealed
//! only once the reader has given a preference.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::deciders::Role;
use super::pairs::{self, Pair};
use super::store::Stored;
use super::trial_results::files::{Files, Native, directory};
use super::{Arm, Setup};
use crate::logs::Actor;
use crate::trial::blind_diff;

/// What a reader may grade both changes on once their merge calls are sealed.
pub(crate) const RUBRIC: [(&str, &str); 3] = [
    (
        "correctness",
        "Which change does what the Ticket and its acceptance checklist ask?",
    ),
    (
        "tests",
        "Which change's tests are proportionate and check the change?",
    ),
    ("scope", "Which change keeps to what was asked?"),
];

/// A preference between the two sides as the reader saw them.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Preference {
    A,
    B,
    Tie,
    BothBad,
}

impl Preference {
    /// Each preference's argument and form value, and its label.
    pub(crate) const ALL: [(Self, &'static str, &'static str); 4] = [
        (Self::A, "a", "A"),
        (Self::B, "b", "B"),
        (Self::Tie, "tie", "tie"),
        (Self::BothBad, "both-bad", "both bad"),
    ];

    pub fn parse(text: &str) -> Result<Self> {
        Self::ALL
            .iter()
            .find(|(_, value, _)| *value == text)
            .map(|(preference, _, _)| *preference)
            .context("a preference must be a, b, tie or both-bad")
    }

    pub fn label(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(preference, _, _)| *preference == self)
            .map_or("", |(_, _, label)| label)
    }

    fn for_arms(self, sides: [Arm; 2]) -> ArmPreference {
        match self {
            Self::A => sides[0].into(),
            Self::B => sides[1].into(),
            Self::Tie => ArmPreference::Tie,
            Self::BothBad => ArmPreference::BothBad,
        }
    }
}

/// A preference in Arm terms, as the panel reads both orders.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArmPreference {
    Baseline,
    Candidate,
    Tie,
    BothBad,
}

impl From<Arm> for ArmPreference {
    fn from(arm: Arm) -> Self {
        match arm {
            Arm::Baseline => Self::Baseline,
            Arm::Candidate => Self::Candidate,
        }
    }
}

/// The Grader panel's result for one pair. A tied merge call is not ready.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct PanelCalls {
    pub baseline_ready: bool,
    pub candidate_ready: bool,
    pub preference: ArmPreference,
}

/// One append-only record, in the sides the reader saw.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Record {
    pub reader: Actor,
    pub recorded_at: String,
    #[serde(flatten)]
    pub call: Call,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case")]
pub(crate) enum Call {
    /// Sealed: made before the preference, and never changed.
    Merge { a_ready: bool, b_ready: bool },
    Preference {
        preference: Preference,
        #[serde(default)]
        rubric: BTreeMap<String, Preference>,
    },
}

#[derive(Serialize)]
pub(crate) struct Criterion {
    pub name: &'static str,
    pub question: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Next {
    Merge,
    Preference,
}

/// What one reader may see of one pair.
#[derive(Serialize)]
pub(crate) struct Page {
    pub experiment: String,
    pub pair: String,
    pub suite: String,
    pub task: String,
    /// Pairs are numbered from 1 within their Eval task.
    pub number: usize,
    pub ticket: Option<String>,
    pub checklist: Vec<String>,
    pub a: pairs::Side,
    pub b: pairs::Side,
    pub rubric: Vec<Criterion>,
    pub reader: String,
    pub next: Next,
    pub mine: Vec<Record>,
    pub revealed: Option<Reveal>,
}

/// Shown only after the reader's preference.
#[derive(Serialize)]
pub(crate) struct Reveal {
    pub a: Arm,
    pub b: Arm,
    pub a_setup: Setup,
    pub b_setup: Setup,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub a_runner: Option<crate::evals::arms::Runner>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b_runner: Option<crate::evals::arms::Runner>,
    pub panel: Option<PanelCalls>,
    pub records: Vec<Record>,
}

/// People's calls on every pair, in Arm terms, for the Experiment view.
#[derive(Default, Serialize)]
pub(crate) struct People {
    pub reviews: Vec<Review>,
    pub agreement: Agreement,
    /// Readers' latest blind merge calls compared per side, two readers at a
    /// time, on pairs more than one has read.
    pub people_agreement: Agreement,
    /// The pairs the Bar gives each named reader.
    pub queues: Vec<super::deciders::Queue>,
}

/// One sealed merge call with the preference the reader then gave, if any.
/// A reader's later reviews of a pair are kept but are no longer blind.
#[derive(Serialize)]
pub(crate) struct Review {
    pub pair: String,
    pub suite: String,
    pub task: String,
    pub trial: usize,
    pub reader: Actor,
    pub recorded_at: String,
    /// False for a review begun after the reader's own reveal.
    pub blind: bool,
    pub baseline_ready: bool,
    pub candidate_ready: bool,
    pub preference: Option<ArmPreference>,
    pub rubric: BTreeMap<String, ArmPreference>,
}

/// Agreement on merge calls, per side: people–panel agreement compares each
/// reader's latest blind call with the panel's on pairs the panel has read.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Agreement {
    pub compared: usize,
    pub agreed: usize,
}

/// The Arms on sides A and B, drawn per pair from the Bar's seed so every
/// reader and every reload sees the same sides.
pub(super) fn sides(seed: u64, pair: &str) -> [Arm; 2] {
    let digest = Sha256::new()
        .chain_update(seed.to_le_bytes())
        .chain_update(pair.as_bytes())
        .finalize();
    if digest[0] & 1 == 0 {
        [Arm::Baseline, Arm::Candidate]
    } else {
        [Arm::Candidate, Arm::Baseline]
    }
}

pub(crate) fn page(id: &str, pair: &str, reader: &Actor) -> Result<Page> {
    let stored = Stored::load(&root()?, id, false)?;
    build(&stored, pair, reader)
}

/// Seal one merge call per side. Each call adds a record.
pub(crate) fn merge(id: &str, pair: &str, a: &str, b: &str, reader: Actor) -> Result<Page> {
    let ready = |side: &str, text: &str| match text {
        "ready" => Ok(true),
        "not-ready" | "not_ready" => Ok(false),
        _ => bail!("the merge call for {side} must be ready or not-ready"),
    };
    let call = Call::Merge {
        a_ready: ready("A", a)?,
        b_ready: ready("B", b)?,
    };
    append(id, pair, reader, |records| {
        ensure!(
            next(records) == Next::Merge,
            "your merge calls for this pair are sealed; give your preference next"
        );
        Ok(call)
    })
}

/// The preference and optional rubric, after this reader's sealed merge calls.
pub(crate) fn prefer(
    id: &str,
    pair: &str,
    preference: &str,
    rubric: &[(String, String)],
    reader: Actor,
) -> Result<Page> {
    let preference = Preference::parse(preference)?;
    let mut graded = BTreeMap::new();
    for (criterion, value) in rubric {
        ensure!(
            RUBRIC.iter().any(|(name, _)| name == criterion),
            "unknown rubric criterion {criterion}; use {}",
            RUBRIC.map(|(name, _)| name).join(", ")
        );
        ensure!(
            graded
                .insert(criterion.clone(), Preference::parse(value)?)
                .is_none(),
            "rubric criterion {criterion} is given twice"
        );
    }
    append(id, pair, reader, |records| {
        ensure!(
            next(records) == Next::Preference,
            "make both merge calls before the preference and rubric"
        );
        Ok(Call::Preference {
            preference,
            rubric: graded,
        })
    })
}

/// People's calls on retained pairs, with people–panel agreement.
pub(super) fn people(files: &impl Files, root: &Path, pairs: &[Pair], seed: u64) -> Result<People> {
    let mut people = People::default();
    for pair in pairs {
        let key = pair.key();
        let sides = sides(seed, &key);
        // Each reader's open review, and their latest blind one.
        let mut open: BTreeMap<String, usize> = BTreeMap::new();
        let mut blind: BTreeMap<String, usize> = BTreeMap::new();
        for record in records(files, root, &key)? {
            let who = record.reader.who().to_owned();
            match record.call {
                Call::Merge { a_ready, b_ready } => {
                    let ready = |arm| if sides[0] == arm { a_ready } else { b_ready };
                    // A reader who has given a preference has seen the Arms.
                    let unseen = !people.reviews.iter().any(|review: &Review| {
                        review.pair == key
                            && review.reader.who() == who
                            && review.preference.is_some()
                    });
                    if unseen {
                        blind.insert(who.clone(), people.reviews.len());
                    }
                    open.insert(who, people.reviews.len());
                    people.reviews.push(Review {
                        pair: key.clone(),
                        suite: pair.suite.clone(),
                        task: pair.task.clone(),
                        trial: pair.trial,
                        reader: record.reader,
                        recorded_at: record.recorded_at,
                        blind: unseen,
                        baseline_ready: ready(Arm::Baseline),
                        candidate_ready: ready(Arm::Candidate),
                        preference: None,
                        rubric: BTreeMap::new(),
                    });
                }
                Call::Preference { preference, rubric } => {
                    let review = open
                        .get(&who)
                        .map(|index| &mut people.reviews[*index])
                        .filter(|review| review.preference.is_none())
                        .context("pairwise record has a preference without merge calls")?;
                    review.preference = Some(preference.for_arms(sides));
                    review.rubric = rubric
                        .into_iter()
                        .map(|(name, value)| (name, value.for_arms(sides)))
                        .collect();
                }
            }
        }
        let latest: Vec<&Review> = blind
            .values()
            .map(|index| &people.reviews[*index])
            .collect();
        for (at, first) in latest.iter().enumerate() {
            for second in &latest[at + 1..] {
                for (one, other) in [
                    (first.baseline_ready, second.baseline_ready),
                    (first.candidate_ready, second.candidate_ready),
                ] {
                    people.people_agreement.compared += 1;
                    people.people_agreement.agreed += usize::from(one == other);
                }
            }
        }
        if let Some(panel) = &pair.panel {
            for index in blind.values() {
                let review = &people.reviews[*index];
                for (person, model) in [
                    (review.baseline_ready, panel.baseline_ready),
                    (review.candidate_ready, panel.candidate_ready),
                ] {
                    people.agreement.compared += 1;
                    people.agreement.agreed += usize::from(person == model);
                }
            }
        }
    }
    Ok(people)
}

/// The CLI's reading of one pair, in the page's order.
pub(super) fn text(page: &Page) -> String {
    let mut text = format!(
        "Pair {}/{} #{} · {}\n\nTicket\n{}\n\nAcceptance checklist\n",
        page.suite,
        page.task,
        page.number,
        page.pair,
        page.ticket
            .as_deref()
            .unwrap_or("The Ticket is unavailable.")
    );
    for (index, item) in page.checklist.iter().enumerate() {
        text.push_str(&format!("{}. {item}\n", index + 1));
    }
    for (label, side) in [("A", &page.a), ("B", &page.b)] {
        text.push_str(&format!("\nChange {label}\n"));
        for file in &side.diff {
            text.push_str(&file.patch);
        }
        text.push_str(&format!("Evidence: {}\n", evidence(side)));
    }
    text.push('\n');
    for record in &page.mine {
        text.push_str(&format!(
            "{} · {}\n",
            record.recorded_at,
            call(&record.call)
        ));
    }
    match &page.revealed {
        Some(reveal) => {
            for (label, arm, setup) in [
                ("A", reveal.a, &reveal.a_setup),
                ("B", reveal.b, &reveal.b_setup),
            ] {
                text.push_str(&format!("Change {label}: {}\n", revealed_side(arm, setup)));
            }
            text.push_str(&match &reveal.panel {
                Some(panel) => format!("Panel: {}\n", panel_line(panel)),
                None => "No Grader panel calls are recorded for this pair.\n".into(),
            });
        }
        None => text.push_str(&format!("{}\n", next_command(page))),
    }
    text
}

/// The command that records the reader's next calls.
pub(crate) fn next_command(page: &Page) -> String {
    format!(
        "thirdshift experiments {} --pair {} {}",
        page.experiment,
        page.pair,
        match page.next {
            Next::Merge => "--merge <ready|not-ready> <ready|not-ready>",
            Next::Preference =>
                "--prefer <a|b|tie|both-bad> [--rubric <criterion>=<a|b|tie|both-bad>]...",
        }
    )
}

/// Which Arm made one side, with its setup, for the CLI and the page alike.
pub(crate) fn revealed_side(arm: Arm, setup: &Setup) -> String {
    if let Some(runner) = &setup.runner {
        return format!(
            "{} arm · Custom runner {runner} · digest {}",
            capitalized(arm),
            setup.runner_digest.as_deref().unwrap_or("not recorded")
        );
    }
    format!(
        "{} arm · {} · {} · {} · {}",
        capitalized(arm),
        setup.executor(),
        setup.model.as_deref().unwrap_or("default Model"),
        setup.effort.as_deref().unwrap_or("default Effort"),
        setup.reference()
    )
}

/// People's calls for the Experiment's text view, numbering pairs from 1.
pub(super) fn lines(people: &People) -> String {
    let mut text = String::new();
    for review in &people.reviews {
        text.push_str(&format!(
            "\nPeople {}/{} #{} · {} · Baseline {} · Candidate {}{}",
            review.suite,
            review.task,
            review.trial + 1,
            review.reader,
            ready(review.baseline_ready),
            ready(review.candidate_ready),
            review
                .preference
                .map_or(String::new(), |preference| format!(
                    " · prefers {}",
                    arm_preference(preference)
                ))
        ));
        if !review.blind {
            text.push_str(" · after the reveal");
        }
    }
    for queue in &people.queues {
        text.push_str(&format!(
            "\nQueue {} · {} pairs: {}",
            queue.reader,
            queue.pairs.len(),
            queue.pairs.join(", ")
        ));
    }
    if !people.reviews.is_empty() {
        text.push_str(&format!(
            "\n{}\n{}",
            agreement(&people.agreement),
            people_agreement(&people.people_agreement)
        ));
    }
    text
}

pub(crate) fn agreement(agreement: &Agreement) -> String {
    if agreement.compared == 0 {
        "People–panel agreement: no pair read by both".into()
    } else {
        format!(
            "People–panel agreement: {}/{} merge calls",
            agreement.agreed, agreement.compared
        )
    }
}

pub(crate) fn people_agreement(agreement: &Agreement) -> String {
    if agreement.compared == 0 {
        "People–people agreement: no pair read by two readers".into()
    } else {
        format!(
            "People–people agreement: {}/{} merge calls",
            agreement.agreed, agreement.compared
        )
    }
}

/// One side's base-test and cross-test results, without naming an Arm.
pub(crate) fn evidence(side: &pairs::Side) -> String {
    if let Some(error) = &side.error {
        return format!("unavailable: {error}");
    }
    if side.test_files.is_empty() {
        return "no new tests".into();
    }
    let result = |outcome: Option<bool>, yes: &str, no: &str| match outcome {
        Some(true) => yes.to_owned(),
        Some(false) => no.to_owned(),
        None => "did not finish".to_owned(),
    };
    format!(
        "{} new test file(s); on the base: {}; on the other change: {}",
        side.test_files.len(),
        result(side.fails_on_base, "fail", "pass"),
        result(
            side.passes_on_other,
            "pass",
            "fail (a disagreement to look into)"
        )
    )
}

pub(crate) fn call(call: &Call) -> String {
    match call {
        Call::Merge { a_ready, b_ready } => {
            format!("merge calls: A {} · B {}", ready(*a_ready), ready(*b_ready))
        }
        Call::Preference { preference, rubric } => {
            let mut line = format!("preference: {}", preference.label());
            for (criterion, value) in rubric {
                line.push_str(&format!(" · {criterion}: {}", value.label()));
            }
            line
        }
    }
}

pub(crate) fn panel_line(panel: &PanelCalls) -> String {
    format!(
        "Baseline {} · Candidate {} · prefers {}",
        ready(panel.baseline_ready),
        ready(panel.candidate_ready),
        arm_preference(panel.preference)
    )
}

pub(crate) fn arm_preference(preference: ArmPreference) -> &'static str {
    match preference {
        ArmPreference::Baseline => "Baseline",
        ArmPreference::Candidate => "Candidate",
        ArmPreference::Tie => "tie",
        ArmPreference::BothBad => "both bad",
    }
}

pub(crate) fn ready(ready: bool) -> &'static str {
    if ready { "ready" } else { "not ready" }
}

pub(crate) fn capitalized(arm: Arm) -> &'static str {
    match arm {
        Arm::Baseline => "Baseline",
        Arm::Candidate => "Candidate",
    }
}

fn root() -> Result<std::path::PathBuf> {
    Ok(crate::config::home()?.join(".thirdshift/evals/experiments"))
}

fn append(
    id: &str,
    pair: &str,
    reader: Actor,
    call: impl FnOnce(&[Record]) -> Result<Call>,
) -> Result<Page> {
    let stored = Stored::load(&root()?, id, false)?;
    let role = super::deciders::people_role(&stored.description.bar);
    ensure!(
        role != Role::Off,
        "this Experiment's Bar has people off: no one grades its pairs"
    );
    find(&stored, pair)?;
    let folder = stored.directory.join("reviews").join(pair);
    fs::create_dir_all(&folder)?;
    let lock = File::create(folder.join("lock"))?;
    lock.lock()?;
    let mine = mine(records(&Native, &stored.directory, pair)?, &reader);
    let record = Record {
        call: call(&mine)?,
        reader: reader.clone(),
        recorded_at: chrono::Local::now().to_rfc3339(),
    };
    let mut temporary = tempfile::NamedTempFile::new_in(&folder)?;
    serde_json::to_writer_pretty(&mut temporary, &record)?;
    temporary.flush()?;
    // Never replace a record: take the next free number.
    let mut number = records(&Native, &stored.directory, pair)?.len();
    loop {
        match temporary.persist_noclobber(folder.join(format!("{number:06}.json"))) {
            Ok(_) => break,
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                temporary = error.file;
                number += 1;
            }
            Err(error) => return Err(error.error.into()),
        }
    }
    drop(lock);
    if role == Role::Counted {
        Stored::refresh_completed(&root()?, id)?;
    }
    build(&stored, pair, &reader)
}

fn build(stored: &Stored, key: &str, reader: &Actor) -> Result<Page> {
    let pair = find(stored, key)?;
    let task = stored
        .tasks
        .iter()
        .find(|task| task.suite == pair.suite && task.name == pair.task);
    let ticket = task.and_then(|task| ticket(task).ok());
    let all = records(&Native, &stored.directory, key)?;
    let mine = mine(all.clone(), reader);
    let sides = sides(stored.description.bar.seed, key);
    let revealed = mine
        .iter()
        .any(|record| matches!(record.call, Call::Preference { .. }));
    let next = next(&mine);
    let Pair {
        suite,
        task: name,
        trial,
        mut baseline,
        mut candidate,
        panel,
    } = pair;
    let mut take = |arm| match arm {
        Arm::Baseline => std::mem::take(&mut baseline),
        Arm::Candidate => std::mem::take(&mut candidate),
    };
    let (mut a, mut b) = (take(sides[0]), take(sides[1]));
    let runner_identities = runner_identities(&stored.description);
    for side in [&mut a, &mut b] {
        side.diff.sort_by_key(|file| file.test);
        blind_runner_side(side, &runner_identities);
        // Blinded as the panel's packet is: added lines that would name the
        // Arm go, and so does error text, which can name retained paths.
        for file in &mut side.diff {
            file.patch = blind_diff(&file.patch);
        }
        if side.error.is_some() {
            side.error = Some("the evidence could not run".into());
        }
    }
    Ok(Page {
        experiment: stored.description.id.clone(),
        pair: key.to_owned(),
        suite,
        task: name,
        number: trial + 1,
        checklist: ticket.as_deref().map(checklist).unwrap_or_default(),
        ticket,
        a,
        b,
        rubric: RUBRIC
            .iter()
            .map(|(name, question)| Criterion { name, question })
            .collect(),
        reader: reader.who().to_owned(),
        next,
        mine,
        revealed: revealed.then(|| Reveal {
            a: sides[0],
            b: sides[1],
            a_setup: stored.description.setup(sides[0]).clone(),
            b_setup: stored.description.setup(sides[1]).clone(),
            a_runner: stored.description.runner(sides[0]).cloned(),
            b_runner: stored.description.runner(sides[1]).cloned(),
            panel,
            records: all,
        }),
    })
}

/// Retained diffs and check diagnostics can name an outside workflow even
/// though its Setup and prepared metadata are confined to the reveal.
#[derive(Default)]
struct RunnerIdentities {
    attribution: Vec<String>,
    role_metadata: Vec<String>,
}

fn runner_identities(description: &super::Description) -> RunnerIdentities {
    let mut identities = RunnerIdentities::default();
    for arm in [Arm::Baseline, Arm::Candidate] {
        let setup = description.setup(arm);
        identities
            .attribution
            .extend(setup.runner.iter().chain(&setup.runner_digest).cloned());
        if let Some(runner) = description.runner(arm) {
            for (role, choice) in &runner.manifest.roles {
                identities
                    .role_metadata
                    .extend([role.clone(), choice.model.clone()]);
                identities
                    .role_metadata
                    .extend(choice.effort.iter().cloned());
            }
            for git in runner.provenance.values() {
                identities.attribution.push(git.commit.clone());
                identities.attribution.extend(git.origin.iter().cloned());
            }
        }
    }
    for values in [&mut identities.attribution, &mut identities.role_metadata] {
        values.retain(|value| !value.is_empty());
        // Redact a full identity before any other identity that is its prefix.
        values.sort_by_key(|value| std::cmp::Reverse(value.len()));
        values.dedup();
    }
    identities
}

fn blind_runner_side(side: &mut pairs::Side, identities: &RunnerIdentities) {
    if identities.attribution.is_empty() {
        return;
    }
    let blind = |text: &mut String, values: &[String]| {
        for identity in values {
            *text = text.replace(identity, "[withheld]");
        }
    };
    for file in &mut side.diff {
        file.patch = blind_runner_diff(&file.patch, identities);
        blind(&mut file.path, &identities.attribution);
        blind(&mut file.patch, &identities.attribution);
    }
    for file in &mut side.test_files {
        blind(file, &identities.attribution);
    }
    for check in side.base.iter_mut().chain(&mut side.cross) {
        for argument in check.command.iter_mut().flatten() {
            blind(argument, &identities.attribution);
        }
        blind(&mut check.reason, &identities.attribution);
        blind(&mut check.reason, &identities.role_metadata);
    }
}

fn blind_runner_diff(diff: &str, identities: &RunnerIdentities) -> String {
    let mut role_table = false;
    let mut blinded = String::new();
    for line in diff.lines() {
        let Some(added) = line.strip_prefix('+').filter(|_| !line.starts_with("+++")) else {
            role_table = false;
            blinded.push_str(line);
            blinded.push('\n');
            continue;
        };
        let normalized = added
            .trim()
            .trim_start_matches(['#', '>', '-', ' '])
            .replace(['*', '_', '`'], "")
            .to_ascii_lowercase();
        if normalized.starts_with('|') {
            let cells: Vec<_> = normalized
                .trim_matches('|')
                .split('|')
                .map(str::trim)
                .collect();
            role_table |= cells.iter().any(|cell| matches!(*cell, "role" | "roles"));
            if role_table {
                continue;
            }
        } else {
            role_table = false;
        }
        let attribution = ["runner", "runner digest", "roles", "role"]
            .iter()
            .any(|key| {
                normalized
                    .strip_prefix(key)
                    .and_then(|rest| rest.trim_start().strip_prefix([':', '=']))
                    .is_some_and(|value| {
                        identities
                            .attribution
                            .iter()
                            .chain(&identities.role_metadata)
                            .any(|identity| {
                                let identity = identity.to_ascii_lowercase();
                                value.match_indices(&identity).any(|(index, _)| {
                                    let identifier = |c: char| c.is_alphanumeric() || c == '_';
                                    !value[..index].chars().next_back().is_some_and(identifier)
                                        && !value[index + identity.len()..]
                                            .chars()
                                            .next()
                                            .is_some_and(identifier)
                                })
                            })
                    })
            });
        if !attribution {
            blinded.push_str(line);
            blinded.push('\n');
        }
    }
    blinded
}

/// The retained pair named by `key`, which must be one of the Experiment's.
fn find(stored: &Stored, key: &str) -> Result<Pair> {
    let mut pairs = pairs::read(&Native, &stored.directory)?;
    let panel = super::panel::read(
        &Native,
        &stored.directory,
        &stored.description,
        &stored.tasks,
    )?;
    super::panel::attach(&mut pairs, &panel);
    pairs
        .into_iter()
        .find(|pair| pair.key() == key)
        .context("pair not found; pairs exist once both Trials have finished")
}

fn records(files: &impl Files, root: &Path, pair: &str) -> Result<Vec<Record>> {
    let mut records = Vec::new();
    for entry in directory(files, &root.join("reviews").join(pair))? {
        if entry.kind.is_file() && entry.path.extension().is_some_and(|ext| ext == "json") {
            records.push(serde_json::from_slice(&files.read(&entry.path)?)?);
        }
    }
    Ok(records)
}

fn mine(records: Vec<Record>, reader: &Actor) -> Vec<Record> {
    records
        .into_iter()
        .filter(|record| record.reader.who() == reader.who())
        .collect()
}

/// A preference needs merge calls since the reader's last preference.
fn next(mine: &[Record]) -> Next {
    match mine.last() {
        Some(Record {
            call: Call::Merge { .. },
            ..
        }) => Next::Preference,
        _ => Next::Merge,
    }
}

/// The Ticket the Trials started from: the task's `.thirdshift-issue.md`
/// input, read from inside its task folder.
pub(super) fn ticket(task: &super::Task) -> Result<String> {
    let source = task
        .manifest
        .inputs
        .get(crate::prompt::ISSUE_FILE)
        .context("the Eval task has no Ticket input")?;
    let folder = task.directory.canonicalize()?;
    let path = folder.join(source).canonicalize()?;
    ensure!(
        path.starts_with(&folder) && path.is_file(),
        "Eval input must be a file inside its task folder"
    );
    Ok(fs::read_to_string(path)?)
}

/// The items under the Ticket's "Acceptance criteria" heading, in order.
fn checklist(ticket: &str) -> Vec<String> {
    let mut inside = false;
    let mut items = Vec::new();
    for line in ticket.lines().map(str::trim) {
        if let Some(heading) = line.strip_prefix('#') {
            inside = heading
                .trim_start_matches('#')
                .trim()
                .eq_ignore_ascii_case("acceptance criteria");
        } else if inside
            && let Some(item) = ["- [ ] ", "- [x] ", "- [X] ", "- ", "* "]
                .iter()
                .find_map(|marker| line.strip_prefix(marker))
        {
            items.push(item.trim().to_owned());
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sides_are_drawn_per_pair_from_the_seed() {
        let drawn: Vec<_> = (0..16)
            .map(|trial| sides(0, &format!("task--pair--{trial}"))[0])
            .collect();
        assert!(drawn.contains(&Arm::Baseline) && drawn.contains(&Arm::Candidate));
        assert!(
            drawn
                .iter()
                .enumerate()
                .all(|(trial, arm)| { sides(0, &format!("task--pair--{trial}"))[0] == *arm })
        );
        assert!((0..16).any(|seed| {
            sides(seed, "task--pair--0")[0] != sides(seed + 1, "task--pair--0")[0]
        }));
    }

    #[test]
    fn the_checklist_is_the_acceptance_criteria_in_order() {
        let ticket = "# Title\n\n- not this\n\n## Acceptance criteria\n\n- [ ] First\n- [x] Second\n  - Nested\n\nProse.\n\n## Notes\n\n- not this either\n";
        assert_eq!(checklist(ticket), ["First", "Second", "Nested"]);
        assert!(checklist("No criteria here.").is_empty());
    }
}
