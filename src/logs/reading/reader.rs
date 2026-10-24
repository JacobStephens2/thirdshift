//! Captured writer lifetime and byte draining, independent of presentation.
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::PathBuf;

use anyhow::{Context, Result, ensure};

use super::{Command, cache};
use crate::logs::command_record::CommandRecord;

#[derive(Clone, Debug)]
pub(crate) struct Writer {
    pub pid: u32,
    start: String,
}

impl Writer {
    pub(in crate::logs) fn capture(record: &CommandRecord) -> Self {
        Self {
            pid: record.process_id,
            start: record.process_start.clone(),
        }
    }

    pub(crate) fn live(&self) -> bool {
        !self.start.is_empty()
            && crate::logs::process_identity::start(self.pid).as_deref() == Some(&self.start)
    }
}

#[derive(Clone)]
struct SessionOwner {
    record: PathBuf,
    name: String,
    started_at: String,
}

/// A resolved file never selects a newer Command or Session while reading.
#[derive(Clone)]
pub(crate) struct Reader {
    path: PathBuf,
    writer: Option<Writer>,
    session: Option<SessionOwner>,
}

#[derive(Clone, Copy)]
pub(crate) enum Framing {
    Raw,
    Lines,
}

#[derive(Debug)]
pub(crate) struct Batch {
    pub bytes: Vec<u8>,
    pub offset: u64,
    pub done: bool,
}

impl Reader {
    pub(super) fn captured(path: PathBuf, command: Option<&Command>, session: bool) -> Self {
        Self {
            path,
            writer: command.map(|command| Writer::capture(&command.record)),
            session: command.filter(|_| session).map(|command| SessionOwner {
                record: command.record_path.clone(),
                name: command.record.name.clone(),
                started_at: command.record.started_at.clone(),
            }),
        }
    }

    fn live(&self) -> Result<bool> {
        if !self.writer.as_ref().is_some_and(Writer::live) {
            return Ok(false);
        }
        if let Some(owner) = &self.session {
            let tree = match cache::record(&owner.record) {
                Ok(tree) => Some(tree.context("unreadable Command record")?),
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
                {
                    None
                }
                Err(error) => return Err(error),
            };
            if let Some(tree) = tree {
                let current =
                    std::iter::once(&tree.command)
                        .chain(&tree.dispatched)
                        .find(|record| {
                            record.name == owner.name && record.started_at == owner.started_at
                        });
                // A removed dispatched record is no stronger evidence of
                // completion than a removed record file: keep its writer.
                return Ok(current.is_none_or(|record| {
                    record
                        .session_log
                        .as_deref()
                        .map(crate::logs::session::path)
                        .as_ref()
                        == Some(&self.path)
                }));
            }
        }
        Ok(true)
    }

    /// Observe completion before draining; a bounded batch is done only at EOF.
    pub(crate) fn read(&self, offset: u64, framing: Framing, budget: usize) -> Result<Batch> {
        let complete = !self.live()?;
        let mut file = BufReader::new(
            std::fs::File::open(&self.path)
                .with_context(|| format!("could not open {}", self.path.display()))?,
        );
        let length = file.get_ref().metadata()?.len();
        ensure!(offset <= length, "log was truncated; reopen the reader");
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = Vec::new();
        match framing {
            Framing::Raw => {
                file.by_ref().take(budget as u64).read_to_end(&mut bytes)?;
            }
            Framing::Lines => {
                while offset + (bytes.len() as u64) < length && bytes.len() < budget {
                    let mut line = Vec::new();
                    if file.read_until(b'\n', &mut line)? == 0 {
                        break;
                    }
                    if !complete && !line.ends_with(b"\n") {
                        break;
                    }
                    bytes.extend(line);
                }
            }
        }
        let next = offset + bytes.len() as u64;
        ensure!(
            next <= file.get_ref().metadata()?.len(),
            "log was truncated; reopen the reader"
        );
        Ok(Batch {
            bytes,
            offset: next,
            done: complete && next >= file.get_ref().metadata()?.len(),
        })
    }
}
