//! Host-owned live recording. The sandbox sends events, never owns trace files.
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

use crate::harness::{Choice, Harness, interpretation::Interpretation};
use crate::logs::trace::Context as TraceContext;
use crate::session::{limits, trace::Trace};

/// Bytes of unfinished frames the host buffers across every connection.
const MAX_PENDING_BYTES: usize = 64 << 20;
/// Concurrently open record connections, each a host descriptor.
const MAX_OPEN_CONNECTIONS: usize = 64;
/// Time one poll may spend reading, so supervision regains control to
/// enforce the Trial timeout and quota pauses.
const POLL_TIME: std::time::Duration = std::time::Duration::from_millis(50);
/// Polls that drain the sockets once the Trial has ended.
const FINISH_PASSES: usize = 256;

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
        self.pump().map(drop)
    }

    /// Accept and read what is waiting, within the bounds below, returning
    /// whether anything arrived. The sandbox writes this socket, but its
    /// frames, connections and the host's time stay within the Trial's limits.
    fn pump(&mut self) -> Result<bool> {
        let mut progressed = false;
        let deadline = std::time::Instant::now() + POLL_TIME;
        loop {
            match self.listener.accept() {
                Ok((socket, _)) => {
                    ensure!(
                        self.connections.len() < MAX_OPEN_CONNECTIONS,
                        "record_command opened more than {MAX_OPEN_CONNECTIONS} connections at once"
                    );
                    socket.set_nonblocking(true)?;
                    self.connections.push(Connection {
                        reader: BufReader::new(socket),
                        partial: Vec::new(),
                        session: None,
                        ended: false,
                    });
                    progressed = true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error.into()),
            }
        }
        let mut pending: usize = self
            .connections
            .iter()
            .map(|connection| connection.partial.len())
            .sum();
        for connection in &mut self.connections {
            while !connection.ended {
                ensure!(
                    pending < MAX_PENDING_BYTES,
                    "record_command frames exceed {MAX_PENDING_BYTES} bytes"
                );
                if std::time::Instant::now() >= deadline {
                    break;
                }
                let before = connection.partial.len();
                let read = (&mut connection.reader)
                    .take(u64::try_from(MAX_PENDING_BYTES - pending)?)
                    .read_until(b'\n', &mut connection.partial);
                let arrived = connection.partial.len() - before;
                pending += arrived;
                progressed |= arrived > 0;
                match read {
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
                pending -= connection.partial.len();
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
        self.connections.retain(|connection| !connection.ended);
        Ok(progressed)
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
        // Supervision has ended the sandbox: drain what its writers left.
        for _ in 0..FINISH_PASSES {
            if !self.pump()? {
                break;
            }
        }
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
mod tests {
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

    fn recorder() -> (tempfile::TempDir, Recorder) {
        let root = tempfile::tempdir().unwrap();
        for directory in ["session", "repository", "home"] {
            fs::create_dir_all(root.path().join(directory)).unwrap();
        }
        fs::write(
            root.path().join("trace-context.json"),
            serde_json::to_vec(&TraceContext::of_session(root.path())).unwrap(),
        )
        .unwrap();
        let recorder = Recorder::start(root.path()).unwrap();
        (root, recorder)
    }

    /// Sandboxed code reaches this socket; the host process sits outside the
    /// Trial's memory cap, so one endless frame must not grow it unbounded.
    #[test]
    fn an_endless_record_frame_fails_the_recording_instead_of_buffering_on_the_host() {
        let (_root, mut recorder) = recorder();
        let mut socket = UnixStream::connect(recorder.socket()).unwrap();
        let writer = std::thread::spawn(move || {
            let chunk = vec![b'a'; 1 << 20];
            for _ in 0..80 {
                if socket.write_all(&chunk).is_err() {
                    return;
                }
            }
        });
        let mut failed = false;
        while !writer.is_finished() {
            if recorder.poll().is_err() {
                failed = true;
                break;
            }
            let buffered: usize = recorder
                .connections
                .iter()
                .map(|connection| connection.partial.len())
                .sum();
            assert!(buffered <= MAX_PENDING_BYTES, "{buffered} bytes buffered");
        }
        failed = failed || recorder.poll().is_err();
        drop(recorder);
        writer.join().unwrap();
        assert!(failed, "an endless frame was buffered without a bound");
    }

    /// Every record connection holds a descriptor in the host process that
    /// runs concurrent Trials; closed ones must release it.
    #[test]
    fn closed_record_connections_release_their_host_descriptors() {
        let (_root, mut recorder) = recorder();
        for _ in 0..MAX_OPEN_CONNECTIONS * 2 {
            drop(UnixStream::connect(recorder.socket()).unwrap());
            recorder.poll().unwrap();
        }
        assert_eq!(recorder.connections.len(), 0);
    }

    #[test]
    fn too_many_open_record_connections_fail_the_recording() {
        let (_root, mut recorder) = recorder();
        let sockets: Vec<_> = (0..=MAX_OPEN_CONNECTIONS)
            .map(|_| UnixStream::connect(recorder.socket()).unwrap())
            .collect();
        assert!(
            recorder.poll().is_err(),
            "unbounded open record connections"
        );
        assert!(recorder.connections.len() <= MAX_OPEN_CONNECTIONS);
        drop(sockets);
    }

    /// Supervision enforces the Trial timeout and quota pauses between polls,
    /// so a flood of valid frames must not keep one poll running.
    #[test]
    fn a_flood_of_record_frames_returns_control_to_supervision() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let (_root, mut recorder) = recorder();
        let mut socket = UnixStream::connect(recorder.socket()).unwrap();
        writeln!(
            socket,
            "{}",
            json!({"role": "implement", "harness": "claude", "arguments": ["-p", "x"]})
        )
        .unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let writer = std::thread::spawn({
            let stop = Arc::clone(&stop);
            move || {
                let frame = format!("{}\n", json!({"raw": "x\n"}));
                let chunk = frame.repeat((64 << 10) / frame.len());
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
                while !stop.load(Ordering::Relaxed) && std::time::Instant::now() < deadline {
                    if socket.write_all(chunk.as_bytes()).is_err() {
                        return;
                    }
                }
            }
        });
        // Let the frames queue before the poll starts.
        std::thread::sleep(std::time::Duration::from_millis(100));
        let started = std::time::Instant::now();
        let polled = recorder.poll();
        let elapsed = started.elapsed();
        stop.store(true, Ordering::Relaxed);
        drop(recorder);
        writer.join().unwrap();
        polled.unwrap();
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "one poll held supervision for {elapsed:?}"
        );
    }
}
