//! Current-layout log inventory shared by the CLI and Console. IDs name exact
//! retained files, including older Commands with the same issue or start stamp.
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Local;
use serde::Serialize;

use super::{Command, Entry, Observation, Reader, Snapshot};
use crate::config::UserConfig;
use crate::logs::accounts::Source;
use crate::logs::owner;

#[derive(Clone, Copy, Serialize)]
pub(crate) enum Kind {
    #[serde(rename = "Command log")]
    Command,
    #[serde(rename = "Session trace")]
    Session,
}

impl std::fmt::Display for Kind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Command => "Command log",
            Self::Session => "Session trace",
        })
    }
}

#[derive(Serialize)]
pub(crate) struct Log {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    pub id: String,
    pub name: String,
    pub repository: String,
    pub kind: Kind,
    pub live: bool,
    pub command: String,
    pub owners: Vec<String>,
    #[serde(skip)]
    pub path: PathBuf,
    #[serde(skip)]
    pub selector: String,
    #[serde(skip)]
    pub security_details: bool,
    #[serde(skip)]
    pub reader: Reader,
}

#[derive(Default)]
struct Observed {
    live: bool,
    security_details: bool,
    command: String,
    owners: Vec<String>,
    selector: String,
    reader: Option<Reader>,
}

pub(crate) fn id(path: &Path) -> String {
    path.as_os_str()
        .as_encoded_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The filename predicate shared by listing and each inventory-created read.
fn listed_name(path: &Path) -> bool {
    owner::inventory_session(path)
        && matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("log" | "jsonl")
        )
}

pub(crate) const UNLISTED: &str = "log is not in the current inventory";

/// Check only the selected entries, as `load` sees them: the root and
/// literal `commands` path may be aliases, but owner, repository, kind and
/// file must be real entries rather than symlinks. Another account's file
/// must also stay, once resolved, under the accepted directory. That path is
/// the canonical directory discovery stored, so it is not resolved again.
pub(super) fn still_listed(root: &Path, path: &Path, other_account: bool) -> bool {
    fn directory(path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|entry| entry.is_dir())
    }
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let [
        Component::Normal(owner),
        Component::Normal(repository),
        Component::Normal(commands),
        Component::Normal(kind),
        Component::Normal(_),
    ] = relative.components().collect::<Vec<_>>()[..]
    else {
        return false;
    };
    let owner = root.join(owner);
    let repository = owner.join(repository);
    commands == "commands"
        && directory(&owner)
        && directory(&repository)
        && directory(&repository.join(commands).join(kind))
        && std::fs::symlink_metadata(path).is_ok_and(|entry| entry.is_file())
        && listed_name(path)
        && (!other_account || super::inventory::stays_under(root, path))
}

fn load_account_snapshot(source: &Source) -> Result<Option<Snapshot>> {
    match Snapshot::load_using(super::files_for(source), &source.root, Local::now()) {
        Ok(snapshot) => Ok(Some(snapshot)),
        Err(_) if source.account.is_some() => Ok(None),
        Err(error) => Err(error),
    }
}

fn for_each_account_catalog<T>(
    config: &UserConfig,
    mut visit: impl FnMut(&Source, &Snapshot, Vec<Log>) -> Result<Option<T>>,
) -> Result<Option<T>> {
    for source in crate::logs::accounts::discover(config).logs {
        let Some(snapshot) = load_account_snapshot(&source)? else {
            continue;
        };
        let listed = match list(&snapshot, &source, None) {
            Ok(logs) => logs,
            Err(_) if source.account.is_some() => continue,
            Err(error) => return Err(error),
        };
        if let Some(found) = visit(&source, &snapshot, listed)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

pub(crate) fn load(config: &UserConfig, selected_owner: Option<&str>) -> Result<Vec<Log>> {
    let mut logs = Vec::new();
    for_each_account_catalog(config, |_, _, listed| {
        logs.extend(listed);
        Ok(None::<()>)
    })?;
    if let Some(owner) = selected_owner {
        logs.retain(|log| log.owners.iter().any(|id| id == owner));
    }
    logs.sort_by(|a, b| (&a.account, &a.name).cmp(&(&b.account, &b.name)));
    Ok(logs)
}

/// List the physically discovered files of the Snapshot's own capture.
/// Session membership supplies ownership, never listing eligibility.
pub(super) fn list(
    snapshot: &Snapshot,
    source: &Source,
    selected_owner: Option<&str>,
) -> Result<Vec<Log>> {
    let root = &source.root;
    // A child Run keeps its own Sessions, while sharing its Command's log.
    let mut observed = BTreeMap::new();
    fn collect(
        command: &Command,
        source: &Source,
        ancestors: &[String],
        security_details: bool,
        observed: &mut BTreeMap<PathBuf, Observed>,
    ) -> Result<()> {
        let security_details = security_details || command.record.kind == "Security run";
        let mut owners = ancestors.to_vec();
        owners.push(source.id(&command.record_path)?);
        let subject = command
            .record
            .issue
            .as_ref()
            .map(|issue| format!("{}#{}", command.record.repository, issue.number))
            .unwrap_or_else(|| command.record.name.clone());
        let observation = Observation::Record(command);
        let selector = observation.selector().context("Command has no selector")?;
        observed
            .entry(command.command_log.clone())
            .and_modify(|entry| entry.security_details |= security_details)
            .or_insert(Observed {
                live: command.state == "live",
                security_details,
                command: subject.clone(),
                owners: owners.clone(),
                selector: selector.clone(),
                reader: Some(Reader::captured(
                    command.command_log.clone(),
                    Some(command),
                    false,
                )),
            });
        let current = command
            .record
            .session_log
            .as_deref()
            .map(crate::logs::session::path);
        for session in Observation::Record(command).sessions()? {
            let session = crate::logs::session::path(&session);
            let live = command.state == "live" && current.as_ref() == Some(&session);
            observed.insert(
                session.clone(),
                Observed {
                    live,
                    security_details,
                    command: subject.clone(),
                    owners: owners.clone(),
                    selector: selector.clone(),
                    reader: Some(Reader::captured(session, Some(command), true)),
                },
            );
        }
        for child in command.dispatched.iter().chain(&command.children) {
            collect(child, source, &owners, security_details, observed)?;
        }
        Ok(())
    }
    for entry in &snapshot.commands {
        match entry {
            Entry::Record(command) => collect(command, source, &[], false, &mut observed)?,
            Entry::History(command) if command.kind == "Security run" => {
                for path in command.command_log.iter().chain(&command.session_logs) {
                    observed.entry(path.clone()).or_default().security_details = true;
                }
            }
            Entry::History(_) => {}
        }
    }
    let mut logs = Vec::new();
    for repo in snapshot.inventory.repositories() {
        let repository = repo.name();
        for folder in &repo.folders {
            for path in &folder.files {
                if !listed_name(path) {
                    continue;
                }
                let path = if path.extension().is_some_and(|ext| ext == "jsonl") {
                    crate::logs::session::path(path)
                } else {
                    path.clone()
                };
                if source.account.is_some() && std::fs::File::open(&path).is_err() {
                    continue;
                }
                let relative = path.strip_prefix(root)?;
                let Observed {
                    live,
                    security_details,
                    command,
                    mut owners,
                    selector,
                    reader,
                } = observed.remove(&path).unwrap_or_default();
                if let Some(decoded) = owner::decode(&path) {
                    owners.push(source.id(&decoded.command_log(&path))?);
                }
                logs.push(Log {
                    security_details: security_details
                        || folder
                            .path
                            .file_name()
                            .is_some_and(|name| name == crate::logs::CommandKind::Secure.folder())
                        || owner::security_session(&path),
                    account: source.account.clone(),
                    reader: reader
                        .unwrap_or_else(|| Reader::captured(path.clone(), None, false))
                        .within(root, source.account.is_some()),
                    id: source.id(&path)?,
                    name: relative.to_string_lossy().into_owned(),
                    repository: repository.clone(),
                    kind: if path.extension().is_some_and(|ext| ext == "jsonl") {
                        Kind::Session
                    } else {
                        Kind::Command
                    },
                    live,
                    command,
                    owners,
                    selector: if selector.is_empty() {
                        path.to_string_lossy().into_owned()
                    } else {
                        selector
                    },
                    path,
                });
            }
        }
    }
    if let Some(selected) = selected_owner {
        logs.retain(|log| log.owners.iter().any(|owner| owner == selected));
    }
    logs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(logs)
}

pub(crate) struct Opened {
    pub log: Log,
    pub target: Option<crate::logs::status::GitHubTarget>,
}

#[derive(Clone, Copy)]
pub(crate) enum OpenRequest<'a> {
    Id(&'a str),
    Command {
        command: &'a str,
        session: Option<&'a str>,
    },
}

pub(crate) enum OpenOutcome {
    Opened(Box<Opened>),
    Forbidden,
    NotFound,
}

pub(crate) fn find(config: &UserConfig, selected: &str) -> Result<Log> {
    match open(config, OpenRequest::Id(selected), true, false)? {
        OpenOutcome::Opened(opened) => Ok(opened.log),
        OpenOutcome::Forbidden | OpenOutcome::NotFound => anyhow::bail!(UNLISTED),
    }
}

fn open_listed_log(
    source: &Source,
    snapshot: &Snapshot,
    mut log: Log,
    selected_command: Option<&str>,
    allow_security: bool,
    include_target: bool,
) -> OpenOutcome {
    if !allow_security && log.security_details {
        return OpenOutcome::Forbidden;
    }
    let observation = find_observation(snapshot, &log.path, selected_command);
    if let Some(selector) = observation.and_then(Observation::selector) {
        log.selector = selector;
    }
    let target = include_target
        .then(|| {
            let kept = source
                .account
                .is_none()
                .then(|| crate::logs::github::load(&source.root));
            observation.and_then(|obs| {
                crate::logs::status::GitHubTarget::from_observation(obs, kept.as_ref())
            })
        })
        .flatten();
    OpenOutcome::Opened(Box::new(Opened { log, target }))
}

fn open_command(
    config: &UserConfig,
    command: &str,
    session: Option<&str>,
    allow_security: bool,
    include_target: bool,
) -> Result<OpenOutcome> {
    let mut accounts = Vec::new();
    for source in crate::logs::accounts::discover(config).logs {
        let Some(mut snapshot) = load_account_snapshot(&source)? else {
            continue;
        };
        if source.account.is_some() {
            super::adopt_account(&mut snapshot, &source.account);
        }
        accounts.push((source, snapshot));
    }
    let Ok(resolved) = super::selection::resolve_iter(
        accounts.iter().flat_map(|(_, snapshot)| snapshot.entries()),
        command,
    ) else {
        return Ok(OpenOutcome::NotFound);
    };
    let Ok(path) = crate::logs::status::inspect::selected_log(resolved.observation, session) else {
        return Ok(OpenOutcome::NotFound);
    };
    let owner_account = resolved.observation.account().map(str::to_owned);
    let Some((source, snapshot)) = accounts
        .iter()
        .find(|(source, _)| source.account == owner_account)
    else {
        return Ok(OpenOutcome::NotFound);
    };
    let listed = match list(snapshot, source, None) {
        Ok(listed) => listed,
        Err(_) if source.account.is_some() => return Ok(OpenOutcome::NotFound),
        Err(error) => return Err(error),
    };
    let Some(log) = listed.into_iter().find(|log| log.path == path) else {
        return Ok(OpenOutcome::NotFound);
    };
    Ok(open_listed_log(
        source,
        snapshot,
        log,
        Some(command),
        allow_security,
        include_target,
    ))
}

/// Open one listed log by ID or Command selector, enforcing Security scope and
/// optionally computing its GitHub target from the same Snapshot capture.
pub(crate) fn open(
    config: &UserConfig,
    request: OpenRequest<'_>,
    allow_security: bool,
    include_target: bool,
) -> Result<OpenOutcome> {
    match request {
        OpenRequest::Id(id) => for_each_account_catalog(config, |source, snapshot, listed| {
            let Some(log) = listed.into_iter().find(|log| log.id == id) else {
                return Ok(None);
            };
            Ok(Some(open_listed_log(
                source,
                snapshot,
                log,
                None,
                allow_security,
                include_target,
            )))
        })
        .map(|found| found.unwrap_or(OpenOutcome::NotFound)),
        OpenRequest::Command { command, session } => {
            open_command(config, command, session, allow_security, include_target)
        }
    }
}

fn observation_matches_log(observation: Observation<'_>, log_path: &Path) -> bool {
    observation.command_log() == Some(log_path)
        || observation
            .sessions()
            .is_ok_and(|sessions| sessions.iter().any(|path| path == log_path))
}

fn find_nested_session<'a>(command: &'a Command, log_path: &Path) -> Option<Observation<'a>> {
    for child in command.children.iter().chain(&command.dispatched) {
        if let Some(found) = find_nested_session(child, log_path) {
            return Some(found);
        }
        let obs = Observation::Record(child);
        let child_owns_different_log =
            child.command_log == log_path && child.command_log != command.command_log;
        let child_owns_session = log_path != child.command_log
            && obs
                .sessions()
                .is_ok_and(|sessions| sessions.iter().any(|path| path == log_path));
        if child_owns_different_log || child_owns_session {
            return Some(obs);
        }
    }
    None
}

fn find_command_observation<'a>(command: &'a Command, log_path: &Path) -> Option<Observation<'a>> {
    if let Some(found) = find_nested_session(command, log_path) {
        return Some(found);
    }
    let obs = Observation::Record(command);
    if observation_matches_log(obs, log_path) {
        return Some(obs);
    }
    for child in command.children.iter().chain(&command.dispatched) {
        if let Some(found) = find_command_observation(child, log_path) {
            return Some(found);
        }
    }
    None
}

fn find_observation<'a>(
    snapshot: &'a Snapshot,
    log_path: &Path,
    selected_command: Option<&str>,
) -> Option<Observation<'a>> {
    if let Some(selector) = selected_command
        && let Ok(resolved) = snapshot.resolve(selector)
        && observation_matches_log(resolved.observation, log_path)
    {
        return Some(resolved.observation);
    }
    for entry in snapshot.entries() {
        if let Entry::Record(command) = entry
            && let Some(found) = find_command_observation(command, log_path)
        {
            return Some(found);
        }
    }
    for entry in snapshot.entries() {
        if let Entry::History(historical) = entry {
            let obs = Observation::History(historical);
            if observation_matches_log(obs, log_path) {
                return Some(obs);
            }
        }
    }
    None
}

pub(crate) fn show(json: bool, owner: Option<&str>) -> Result<()> {
    let logs = load(&UserConfig::load()?, owner)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({"version": 1, "logs": logs}))?
        );
    } else {
        for log in logs {
            if let Some(account) = &log.account {
                print!("[{account}] ");
            }
            println!(
                "{} · {}{} · {} · {}",
                log.id,
                log.kind,
                if log.live { " · live" } else { "" },
                log.command,
                log.name
            );
        }
    }
    Ok(())
}

pub(crate) fn contents(reader: &Reader) -> Result<serde_json::Value> {
    let bytes = reader.read(0, super::Framing::Raw, usize::MAX)?;
    Ok(serde_json::json!({"version": 1, "contents": String::from_utf8_lossy(&bytes.bytes)}))
}

pub(crate) fn print_file(selected: &str, follow: bool, json: bool) -> Result<()> {
    let config = UserConfig::load()?;
    let log = find(&config, selected)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&contents(&log.reader)?)?);
        return Ok(());
    }
    crate::logs::status::inspect::stream_log(&log.reader, follow)
}
