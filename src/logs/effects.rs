//! The complete record's private clock, terminal and file seam. Adapters
//! perform effects only; none can call the process-wide recording interface.

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, Local};

use super::{activity, command_log};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stream {
    Stdout,
    Stderr,
}

pub(super) struct Tail {
    pub from: u64,
    pub bytes: Vec<u8>,
}

pub(super) trait Outside {
    type CommandLog: Write + Send;

    fn now(&mut self) -> DateTime<FixedOffset>;
    fn emit(&mut self, stream: Stream, line: &str) -> io::Result<()>;
    fn open_command_log(&mut self, path: &Path) -> Result<Self::CommandLog>;
    fn command_log_exists(&mut self, path: &Path) -> bool;
    fn activity_tail(&mut self, path: &Path, limit: u64) -> Result<Tail>;
    fn append_activity(&mut self, path: &Path, line: &str) -> Result<()>;
}

pub(super) struct OnMachine;

impl Outside for OnMachine {
    type CommandLog = File;

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

    fn open_command_log(&mut self, path: &Path) -> Result<File> {
        command_log::open(path)
    }

    fn command_log_exists(&mut self, path: &Path) -> bool {
        path.exists()
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
