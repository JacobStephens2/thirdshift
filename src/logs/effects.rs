//! The complete record's private clock, terminal and file seam. Adapters
//! perform effects only; none can call the process-wide recording interface.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, Local};

use super::{activity, owner};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stream {
    Stdout,
    Stderr,
}

pub(super) struct Tail {
    pub from: u64,
    pub bytes: Vec<u8>,
}

pub(super) trait Outside: owner::Files {
    fn review_summary(&mut self, log: &Path) -> Option<Vec<u8>>;
    fn session_signals(&mut self, log: &Path) -> Option<super::session_signals::SessionSignals>;
    fn command_records(
        &mut self,
        root: &Path,
    ) -> Result<BTreeMap<PathBuf, super::command_record::CommandTree>>;
    fn now(&mut self) -> DateTime<FixedOffset>;
    fn emit(&mut self, stream: Stream, line: &str) -> io::Result<()>;
    fn activity_tail(&mut self, path: &Path, limit: u64) -> Result<Tail>;
    fn append_activity(&mut self, path: &Path, line: &str) -> Result<()>;
    fn hostname(&mut self) -> String;
    fn process_start(&mut self, pid: u32) -> Option<String>;
    fn replace_record(&mut self, path: &Path, bytes: &[u8]) -> Result<()>;
    fn stop_request(&mut self, record: &Path) -> Option<super::stop::Request>;
}

pub(super) struct OnMachine;

impl owner::Files for OnMachine {
    type CommandLog = File;

    fn create_command_log(&mut self, path: &Path) -> io::Result<File> {
        owner::create(path)
    }
}

impl Outside for OnMachine {
    fn session_signals(&mut self, log: &Path) -> Option<super::session_signals::SessionSignals> {
        super::session_signals::read(log).ok().flatten()
    }

    fn review_summary(&mut self, log: &Path) -> Option<Vec<u8>> {
        std::fs::read(super::session::original(log).with_extension("review-summary.json")).ok()
    }

    fn command_records(
        &mut self,
        root: &Path,
    ) -> Result<BTreeMap<PathBuf, super::command_record::CommandTree>> {
        super::reading::repository_records(root)
    }

    fn stop_request(&mut self, record: &Path) -> Option<super::stop::Request> {
        super::stop::requested(record)
    }

    fn hostname(&mut self) -> String {
        crate::host::name().unwrap_or_else(|| "unknown".into())
    }

    fn process_start(&mut self, pid: u32) -> Option<String> {
        super::process_identity::start(pid)
    }

    fn replace_record(&mut self, path: &Path, bytes: &[u8]) -> Result<()> {
        let parent = path
            .parent()
            .context("Command record has no parent directory")?;
        std::fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(bytes)?;
        temporary.persist(path)?;
        Ok(())
    }

    fn now(&mut self) -> DateTime<FixedOffset> {
        Local::now().fixed_offset()
    }

    fn emit(&mut self, stream: Stream, line: &str) -> io::Result<()> {
        fn to(mut terminal: impl Write, line: &str) -> io::Result<()> {
            let written = writeln!(terminal, "{line}");
            let flushed = terminal.flush();
            written.and(flushed)
        }
        match stream {
            Stream::Stdout => to(io::stdout(), line),
            Stream::Stderr => to(io::stderr(), line),
        }
    }

    fn activity_tail(&mut self, path: &Path, limit: u64) -> Result<Tail> {
        activity::tail(path, limit)
    }

    fn append_activity(&mut self, path: &Path, line: &str) -> Result<()> {
        activity::open(path)?
            .write_all(line.as_bytes())
            .with_context(|| format!("could not write {}", path.display()))
    }
}
