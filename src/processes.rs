//! Custom processes: sequential skill and command Steps from
//! `~/.thirdshift/processes/<name>.toml`.

use std::collections::BTreeSet;
use std::fmt::Write;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use toml_edit::{Document, Item, Key, Table};

use crate::args::ProcessAction;
use crate::config::UserConfig;
use crate::git::Git;
use crate::harness::{Asked, Choice};
use crate::logs::{self, Begin, Work};
use crate::process::{self, Control, Interruption};
use crate::skill;
use crate::worktree::SkillWorktree;
use crate::{interrupt, launch, progress, prompt, skills};

/// The issue URL when the Process has an issue; unset otherwise.
const ISSUE_URL: &str = "THIRDSHIFT_ISSUE_URL";
/// The folder of Step result files, beside the Command log.
const RESULTS_DIR: &str = "THIRDSHIFT_RESULTS_DIR";
/// The path thirdshift writes this Step's combined output to when it finishes.
const STEP_RESULT: &str = "THIRDSHIFT_STEP_RESULT";

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
    optional: bool,
    kind: DraftKind,
}

enum DraftKind {
    Skill { skill: String, line: usize },
    Command { command: String },
}

struct ResolvedStep {
    name: String,
    optional: bool,
    kind: StepKind,
}

enum StepKind {
    Skill { name: String, path: PathBuf },
    Command(String),
}

struct Prepared {
    worktree: SkillWorktree,
    _written: WrittenGuard,
    choice: Choice,
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
        ProcessAction::Check(name) => check(&name),
        ProcessAction::Run(name) => run(&name),
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
        if !single_component(name) || reserved(name) {
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

fn check(name: &str) -> Result<String> {
    interrupt::install()?;
    let config = UserConfig::load()?;
    let prepared = prepare(name, &config)?;
    Ok(plan(name, &prepared))
}

fn run(name: &str) -> Result<String> {
    interrupt::install()?;
    let config = UserConfig::load()?;
    let prepared = prepare(name, &config)?;
    let repo = launch::repo()?;
    logs::begin(Begin::Process(name));
    logs::configured(&config);
    logs::started(Work::Process { repo: &repo, name }, &prepared.choice);
    let result = execute_steps(name, &prepared);
    let (outcome, cause) = classify(&result);
    drop(prepared);
    logs::ended(&outcome);
    logs::complete(&outcome, cause.as_deref());
    result
}

fn execute_steps(process: &str, prepared: &Prepared) -> Result<String> {
    let adapter = prepared.choice.harness.adapter();
    skills::link_into(prepared.worktree.path(), adapter)?;
    adapter.link_instruction_fallback(prepared.worktree.path())?;
    let results = logs::command_log_path()
        .context("a Process has no Command log")?
        .with_extension("steps");
    fs::create_dir_all(&results)
        .with_context(|| format!("could not create {}", results.display()))?;
    let mut earlier = Vec::new();
    let mut last = String::new();
    for (index, step) in prepared.steps.iter().enumerate() {
        let started = Instant::now();
        let path = results.join(format!("{}.txt", step.name));
        match run_step(
            process,
            prepared,
            step,
            &earlier,
            &results,
            &path,
            index == 0,
        ) {
            Ok(message) => {
                let duration = progress::duration(started.elapsed());
                progress::step(format_args!("step {}: {duration} succeeded", step.name));
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
                if step.optional && !interrupt::requested() {
                    continue;
                }
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
    results: &Path,
    result_file: &Path,
    first: bool,
) -> Result<String> {
    crate::resources::wait()?;
    match &step.kind {
        StepKind::Skill { name, path } => {
            progress::step(format_args!(
                "running step {} with {}",
                step.name, prepared.choice
            ));
            let prompt = prompt::process_step(process, &step.name, name, path, earlier, first);
            skill::run_session(
                prepared.worktree.path(),
                &prepared.choice,
                &format!("Step {}", step.name),
                &prompt,
            )
        }
        StepKind::Command(command) => {
            progress::step(format_args!("running step {}", step.name));
            run_command(
                prepared.worktree.path(),
                command,
                results,
                result_file,
                None,
            )
        }
    }
}

fn run_command(
    worktree: &Path,
    command: &str,
    results: &Path,
    result_file: &Path,
    issue_url: Option<&str>,
) -> Result<String> {
    let mut child = Command::new("sh");
    child
        .arg("-c")
        .arg(command)
        .current_dir(worktree)
        .env(RESULTS_DIR, results)
        .env(STEP_RESULT, result_file);
    match issue_url {
        Some(url) => {
            child.env(ISSUE_URL, url);
        }
        None => {
            child.env_remove(ISSUE_URL);
        }
    }
    let output = process::output(
        &mut child,
        None,
        Control {
            name: "command",
            interruption: Interruption::Ordinary,
            stop: &|child| process::stop(child, &[libc::SIGTERM]),
        },
    )?;
    if !output.status.success() {
        bail!("{}", exit_status(output.status));
    }
    Ok(combined(&output.stdout, &output.stderr))
}

fn combined(stdout: &[u8], stderr: &[u8]) -> String {
    let mut text = String::from_utf8_lossy(stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(stderr));
    text
}

fn exit_status(status: std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exit status {code}"),
        None => format!("exit status {status}"),
    }
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
        "{name}: {description}\nthrowaway worktree at the Base branch\nharness: {choice} ({chosen})\n",
        description = prepared.description,
        choice = prepared.choice,
        chosen = prepared.choice.chosen_by,
    );
    for step in &prepared.steps {
        let mut line = match &step.kind {
            StepKind::Skill { name, path } => {
                format!("{}: {} {}", step.name, name, path.display())
            }
            StepKind::Command(command) => {
                let shown = command.replace(['\n', '\r'], " ");
                format!("{}: command {shown}", step.name)
            }
        };
        if step.optional {
            line.push_str(" (optional)");
        }
        let _ = writeln!(text, "{line}");
    }
    text.trim_end().to_string()
}

fn prepare(name: &str, config: &UserConfig) -> Result<Prepared> {
    let path = process_path(name)?;
    refuse_reserved(&path, name)?;
    let text = read_process(&path)?;
    let file = parse(&path, &text)?;
    let base = launch::checked_out_base()?;
    let worktree = SkillWorktree::at(&Git::new(std::env::current_dir()?), base.origin_commit())?;
    let written = WrittenGuard;
    let mut steps = Vec::new();
    for step in &file.steps {
        steps.push(resolve(&path, step, worktree.path())?);
    }
    let mut choice = Choice::of(&Asked::default(), &config.harness);
    choice.check_without_probe()?;
    Ok(Prepared {
        worktree,
        _written: written,
        choice,
        description: file.description,
        steps,
    })
}

fn resolve(path: &Path, step: &StepDraft, worktree: &Path) -> Result<ResolvedStep> {
    let kind = match &step.kind {
        DraftKind::Skill { skill, line } => match skills::find(worktree, skill) {
            Ok(skill) => StepKind::Skill {
                name: skill.name,
                path: skill.path,
            },
            Err(error) => {
                return Err(rule(path, *line, "skill not found", format!("{error:#}")));
            }
        },
        DraftKind::Command { command } => StepKind::Command(command.clone()),
    };
    Ok(ResolvedStep {
        name: step.name.clone(),
        optional: step.optional,
        kind,
    })
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
    let mut command = None;
    let mut optional = false;
    for (key, item) in table.iter() {
        let line = key_line(text, table.key(key));
        match key {
            "name" => name = Some(step_name(path, line, item)?),
            "skill" => skill = Some(step_skill(path, line, item)?),
            "command" => command = Some(step_command(path, line, item)?),
            "optional" => optional = step_optional(path, line, item)?,
            other => return Err(rule(path, line, "unknown key", other)),
        }
    }
    let Some((name, name_line)) = name else {
        return Err(rule(path, table_line, "step name", "a step needs a name"));
    };
    let kind = match (skill, command) {
        (Some((skill, line)), None) => DraftKind::Skill { skill, line },
        (None, Some((command, _))) => DraftKind::Command { command },
        (None, None) => {
            return Err(rule(
                path,
                name_line,
                "exactly one kind",
                format!("step {name} has no kind"),
            ));
        }
        (Some(_), Some((_, line))) => {
            return Err(rule(
                path,
                line,
                "exactly one kind",
                format!("step {name} has more than one kind"),
            ));
        }
    };
    Ok(StepDraft {
        name,
        name_line,
        optional,
        kind,
    })
}

fn step_optional(path: &Path, line: usize, item: &Item) -> Result<bool> {
    item.as_bool()
        .ok_or_else(|| rule(path, line, "optional", "optional must be a boolean"))
}

fn step_name(path: &Path, line: usize, item: &Item) -> Result<(String, usize)> {
    let Some(value) = item.as_str() else {
        return Err(rule(path, line, "step name", "name must be a string"));
    };
    if !single_component(value) {
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

fn step_command(path: &Path, line: usize, item: &Item) -> Result<(String, usize)> {
    let Some(value) = item.as_str() else {
        return Err(rule(
            path,
            line,
            "exactly one kind",
            "command must be a string",
        ));
    };
    if value.is_empty() {
        return Err(rule(
            path,
            line,
            "exactly one kind",
            "command must not be empty",
        ));
    }
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

fn single_component(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0', '\n', '\r'])
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
