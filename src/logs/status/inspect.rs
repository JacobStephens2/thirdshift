//! Resolve human Command names and read one Command's state and logs.

use std::collections::BTreeSet;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use super::{Command, Entry};
use crate::issue::IssueUrl;
use crate::logs::effects::Outside;

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
enum Selected<'a> {
    Record(&'a Command),
    History(&'a super::history::Command),
}

impl<'a> Selected<'a> {
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
                    let kind = match command.kind.as_str() {
                        "Architect run" => "architect",
                        "Pickup run" => "pickup",
                        "Security run" => "secure",
                        _ => return false,
                    };
                    stamp(path).is_some_and(|stamp| *name == format!("{kind}-{stamp}"))
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

    fn command_log(self) -> Option<&'a Path> {
        match self {
            Self::Record(command) => Some(&command.command_log),
            Self::History(command) => command.command_log.as_deref(),
        }
    }

    fn print(self) -> Result<()> {
        match self {
            Self::Record(command) => print_details(command, "")?,
            Self::History(command) => command.print(),
        }
        if let Some(path) = self.command_log() {
            println!("Command log: {}", path.display());
        }
        if matches!(self, Self::History(_)) {
            for path in self.sessions()? {
                println!("Session log: {}", path.display());
            }
        }
        Ok(())
    }

    fn sessions(self) -> Result<Vec<PathBuf>> {
        match self {
            Self::Record(command) => {
                let mut paths = Vec::new();
                for path in command
                    .record
                    .stages
                    .iter()
                    .filter_map(|stage| stage.session_log.as_ref())
                    .chain(command.record.session_log.as_ref())
                {
                    if !paths.contains(path) {
                        paths.push(path.clone());
                    }
                }
                Ok(paths)
            }
            Self::History(command) => {
                let mut paths = command.session_logs.clone();
                if let Some(log) = &command.command_log
                    && let Some(stamp) = stamp(log)
                {
                    let prefix = command
                        .issue
                        .as_ref()
                        .map(|issue| issue.number.to_string())
                        .unwrap_or_else(|| {
                            match command.kind.as_str() {
                                "Architect run" => "architect",
                                "Security run" => "secure",
                                _ => "pickup",
                            }
                            .into()
                        });
                    let prefix = format!("{prefix}-{stamp}-");
                    for entry in
                        super::entries(log.parent().context("Command log has no directory")?)?
                    {
                        let name = entry.file_name();
                        let name = name.to_string_lossy();
                        if entry.file_type()?.is_file()
                            && name.starts_with(&prefix)
                            && name.ends_with(".jsonl")
                            && !paths.contains(&entry.path())
                        {
                            paths.push(entry.path());
                        }
                    }
                }
                let mut dated = paths
                    .into_iter()
                    .map(|path| {
                        let modified = std::fs::metadata(&path)?.modified()?;
                        Ok((modified, path))
                    })
                    .collect::<Result<Vec<_>>>()?;
                dated.sort();
                Ok(dated.into_iter().map(|(_, path)| path).collect())
            }
        }
    }

    fn session(self, kind: &str) -> Result<PathBuf> {
        self.sessions()?
            .into_iter()
            .rev()
            .find(|path| {
                let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                    return false;
                };
                let Some((_, rest)) = name.split_once('-') else {
                    return false;
                };
                let Some(session) = rest.get(21..).and_then(|rest| rest.strip_suffix(".jsonl"))
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

fn records<'a>(command: &'a Command, found: &mut Vec<Selected<'a>>) {
    found.push(Selected::Record(command));
    for child in command.dispatched.iter().chain(&command.children) {
        records(child, found);
    }
}

fn select<'a>(entries: &'a [Entry], input: &str) -> Result<Selected<'a>> {
    let name = Name::parse(input)?;
    let mut candidates = Vec::new();
    for entry in entries {
        match entry {
            Entry::Record(command) => records(command, &mut candidates),
            Entry::History(command) => candidates.push(Selected::History(command)),
        }
    }
    let repositories: BTreeSet<_> = candidates
        .iter()
        .filter(|command| name.matches_repo(command.repository()))
        .map(|command| command.repository().to_ascii_lowercase())
        .collect();
    if matches!(&name, Name::Issue { repository, .. } if !repository.contains('/'))
        && repositories.len() > 1
    {
        bail!(
            "ambiguous Command name {input}; matching repositories: {}",
            repositories.into_iter().collect::<Vec<_>>().join(", ")
        );
    }
    candidates.retain(|command| command.matches(&name));
    if matches!(name, Name::Pass(_)) {
        let repositories: BTreeSet<_> = candidates
            .iter()
            .map(|command| command.repository())
            .collect();
        if repositories.len() > 1 {
            bail!(
                "ambiguous Command name {input}; matching repositories: {}",
                repositories.into_iter().collect::<Vec<_>>().join(", ")
            );
        }
    }
    candidates.sort_by_key(|command| command.ordering());
    candidates
        .into_iter()
        .next()
        .with_context(|| format!("no Command matches {input}"))
}

fn print_details(command: &Command, indent: &str) -> Result<()> {
    println!(
        "{indent}{} {} · {}",
        command.record.kind,
        command.record.name,
        super::stage_text(command)
    );
    if let Some(issue) = &command.record.issue {
        println!(
            "{indent}{}",
            issue.title.as_deref().unwrap_or("(title unavailable)")
        );
    }
    for stage in &command.record.stages {
        println!("{indent}  {} · {}", stage.name, stage.began_at);
    }
    for path in Selected::Record(command).sessions()? {
        println!("{indent}Session log: {}", path.display());
    }
    if let Some(event) = &command.latest_event {
        println!("{indent}{event}");
    }
    super::print_tickets(&command.record, &format!("{indent}  "));
    for child in command.dispatched.iter().chain(&command.children) {
        print_details(child, &format!("{indent}  "))?;
    }
    Ok(())
}

pub fn show(input: &str, json: bool) -> Result<()> {
    let mut status = super::read(true)?;
    for entry in &mut status.commands {
        if let Entry::Record(command) = entry {
            latest_events(command);
        }
    }
    let command = select(&status.commands, input)?;
    if json {
        #[derive(Serialize)]
        struct Show<'a> {
            version: u32,
            hostname: String,
            command: Selected<'a>,
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&Show {
                version: 1,
                hostname: status.hostname,
                command
            })?
        );
    } else {
        command.print()?;
    }
    Ok(())
}

fn stamp(path: &Path) -> Option<&str> {
    let stem = path.file_stem()?.to_str()?;
    let stamp = stem.get(stem.len().checked_sub(20)?..)?;
    chrono::DateTime::parse_from_str(stamp, "%Y%m%dT%H%M%S%z").ok()?;
    Some(stamp)
}

fn latest_events(command: &mut Command) {
    if command.state == "live" {
        command.latest_event = latest_event(command);
    }
    for child in command.dispatched.iter_mut().chain(&mut command.children) {
        latest_events(child);
    }
}

fn latest_event(command: &Command) -> Option<String> {
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

pub fn logs(input: &str, session: Option<&str>, follow: bool) -> Result<()> {
    let status = super::read(true)?;
    let command = select(&status.commands, input)?;
    let path = match session {
        Some(kind) => command.session(kind)?,
        None => command
            .command_log()
            .context("this Command has no Command log")?
            .to_path_buf(),
    };
    let mut file =
        std::fs::File::open(&path).with_context(|| format!("could not open {}", path.display()))?;
    let owner = status.commands.iter().find_map(|entry| {
        if let (Entry::Record(root), Selected::Record(selected)) = (entry, command)
            && contains(root, selected)
        {
            return Some(root.as_ref());
        }
        None
    });
    let mut stdout = io::stdout().lock();
    loop {
        // Check before draining, so a completed Command's final bytes are
        // always copied even when it ended between two polls.
        let ended = owner.is_none_or(ended);
        io::copy(&mut file, &mut stdout)?;
        stdout.flush()?;
        if !follow || ended {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn contains(root: &Command, selected: &Command) -> bool {
    std::ptr::eq(root, selected)
        || root
            .dispatched
            .iter()
            .chain(&root.children)
            .any(|child| contains(child, selected))
}

fn ended(command: &Command) -> bool {
    let record = &command.record;
    // The outcome is recorded before the final summary and notification are
    // printed. Follow the captured process identity through that last output.
    record.process_start.is_empty()
        || super::OnMachine.process_start(record.process_id).as_deref()
            != Some(&record.process_start)
}
