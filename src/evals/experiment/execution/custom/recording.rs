//! Host-owned live recording. The sandbox sends events, never owns trace files.
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::ExitStatusExt;
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
    stream: Option<Interpretation>,
    web_search: bool,
    limit: Option<limits::Limit>,
}

impl Session {
    fn record(&mut self, raw: &str) -> Result<()> {
        let stream = self.stream.as_mut().context("recording already ended")?;
        let events = stream.decode(raw).events;
        self.limit = self.limit.take().or_else(|| stream.plan_limit().cloned());
        self.web_search |= super::sessions::web_search(&events);
        self.trace.record(raw, &events)?;
        Ok(())
    }

    fn poll_limits(&mut self) -> Result<()> {
        let records = self
            .stream
            .as_mut()
            .map(|stream| stream.limit_records())
            .unwrap_or_default();
        for raw in records {
            self.record(&raw)?;
        }
        Ok(())
    }

    fn finish(&mut self, error: Option<&str>) -> Result<()> {
        self.poll_limits()?;
        let Some(stream) = self.stream.take() else {
            return Ok(());
        };
        self.limit = self.limit.take().or_else(|| stream.plan_limit().cloned());
        let completion =
            stream.finish(Ok(std::process::ExitStatus::from_raw(if error.is_some() {
                1 << 8
            } else {
                0
            })));
        if let Some(report) = completion.report {
            for record in report.retained_records {
                self.web_search |= super::sessions::web_search(&record.events);
                self.trace.record(&record.raw, &record.events)?;
            }
            self.web_search |= super::sessions::web_search(&report.events);
            self.trace.reconcile(&report.events)?;
        }
        let error = error
            .map(str::to_owned)
            .or_else(|| completion.outcome.err().map(|error| error.to_string()));
        self.trace.finish(error.as_deref())?;
        Ok(())
    }
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
                            session.finish(Some("recording connection closed"))?;
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
                            session.record(&raw)?;
                        }
                        Frame::End { error } => {
                            session.finish(error.as_deref())?;
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
                    let prompt = start
                        .arguments
                        .windows(2)
                        .find(|pair| matches!(pair[0].as_str(), "-p" | "--prompt"))
                        .map(|pair| pair[1].clone())
                        .or_else(|| {
                            start
                                .arguments
                                .iter()
                                .find_map(|arg| arg.strip_prefix("--prompt=").map(str::to_owned))
                        })
                        .unwrap_or_else(|| crate::args::shell_command(start.arguments.clone()));
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
                    let mut stream = harness.adapter().interpretation(&self.repository, &prompt);
                    stream.set_home(&self.home);
                    connection.session = Some(Session {
                        trace,
                        stream: Some(stream),
                        web_search: false,
                        limit: None,
                    });
                }
            }
            if let Some(session) = &mut connection.session {
                session.poll_limits()?;
                self.web_search |= session.web_search;
                self.limit = self.limit.take().or_else(|| session.limit.clone());
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
                    session.finish(Some("Trial ended during recording"))?;
                    self.web_search |= session.web_search;
                    self.limit = self.limit.take().or_else(|| session.limit.clone());
                }
                connection.ended = true;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod spec_2191_live_retained_repro {
    use super::*;
    use serde_json::json;
    use std::io::{Cursor, Write};

    #[test]
    fn live_kimi_preserves_usage_from_its_trial_home() {
        let root = tempfile::tempdir().unwrap();
        for directory in [
            "session",
            "repository",
            "home/.kimi-code/sessions/ws/ses_7f3c2a/agents/main",
        ] {
            fs::create_dir_all(root.path().join(directory)).unwrap();
        }
        fs::write(
            root.path().join("trace-context.json"),
            serde_json::to_vec(&TraceContext::of_session(root.path())).unwrap(),
        )
        .unwrap();
        fs::write(
            root.path()
                .join("home/.kimi-code/sessions/ws/ses_7f3c2a/agents/main/wire.jsonl"),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/kimi-wire.jsonl"
            )),
        )
        .unwrap();
        let mut recorder = Recorder::start(root.path()).unwrap();
        let mut socket = UnixStream::connect(recorder.socket()).unwrap();
        // Buffer the frames so formatting's small writes cannot fill the
        // socket before this single-threaded test starts the host reader.
        let mut frames = Vec::new();
        writeln!(
            frames,
            "{}",
            json!({"role":"implement", "harness":"kimi", "arguments":["-p", "test"]})
        )
        .unwrap();
        for raw in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/normalized-kimi.jsonl"
        ))
        .lines()
        {
            writeln!(frames, "{}", json!({"raw": raw})).unwrap();
        }
        writeln!(frames, "{}", json!({"error": null})).unwrap();
        socket.write_all(&frames).unwrap();
        drop(socket);
        recorder.finish().unwrap();
        assert_eq!(recorder.sessions, 1);
        let summary = crate::session::usage::from_spans(Cursor::new(
            fs::read(root.path().join("session/custom-live-1.spans.jsonl")).unwrap(),
        ))
        .unwrap();
        let kimi = summary
            .models
            .iter()
            .find(|model| model.model.as_deref() == Some("kimi-k3.1-pro"));
        assert!(
            kimi.is_some(),
            "a recorded Kimi session lost its native usage: {summary:?}"
        );
        let tokens = kimi.unwrap().measurement;
        assert_eq!(tokens.input_tokens, Some(13_204));
        assert_eq!(tokens.output_tokens, Some(275));
    }
}
