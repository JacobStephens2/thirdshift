//! Shared naming, Session inventory and captured selection ownership.

use std::collections::BTreeSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use super::{Command, Entry};
use crate::issue::IssueUrl;
use crate::logs::effects::Outside;
use crate::logs::owner;

enum Name {
    Issue { repository: String, number: u64 },
    Pass(String),
}

impl Name {
    fn parse(name: &str) -> Result<Self> {
        if name.starts_with("https://") {
            let issue = IssueUrl::parse(name)?;
            return Ok(Self::Issue {
                repository: issue.repo_slug(),
                number: issue.number,
            });
        }
        if let Some((repository, number)) = name.split_once('#') {
            let number = number
                .parse::<std::num::NonZeroU64>()
                .with_context(|| format!("invalid Command name: {name}"))?
                .get();
            if !repository.is_empty() {
                return Ok(Self::Issue {
                    repository: repository.into(),
                    number,
                });
            }
        }
        if ["architect-", "pickup-", "secure-"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
        {
            return Ok(Self::Pass(name.into()));
        }
        bail!("invalid Command name: {name}; use repo#number, an Issue URL or kind-stamp")
    }

    fn matches_repo(&self, repo: &str) -> bool {
        match self {
            Self::Issue { repository, .. } => {
                repo.eq_ignore_ascii_case(repository)
                    || (!repository.contains('/')
                        && repo
                            .rsplit('/')
                            .next()
                            .is_some_and(|name| name.eq_ignore_ascii_case(repository)))
            }
            Self::Pass(_) => true,
        }
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(untagged)]
pub(in crate::logs) enum Observation<'a> {
    Record(&'a Command),
    History(&'a super::history::Command),
}

impl<'a> Observation<'a> {
    fn repository(self) -> &'a str {
        match self {
            Self::Record(command) => &command.record.repository,
            Self::History(command) => &command.repository,
        }
    }

    fn matches(self, name: &Name) -> bool {
        if !name.matches_repo(self.repository()) {
            return false;
        }
        match name {
            Name::Issue { number, .. } => {
                let issue = match self {
                    Self::Record(command) => command.record.issue.as_ref(),
                    Self::History(command) => command.issue.as_ref(),
                };
                issue.is_some_and(|issue| issue.number == *number)
            }
            Name::Pass(name) => match self {
                Self::Record(command) => command.record.name == *name,
                Self::History(command) => command.command_log.as_ref().is_some_and(|path| {
                    let Some(kind) = pass_prefix(&command.kind) else {
                        return false;
                    };
                    super::command_stamp(path)
                        .is_some_and(|(stamp, _)| *name == format!("{kind}-{stamp}"))
                }),
            },
        }
    }

    fn ordering(
        self,
    ) -> (
        bool,
        std::cmp::Reverse<Option<chrono::DateTime<chrono::FixedOffset>>>,
    ) {
        let (state, started) = match self {
            Self::Record(command) => (command.state, command.record.started_at.as_str()),
            Self::History(command) => (command.state, command.started_at.as_str()),
        };
        (
            state != "live",
            std::cmp::Reverse(chrono::DateTime::parse_from_rfc3339(started).ok()),
        )
    }

    pub(in crate::logs) fn command_log(self) -> Option<&'a Path> {
        match self {
            Self::Record(command) => Some(&command.command_log),
            Self::History(command) => command.command_log.as_deref(),
        }
    }

    pub(in crate::logs) fn sessions(self) -> Result<Vec<PathBuf>> {
        let (mut paths, issue, kind) = match self {
            Self::Record(command) => {
                let mut paths = Vec::new();
                for path in command.record.session_references() {
                    if !paths.contains(path) {
                        paths.push(path.clone());
                    }
                }
                (
                    paths,
                    command.record.issue.as_ref(),
                    command.record.kind.as_str(),
                )
            }
            Self::History(command) => (
                command.session_logs.clone(),
                command.issue.as_ref(),
                command.kind.as_str(),
            ),
        };
        // A stage can cover several sessions (for example Architecture review
        // and its Resume), so stage references alone are not a log inventory.
        if let Some(log) = self.command_log()
            && owner::decode(log).is_some()
        {
            let prefix = issue
                .map(|issue| issue.number.to_string())
                .or_else(|| pass_prefix(kind).map(String::from))
                .context("cannot name Session logs for this Command kind")?;
            for entry in super::entries(log.parent().context("Command log has no directory")?)? {
                let path = entry.path();
                if entry.file_type()?.is_file()
                    && owner::decode(&path).is_some_and(|decoded| {
                        decoded.session_kind.is_some() && decoded.subject == prefix
                    })
                    && owner::session_matches(&path, log)
                    && !paths.contains(&path)
                {
                    paths.push(path);
                }
            }
        }
        let mut dated = paths
            .into_iter()
            .map(|path| {
                let modified = std::fs::metadata(&path)
                    .and_then(|metadata| metadata.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                (modified, path)
            })
            .collect::<Vec<_>>();
        dated.sort();
        Ok(dated.into_iter().map(|(_, path)| path).collect())
    }

    pub(in crate::logs) fn session(self, kind: &str) -> Result<PathBuf> {
        self.sessions()?
            .into_iter()
            .rev()
            .find(|path| {
                let Some(session) = owner::decode(path).and_then(|decoded| decoded.session_kind)
                else {
                    return false;
                };
                let original = session.strip_suffix("-resume").unwrap_or(session);
                session == kind
                    || original == kind
                    || (kind == "repair"
                        && original
                            .strip_prefix("repair-")
                            .is_some_and(|number| number.parse::<u32>().is_ok()))
            })
            .with_context(|| format!("no {kind} Session log for this Command"))
    }
}

fn pass_prefix(kind: &str) -> Option<&'static str> {
    match kind {
        "Architect run" => Some("architect"),
        "Pickup run" => Some("pickup"),
        "Security run" => Some("secure"),
        _ => None,
    }
}

/// A selected Run and its captured outer process/record. Child refusal is an
/// observation of the enclosing Run/Spec, independent of signal authority.
#[derive(Clone, Copy)]
pub(in crate::logs) struct Selection<'a> {
    pub(in crate::logs) observation: Observation<'a>,
    pub(in crate::logs) owner: Option<&'a Command>,
    pub(in crate::logs) child_owner: Option<&'a Command>,
}

impl Selection<'_> {
    pub(in crate::logs) fn stop_target(self, input: &str) -> Result<PathBuf> {
        if let Some(owner) = self.child_owner {
            bail!(
                "cannot stop {input}: child Run; stop {} {} instead",
                owner.record.kind,
                owner.record.name
            );
        }
        match self.observation {
            Observation::Record(command) if command.state == "live" => Ok(self
                .owner
                .context("Command has no owner")?
                .record_path
                .clone()),
            Observation::Record(command) => bail!(
                "cannot stop {input}: Command {}{}",
                command.state,
                command
                    .record
                    .outcome
                    .as_ref()
                    .map(|outcome| format!(": {outcome}"))
                    .unwrap_or_default()
            ),
            Observation::History(command) => {
                bail!("cannot stop {input}: Command {}", command.state)
            }
        }
    }

    /// Outcomes precede final output. Recheck the captured process, not state.
    pub(in crate::logs) fn process_exited(self) -> bool {
        self.owner.is_none_or(|command| {
            let record = &command.record;
            record.process_start.is_empty()
                || crate::logs::effects::OnMachine
                    .process_start(record.process_id)
                    .as_deref()
                    != Some(&record.process_start)
        })
    }
}

fn records<'a>(
    command: &'a Command,
    outer: &'a Command,
    enclosing: &'a Command,
    found: &mut Vec<Selection<'a>>,
) {
    found.push(Selection {
        observation: Observation::Record(command),
        owner: Some(outer),
        child_owner: command.record.parent_record.as_ref().map(|_| enclosing),
    });
    for dispatch in &command.dispatched {
        records(dispatch, outer, dispatch, found);
    }
    for child in &command.children {
        records(child, outer, enclosing, found);
    }
}

pub(super) fn resolve<'a>(entries: &'a [Entry], input: &str) -> Result<Selection<'a>> {
    let name = Name::parse(input)?;
    let mut candidates = Vec::new();
    for entry in entries {
        match entry {
            Entry::Record(command) => records(command, command, command, &mut candidates),
            Entry::History(command) => candidates.push(Selection {
                observation: Observation::History(command),
                owner: None,
                child_owner: None,
            }),
        }
    }
    let repositories: BTreeSet<_> = candidates
        .iter()
        .filter(|command| name.matches_repo(command.observation.repository()))
        .map(|command| command.observation.repository().to_ascii_lowercase())
        .collect();
    if matches!(&name, Name::Issue { repository, .. } if !repository.contains('/'))
        && repositories.len() > 1
    {
        bail!(
            "ambiguous Command name {input}; matching repositories: {}",
            repositories.into_iter().collect::<Vec<_>>().join(", ")
        );
    }
    candidates.retain(|command| command.observation.matches(&name));
    if matches!(name, Name::Pass(_)) {
        let repositories: BTreeSet<_> = candidates
            .iter()
            .map(|command| command.observation.repository())
            .collect();
        if repositories.len() > 1 {
            bail!(
                "ambiguous Command name {input}; matching repositories: {}",
                repositories.into_iter().collect::<Vec<_>>().join(", ")
            );
        }
    }
    candidates.sort_by_key(|command| command.observation.ordering());
    candidates
        .into_iter()
        .next()
        .with_context(|| format!("no Command matches {input}"))
}

fn latest_event(command: &Command) -> Option<String> {
    if command.state != "live" {
        return None;
    }
    let path = command.record.session_log.as_ref()?;
    let mut file = std::fs::File::open(path).ok()?;
    // A show reads at most 64 KiB, independent of the transcript's size.
    let start = file.metadata().ok()?.len().saturating_sub(64 * 1024);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.take(64 * 1024).read_to_end(&mut bytes).ok()?;
    let tail = String::from_utf8_lossy(&bytes);
    let mut lines = tail.lines();
    if start > 0 {
        lines.next();
    }
    let harness = crate::harness::Harness::named(&command.record.harness)?;
    let mut stream = harness.adapter().interpretation(Path::new(""), "");
    let event = lines.flat_map(|line| stream.condense(line)).last()?;
    let event = event
        .strip_prefix("$ ")
        .map_or_else(|| event.clone(), |command| format!("Bash: {command}"));
    Some(event.lines().collect::<Vec<_>>().join(" "))
}

#[derive(Serialize)]
#[serde(untagged)]
pub(in crate::logs) enum Detailed<'a> {
    Record(Box<Details<'a>>),
    History(Box<super::history::Command>),
}

/// The detailed view adds a complete Session inventory and a live event to
/// each Run while retaining the status reader's version 1 field names.
#[derive(Serialize)]
pub(in crate::logs) struct Details<'a> {
    #[serde(flatten)]
    record: &'a crate::logs::command_record::CommandRecord,
    state: &'static str,
    health: &'a Option<super::Health>,
    usual_ranges: &'a std::collections::BTreeMap<String, super::UsualRange>,
    stage: &'a Option<super::CurrentStage>,
    source: &'static str,
    command_log: &'a Path,
    session_logs: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_event: Option<String>,
    children: Vec<Details<'a>>,
    dispatched: Vec<Details<'a>>,
}

impl<'a> Details<'a> {
    fn new(command: &'a Command) -> Result<Self> {
        Ok(Self {
            record: &command.record,
            state: command.state,
            health: &command.health,
            usual_ranges: &command.usual_ranges,
            stage: &command.stage,
            source: command.source,
            command_log: &command.command_log,
            session_logs: Observation::Record(command).sessions()?,
            latest_event: latest_event(command),
            children: command
                .children
                .iter()
                .map(Self::new)
                .collect::<Result<_>>()?,
            dispatched: command
                .dispatched
                .iter()
                .map(Self::new)
                .collect::<Result<_>>()?,
        })
    }
}

impl<'a> Observation<'a> {
    pub(in crate::logs) fn details(self) -> Result<Detailed<'a>> {
        Ok(match self {
            Self::Record(command) => Detailed::Record(Box::new(Details::new(command)?)),
            Self::History(command) => {
                let mut details = command.clone();
                details.session_logs = self.sessions()?;
                Detailed::History(Box::new(details))
            }
        })
    }
}

impl Command {
    pub(in crate::logs) fn sessions(&self) -> Result<Vec<PathBuf>> {
        Observation::Record(self).sessions()
    }
    pub(in crate::logs) fn latest_event(&self) -> Option<String> {
        latest_event(self)
    }
}
