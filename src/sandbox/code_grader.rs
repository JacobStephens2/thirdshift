//! Own a code Grader's private packet, snapshot and supervised check lifecycle.

use std::ffi::OsString;
use std::fs;
use std::io::{Read, Seek};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::{
    Binds, Environment, Launch, SHELL, Unavailable, base_argv, disabled, find_bwrap,
    toolchains_into,
};
use crate::config;
use crate::process::{self, Control, Interruption};
use crate::scope::Caps;
use crate::trial::Reports;

#[derive(Serialize, Deserialize)]
pub(crate) struct Check {
    pub command: Option<Vec<String>>,
    pub passed: bool,
    pub reason: String,
}

/// Run ordered checks against one private snapshot. Each process and its
/// scope/status finish before the packet can be released, including on errors.
pub(crate) fn execute(
    repository: &Path,
    reports: &Reports,
    commands: &[Vec<String>],
    timeout: Duration,
    caps: &Caps,
) -> Result<Vec<Check>> {
    let packet = tempfile::Builder::new().prefix("grader-").tempdir()?;
    // Bind sources remain visible in /proc/self/mountinfo. A neutral
    // destination alone cannot hide the original Trial's Arm pathname.
    let snapshot = packet.path().join("repository");
    copy_repository(repository, &snapshot)?;
    let reports = reports.read_and_blind()?;
    for (text, name) in [
        (reports.standards, "standards.md"),
        (reports.spec, "spec.md"),
    ] {
        if let Some(text) = text {
            fs::write(packet.path().join(name), text)?;
        }
    }
    let mut checks = Vec::new();
    for command in commands {
        let mut child = launch(&snapshot, packet.path(), command, caps)?;
        let result = process::output_until(
            &mut child.command,
            None,
            Control {
                name: "code Grader",
                interruption: Interruption::Ordinary,
                stop: &|child| process::stop(child, &[libc::SIGTERM, libc::SIGKILL]),
            },
            Instant::now() + timeout,
            &|| {
                child.scope.sample();
                Ok(())
            },
        );
        crate::interrupt::check()?;
        if !result.as_ref().is_ok_and(|result| result.status.success()) && child.scope.oom_killed()
        {
            bail!("code Grader hit its memory cap; raise trial.memory_max to retry");
        }
        child.check_started(&result)?;
        let (passed, reason) = match result {
            Ok(result) => (
                result.status.success(),
                format!(
                    "{}\n{}{}",
                    result.status,
                    String::from_utf8_lossy(&result.stdout),
                    String::from_utf8_lossy(&result.stderr)
                ),
            ),
            Err(error) => (false, format!("{error:#}")),
        };
        checks.push(Check {
            command: Some(command.clone()),
            passed,
            reason,
        });
    }
    Ok(checks)
}

fn copy_repository(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_repository(&entry.path(), &target)?;
        } else if kind.is_symlink() {
            std::os::unix::fs::symlink(fs::read_link(entry.path())?, &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target)?;
        }
        // Skip sockets, FIFOs, devices and other special files: Trial
        // sessions leave them behind, and no check can use them once the
        // Trial's servers are gone. Copying them would fail the whole
        // Experiment over one Trial's ordinary leftovers.
    }
    Ok(())
}

/// Code Graders need no login or provider. Bind only their report packet and
/// prepared repository, at neutral paths, and leave their network offline.
fn launch(repository: &Path, packet: &Path, args: &[String], caps: &Caps) -> Result<LaunchedCheck> {
    let packet_path = if disabled() {
        packet
    } else {
        Path::new("/sandbox/packet")
    };
    let mut packet_env = vec![(OsString::from("THIRDSHIFT_TRIAL_LOG"), OsString::new())];
    for (name, file) in [
        ("THIRDSHIFT_TRIAL_STANDARDS_REPORT", "standards.md"),
        ("THIRDSHIFT_TRIAL_SPEC_REPORT", "spec.md"),
    ] {
        packet_env.push((
            name.into(),
            if packet.join(file).is_file() {
                packet_path.join(file).into_os_string()
            } else {
                OsString::new()
            },
        ));
    }
    // An unlinked status file stays outside the packet and the sandbox. Only
    // bwrap writes it; the check's stdout and stderr cannot claim startup.
    let status = (!disabled()).then(tempfile::tempfile).transpose()?;
    let (mut command, scope) = if disabled() {
        let launched =
            crate::scope::grader_command(Path::new(&args[0]), &args[1..], repository, caps)?;
        (launched.command, launched.scope)
    } else {
        if find_bwrap().is_none() {
            return Err(anyhow::Error::new(Unavailable {
                diagnostic: "bwrap is not on PATH".into(),
            }));
        }
        let mut binds = Binds::default();
        binds.read_write(repository, Path::new("/sandbox/repository"));
        binds.read_write(packet, Path::new("/sandbox/packet"));
        // Spec-review repositories have a local origin whose absolute host
        // path identifies the Arm. Keep Git usable without exposing that path.
        // The snapshot holds Trial-controlled contents: a symlinked `.git`
        // or config would redirect this host-side rewrite outside it, so
        // only touch a real directory holding a real (or missing) config.
        let git_dir = repository.join(".git");
        let git_config = git_dir.join("config");
        let confined = fs::symlink_metadata(&git_dir).is_ok_and(|kind| kind.is_dir())
            && match fs::symlink_metadata(&git_config) {
                Ok(kind) => kind.is_file(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
                Err(_) => false,
            };
        if confined && git_dir.join("eval-origin.git").is_dir() {
            crate::git::Git::new(repository).run(&[
                "config",
                "--file",
                git_config.to_str().context("invalid Grader config path")?,
                "remote.origin.url",
                "/sandbox/repository/.git/eval-origin.git",
            ])?;
        }
        let mut environment = Environment::default();
        toolchains_into(&config::home()?, &mut binds, &mut environment);
        environment.set_str("HOME", "/sandbox/home");
        let environment = environment.finish(&[]);
        let mut argv = base_argv();
        // Clear only the sandbox's environment. The outer memory scope
        // still needs its host environment to reach the systemd user bus.
        argv.push("--clearenv".into());
        for (name, value) in environment.iter().chain(&packet_env) {
            argv.extend([OsString::from("--setenv"), name.clone(), value.clone()]);
        }
        binds.argv(&mut argv);
        let status_fd = status
            .as_ref()
            .expect("sandbox has a status file")
            .as_raw_fd();
        argv.extend([
            OsString::from("--json-status-fd"),
            status_fd.to_string().into(),
        ]);
        argv.extend(
            ["--tmpfs", "/sandbox/home", "--chdir", "/sandbox/repository"].map(OsString::from),
        );
        let program = Path::new(&args[0]);
        let program = program
            .strip_prefix(repository)
            .map(|relative| Path::new("/sandbox/repository").join(relative))
            .unwrap_or_else(|_| program.to_owned());
        // The shell distinguishes a bad check command from a bwrap startup
        // error: exec failure here is an ordinary failed mechanical check.
        let launch = Launch {
            argv,
            env: Vec::new(),
            program,
            prefix: [SHELL, "-c", "exec \"$0\" \"$@\""]
                .map(OsString::from)
                .to_vec(),
            stderr: None,
        };
        let launched = launch.scoped_command(&args[1..], repository, caps)?;
        let (mut command, scope) = (launched.command, launched.scope);
        // SAFETY: fcntl is async-signal-safe. The status file remains owned
        // by LaunchedCheck until its child has ended; no allocation occurs here.
        unsafe {
            command.pre_exec(move || {
                if libc::fcntl(status_fd, libc::F_SETFD, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        (command, scope)
    };
    if disabled() {
        command.envs(packet_env);
    }
    command.current_dir(repository);
    Ok(LaunchedCheck {
        command,
        status,
        scope,
    })
}

/// A Code Grader's command and launcher-owned startup evidence travel together.
struct LaunchedCheck {
    command: Command,
    status: Option<fs::File>,
    scope: crate::scope::Scope,
}

impl LaunchedCheck {
    /// Only the launcher's status decides whether a check started. A check
    /// can fail normally even when its own diagnostic mentions bwrap.
    fn check_started(&self, result: &Result<std::process::Output>) -> Result<()> {
        let Some(status) = &self.status else {
            return Ok(());
        };
        let diagnostic = match result {
            Err(error) if error.is::<crate::process::NotStarted>() => format!("{error:#}"),
            Ok(output) => {
                let mut status = status.try_clone()?;
                status.rewind()?;
                let mut records = String::new();
                status.read_to_string(&mut records)?;
                let started = records.lines().any(|line| {
                    serde_json::from_str::<serde_json::Value>(line)
                        .ok()
                        .is_some_and(|record| record["child-pid"].as_u64().is_some())
                });
                if started {
                    return Ok(());
                }
                String::from_utf8_lossy(&output.stderr).into_owned()
            }
            _ => return Ok(()),
        };
        Err(anyhow::Error::new(Unavailable { diagnostic }))
    }
}
