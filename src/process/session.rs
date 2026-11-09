//! Final cleanup of work that escaped a Harness's process-tree snapshot.

use std::collections::BTreeSet;
use std::io;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::progress;

pub(crate) const MARKER: &str = "THIRDSHIFT_SESSION";

/// Explicit overrides fence nested and parallel sessions from inherited IDs.
pub(crate) fn mark(command: &mut Command) -> String {
    let marker = marker(command).unwrap_or_else(|| format!("{:032x}", rand::random::<u128>()));
    command.env(MARKER, &marker);
    marker
}

fn marker(command: &Command) -> Option<String> {
    command.get_envs().find_map(|(name, value)| {
        (name == MARKER)
            .then_some(value)
            .flatten()
            .map(|value| value.to_string_lossy().into_owned())
    })
}

pub(super) struct Session {
    marker: String,
    #[cfg(test)]
    fault: super::faults::Fault,
}

impl Session {
    pub fn of(command: &Command) -> Option<Self> {
        Some(Self {
            marker: marker(command)?,
            #[cfg(test)]
            fault: super::faults::Fault::from(command),
        })
    }

    pub fn sweep(self, scope: Option<&crate::scope::Scope>) {
        let mut warning = None;
        let mut killed = BTreeSet::new();
        let result = (|| {
            #[cfg(test)]
            self.fault.fire("session-sweep")?;
            let deadline = Instant::now() + super::tree::STOP_GRACE;
            let mut killed_at = None;
            loop {
                let pids = match scope {
                    Some(scope) => scope.processes()?,
                    None => marked_processes(&self.marker, &mut warning)?,
                };
                if pids.is_empty() {
                    break;
                }
                if killed_at.is_some_and(|at: Instant| at.elapsed() >= Duration::from_secs(2)) {
                    bail!("{} processes remain after SIGKILL", pids.len());
                }
                let signal = if Instant::now() >= deadline {
                    libc::SIGKILL
                } else {
                    libc::SIGTERM
                };
                if let Some(scope) = scope {
                    scope.signal(signal)?;
                    killed.extend(pids);
                } else {
                    for pid in pids {
                        if signal == libc::SIGTERM && killed.contains(&pid) {
                            continue;
                        }
                        match super::signal(pid, signal) {
                            Ok(()) => {
                                killed.insert(pid);
                            }
                            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => {}
                            Err(error) => {
                                warning.get_or_insert(error.into());
                            }
                        }
                    }
                }
                if signal == libc::SIGKILL {
                    killed_at.get_or_insert_with(Instant::now);
                }
                thread::sleep(Duration::from_millis(100));
            }
            Ok(())
        })();
        if let Err(error) = result {
            warning.get_or_insert(error);
        }
        if !killed.is_empty() {
            progress::step(format_args!(
                "session {}: killed {} surviving processes",
                self.marker,
                killed.len()
            ));
        }
        if let Some(error) = warning {
            progress::step(format_args!(
                "warning: could not fully sweep session {}: {error:#}",
                self.marker
            ));
        }
    }
}

fn marked_processes(marker: &str, warning: &mut Option<anyhow::Error>) -> Result<Vec<libc::pid_t>> {
    let output = Command::new("ps")
        .args(["-A", "-o", "pid=", "-o", "uid=", "-o", "stat="])
        .output()
        .context("could not list processes for the session sweep")?;
    if !output.status.success() {
        bail!(
            "could not list processes for the session sweep: {}",
            output.status
        );
    }
    // SAFETY: geteuid has no arguments or memory preconditions.
    let uid = unsafe { libc::geteuid() };
    let expected = format!("{MARKER}={marker}");
    let mut found = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut fields = line.split_whitespace();
        let pid: libc::pid_t = fields.next().context("missing process ID")?.parse()?;
        let owner: libc::uid_t = fields.next().context("missing process owner")?.parse()?;
        let state = fields.next().context("missing process state")?;
        if owner != uid
            || state.starts_with('Z')
            || pid <= 0
            || pid == std::process::id() as libc::pid_t
        {
            continue;
        }
        match environment(pid) {
            Ok(bytes)
                if bytes
                    .split(|&byte| byte == 0)
                    .any(|entry| entry == expected.as_bytes()) =>
            {
                found.push(pid);
            }
            Ok(_) => {}
            Err(error) if matches!(error.raw_os_error(), Some(libc::ESRCH | libc::ENOENT)) => {}
            Err(error) => {
                warning.get_or_insert_with(|| {
                    anyhow::anyhow!("could not read process {pid}'s environment: {error}")
                });
            }
        }
    }
    Ok(found)
}

fn environment(pid: libc::pid_t) -> io::Result<Vec<u8>> {
    std::fs::read(format!("/proc/{pid}/environ"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_that_exits_before_its_environment_is_read_does_not_warn() {
        const FIXTURE: &str = "THIRDSHIFT_VANISHED_PROCESS_FIXTURE";
        if std::env::var_os(FIXTURE).is_some() {
            let mut command = Command::new("unused");
            mark(&mut command);
            Session::of(&command).unwrap().sweep(None);
            return;
        }

        let directory = tempfile::tempdir().unwrap();
        let ps = directory.path().join("ps");
        // Model a process disappearing after ps listed it. This PID is above
        // the supported systems' PID limits, so it cannot name a live process.
        // SAFETY: geteuid has no arguments or memory preconditions.
        let uid = unsafe { libc::geteuid() };
        crate::test_support::write_executable(
            &ps,
            &format!("#!/bin/sh\nprintf '2147483647 {uid} S\\n'\n"),
        );
        crate::test_support::assert_test_without_warnings(
            "process::session::tests::a_process_that_exits_before_its_environment_is_read_does_not_warn",
            |command| {
                command.env(FIXTURE, "1").env("PATH", directory.path());
            },
        );
    }
}
