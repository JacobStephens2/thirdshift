//! Short private analysis sessions share sandbox, caps, native quota location
//! and recording. Their callers own the input packet and response validation.
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Result, bail, ensure};

/// A session output is a proposal or a Grader response, not a host file.
pub(crate) const SESSION_OUTPUT_MAX: u64 = 8 * 1024 * 1024;

use super::recording::{Input, Recording};
use crate::harness::Choice;
use crate::scope::Caps;

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
    let adapter = choice.harness.adapter();
    let invocation = adapter.trial_session(choice, None, prompt);
    let sandbox = crate::sandbox::for_session(choice, binary, workspace, artifacts)?;
    let launch = sandbox.as_ref().map(|prepared| prepared.launch());
    // The scope encloses the sandbox and all tools. Native quota records
    // live beside the sandbox's login, rather than in the host's home.
    let mut launched = match &launch {
        Some(launch) => launch.scoped_command(&invocation.args, workspace, caps)?,
        None => crate::scope::grader_command(binary, &invocation.args, workspace, caps)?,
    };
    let mut interpretation = adapter.interpretation(workspace, prompt);
    if let Some(launch) = &launch
        && let Some((_, home)) = launch.env.iter().find(|(name, _)| name == "HOME")
    {
        interpretation.set_home(Path::new(home));
    }
    launched
        .command
        .stderr(fs::File::create(artifacts.join("stderr"))?);
    for (name, path) in environment {
        launched.command.env(name, path);
    }
    let scope = launched.scope;
    fs::write(artifacts.join("scope"), scope.unit())?;
    let ended = Recording::for_trial().run(
        kind,
        choice,
        &mut launched.command,
        Input {
            prompt,
            stdin: invocation.stdin.as_deref(),
            deadline: Some(Instant::now() + timeout),
            private: true,
        },
        &artifacts.join("stdout.jsonl"),
        interpretation,
    );
    crate::interrupt::check()?;
    let ended = ended.map_err(|error| {
        // Preserve the shared stop signal so Scoring and Experiments apply
        // their own pause policy. Other diagnostics stay in private artifacts.
        if error.is::<super::limits::Reached>() {
            error
        } else if scope.oom_killed() {
            anyhow::anyhow!("{kind} hit its memory cap ({}); see private Grader artifacts, and raise the trial.memory_max User config to retry", crate::scope::OOM_KILL_CAUSE)
        } else {
            anyhow::anyhow!("{kind} session failed; see private Grader artifacts")
        }
    })?;
    ensure!(
        ended.killed.is_empty(),
        "{kind} left unfinished background work"
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
mod tests {
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
