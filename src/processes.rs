//! Custom processes: sequential skill Steps from `~/.thirdshift/processes/<name>.toml`.

use std::collections::BTreeSet;
use std::fmt::Write;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use toml_edit::{Document, Item, Key, Table};

use crate::args::{ProcessAction, ProcessRun};
use crate::config::UserConfig;
use crate::git::Git;
use crate::harness::{self, Asked, Choice, ChosenBy, Harness};
use crate::logs::{self, Begin, Work};
use crate::skill;
use crate::worktree::SkillWorktree;
use crate::{interrupt, launch, progress, prompt, skills};

const RESERVED: &[&str] = &[
    "check",
    "list",
    "ticket-run",
    "spec-run",
    "architect-run",
    "pickup-run",
    "security-run",
];

struct ProcessFile {
    description: String,
    steps: Vec<StepDraft>,
}

struct StepDraft {
    name: String,
    name_line: usize,
    skill: String,
    skill_line: usize,
    harness: Option<Harness>,
    /// The line a refusal names: the Step's `harness` when it has one, else its name.
    report_line: usize,
    model: Option<String>,
    effort: Option<String>,
}

struct ResolvedStep {
    name: String,
    report_line: usize,
    skill_name: String,
    skill_path: PathBuf,
    choice: Choice,
    sources: Sources,
    /// Why this Step's Harness can't start, when the preflight failed.
    start_failure: Option<String>,
}

/// Where a Step's Harness, Model and Effort each came from.
#[derive(Clone, Copy)]
struct Sources {
    harness: Source,
    model: Source,
    effort: Source,
}

/// Where one of a Step's Harness, Model or Effort came from.
#[derive(Clone, Copy)]
enum Source {
    Step,
    Command,
    UserConfig,
    Default,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(match self {
            Source::Step => "Step",
            Source::Command => "command",
            Source::UserConfig => "User config",
            Source::Default => "default",
        })
    }
}

struct Prepared {
    path: PathBuf,
    worktree: SkillWorktree,
    _written: WrittenGuard,
    /// The Command's Harness, Model and Effort. A Step that names one of
    /// its own uses that instead.
    command: Choice,
    description: String,
    steps: Vec<ResolvedStep>,
}

struct WrittenGuard;

impl Drop for WrittenGuard {
    fn drop(&mut self) {
        skills::remove_written();
    }
}

pub fn execute(action: ProcessAction) -> Result<String> {
    match action {
        ProcessAction::List => list(),
        ProcessAction::Check(command) => check(&command),
        ProcessAction::Run(command) => run(&command),
    }
}

fn list() -> Result<String> {
    let dir = processes_dir()?;
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("could not list {}", dir.display()));
        }
    };
    let mut lines = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("could not list {}", dir.display()))?;
        let file_name = entry.file_name();
        let Some(name) = file_name
            .to_str()
            .and_then(|name| name.strip_suffix(".toml"))
        else {
            continue;
        };
        if !crate::args::one_path_component(name) || reserved(name) {
            continue;
        }
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(file) = parse(&path, &text) else {
            continue;
        };
        lines.push(format!("{name}: {}", file.description));
    }
    lines.sort();
    Ok(lines.join("\n"))
}

fn check(command: &ProcessRun) -> Result<String> {
    interrupt::install()?;
    let config = UserConfig::load()?;
    let prepared = prepare(&command.name, &command.asked, &config)?;
    let text = plan(&command.name, &prepared);
    if let Err(error) = refuse_unstartable(&prepared) {
        // The plan says whether each Harness starts, including one that can't.
        println!("{text}");
        return Err(error);
    }
    Ok(text)
}

fn run(command: &ProcessRun) -> Result<String> {
    interrupt::install()?;
    let config = UserConfig::load()?;
    let name = &command.name;
    let prepared = prepare(name, &command.asked, &config)?;
    refuse_unstartable(&prepared)?;
    let repo = launch::repo()?;
    logs::begin(Begin::Process(name));
    logs::configured(&config);
    logs::started(Work::Process { repo: &repo, name }, &prepared.command);
    let result = execute_steps(name, &prepared);
    let (outcome, cause) = classify(&result);
    drop(prepared);
    logs::ended(&outcome);
    logs::complete(&outcome, cause.as_deref());
    result
}

fn execute_steps(process: &str, prepared: &Prepared) -> Result<String> {
    link_harnesses(prepared.worktree.path(), &prepared.steps)?;
    let results = logs::command_log_path()
        .context("a Process has no Command log")?
        .with_extension("steps");
    fs::create_dir_all(&results)
        .with_context(|| format!("could not create {}", results.display()))?;
    let mut earlier = Vec::new();
    let mut last = String::new();
    for step in &prepared.steps {
        let started = Instant::now();
        match run_step(process, prepared, step, &earlier) {
            Ok(message) => {
                let duration = progress::duration(started.elapsed());
                progress::step(format_args!("step {}: {duration} succeeded", step.name));
                let path = results.join(format!("{}.txt", step.name));
                fs::write(&path, &message)
                    .with_context(|| format!("could not write {}", path.display()))?;
                earlier.push((step.name.clone(), path));
                last = message;
            }
            Err(error) => {
                let duration = progress::duration(started.elapsed());
                progress::step(format_args!(
                    "step {}: {duration} {}: {}",
                    step.name,
                    failure_outcome(),
                    one_line(&error)
                ));
                return Err(error);
            }
        }
    }
    Ok(last)
}

fn run_step(
    process: &str,
    prepared: &Prepared,
    step: &ResolvedStep,
    earlier: &[(String, PathBuf)],
) -> Result<String> {
    crate::resources::wait()?;
    progress::step(format_args!(
        "running step {} with {}",
        step.name, step.choice
    ));
    let prompt = prompt::process_step(
        process,
        &step.name,
        &step.skill_name,
        &step.skill_path,
        earlier,
    );
    skill::run_session(
        prepared.worktree.path(),
        &step.choice,
        &format!("Step {}", step.name),
        &prompt,
    )
}

/// Link each distinct Harness's Factory skills before the first session.
fn link_harnesses(worktree: &Path, steps: &[ResolvedStep]) -> Result<()> {
    let mut linked = Vec::new();
    for step in steps {
        if linked.contains(&step.choice.harness) {
            continue;
        }
        linked.push(step.choice.harness);
        let adapter = step.choice.harness.adapter();
        skills::link_into(worktree, adapter)?;
        adapter.link_instruction_fallback(worktree)?;
    }
    Ok(())
}

/// The first Step whose Harness can't start, as a refusal. No session has run.
fn refuse_unstartable(prepared: &Prepared) -> Result<()> {
    let Some(step) = prepared
        .steps
        .iter()
        .find(|step| step.start_failure.is_some())
    else {
        return Ok(());
    };
    let reason = step.start_failure.as_deref().unwrap_or_default();
    Err(rule(
        &prepared.path,
        step.report_line,
        "harness",
        format!(
            "step {}'s Harness {} can't start: {reason}",
            step.name,
            step.choice.harness.name()
        ),
    ))
}

fn classify(result: &Result<String>) -> (String, Option<String>) {
    match result {
        Ok(_) => ("succeeded".to_string(), None),
        Err(error) => (failure_outcome().to_string(), Some(one_line(error))),
    }
}

fn failure_outcome() -> &'static str {
    if interrupt::requested() {
        "interrupted"
    } else {
        "failed"
    }
}

fn one_line(error: &anyhow::Error) -> String {
    format!("{error:#}")
        .replace(['\n', '\r'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn plan(name: &str, prepared: &Prepared) -> String {
    let mut text = format!(
        "{name}: {description}\nthrowaway worktree at the Base branch\n",
        description = prepared.description,
    );
    for step in &prepared.steps {
        let _ = writeln!(
            text,
            "{}: {} {}",
            step.name,
            step.skill_name,
            step.skill_path.display()
        );
        let _ = writeln!(
            text,
            "harness: {} ({})",
            step.choice.harness.name(),
            step.sources.harness
        );
        let _ = writeln!(
            text,
            "model: {} ({})",
            step.choice.model.as_deref().unwrap_or("default"),
            step.sources.model
        );
        let _ = writeln!(
            text,
            "effort: {} ({})",
            step.choice.effort.as_deref().unwrap_or("default"),
            step.sources.effort
        );
        let _ = writeln!(
            text,
            "starts: {}",
            match &step.start_failure {
                None => "yes".to_string(),
                Some(reason) => format!("no: {reason}"),
            }
        );
    }
    text.trim_end().to_string()
}

fn prepare(name: &str, asked: &Asked, config: &UserConfig) -> Result<Prepared> {
    let path = process_path(name)?;
    refuse_reserved(&path, name)?;
    let text = read_process(&path)?;
    let file = parse(&path, &text)?;
    let base = launch::checked_out_base()?;
    let worktree = SkillWorktree::at(&Git::new(std::env::current_dir()?), base.origin_commit())?;
    let written = WrittenGuard;
    let command = Choice::of(asked, &config.harness);
    let mut steps = Vec::new();
    for step in &file.steps {
        steps.push(resolve(
            &path,
            step,
            worktree.path(),
            &command,
            asked,
            config,
        )?);
    }
    Ok(Prepared {
        path,
        worktree,
        _written: written,
        command,
        description: file.description,
        steps,
    })
}

fn resolve(
    path: &Path,
    step: &StepDraft,
    worktree: &Path,
    command: &Choice,
    asked: &Asked,
    config: &UserConfig,
) -> Result<ResolvedStep> {
    let skill = match skills::find(worktree, &step.skill) {
        Ok(skill) => skill,
        Err(error) => {
            return Err(rule(
                path,
                step.skill_line,
                "skill not found",
                format!("{error:#}"),
            ));
        }
    };
    let (mut choice, sources) = step_choice(step, command, asked, &config.harness);
    let start_failure = match choice.check_without_probe() {
        Ok(()) => None,
        Err(error) => Some(one_line(&error)),
    };
    Ok(ResolvedStep {
        name: step.name.clone(),
        report_line: step.report_line,
        skill_name: skill.name,
        skill_path: skill.path,
        choice,
        sources,
        start_failure,
    })
}

/// The Step's Harness, Model and Effort. One it leaves out uses the
/// Command's for that setting: the command's word, else the User config for
/// this Step's Harness, else that Harness's default. A command model or
/// effort applies even when the Step names a different Harness.
fn step_choice(
    step: &StepDraft,
    command: &Choice,
    asked: &Asked,
    settings: &harness::Settings,
) -> (Choice, Sources) {
    let (harness, harness_source, chosen_by) = match step.harness {
        Some(harness) => (harness, Source::Step, ChosenBy::Step),
        None => (
            command.harness,
            source_from(command.chosen_by),
            command.chosen_by,
        ),
    };
    let configured = settings.of(harness);
    let (model, model_source) = left_out(
        step.model.clone(),
        asked.model_and_effort.model.as_deref(),
        command.model.clone(),
        configured.model.clone(),
    );
    let (effort, effort_source) = left_out(
        step.effort.clone(),
        asked.model_and_effort.effort.as_deref(),
        command.effort.clone(),
        configured.effort.clone(),
    );
    (
        Choice {
            harness,
            model,
            effort,
            chosen_by,
        },
        Sources {
            harness: harness_source,
            model: model_source,
            effort: effort_source,
        },
    )
}

/// A Step value, else the command's word, else the User config, else default.
fn left_out(
    named: Option<String>,
    command_asked: Option<&str>,
    command_value: Option<String>,
    configured: Option<String>,
) -> (Option<String>, Source) {
    if let Some(value) = named {
        (Some(value), Source::Step)
    } else if command_asked.is_some() {
        (command_value, Source::Command)
    } else if let Some(value) = configured {
        (Some(value), Source::UserConfig)
    } else {
        (None, Source::Default)
    }
}

fn source_from(chosen_by: ChosenBy) -> Source {
    match chosen_by {
        ChosenBy::Command => Source::Command,
        ChosenBy::UserConfig | ChosenBy::SecurityConfig => Source::UserConfig,
        ChosenBy::Default | ChosenBy::Step => Source::Default,
    }
}

fn read_process(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == ErrorKind::NotFound => Err(rule(
            path,
            1,
            "missing file",
            "the process file does not exist",
        )),
        Err(error) => Err(rule(
            path,
            1,
            "file",
            format!("could not read the file: {error}"),
        )),
    }
}

fn refuse_reserved(path: &Path, name: &str) -> Result<()> {
    if reserved(name) {
        Err(rule(
            path,
            1,
            "reserved name",
            format!("{name} is reserved"),
        ))
    } else {
        Ok(())
    }
}

fn parse(path: &Path, text: &str) -> Result<ProcessFile> {
    let doc = Document::parse(text).map_err(|error: toml_edit::TomlError| {
        let line = error
            .span()
            .map(|span| line_of(text, span.start))
            .unwrap_or(1);
        rule(path, line, "syntax", error)
    })?;
    let table = doc.as_table();
    let mut saw_version = false;
    let mut description = None;
    let mut steps = None;
    for (key, item) in table.iter() {
        let line = key_line(text, table.key(key));
        match key {
            "version" => {
                let Some(value) = item.as_integer() else {
                    return Err(rule(path, line, "version", "version must be the integer 1"));
                };
                if value != 1 {
                    return Err(rule(path, line, "version", "version must be 1"));
                }
                saw_version = true;
            }
            "description" => {
                description = Some(description_of(path, line, item)?);
            }
            "steps" => {
                let parsed = parse_steps(path, text, line, item)?;
                if parsed.is_empty() {
                    return Err(rule(path, line, "steps", "at least one step is required"));
                }
                steps = Some(parsed);
            }
            other => return Err(rule(path, line, "unknown key", other)),
        }
    }
    if !saw_version {
        return Err(rule(path, 1, "version", "version = 1 is required"));
    }
    let Some(description) = description else {
        return Err(rule(path, 1, "description", "description is required"));
    };
    let Some(steps) = steps else {
        return Err(rule(path, 1, "steps", "at least one step is required"));
    };
    Ok(ProcessFile { description, steps })
}

fn description_of(path: &Path, line: usize, item: &Item) -> Result<String> {
    let Some(value) = item.as_str() else {
        return Err(rule(
            path,
            line,
            "description",
            "description must be a string",
        ));
    };
    if value.is_empty() {
        return Err(rule(
            path,
            line,
            "description",
            "description must not be empty",
        ));
    }
    if value.chars().any(|c| c == '\n' || c == '\r') {
        return Err(rule(
            path,
            line,
            "description",
            "description must be one line",
        ));
    }
    Ok(value.to_string())
}

fn parse_steps(path: &Path, text: &str, line: usize, item: &Item) -> Result<Vec<StepDraft>> {
    let Some(tables) = item.as_array_of_tables() else {
        return Err(rule(
            path,
            line,
            "steps",
            "steps must be an array of tables",
        ));
    };
    let mut steps = Vec::new();
    let mut names = BTreeSet::new();
    for table in tables.iter() {
        let table_line = table
            .span()
            .map(|span| line_of(text, span.start))
            .unwrap_or(line);
        let step = parse_step(path, text, table_line, table)?;
        if !names.insert(step.name.clone()) {
            return Err(rule(
                path,
                step.name_line,
                "step name",
                format!("duplicate step {}", step.name),
            ));
        }
        steps.push(step);
    }
    Ok(steps)
}

fn parse_step(path: &Path, text: &str, table_line: usize, table: &Table) -> Result<StepDraft> {
    let mut name = None;
    let mut skill = None;
    let mut harness = None;
    let mut model = None;
    let mut effort = None;
    for (key, item) in table.iter() {
        let line = key_line(text, table.key(key));
        match key {
            "name" => name = Some(step_name(path, line, item)?),
            "skill" => skill = Some(step_skill(path, line, item)?),
            "harness" => harness = Some(step_harness(path, line, item)?),
            "model" => model = Some(step_word(path, line, item, "model")?),
            "effort" => effort = Some(step_word(path, line, item, "effort")?),
            other => return Err(rule(path, line, "unknown key", other)),
        }
    }
    let Some((name, name_line)) = name else {
        return Err(rule(path, table_line, "step name", "a step needs a name"));
    };
    let Some((skill, skill_line)) = skill else {
        return Err(rule(
            path,
            name_line,
            "exactly one kind",
            format!("step {name} has no kind"),
        ));
    };
    let (harness, report_line) = match harness {
        Some((harness, line)) => (Some(harness), line),
        None => (None, name_line),
    };
    Ok(StepDraft {
        name,
        name_line,
        skill,
        skill_line,
        harness,
        report_line,
        model,
        effort,
    })
}

fn step_harness(path: &Path, line: usize, item: &Item) -> Result<(Harness, usize)> {
    let value = step_word(path, line, item, "harness")?;
    let Some(harness) = Harness::named(&value) else {
        return Err(rule(path, line, "unknown harness", &value));
    };
    Ok((harness, line))
}

fn step_word(path: &Path, line: usize, item: &Item, rule_name: &str) -> Result<String> {
    let Some(value) = item.as_str() else {
        return Err(rule(
            path,
            line,
            rule_name,
            format!("{rule_name} must be a string"),
        ));
    };
    if value.trim().is_empty() {
        return Err(rule(
            path,
            line,
            rule_name,
            format!("{rule_name} must not be empty"),
        ));
    }
    Ok(value.to_string())
}

fn step_name(path: &Path, line: usize, item: &Item) -> Result<(String, usize)> {
    let Some(value) = item.as_str() else {
        return Err(rule(path, line, "step name", "name must be a string"));
    };
    if !crate::args::one_path_component(value) {
        return Err(rule(
            path,
            line,
            "step name",
            "name must be a single path component",
        ));
    }
    Ok((value.to_string(), line))
}

fn step_skill(path: &Path, line: usize, item: &Item) -> Result<(String, usize)> {
    let Some(value) = item.as_str() else {
        return Err(rule(
            path,
            line,
            "exactly one kind",
            "skill must be a string",
        ));
    };
    Ok((value.to_string(), line))
}

fn processes_dir() -> Result<PathBuf> {
    Ok(crate::config::home()?.join(".thirdshift/processes"))
}

fn process_path(name: &str) -> Result<PathBuf> {
    Ok(processes_dir()?.join(format!("{name}.toml")))
}

fn reserved(name: &str) -> bool {
    RESERVED.contains(&name)
}

fn line_of(text: &str, offset: usize) -> usize {
    text.bytes()
        .take(offset)
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn key_line(text: &str, key: Option<&Key>) -> usize {
    key.and_then(Key::span)
        .map(|span| line_of(text, span.start))
        .unwrap_or(1)
}

fn rule(path: &Path, line: usize, rule: &str, detail: impl std::fmt::Display) -> anyhow::Error {
    anyhow!("{path}:{line}: {rule}: {detail}", path = path.display())
}
