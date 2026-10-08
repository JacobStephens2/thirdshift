//! One Command's owned recording implementation. Production and tests use
//! these same operations; only the effects adapter differs.

use std::collections::VecDeque;
use std::fmt::Display;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::activity::{self, Kind};
use super::effects::{Outside, Stream};
use super::{Begin, Pass, Work, root_under};
use crate::config::UserConfig;
use crate::harness::Choice;
use crate::issue::Repo;
use crate::progress;

const STAMP_FORMAT: &str = "%Y%m%dT%H%M%S%z";

pub(super) struct Record<O: Outside> {
    outside: O,
    stamp: Option<String>,
    logs_dir: Option<PathBuf>,
    child: bool,
    started: bool,
    command_log: Log<O::CommandLog>,
    activity_started: Option<Started>,
    activity_warned: bool,
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
            started: false,
            command_log: Log::NotKept,
            activity_started: None,
            activity_warned: false,
        }
    }

    pub(super) fn begin(&mut self, begin: Begin) {
        let (starting, quiet) = match begin {
            Begin::ChildRun(stamp) => {
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
        if !first || self.child {
            return;
        }
        let root = self.root(&work.repo());
        let stamp = self.stamp();
        self.keep(work.command_log(&root, &stamp));
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
            let line = format!("{} ended: {outcome}", started.subject());
            self.activity_write(&root, &line, None);
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
