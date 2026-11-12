//! Render one Command and stream its selected log bytes.

use std::io::Write;

use anyhow::{Context, Result};
use chrono::{DateTime, Local};

use crate::config::UserConfig;
use crate::logs::reading::{Command, Framing, Observation, Reader, Snapshot};

pub(crate) fn log_path(
    config: &UserConfig,
    input: &str,
    session: Option<&str>,
) -> Result<std::path::PathBuf> {
    let snapshot = Snapshot::for_user(config, Local::now())?;
    let command = snapshot.resolve(input)?.observation;
    selected_log(command, session)
}

fn selected_log(command: Observation<'_>, session: Option<&str>) -> Result<std::path::PathBuf> {
    match session {
        Some(session) => command.session(session),
        None => command
            .command_log()
            .context("this Command has no Command log")
            .map(std::path::Path::to_path_buf),
    }
}

pub(crate) fn url(route: &str, command: &str, session: Option<&std::path::Path>) -> String {
    let mut url = reqwest::Url::parse("http://localhost").expect("fixed URL");
    url.set_path(route);
    url.query_pairs_mut().append_pair("command", command);
    if let Some(session) = session {
        url.query_pairs_mut()
            .append_pair("session", &session.to_string_lossy());
    }
    format!("{}?{}", url.path(), url.query().unwrap_or_default())
}

pub(crate) fn machine_url(route: &str, machine: &str) -> String {
    let mut url = reqwest::Url::parse("http://localhost").expect("fixed URL");
    url = url.join(route).expect("Console route");
    url.query_pairs_mut().append_pair("machine", machine);
    format!("{}?{}", url.path(), url.query().unwrap_or_default())
}

fn shown_heading(command: &Command) -> String {
    if let Some(name) = command.record.process_label() {
        return format!("Process {name}");
    }
    format!(
        "{} {}",
        crate::logs::display_name(&command.record.kind),
        command.record.name
    )
}

/// One Process Step as `show` and the Console details print it.
pub(crate) struct ShownStep {
    pub message: String,
    pub result: Option<std::path::PathBuf>,
}

/// Each Process Step's Command log line, and the result file when the Step wrote one.
/// An Other-account Command log is read only when its canonical path stays under
/// `accepted`. The serving account passes no directory.
pub(crate) fn process_steps(
    command_log: &std::path::Path,
    accepted: Option<&std::path::Path>,
) -> Vec<ShownStep> {
    let Some(path) = crate::logs::reading::steps_command_log(command_log, accepted) else {
        return Vec::new();
    };
    let Ok(log) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let results = command_log.with_extension("steps");
    log.lines()
        .filter_map(step_message)
        .map(|message| {
            let result = step_name(message)
                .map(|name| results.join(format!("{name}.txt")))
                .filter(|path| path.is_file());
            ShownStep {
                message: message.to_string(),
                result,
            }
        })
        .collect()
}

fn print_steps(command: &Command, indent: &str) {
    if command.record.kind != "Process" {
        return;
    }
    for step in process_steps(&command.command_log, command.accepted_log_directory()) {
        println!("{indent}{}", step.message);
        if let Some(path) = &step.result {
            println!("{indent}result: {}", path.display());
        }
    }
}

fn step_message(line: &str) -> Option<&str> {
    use crate::progress::ChildLine;
    match ChildLine::of(line) {
        ChildLine::Own(message) if message.starts_with("step ") => Some(message),
        ChildLine::Own(_) | ChildLine::Relayed | ChildLine::Continuation => None,
    }
}

/// New ambiguous names are JSON strings. Keep reading earlier plain names
/// for Command logs written before quoted Step identities were introduced.
fn step_name(message: &str) -> Option<String> {
    let rest = message.strip_prefix("step ")?;
    if rest.starts_with('"') {
        let mut names = serde_json::Deserializer::from_str(rest).into_iter::<String>();
        let name = names.next()?.ok()?;
        return rest[names.byte_offset()..]
            .starts_with(": ")
            .then_some(name);
    }
    let colon = duration_separator(rest)?;
    let name = &rest[..colon];
    (!name.is_empty()).then(|| name.to_string())
}

fn duration_separator(rest: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(at) = rest[from..].find(": ") {
        let absolute = from + at;
        if starts_with_duration(&rest[absolute + 2..]) {
            return Some(absolute);
        }
        from = absolute + 2;
    }
    None
}

fn starts_with_duration(text: &str) -> bool {
    let mut words = text.split(' ');
    let Some(first) = words.next() else {
        return false;
    };
    if let Some(hours) = first.strip_suffix('h') {
        return is_uint(hours)
            && words
                .next()
                .is_some_and(|word| word.strip_suffix('m').is_some_and(is_uint))
            && words
                .next()
                .is_some_and(|word| word.strip_suffix('s').is_some_and(is_uint));
    }
    if let Some(minutes) = first.strip_suffix('m') {
        return is_uint(minutes)
            && words
                .next()
                .is_some_and(|word| word.strip_suffix('s').is_some_and(is_uint));
    }
    first.strip_suffix('s').is_some_and(is_uint)
}

fn is_uint(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

fn print(command: Observation<'_>, now: DateTime<Local>) -> Result<()> {
    match command {
        Observation::Record(command) => print_details(command, "", now)?,
        Observation::History(command) => super::print_history(command),
    }
    if let Some(path) = command.command_log() {
        println!("Command log: {}", path.display());
    }
    if matches!(command, Observation::History(_)) {
        for path in command.sessions()? {
            println!("Session trace: {}", path.display());
        }
    }
    Ok(())
}

fn print_details(command: &Command, indent: &str, now: DateTime<Local>) -> Result<()> {
    if let Some(account) = &command.account {
        println!("{indent}[{account}]");
    }
    println!(
        "{indent}{} · {}",
        shown_heading(command),
        super::stage_text(command)
    );
    if let Some(issue) = &command.record.issue {
        println!(
            "{indent}{}",
            issue.title.as_deref().unwrap_or("(title unavailable)")
        );
    }
    for timing in command
        .record
        .stage_times((command.state == "live").then_some(now.fixed_offset()))
    {
        let stage = timing.stage;
        let usual = command
            .usual_ranges
            .get(&stage.name)
            .map(|range| format!(" · usual {}", range.text()))
            .unwrap_or_default();
        println!(
            "{indent}  {} · {} · {}{usual}",
            crate::logs::display_name(&stage.name),
            stage.began_at,
            super::stage_time(timing.seconds(), stage.suspended_seconds),
        );
    }
    for path in command.sessions()? {
        println!("{indent}Session trace: {}", path.display());
    }
    if let Some(event) = command.latest_event() {
        println!("{indent}{event}");
    }
    print_steps(command, indent);
    super::print_pause(&command.record, indent);
    super::print_tickets(&command.record, &format!("{indent}  "));
    for child in command.dispatched.iter().chain(&command.children) {
        print_details(child, &format!("{indent}  "), now)?;
    }
    Ok(())
}

pub fn show(input: &str, json: bool) -> Result<()> {
    let config = UserConfig::load()?;
    let snapshot = Snapshot::details_for_user(&config, Local::now(), input)?;
    let command = snapshot.resolve(input)?.observation;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&envelope(details(&config, &snapshot, command)?))?
        );
    } else {
        print(command, snapshot.observed_at())?;
    }
    Ok(())
}

pub(in crate::logs) fn details(
    config: &UserConfig,
    snapshot: &Snapshot,
    command: Observation<'_>,
) -> Result<serde_json::Value> {
    let kept = crate::logs::github::load(&config.logs_dir);
    let github = if let Observation::Record(record) = command
        && let Some(row) = command.selector()
    {
        let record = &record.record;
        let urls = record
            .pull_request
            .as_deref()
            .into_iter()
            .chain(record.issue.as_ref().map(|issue| issue.url.as_str()))
            .collect();
        kept.command_answer(&row, record.pull_request.as_deref(), urls)
    } else {
        Default::default()
    };
    let mut value = crate::logs::metrics::details(snapshot, command)?;
    value
        .as_object_mut()
        .expect("Command details object")
        .extend(
            serde_json::to_value(github)?
                .as_object()
                .expect("GitHub answer object")
                .clone(),
        );
    Ok(value)
}

pub(crate) fn envelope(command: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"version": 1, "hostname": crate::host::name().unwrap_or_else(|| "unknown".into()), "command": command})
}

pub(crate) fn read_log(
    config: &UserConfig,
    input: &str,
    session: Option<&str>,
    tail: bool,
) -> Result<serde_json::Value> {
    let snapshot = Snapshot::for_user(config, Local::now())?;
    let selection = snapshot.resolve(input)?;
    let path = selected_log(selection.observation, session)?;
    let reader = selection.reader(path);
    if tail {
        let bytes = reader.tail(4 * 1024)?;
        return Ok(serde_json::json!({"version": 1, "contents": String::from_utf8_lossy(&bytes)}));
    }
    super::super::catalog::contents(&reader)
}

pub fn logs(
    input: &str,
    session: Option<&str>,
    follow: bool,
    json: bool,
    tail: bool,
) -> Result<()> {
    let config = UserConfig::load()?;
    if tail {
        let value = read_log(&config, input, session, true)?;
        if json {
            println!("{}", serde_json::to_string_pretty(&value)?);
        } else {
            std::io::stdout()
                .write_all(value["contents"].as_str().unwrap_or_default().as_bytes())?;
        }
        return Ok(());
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&read_log(&config, input, session, false)?)?
        );
        return Ok(());
    }
    let snapshot = Snapshot::for_user(&config, Local::now())?;
    let selection = snapshot.resolve(input)?;
    let command = selection.observation;
    let path = selected_log(command, session)?;
    stream_log(&selection.reader(path), follow)
}

pub(in crate::logs) fn stream_log(reader: &Reader, follow: bool) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    let mut offset = 0;
    loop {
        let batch = reader.read(offset, Framing::Raw, 256 * 1024)?;
        stdout.write_all(&batch.bytes)?;
        stdout.flush()?;
        if batch.done || (!follow && batch.bytes.len() < 256 * 1024) {
            return Ok(());
        }
        let progressed = batch.offset != offset;
        offset = batch.offset;
        if !progressed {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}
