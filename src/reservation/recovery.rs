//! A reservation expires only on local proof of death, never on age.
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{git::Git, issue::Repo, progress};

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub(super) struct Owner {
    pub host: Option<String>,
    pub launch: Option<String>,
    pub process: Option<u32>,
    pub started: Option<String>,
}

impl Owner {
    fn parse(message: &str) -> Self {
        // Duplicated or malformed fields cannot establish ownership.
        let field = |name: &str| {
            let prefix = format!("{name}: ");
            let mut values = message
                .lines()
                .filter_map(|line| line.strip_prefix(&prefix));
            let value = values.next()?.trim();
            (values.next().is_none() && !value.is_empty()).then(|| value.to_owned())
        };
        Self {
            host: field("Host"),
            launch: field("Launch"),
            process: field("Process")
                .and_then(|pid| pid.parse().ok())
                .filter(|pid| *pid > 0 && *pid <= i32::MAX as u32),
            started: field("Started"),
        }
    }

    pub(super) fn started(&self) -> Option<DateTime<Utc>> {
        let value = self.started.as_deref()?;
        DateTime::parse_from_rfc3339(value)
            .ok()
            .map(|time| time.with_timezone(&Utc))
            .or_else(|| {
                chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f UTC")
                    .ok()
                    .map(|time| time.and_utc())
            })
    }

    pub(super) fn testable(&self, host: Option<&str>) -> bool {
        host.is_some_and(|host| host != "unknown" && self.host.as_deref() == Some(host))
            && self.process.is_some()
            && self.started().is_some()
    }

    fn orphaned(&self, processes: &impl Processes) -> bool {
        if !self.testable(processes.host().as_deref()) {
            return false;
        }
        let Some(started) = self.started() else {
            return false;
        };
        // An unreadable boot clock is uncertainty even if the PID is absent.
        let Ok(booted) = processes.booted() else {
            return false;
        };
        if booted > started {
            return true;
        }
        match processes.started(self.process.unwrap()) {
            Ok(None) => true,
            Ok(Some(process_start)) => process_start > started,
            Err(_) => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub(super) struct Observation {
    pub reference: String,
    pub sha: String,
    pub owner: Owner,
}

impl Observation {
    pub(super) fn read(git: &Git, reference: &str, sha: &str) -> Result<Self> {
        ensure!(
            valid_ref(reference) && valid_sha(sha),
            "invalid reservation ref or SHA"
        );
        // Fetch the observed object, not a newer owner, and create no tracking ref.
        git.run(&[
            "fetch",
            "--no-tags",
            "--no-write-fetch-head",
            "--no-recurse-submodules",
            "origin",
            sha,
        ])?;
        let message = git.run(&["show", "-s", "--format=%B", sha])?;
        Ok(Self {
            reference: reference.into(),
            sha: sha.into(),
            owner: Owner::parse(&message),
        })
    }

    pub(super) fn blocking(&self, repo: &str) -> String {
        match self.reference.as_str() {
            "refs/heads/thirdshift/pickup/ci-red" => format!(
                "Fix run Passes on {repo} skip their Base branch CI step; Pickup cannot file CI-red Tickets"
            ),
            "refs/thirdshift/reservations/release" => format!("Release run Passes on {repo} skip"),
            reference if reference.starts_with("refs/thirdshift/reservations/") => format!(
                "Review and Fix runs on {repo} cannot take PR #{}",
                reference.rsplit('/').next().unwrap_or_default()
            ),
            reference => format!(
                "Pickup and Triage Passes on {repo} cannot take issue #{}; linked PRs wait for review",
                reference.rsplit('/').next().unwrap_or_default()
            ),
        }
    }
}

fn valid_sha(sha: &str) -> bool {
    matches!(sha.len(), 40 | 64) && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn valid_ref(reference: &str) -> bool {
    let numbered =
        |value: &str| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
    reference
        .strip_prefix("refs/heads/thirdshift/pickup/")
        .is_some_and(|tail| tail == "ci-red" || numbered(tail))
        || reference
            .strip_prefix("refs/thirdshift/reservations/")
            .is_some_and(|tail| tail == "release" || numbered(tail))
}

pub(super) fn inventory(git: &Git) -> Result<Vec<(String, String)>> {
    let remote = git.run(&[
        "ls-remote",
        "--refs",
        "origin",
        "refs/heads/thirdshift/pickup/*",
        "refs/thirdshift/reservations/*",
    ])?;
    Ok(remote
        .lines()
        .filter_map(|line| {
            let (sha, reference) = line.split_once('\t')?;
            (valid_ref(reference) && valid_sha(sha)).then(|| (reference.into(), sha.into()))
        })
        .collect())
}

/// Shared Pass startup. Failed observations or leases only warn; work selection
/// continues and still observes whichever reservations origin currently holds.
pub(crate) fn recover(git: &Git, repo: &Repo) {
    let result = (|| -> Result<()> {
        let observed = inventory(git)?;
        if let Err(error) = super::inbox::reconcile(git, repo, &observed) {
            event(
                repo,
                format!("warning: could not reconcile reservation observations: {error:#}"),
            );
        }
        let native = Native;
        for (reference, sha) in observed {
            let reservation = match Observation::read(git, &reference, &sha) {
                Ok(reservation) => reservation,
                Err(error) => {
                    event(
                        repo,
                        format!("warning: could not inspect reservation {reference}: {error:#}"),
                    );
                    continue;
                }
            };
            if !reservation.owner.orphaned(&native) {
                continue;
            }
            let owner = &reservation.owner;
            let details = format!(
                "{reference} (Process: {}; Started: {}); {}",
                owner.process.unwrap(),
                owner.started.as_deref().unwrap_or_default(),
                reservation.blocking(&repo.slug())
            );
            match git.run(&[
                "push",
                "--no-verify",
                &format!("--force-with-lease={reference}:{sha}"),
                "origin",
                &format!(":{reference}"),
            ]) {
                Ok(_) => event(repo, format!("released orphaned reservation {details}")),
                Err(error) => event(
                    repo,
                    format!(
                        "warning: could not release orphaned reservation {details}; observed SHA {sha}; {error:#}"
                    ),
                ),
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        event(
            repo,
            format!("warning: could not check origin reservations: {error:#}"),
        );
    }
}

fn event(repo: &Repo, message: String) {
    crate::logs::activity_for(repo, &message);
    // Recovery must remain visible even when an idle Pass quiets its skips.
    crate::logs::show_held();
    progress::step(message);
}

pub(super) fn blocked(git: &Git, reference: &str, sha: &str) {
    let result = (|| -> Result<()> {
        let origin = git.run(&["config", "remote.origin.url"])?;
        let repo = Repo::of_origin(&origin).context("reservation origin is not GitHub")?;
        let observation = Observation::read(git, reference, sha)?;
        super::inbox::blocked(git, &repo, observation)
    })();
    if let Err(error) = result {
        progress::warn(
            &error,
            format_args!("could not keep blocked reservation {reference}"),
        );
    }
}

trait Processes {
    fn host(&self) -> Option<String>;
    fn booted(&self) -> Result<DateTime<Utc>>;
    fn started(&self, pid: u32) -> Result<Option<DateTime<Utc>>>;
}

struct Native;

/// The same conservative local proof used to release origin reservations.
pub(crate) fn owner_is_dead(host: Option<String>, process: u32, started: String) -> bool {
    Owner {
        host,
        launch: None,
        process: Some(process),
        started: Some(started),
    }
    .orphaned(&Native)
}

impl Processes for Native {
    fn host(&self) -> Option<String> {
        crate::host::name()
    }

    #[cfg(target_os = "linux")]
    fn booted(&self) -> Result<DateTime<Utc>> {
        let stat = std::fs::read_to_string("/proc/stat")?;
        let seconds: i64 = stat
            .lines()
            .find_map(|line| line.strip_prefix("btime "))
            .context("no boot time")?
            .parse()?;
        DateTime::from_timestamp(seconds, 0)
            .filter(|_| seconds > 0)
            .context("invalid boot time")
    }

    #[cfg(target_os = "macos")]
    fn booted(&self) -> Result<DateTime<Utc>> {
        let mut boot = std::mem::MaybeUninit::<libc::timeval>::uninit();
        let mut size = std::mem::size_of::<libc::timeval>();
        let mut mib = [libc::CTL_KERN, libc::KERN_BOOTTIME];
        // SAFETY: sysctl receives a correctly sized output buffer and no input.
        let result = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                2,
                boot.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        ensure!(
            result == 0 && size == std::mem::size_of::<libc::timeval>(),
            "unreadable boot time"
        );
        // SAFETY: the successful call initialized the complete timeval.
        let boot = unsafe { boot.assume_init() };
        DateTime::from_timestamp(boot.tv_sec, (boot.tv_usec as u32) * 1000)
            .filter(|_| boot.tv_sec > 0)
            .context("invalid boot time")
    }

    fn started(&self, pid: u32) -> Result<Option<DateTime<Utc>>> {
        // SAFETY: signal zero checks one positive PID without sending a signal.
        if unsafe { libc::kill(pid as i32, 0) } != 0 {
            let error = std::io::Error::last_os_error();
            return if error.raw_os_error() == Some(libc::ESRCH) {
                Ok(None)
            } else {
                Err(error.into())
            };
        }
        native_start(pid).map(Some)
    }
}

#[cfg(target_os = "linux")]
fn native_start(pid: u32) -> Result<DateTime<Utc>> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let fields = stat.rsplit_once(") ").context("invalid process stat")?.1;
    let ticks: i64 = fields
        .split_whitespace()
        .nth(19)
        .context("missing start time")?
        .parse()?;
    // SAFETY: sysconf reads the native clock frequency and takes no pointer.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    ensure!(hz > 0 && ticks >= 0, "invalid process clock");
    // btime has second precision. Only a start provably beyond that uncertainty
    // can establish reuse; rounding must never condemn a live owner.
    Ok(Native.booted()? + chrono::Duration::seconds(ticks / hz))
}

#[cfg(target_os = "macos")]
fn native_start(pid: u32) -> Result<DateTime<Utc>> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    // SAFETY: proc_pidinfo receives the correctly sized native output buffer.
    let read = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size as i32,
        )
    };
    ensure!(read == size as i32, "unreadable process start time");
    // SAFETY: the successful call initialized the entire struct.
    let info = unsafe { info.assume_init() };
    DateTime::from_timestamp(
        info.pbi_start_tvsec as i64,
        (info.pbi_start_tvusec as u32) * 1000,
    )
    .context("invalid process start time")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Probe {
        boot: Option<i64>,
        process: Result<Option<i64>, ()>,
    }

    fn time(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(seconds, 0).unwrap()
    }

    impl Processes for Probe {
        fn host(&self) -> Option<String> {
            Some("local".into())
        }
        fn booted(&self) -> Result<DateTime<Utc>> {
            self.boot.map(time).context("unreadable boot")
        }
        fn started(&self, _: u32) -> Result<Option<DateTime<Utc>>> {
            self.process
                .map(|value| value.map(time))
                .map_err(|_| anyhow::anyhow!("unreadable process"))
        }
    }

    #[test]
    fn only_local_proof_of_death_allows_recovery() {
        let owner = Owner::parse("Host: local\nProcess: 42\nStarted: 2026-11-17 12:00:00 UTC");
        let start = owner.started().unwrap().timestamp();
        for (boot, process, orphaned) in [
            (Some(start - 100), Ok(None), true),
            (Some(start - 100), Ok(Some(start + 10)), true),
            (Some(start + 10), Ok(Some(start - 20)), true),
            (Some(start - 100), Ok(Some(start - 20)), false),
            (Some(start - 100), Ok(Some(start)), false),
            (Some(start - 100), Err(()), false),
            (None, Ok(None), false),
            (None, Ok(Some(start + 10)), false),
        ] {
            assert_eq!(
                owner.orphaned(&Probe { boot, process }),
                orphaned,
                "boot {boot:?}, process {process:?}"
            );
        }
        for message in [
            "Process: 42\nStarted: 2026-11-17T12:00:00Z",
            "Host: remote\nProcess: 42\nStarted: 2026-11-17T12:00:00Z",
            "Host: local\nStarted: 2026-11-17T12:00:00Z",
            "Host: local\nProcess: 42\nStarted: malformed",
            "Host: local\nHost: remote\nProcess: 42\nStarted: 2026-11-17T12:00:00Z",
            "Host: local\nProcess: 0\nStarted: 2026-11-17T12:00:00Z",
        ] {
            assert!(
                !Owner::parse(message).orphaned(&Probe {
                    boot: Some(start - 100),
                    process: Ok(None)
                }),
                "{message}"
            );
        }
    }

    #[test]
    fn native_probe_distinguishes_a_reaped_process_from_this_live_process() {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(Native.started(pid).unwrap().is_none());
        let started = Native.started(std::process::id()).unwrap().unwrap();
        assert!(started <= Utc::now());
        assert!(Native.booted().unwrap() <= started);
    }
}
