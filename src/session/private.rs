//! Private Sessions own sandbox preparation, launch, recording and completion.
//! Callers choose their Grader and own packet preparation and response validation.
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};

/// A session output is a proposal or a Grader response, not a host file.
pub(crate) const SESSION_OUTPUT_MAX: u64 = 8 * 1024 * 1024;

use super::recording::{Input, Policy, Recording};
use crate::harness::Choice;
use crate::issue::Repo;
use crate::process::Invocation;
use crate::scope::Caps;
use crate::{logs, sandbox};

pub(crate) struct Request<'a> {
    pub kind: &'a str,
    pub choice: &'a Choice,
    pub binary: &'a Path,
    pub workspace: &'a Path,
    pub artifacts: &'a Path,
    pub prompt: &'a str,
    pub timeout: Duration,
    pub caps: &'a Caps,
    pub environment: &'a [(&'a str, &'a Path)],
}

pub(crate) fn run(request: Request<'_>) -> Result<()> {
    crate::resources::wait()?;
    let Request {
        kind,
        choice,
        binary,
        workspace,
        artifacts,
        prompt,
        timeout,
        caps,
        environment,
    } = request;
    execute(Session {
        kind,
        choice,
        binary,
        workspace,
        artifacts,
        prompt,
        profile: Profile::Auxiliary {
            timeout,
            caps,
            environment,
        },
    })
}

/// Run the independent Grader selected by Scoring. Its temporary sandbox
/// output lives beside the packet, through supervision and recording only.
pub(crate) fn run_scoring(
    choice: &Choice,
    packet: &Path,
    prompt: &str,
    repo: &Repo,
    ordinal: usize,
) -> Result<()> {
    let output = tempfile::Builder::new()
        .prefix("grader-")
        .tempdir_in(packet.parent().context("packet has no parent")?)?;
    let binary: std::path::PathBuf = if sandbox::disabled() {
        choice.harness.name().into()
    } else {
        sandbox::resolve_binary(choice.harness.name())?
    };
    execute(Session {
        kind: &super::Kind::ScoringGrader(ordinal).token(),
        choice,
        binary: &binary,
        workspace: packet,
        artifacts: output.path(),
        prompt,
        profile: Profile::Scoring { repo, ordinal },
    })
}

enum Profile<'a> {
    Auxiliary {
        timeout: Duration,
        caps: &'a Caps,
        environment: &'a [(&'a str, &'a Path)],
    },
    Scoring {
        repo: &'a Repo,
        ordinal: usize,
    },
}

struct Session<'a> {
    kind: &'a str,
    choice: &'a Choice,
    binary: &'a Path,
    workspace: &'a Path,
    artifacts: &'a Path,
    prompt: &'a str,
    profile: Profile<'a>,
}

fn execute(session: Session<'_>) -> Result<()> {
    let Session {
        kind,
        choice,
        binary,
        workspace,
        artifacts,
        prompt,
        profile,
    } = session;
    let adapter = choice.harness.adapter();
    // Both guards outlive recording, supervision and failure classification.
    let sandbox = sandbox::for_session(choice, binary, workspace, artifacts)?;
    let environment = match &profile {
        Profile::Auxiliary { environment, .. } => *environment,
        Profile::Scoring { .. } => &[],
    };
    let launch = sandbox.as_ref().map(|prepared| {
        let mut launch = prepared.launch();
        for (name, path) in environment {
            launch.set_env(name, path);
        }
        launch
    });
    let (invocation, mut launched, recording, log, mut input) = match &profile {
        Profile::Auxiliary { timeout, caps, .. } => {
            let invocation = adapter.trial_session(choice, None, prompt);
            // The scope encloses the sandbox and all tools. Native quota
            // records live beside the launch HOME, not the host's history.
            let mut launched = match &launch {
                Some(launch) => launch.scoped_command(&invocation.args, workspace, caps)?,
                None => crate::scope::grader_command(binary, &invocation.args, workspace, caps)?,
            };
            launched
                .command()
                .stderr(File::create(artifacts.join("stderr"))?);
            fs::write(artifacts.join("scope"), launched.unit())?;
            (
                invocation,
                Box::new(launched) as Box<dyn Invocation>,
                Recording::for_trial(),
                artifacts.join("stdout.jsonl"),
                Input {
                    deadline: Some(Instant::now() + *timeout),
                    ..Input::default()
                },
            )
        }
        Profile::Scoring { repo, ordinal } => {
            let invocation = adapter.session(choice, None, prompt);
            let command = match &launch {
                Some(launch) => launch.command(&invocation.args)?,
                None => {
                    let mut command = Command::new(adapter.name());
                    command.args(&invocation.args);
                    command
                }
            };
            let recording = Recording::capture();
            let log = recording.session_path(&logs::session_dir(repo), "scoring", kind)?;
            logs::session(
                super::Logged::of(super::Kind::ScoringGrader(*ordinal)),
                &log,
            );
            (
                invocation,
                Box::new(command) as Box<dyn Invocation>,
                recording,
                log,
                // Scores need only the stream; retained exports can read
                // production history or start a CLI outside the sandbox.
                Input {
                    policy: Policy::StreamOnly,
                    stop_at_limit: true,
                    ..Input::default()
                },
            )
        }
    };
    launched.command().current_dir(workspace);
    for (name, path) in environment {
        launched.command().env(name, path);
    }
    input.prompt = prompt;
    input.stdin = invocation.stdin.as_deref();
    input.private = true;
    let ended = recording.run(kind, choice, launched.as_mut(), input, &log);
    if matches!(profile, Profile::Auxiliary { .. }) {
        crate::interrupt::check()?;
    }
    let ended = ended.map_err(|error| match profile {
        Profile::Scoring { .. } => sandbox.as_ref()
            .and_then(|sandbox| sandbox.startup_failure(&error))
            .map(anyhow::Error::new)
            .unwrap_or(error),
        Profile::Auxiliary { .. } => {
            // Preserve typed plan-limit stops; diagnose memory only after
            // supervision has finished observing the complete scope lifetime.
            if error.is::<super::limits::Reached>() {
                error
            } else if launched.parts().1.is_some_and(|scope| scope.oom_killed()) {
                anyhow::anyhow!("{kind} hit its memory cap ({}); see private Grader artifacts, and raise the trial.memory_max User config to retry", crate::scope::OOM_KILL_CAUSE)
            } else {
                anyhow::anyhow!("{kind} session failed; see private Grader artifacts")
            }
        }
    })?;
    let label = match profile {
        Profile::Scoring { .. } => "Grader",
        Profile::Auxiliary { .. } => kind,
    };
    ensure!(
        ended.killed.is_empty(),
        "{label} left unfinished background work"
    );
    Ok(())
}

/// Read a file the session was asked to write. Open it without following a
/// symlink, and only then accept a regular file that has no other name and
/// fits in `max_bytes`. The host read happens after the session exits, so a
/// symlink would be resolved on the host, outside the sandbox.
pub(crate) fn read_session_file(path: &Path, max_bytes: u64) -> Result<Vec<u8>> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.raw_os_error() == Some(libc::ELOOP) => {
            bail!("{} is not a regular file", path.display())
        }
        Err(error) => return Err(error.into()),
    };
    session_file(path, &file, max_bytes)?;
    let mut bytes = Vec::new();
    (&file).take(max_bytes + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        bail!("{} exceeds {max_bytes} bytes", path.display());
    }
    Ok(bytes)
}

pub(crate) fn is_missing_file(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
}

fn session_file(path: &Path, file: &File, max_bytes: u64) -> Result<()> {
    let pinned = file.metadata()?;
    let current = fs::symlink_metadata(path)?;
    if !pinned.is_file()
        || !current.is_file()
        || pinned.nlink() != 1
        || current.dev() != pinned.dev()
        || current.ino() != pinned.ino()
    {
        bail!("{} is not a regular file", path.display());
    }
    if pinned.len() > max_bytes {
        bail!("{} exceeds {max_bytes} bytes", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod file_tests {
    use std::fs::{self, File};
    use std::io::Write;
    use std::os::unix::fs::symlink;

    use super::*;

    #[test]
    fn a_regular_session_file_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proposal.json");
        fs::write(&path, b"{\"commit\":null}").unwrap();
        assert_eq!(read_session_file(&path, 64).unwrap(), b"{\"commit\":null}");
    }

    #[test]
    fn a_symlink_is_not_a_session_file() {
        let dir = tempfile::tempdir().unwrap();
        let host = dir.path().join("host.json");
        let path = dir.path().join("proposal.json");
        fs::write(&host, b"{\"commit\":null,\"reason\":\"host file\"}").unwrap();
        symlink(&host, &path).unwrap();
        let error = read_session_file(&path, 1024).unwrap_err();
        assert!(
            format!("{error:#}").contains("not a regular file"),
            "{error:#}"
        );
        assert!(is_missing_file(
            &read_session_file(&dir.path().join("absent"), 64).unwrap_err()
        ));
    }

    #[test]
    fn a_hard_link_fifo_or_oversize_file_is_not_a_session_file() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("original");
        let linked = dir.path().join("linked");
        fs::write(&original, b"same inode").unwrap();
        fs::hard_link(&original, &linked).unwrap();
        assert!(
            format!("{:#}", read_session_file(&linked, 1024).unwrap_err())
                .contains("not a regular file")
        );

        let fifo = dir.path().join("fifo");
        let name = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        // SAFETY: `name` is a NUL-terminated path in this temp directory.
        // mkfifo only creates a fifo there.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(
            format!("{:#}", read_session_file(&fifo, 1024).unwrap_err())
                .contains("not a regular file")
        );

        let big = dir.path().join("big");
        let mut file = File::create(&big).unwrap();
        file.write_all(&[b'x'; 8]).unwrap();
        assert!(
            format!("{:#}", read_session_file(&big, 4).unwrap_err()).contains("exceeds 4 bytes")
        );
    }
}
