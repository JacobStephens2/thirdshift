//! One Command's owned recording implementation. Production and tests use
//! these same operations; only the effects adapter differs.

use std::collections::VecDeque;
use std::fmt::Display;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};

use super::activity::{self, Kind};
use super::command_record::{CommandRecord, CommandTree, Stage, StageName, TicketRecord};
use super::effects::{Outside, Stream};
use super::{Begin, CommandKind, Pass, Work, root_under};
use crate::config::UserConfig;
use crate::harness::Choice;
use crate::issue::Repo;
use crate::progress;

const STAMP_FORMAT: &str = "%Y%m%dT%H%M%S%z";
static NEXT_RECORD: AtomicU64 = AtomicU64::new(0);

pub(super) struct Record<O: Outside> {
    outside: O,
    stamp: Option<String>,
    logs_dir: Option<PathBuf>,
    child: bool,
    command: CommandKind,
    started: bool,
    command_log: Log<O::CommandLog>,
    activity_started: Option<Started>,
    activity_warned: bool,
    command_record: Option<(PathBuf, CommandTree)>,
    parent_record: Option<PathBuf>,
    parent_dispatch: Option<String>,
    record_warned: bool,
}

enum Log<W> {
    NotKept,
    Held {
        lines: Vec<(Stream, String)>,
        shown: bool,
    },
    Kept {
        path: PathBuf,
        writer: W,
    },
    Failed {
        path: Option<PathBuf>,
    },
}

struct Started {
    root: PathBuf,
    kind: Kind,
    issue: Option<u64>,
}

impl Started {
    fn subject(&self) -> String {
        match self.issue {
            Some(number) => format!("{} #{number}", self.kind),
            None => self.kind.to_string(),
        }
    }
}

impl<O: Outside> Record<O> {
    pub(super) fn new(outside: O) -> Self {
        Self {
            outside,
            stamp: None,
            logs_dir: None,
            child: false,
            command: CommandKind::Issue,
            started: false,
            command_log: Log::NotKept,
            activity_started: None,
            activity_warned: false,
            command_record: None,
            parent_record: None,
            parent_dispatch: None,
            record_warned: false,
        }
    }

    pub(super) fn begin(&mut self, begin: Begin) {
        self.command = match &begin {
            Begin::Run(_) => CommandKind::Issue,
            Begin::ChildRun(_, command) => *command,
            Begin::ArchitectRun => CommandKind::Architect,
            Begin::PickupRun => CommandKind::Pickup,
            Begin::SecurityRun => CommandKind::Secure,
        };
        let (starting, quiet) = match begin {
            Begin::ChildRun(stamp, _) => {
                self.child = true;
                self.stamp.get_or_insert_with(|| stamp.to_string());
                return;
            }
            Begin::Run(issue) => (format!("starting on {}", issue.url), false),
            Begin::ArchitectRun => ("Architect run starting".into(), true),
            Begin::PickupRun => ("Pickup run starting".into(), true),
            Begin::SecurityRun => ("Security run starting".into(), true),
        };
        let now = self.outside.now();
        self.stamp
            .get_or_insert_with(|| now.format(STAMP_FORMAT).to_string());
        self.command_log = Log::Held {
            lines: Vec::new(),
            shown: !quiet,
        };
        self.step(format_args!("{starting}, {}", now.format("%Y-%m-%d %z")));
    }

    pub(super) fn configured(&mut self, config: &UserConfig) {
        self.logs_dir = Some(config.logs_dir.clone());
        if !config.quiet_skips {
            self.show_held();
        }
    }

    pub(super) fn started(&mut self, work: Work, harness: &Choice) {
        let first = !self.started;
        self.started = true;
        if self.child && first {
            if self.parent_record.is_some() {
                self.start_record(work, harness);
            }
            return;
        }
        if !first || self.child {
            if !self.child
                && self.command != CommandKind::Issue
                && matches!(work, Work::Run(_) | Work::SpecRun(_))
                && let Some((_, tree)) = &mut self.command_record
                && let Some(dispatched) = tree.dispatched.last_mut()
            {
                // The launch checks have now established whether the issue is
                // a Spec. The dispatch itself was kept before those checks.
                dispatched.kind = work.kind().to_string();
                self.stage(work.initial_stage().unwrap(), None);
            }
            return;
        }
        let root = self.root(&work.repo());
        let stamp = self.stamp();
        let began = matches!(self.command_log, Log::Held { .. });
        self.keep(work.command_log(&root, &stamp));
        if began {
            self.start_record(work, harness);
        }
        self.step(format_args!("sessions run on {harness}"));
        let command_log = match self.command_log_path() {
            Some(path) => path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .display()
                .to_string(),
            None => "no Command log".into(),
        };
        let started = Started {
            root: root.clone(),
            kind: work.kind(),
            issue: work.issue(),
        };
        let line = format!("{} started: {command_log}, on {harness}", started.subject());
        if self.activity_started.is_none() {
            self.activity_started = Some(started);
            self.activity_write(&root, &line, None);
        }
    }

    pub(super) fn child_of(&mut self, parent: Option<PathBuf>) {
        self.parent_record = parent;
    }

    pub(super) fn child_dispatch(&mut self, issue: Option<String>) {
        self.parent_dispatch = issue;
    }

    pub(super) fn dispatch_issue(&self) -> Option<String> {
        self.command_record
            .as_ref()?
            .1
            .dispatched
            .last()
            .filter(|record| record.outcome.is_none())?
            .issue
            .as_ref()
            .map(|issue| issue.url.clone())
    }

    pub(super) fn record_path(&self) -> Option<PathBuf> {
        self.command_record.as_ref().map(|(path, _)| path.clone())
    }

    pub(super) fn stop_request(&self) -> Option<super::stop::Request> {
        let path = self
            .parent_record
            .as_ref()
            .or_else(|| self.command_record.as_ref().map(|(path, _)| path))?;
        super::stop::requested(path)
    }

    fn start_record(&mut self, work: Work, harness: &Choice) {
        let stamp = self.stamp();
        let mut facts = CommandRecord::new(work, &stamp, harness, self.outside.now());
        facts.hostname = self.outside.hostname();
        facts.process_start = self
            .outside
            .process_start(facts.process_id)
            .unwrap_or_default();
        facts.parent_record = self.parent_record.clone();
        facts.parent_dispatch = self.parent_dispatch.clone();
        let path = if let Some(parent) = &self.parent_record {
            let dispatch = self
                .parent_dispatch
                .as_ref()
                .and_then(|issue| crate::issue::IssueUrl::parse(issue).ok())
                .map_or_else(String::new, |issue| format!("dispatch-{}-", issue.number));
            parent.with_extension(format!(
                "{dispatch}issue-{}.child.json",
                work.issue().unwrap()
            ))
        } else {
            // Process identity and a serial separate Commands sharing a stamp.
            let owner = format!(
                "{}-{}-{}",
                facts.process_id,
                facts.process_start.replace(':', "-"),
                NEXT_RECORD.fetch_add(1, Ordering::Relaxed)
            );
            let root = self.root(&work.repo());
            work.command_log(&root, &stamp)
                .with_extension(format!("{owner}.command.json"))
        };
        self.command_record = Some((
            path,
            CommandTree {
                command: facts,
                dispatched: Vec::new(),
            },
        ));
        if let Some(stage) = work.initial_stage() {
            self.stage(stage, None);
        } else {
            self.write_record();
        }
    }

    pub(super) fn tickets(&mut self, tickets: Vec<TicketRecord>) {
        if let Some((_, tree)) = &mut self.command_record {
            tree.active().tickets = tickets;
            self.write_record();
        }
    }

    pub(super) fn skipped(&mut self, pass: Pass, repo: &Repo, reason: impl Display) {
        let kind = Kind::Pass(pass);
        self.activity_write(
            &self.root(repo),
            &format!("{kind} skipped: {reason}"),
            Some(kind),
        );
    }

    pub(super) fn ended(&mut self, outcome: impl Display) {
        self.show_held();
        if let Some(started) = &self.activity_started {
            let root = started.root.clone();
            let subject = started.subject();
            if let Some(request) = self.stop_request() {
                self.activity_write(
                    &root,
                    &format!("{subject} stopped from the CLI by {}", request.who),
                    None,
                );
            }
            let line = format!("{subject} ended: {outcome}");
            self.activity_write(&root, &line, None);
        }
    }

    pub(super) fn issue_title(&mut self, title: Option<String>) {
        if let Some((_, tree)) = &mut self.command_record
            && let Some(issue) = &mut tree.active().issue
        {
            issue.title = title;
            self.write_record();
        }
    }

    pub(super) fn issue_branch(&mut self, branch: Option<&str>) {
        if let Some((_, tree)) = &mut self.command_record {
            tree.active().issue_branch = branch.map(String::from);
            self.write_record();
        }
    }

    pub(super) fn stage(&mut self, stage: StageName, log: Option<&Path>) {
        let name = stage.name();
        let Some((_, tree)) = &mut self.command_record else {
            return;
        };
        let facts = tree.active();
        if let Some(log) = log {
            facts.session_log = Some(log.into());
        }
        if facts.stages.last().is_none_or(|stage| stage.name != name) {
            facts.stages.push(Stage {
                name: name.into(),
                began_at: self.outside.now().to_rfc3339(),
                session_log: facts.session_log.clone(),
            });
        } else if let Some(stage) = facts.stages.last_mut() {
            stage.session_log = facts.session_log.clone();
        }
        self.write_record();
    }

    pub(super) fn finished(&mut self, account: &crate::run_ending::Account<'_>) {
        if let Some((_, tree)) = &mut self.command_record {
            tree.command.finish(account, self.outside.now());
            self.write_record();
        }
    }

    pub(super) fn dispatch_ended(&mut self, account: &crate::run_ending::Account<'_>) {
        if let Some((_, tree)) = &mut self.command_record
            && let Some(dispatched) = tree.dispatched.last_mut()
        {
            dispatched.finish(account, self.outside.now());
            self.write_record();
        }
    }

    pub(super) fn dispatching(&mut self, work: Work, harness: &Choice) {
        let stamp = self.stamp();
        let mut facts = CommandRecord::new(work, &stamp, harness, self.outside.now());
        if let Some((_, tree)) = &mut self.command_record {
            facts.hostname = tree.command.hostname.clone();
            facts.process_start = tree.command.process_start.clone();
            tree.dispatched.push(facts);
            self.write_record();
        }
    }

    fn write_record(&mut self) {
        let Some((path, facts)) = &self.command_record else {
            return;
        };
        let written = serde_json::to_vec_pretty(facts)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| self.outside.replace_record(path, &bytes));
        if let Err(error) = written
            && !self.record_warned
        {
            self.record_warned = true;
            self.step(format_args!(
                "warning: could not keep the Command record: {error:#}"
            ));
        }
    }

    pub(super) fn show_held(&mut self) {
        if let Log::Held { lines, shown } = &mut self.command_log
            && !*shown
        {
            for (stream, line) in lines.iter() {
                let _ = self.outside.emit(*stream, line);
            }
            *shown = true;
        }
    }

    pub(super) fn root(&self, repo: &Repo) -> PathBuf {
        let logs_dir = self.logs_dir.as_deref().expect(
            "the root of a repository's logs is asked for before the User config is loaded",
        );
        root_under(logs_dir, repo)
    }

    pub(super) fn command_kind(&self) -> CommandKind {
        self.command
    }

    pub(super) fn stamp(&mut self) -> String {
        self.stamp
            .get_or_insert_with(|| self.outside.now().format(STAMP_FORMAT).to_string())
            .clone()
    }

    pub(super) fn command_log_path(&self) -> Option<PathBuf> {
        match &self.command_log {
            Log::Kept { path, .. } => Some(path.clone()),
            Log::Failed { path } => path.clone(),
            Log::NotKept | Log::Held { .. } => None,
        }
    }

    pub(super) fn print(&mut self, line: &str) {
        self.output(Stream::Stdout, line.to_string());
    }
    pub(super) fn eprint(&mut self, line: &str) {
        self.output(Stream::Stderr, line.to_string());
    }

    fn step(&mut self, message: impl Display) {
        let line = self.progress_line(message);
        self.output(Stream::Stderr, line);
    }

    fn progress_line(&mut self, message: impl Display) -> String {
        progress::progress_line(self.outside.now().format("%H:%M:%S"), message)
    }

    /// Warnings enter the same ordered output path without recursion. A
    /// failed writer is disabled before its warning is queued.
    fn output(&mut self, stream: Stream, line: String) {
        let mut pending = VecDeque::from([(stream, line)]);
        while let Some((stream, line)) = pending.pop_front() {
            if !matches!(self.command_log, Log::Held { shown: false, .. }) {
                let _ = self.outside.emit(stream, &line);
            }
            match &mut self.command_log {
                Log::NotKept | Log::Failed { .. } => {}
                Log::Held { lines, .. } => lines.push((stream, line)),
                Log::Kept { path, writer } => {
                    if let Err(error) = writeln!(writer, "{line}") {
                        let error = anyhow::Error::new(error)
                            .context(format!("could not write {}", path.display()));
                        self.command_log = Log::Failed {
                            path: Some(path.clone()),
                        };
                        let warning = self.progress_line(format_args!(
                            "warning: could not keep the Command log: {error:#}"
                        ));
                        pending.push_back((Stream::Stderr, warning));
                    }
                }
            }
        }
    }

    fn keep(&mut self, path: PathBuf) {
        self.show_held();
        if !matches!(self.command_log, Log::Held { .. }) {
            return;
        }
        let Log::Held { lines, .. } = std::mem::replace(&mut self.command_log, Log::NotKept) else {
            unreachable!()
        };
        let created: Result<O::CommandLog> = (|| {
            let mut writer = self.outside.open_command_log(&path)?;
            for (_, line) in lines {
                writeln!(writer, "{line}")
                    .with_context(|| format!("could not write {}", path.display()))?;
            }
            Ok(writer)
        })();
        match created {
            Ok(writer) => {
                self.command_log = Log::Kept {
                    path: path.clone(),
                    writer,
                };
                self.step(format_args!("logging this command to {}", path.display()));
            }
            Err(error) => {
                self.command_log = Log::Failed {
                    path: self.outside.command_log_exists(&path).then_some(path),
                };
                self.step(format_args!(
                    "warning: could not keep the Command log: {error:#}"
                ));
            }
        }
    }

    fn activity_write(&mut self, root: &Path, line: &str, unless_last_of: Option<Kind>) {
        let appended = self.activity_append(&root.join(activity::FILE), line, unless_last_of);
        if let Err(error) = appended
            && !self.activity_warned
        {
            self.activity_warned = true;
            self.show_held();
            self.step(format_args!(
                "warning: could not keep the Activity log: {error:#}"
            ));
        }
    }

    fn activity_append(
        &mut self,
        path: &Path,
        line: &str,
        unless_last_of: Option<Kind>,
    ) -> Result<()> {
        if let Some(kind) = unless_last_of {
            let tail = self.outside.activity_tail(path, activity::TAIL)?;
            let text = String::from_utf8_lossy(&tail.bytes);
            let whole = if tail.from > 0 {
                text.split_once('\n').map_or("", |(_, rest)| rest)
            } else {
                &text
            };
            if activity::last_of(whole, kind) == Some(line) {
                return Ok(());
            }
        }
        let stamped = format!(
            "{} {line}\n",
            self.outside.now().format(activity::TIME_FORMAT)
        );
        self.outside.append_activity(path, &stamped)
    }
}

#[cfg(test)]
mod tests;
