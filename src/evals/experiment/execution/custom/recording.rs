//! Host-owned live recording. The sandbox sends events, never owns trace files.
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

use crate::harness::{Choice, Harness, interpretation::Interpretation};
use crate::logs::trace::Context as TraceContext;
use crate::session::{limits, trace::Trace};

pub(in crate::evals::experiment::execution) struct Recorder {
    listener: UnixListener,
    directory: tempfile::TempDir,
    destination: PathBuf,
    repository: PathBuf,
    home: PathBuf,
    context: TraceContext,
    connections: Vec<Connection>,
    pub sessions: usize,
    pub web_search: bool,
    pub limit: Option<limits::Limit>,
}

struct Connection {
    reader: BufReader<UnixStream>,
    partial: Vec<u8>,
    session: Option<Session>,
    ended: bool,
}

struct Session {
    trace: Trace,
    stream: Interpretation,
}

#[derive(Deserialize)]
struct Start {
    role: String,
    harness: String,
    arguments: Vec<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Frame {
    Raw { raw: String },
    End { error: Option<String> },
}

impl Recorder {
    pub fn start(attempt: &Path) -> Result<Self> {
        let directory = tempfile::Builder::new().prefix("trial-record-").tempdir()?;
        let listener = UnixListener::bind(directory.path().join("socket"))?;
        listener.set_nonblocking(true)?;
        fs::write(
            attempt.join("record-command.py"),
            include_str!("record-command.py"),
        )?;
        Ok(Self {
            listener,
            directory,
            destination: attempt.join("session"),
            repository: attempt.join("repository"),
            home: attempt.join("home"),
            context: serde_json::from_slice(&fs::read(attempt.join("trace-context.json"))?)?,
            connections: Vec::new(),
            sessions: 0,
            web_search: false,
            limit: None,
        })
    }

    pub fn socket(&self) -> PathBuf {
        self.directory.path().join("socket")
    }

    pub fn command(&self, attempt: &Path) -> Vec<String> {
        vec![
            "python3".into(),
            attempt
                .join("record-command.py")
                .to_string_lossy()
                .into_owned(),
            self.socket().to_string_lossy().into_owned(),
        ]
    }

    pub fn collect(&mut self) -> Result<super::sessions::Import> {
        self.finish()?;
        super::sessions::import(
            &self.destination.parent().unwrap().join("output"),
            &self.destination,
            &self.repository,
            &self.context,
        )
    }

    pub fn poll(&mut self) -> Result<()> {
        loop {
            match self.listener.accept() {
                Ok((socket, _)) => {
                    socket.set_nonblocking(true)?;
                    self.connections.push(Connection {
                        reader: BufReader::new(socket),
                        partial: Vec::new(),
                        session: None,
                        ended: false,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error.into()),
            }
        }
        for connection in &mut self.connections {
            while !connection.ended {
                match connection.reader.read_until(b'\n', &mut connection.partial) {
                    Ok(0) => {
                        if let Some(session) = &mut connection.session {
                            session.trace.finish(Some("recording connection closed"))?;
                        }
                        connection.ended = true;
                        break;
                    }
                    Ok(_) if !connection.partial.ends_with(b"\n") => continue,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) => return Err(error.into()),
                    Ok(_) => {}
                }
                let line = std::mem::take(&mut connection.partial);
                if let Some(session) = &mut connection.session {
                    match serde_json::from_slice::<Frame>(&line)? {
                        Frame::Raw { raw } => {
                            let events = session.stream.decode(&raw).events;
                            self.web_search |= super::sessions::web_search(&events);
                            session.trace.record(&raw, &events)?;
                        }
                        Frame::End { error } => {
                            session.trace.finish(error.as_deref())?;
                            connection.ended = true;
                        }
                    }
                } else {
                    let start: Start = serde_json::from_slice(&line)?;
                    ensure!(
                        !start.role.trim().is_empty(),
                        "record_command requires a role"
                    );
                    let harness =
                        Harness::named(&start.harness).context("unsupported recording Harness")?;
                    self.sessions += 1;
                    let log = self
                        .destination
                        .join(format!("custom-live-{}.jsonl", self.sessions));
                    let mut command = std::process::Command::new(&start.harness);
                    command.args(&start.arguments).current_dir(&self.repository);
                    let prompt = crate::args::shell_command(start.arguments.clone());
                    let trace = Trace::create(
                        &log,
                        "custom",
                        &Choice {
                            harness,
                            ..Choice::default()
                        },
                        &prompt,
                        self.context.clone(),
                        &command,
                        None,
                    )?
                    .with_harness(harness.name())
                    .with_trial_role(Some(&start.role))?;
                    let mut stream = harness
                        .adapter()
                        .interpretation(&self.repository, "")
                        .without_retained();
                    stream.set_home(&self.home);
                    connection.session = Some(Session { trace, stream });
                }
            }
            if let Some(session) = &mut connection.session {
                for raw in session.stream.limit_records() {
                    let events = session.stream.decode(&raw).events;
                    session.trace.record(&raw, &events)?;
                }
                if let Some(limit) = session.stream.plan_limit() {
                    self.limit = Some(limit.clone());
                }
            }
        }
        Ok(())
    }

    pub fn check(&mut self, signal: &Path) -> Result<()> {
        self.poll()?;
        if let Some(limit) = &self.limit {
            limits::notify(signal, limit)?;
            return Err(limits::Reached(limit.clone()).into());
        }
        Ok(())
    }

    pub fn finish(&mut self) -> Result<()> {
        self.poll()?;
        for connection in &mut self.connections {
            if !connection.ended {
                if let Some(session) = &mut connection.session {
                    session.trace.finish(Some("Trial ended during recording"))?;
                }
                connection.ended = true;
            }
        }
        Ok(())
    }
}
