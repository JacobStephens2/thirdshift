//! One-time local conversion, with a durable round-trip report as the cleanup
//! receipt. Originals and existing traces are never overwritten by conversion.
mod readings;

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, ensure};
use chrono::Local;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::reading::{Command, Entry, Snapshot};
use super::{owner, trace};
use crate::config::UserConfig;
use crate::harness::{Choice, Harness};
use crate::session::trace::Trace;

const REPORT: &str = "session-trace-conversion.json";

#[derive(Default, Serialize, Deserialize)]
struct Report {
    version: u32,
    sessions: Vec<Session>,
}

#[derive(Serialize, Deserialize)]
struct Session {
    source: PathBuf,
    passed: bool,
    readers: BTreeMap<String, bool>,
    codex_diffs: usize,
    error: Option<String>,
    inputs: BTreeMap<PathBuf, Option<String>>,
}

pub(crate) fn run(remove: bool) -> Result<()> {
    let config = UserConfig::load()?;
    fs::create_dir_all(&config.logs_dir)?;
    let root = config.logs_dir.canonicalize()?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(".session-trace-conversion.lock"))?;
    lock.lock()?;
    let report_path = root.join(REPORT);
    let mut report: Report = match fs::read(&report_path) {
        Ok(bytes) => serde_json::from_slice(&bytes).context("invalid conversion report")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !remove => Report {
            version: 1,
            ..Default::default()
        },
        Err(error) => {
            return Err(error).context("read the conversion report before removing originals");
        }
    };
    ensure!(report.version == 1, "unsupported conversion report version");
    if remove {
        let mut removed = 0;
        for session in &report.sessions {
            if !session.passed || !root.join(&session.source).exists() {
                continue;
            }
            match unchanged(&root, &session.inputs) {
                Ok(()) => {
                    fs::remove_file(confined(&root, &session.source)?)?;
                    removed += 1;
                }
                Err(error) => eprintln!("Kept {}: {error:#}", session.source.display()),
            }
        }
        println!(
            "Removed {removed} originals; report: {}",
            report_path.display()
        );
        return Ok(());
    }
    let snapshot = Snapshot::load(&root, Local::now())?;
    let mut sources = Vec::new();
    inventory(&root, &mut sources)?;
    sources.sort();
    let mut converted = 0;
    for source in sources {
        let relative = source.strip_prefix(&root)?.to_owned();
        if report
            .sessions
            .iter()
            .any(|session| session.source == relative && session.passed)
        {
            continue;
        }
        let mut session = Session {
            source: relative.clone(),
            passed: false,
            readers: BTreeMap::new(),
            codex_diffs: 0,
            error: None,
            inputs: BTreeMap::new(),
        };
        let result = convert(&root, &source, &snapshot, &mut session);
        match result {
            Ok(()) => {
                session.passed = true;
                converted += 1;
            }
            Err(error) => session.error = Some(format!("{error:#}")),
        }
        report.sessions.retain(|old| old.source != relative);
        report.sessions.push(session);
        // A crash loses at most the current session's receipt, never an original.
        save(&report_path, &report)?;
    }
    save(&report_path, &report)?;
    println!(
        "Converted {converted} Sessions; report: {}",
        report_path.display()
    );
    println!(
        "{} passed; {} kept for inspection. Read the report, then run thirdshift logs --remove-converted.",
        report.sessions.iter().filter(|s| s.passed).count(),
        report.sessions.iter().filter(|s| !s.passed).count()
    );
    Ok(())
}

fn inventory(folder: &Path, sources: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let path = entry.path();
        if kind.is_dir() {
            inventory(&path, sources)?;
        } else if kind.is_file()
            && path.extension().is_some_and(|ext| ext == "jsonl")
            && !path
                .file_name()
                .unwrap()
                .as_encoded_bytes()
                .ends_with(b".logs.jsonl")
            && !path
                .file_name()
                .unwrap()
                .as_encoded_bytes()
                .ends_with(b".spans.jsonl")
            && owner::decode(&path).is_some_and(|decoded| decoded.session_kind.is_some())
        {
            sources.push(path);
        }
    }
    Ok(())
}

fn command_for<'a>(commands: &'a [Entry], source: &Path) -> Option<(&'a Command, Vec<u64>)> {
    fn find<'a>(
        command: &'a Command,
        source: &Path,
        relays: Vec<u64>,
    ) -> Option<(&'a Command, Vec<u64>)> {
        for child in &command.children {
            let mut nested = relays.clone();
            if let Some(issue) = &child.record.issue {
                nested.push(issue.number);
            }
            if let Some(found) = find(child, source, nested) {
                return Some(found);
            }
        }
        for child in &command.dispatched {
            if let Some(found) = find(child, source, relays.clone()) {
                return Some(found);
            }
        }
        if command
            .record
            .session_references()
            .any(|path| path == source)
            || owner::decode(source).is_some_and(|decoded| {
                let subject = command
                    .record
                    .issue
                    .as_ref()
                    .map(|i| i.number.to_string())
                    .unwrap_or_else(|| {
                        super::reading::pass_prefix(&command.record.kind)
                            .unwrap_or("")
                            .into()
                    });
                decoded.subject == subject && owner::session_matches(source, &command.command_log)
            })
        {
            Some((command, relays))
        } else {
            None
        }
    }
    commands.iter().find_map(|entry| match entry {
        Entry::Record(command) => find(command, source, Vec::new()),
        _ => None,
    })
}

fn convert(root: &Path, source: &Path, snapshot: &Snapshot, session: &mut Session) -> Result<()> {
    ensure!(
        !source.with_extension("logs.jsonl").exists()
            && !source.with_extension("spans.jsonl").exists(),
        "Session already has a trace; left unchanged"
    );
    let original = fs::read_to_string(source)?;
    let decoded = owner::decode(source).context("invalid Session name")?;
    let models_path = source.with_extension("models.json");
    let models: serde_json::Value = match fs::read(&models_path) {
        Ok(bytes) => serde_json::from_slice(&bytes).context("invalid Model evidence")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::Value::Null,
        Err(error) => return Err(error.into()),
    };
    let retained = command_for(snapshot.entries(), source);
    ensure!(
        retained
            .as_ref()
            .is_none_or(|(command, _)| command.state != "live"),
        "Session belongs to a live Command"
    );
    let harness = models["harness"]
        .as_str()
        .and_then(Harness::named)
        .or_else(|| {
            retained
                .as_ref()
                .and_then(|(c, _)| Harness::named(&c.record.harness))
        })
        .or_else(|| readings::harness(&original))
        .context("cannot identify Harness")?;
    let fallback = readings::historical_command(source, harness, &models)?;
    let has_record = retained.is_some();
    let (command, relays) = retained.unwrap_or((&fallback, Vec::new()));
    let mut command_log =
        super::status::waterfall::matching::Log::read(&command.command_log, decoded.time())?;
    let event_end = original
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|event| crate::session::trace::event_time(&event))
        .max()
        .and_then(|nanos| {
            chrono::DateTime::from_timestamp(
                (nanos / 1_000_000_000) as i64,
                (nanos % 1_000_000_000) as u32,
            )
        })
        .map(|time| time.fixed_offset());
    let ended = command
        .record
        .ended_at
        .as_deref()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .or_else(|| command_log.last_time())
        .max(event_end)
        .unwrap_or(decoded.time());
    for path in [
        source.to_owned(),
        models_path,
        command.command_log.clone(),
        command.record_path.clone(),
    ] {
        // A missing sidecar/record is also a fact that cleanup must recheck.
        let relative = path
            .strip_prefix(root)
            .context("Session dependency is outside the logs folder")?
            .to_owned();
        session.inputs.insert(relative, fingerprint(&path)?);
    }
    let before = readings::read(source, source, command, &relays, harness, &models)?;
    let temporary = tempfile::tempdir_in(source.parent().unwrap())?;
    let output = temporary.path().join(source.file_name().unwrap());
    let choice = Choice {
        harness,
        model: models["requested"]
            .as_str()
            .map(str::to_owned)
            .or(command.record.model.clone()),
        effort: models["effort"]
            .as_str()
            .map(str::to_owned)
            .or(command.record.effort.clone()),
        ..Choice::default()
    };
    let mut context = trace::Context::of_session(&command.command_log);
    context.repository = command.record.repository.clone();
    if has_record {
        // Recover the outer Command's identity, then this Run's stage ancestry.
        context = trace::Context::of_session(&command.command_log).for_record(&command.record);
        let kind = decoded.session_kind.unwrap();
        if let Some(stage) = command
            .record
            .stages
            .iter()
            .find(|stage| stage.session_log.as_deref() == Some(source) || stage.name == kind)
        {
            context.stage = context.stage_id(stage);
        }
    }
    let mut trace = Trace::historical(
        source,
        &output,
        decoded.session_kind.unwrap(),
        &choice,
        context,
        readings::nanos(decoded.time()),
    )?;
    let timing = command
        .record
        .stage_times(None)
        .find(|timing| timing.stage.session_log.as_deref() == Some(source));
    let began = timing
        .as_ref()
        .and_then(|t| t.began)
        .unwrap_or(decoded.time());
    let until = timing
        .and_then(|t| t.ended)
        .unwrap_or(ended)
        .max(event_end.unwrap_or(decoded.time()));
    let mut stream = if harness == Harness::Codex {
        crate::harness::codex::historical(began..=until)
    } else {
        harness
            .adapter()
            .interpretation(Path::new(""), "")
            .without_retained()
    };
    let mut cursor = 0;
    let mut estimated = decoded.time();
    let mut seen = Vec::new();
    for raw in original.lines() {
        let facts = stream.decode(raw);
        let envelope: serde_json::Value = serde_json::from_str(raw).unwrap_or_default();
        for (index, label) in facts.progress.iter().enumerate() {
            let file = envelope["item"]["changes"][index]["path"].as_str();
            if let Some(time) = command_log.estimate(
                label,
                file,
                decoded.session_kind.unwrap(),
                &relays,
                (Some(decoded.time()), Some(ended)),
                &mut cursor,
            ) {
                estimated = time;
            }
        }
        trace.record_at(raw, &facts.events, readings::nanos(estimated))?;
        seen.extend(facts.events);
    }
    let completion = stream.finish(Ok(std::os::unix::process::ExitStatusExt::from_raw(0)));
    let mut amendments = String::new();
    if let Some(report) = completion.report {
        for event in report.events.iter().filter(|event| {
            !seen.contains(event)
                && matches!(event,
            crate::harness::interpretation::event::Event::ToolCallEnded { result, .. }
            if result["source"] == "codex_session_file")
        }) {
            let raw = serde_json::to_string(event)?;
            trace.record_at(&raw, std::slice::from_ref(event), readings::nanos(ended))?;
            amendments.push_str(&format!("{raw}\n"));
            session.codex_diffs += 1;
        }
    }
    trace.finish_at(None, readings::nanos(ended))?;
    drop(trace);
    let after = readings::read(
        &output.with_extension("logs.jsonl"),
        source,
        command,
        &relays,
        harness,
        &models,
    )?;
    // Recovered Codex diffs are intentional additions. Compare against the
    // original rendered with exactly those amendments, and disclose their count.
    let mut expected = before;
    if !amendments.is_empty() {
        expected.conversation = crate::console::conversation::render(
            &format!("{}\n{amendments}", original.trim_end()),
            super::catalog::Kind::Session,
        );
    }
    session.readers = expected.compare(&after);
    ensure!(
        session.readers.values().all(|same| *same),
        "round-trip reader difference"
    );
    unchanged(root, &session.inputs)?;
    for ext in ["logs.jsonl", "spans.jsonl"] {
        let destination = source.with_extension(ext);
        // link is an atomic no-clobber publication on the same filesystem.
        fs::hard_link(output.with_extension(ext), &destination)?;
        session.inputs.insert(
            destination.strip_prefix(root)?.to_owned(),
            fingerprint(&destination)?,
        );
    }
    Ok(())
}

fn fingerprint(path: &Path) -> Result<Option<String>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(format!("{:x}", Sha256::digest(bytes)))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

fn confined(root: &Path, relative: &Path) -> Result<PathBuf> {
    ensure!(
        relative
            .components()
            .all(|part| matches!(part, Component::Normal(_))),
        "invalid report path"
    );
    let path = root.join(relative);
    let parent = path.parent().context("missing parent")?.canonicalize()?;
    ensure!(
        parent.starts_with(root),
        "report path escaped the logs folder"
    );
    ensure!(
        !path
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink()),
        "report path is a symlink"
    );
    Ok(path)
}

fn unchanged(root: &Path, inputs: &BTreeMap<PathBuf, Option<String>>) -> Result<()> {
    ensure!(!inputs.is_empty(), "conversion has no receipt");
    for (relative, expected) in inputs {
        let path = confined(root, relative)?;
        ensure!(
            fingerprint(&path)? == *expected,
            "{} changed since conversion",
            relative.display()
        );
    }
    Ok(())
}

fn save(path: &Path, report: &Report) -> Result<()> {
    use std::io::Write;
    let mut temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    temporary.write_all(&serde_json::to_vec_pretty(report)?)?;
    temporary.persist(path)?;
    Ok(())
}
