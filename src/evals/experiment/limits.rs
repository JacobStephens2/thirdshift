//! The current Experiment watches version-1 Trial logs itself, including
//! when an Arm build predates live limit detection.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::harness::{Choice, interpretation::Interpretation};
use crate::session::limits::{self, Reached};
use crate::trial::{Outcome, Output, Reports, Timings, VoidCause};

pub(super) struct Monitor {
    sessions: Vec<Session>,
}

struct Session {
    log: PathBuf,
    home: PathBuf,
    file: Option<BufReader<File>>,
    partial: Vec<u8>,
    stream: Option<Interpretation>,
}

impl Monitor {
    pub(super) fn new(output: &Path, repository: &Path, choice: &Choice) -> Self {
        Self {
            sessions: [
                "trial-spec-review",
                "trial-spec-review-resume",
                "trial-code-review",
                "trial-code-review-resume",
            ]
            .into_iter()
            .map(|name| Session {
                log: output.join(format!("{name}.jsonl")),
                home: output.join("home"),
                file: None,
                partial: Vec::new(),
                stream: Some(
                    choice
                        .harness
                        .adapter()
                        .interpretation(repository, "")
                        .without_retained(),
                ),
            })
            .collect(),
        }
    }

    pub(super) fn check(&mut self, signal: &Path) -> Result<()> {
        for session in &mut self.sessions {
            session.read()?;
            let stream = session.stream.as_mut().unwrap();
            for raw in stream.limit_records() {
                drop(stream.condense(&raw));
            }
            if stream.plan_limit_reached() {
                limits::notify(signal)?;
                return Err(Reached.into());
            }
        }
        Ok(())
    }

    pub(super) fn cut_short(mut self, total_ms: u128) -> Output {
        let session = self
            .sessions
            .iter_mut()
            .rev()
            .find(|session| session.stream.as_ref().unwrap().plan_limit_reached());
        let (log, session_id) = session.map_or((None, None), |session| {
            (
                Some(session.log.clone()),
                session.stream.take().unwrap().into_session_id(),
            )
        });
        let artifact = |extension| {
            log.as_ref()
                .map(|log| log.with_extension(extension))
                .filter(|path| path.is_file())
        };
        Output {
            version: 1,
            outcome: Outcome::Void,
            timings: Timings {
                total_ms,
                session_ms: total_ms,
            },
            void_cause: Some(VoidCause::UsageLimit),
            cause: Some(Reached.to_string()),
            session_id,
            model_evidence: artifact("models.json"),
            reports: Reports {
                standards: artifact("standards"),
                spec: artifact("spec"),
            },
            log,
        }
    }
}

impl Session {
    fn read(&mut self) -> Result<()> {
        // Older Arms create their fresh home only after the outer monitor
        // starts. Follow it once it exists, retaining host-home observation
        // for Arms that predate sandboxing and for pass-through launches.
        if !crate::sandbox::disabled() && self.home.is_dir() {
            self.stream.as_mut().unwrap().set_home(&self.home);
        }
        if self.file.is_none() {
            match File::open(&self.log) {
                Ok(file) => self.file = Some(BufReader::new(file)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error.into()),
            }
        }
        let file = self.file.as_mut().unwrap();
        loop {
            match file.read_until(b'\n', &mut self.partial)? {
                0 => break,
                _ if !self.partial.ends_with(b"\n") => break,
                _ => (),
            }
            drop(
                self.stream
                    .as_mut()
                    .unwrap()
                    .condense(&String::from_utf8_lossy(&self.partial)),
            );
            self.partial.clear();
        }
        Ok(())
    }
}
