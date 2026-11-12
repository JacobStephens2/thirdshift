//! Host measurements and the policy that admits another agent session.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

use crate::config::UserConfig;

/// Zero disables each limit independently. Memory and disk use decimal MB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub min_available_memory_mb: u64,
    pub max_load_per_cpu_percent: u32,
    pub min_available_disk_mb: u64,
}

/// Room for a typical session's worktree and build output, so a full disk
/// delays sessions without any configuration.
pub const DEFAULT_MIN_AVAILABLE_DISK_MB: u64 = 5120;

impl Default for Limits {
    fn default() -> Self {
        Limits {
            min_available_disk_mb: DEFAULT_MIN_AVAILABLE_DISK_MB,
            ..Limits::OFF
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Measurements {
    pub total_memory_mb: u64,
    pub available_memory_mb: u64,
    pub logical_cpus: usize,
    pub load_one: f64,
    pub available_disk_mb: u64,
}

#[derive(Serialize)]
pub struct Snapshot {
    pub host: Measurements,
    pub limits: Limits,
    pub load_per_cpu_percent: f64,
    pub ready: bool,
    pub reasons: Vec<String>,
}

/// Read only RAM, the CPU inventory and free disk space, without enumerating
/// processes.
pub fn observe(limits: Limits) -> Result<Snapshot> {
    let system = System::new_with_specifics(
        RefreshKind::nothing()
            .with_memory(MemoryRefreshKind::nothing().with_ram())
            .with_cpu(CpuRefreshKind::nothing()),
    );
    ensure!(system.total_memory() > 0, "can't read host memory");
    ensure!(!system.cpus().is_empty(), "can't read host CPU count");
    let mut load_one = 0.0;
    // SAFETY: getloadavg receives a valid pointer to one writable double.
    ensure!(
        unsafe { libc::getloadavg(&mut load_one, 1) } == 1
            && load_one.is_finite()
            && load_one >= 0.0,
        "can't read host CPU load"
    );
    let cwd = std::env::current_dir().context("can't read the current directory")?;
    Ok(limits.assess(Measurements {
        total_memory_mb: system.total_memory() / 1_000_000,
        available_memory_mb: system.available_memory() / 1_000_000,
        logical_cpus: system.cpus().len(),
        load_one,
        available_disk_mb: available_disk_mb(&cwd)?,
    }))
}

/// Free space an unprivileged user may write, on the filesystem that will
/// hold worktrees for `cwd`, or the temporary directory's when lower.
fn available_disk_mb(cwd: &Path) -> Result<u64> {
    Ok(free_mb(&worktree_parent(cwd))?.min(free_mb(&std::env::temp_dir())?))
}

/// Worktrees are created beside the repository root, so in its parent; outside
/// any repository, the directory itself.
fn worktree_parent(cwd: &Path) -> PathBuf {
    cwd.ancestors()
        .find(|directory| directory.join(".git").exists())
        .and_then(Path::parent)
        .unwrap_or(cwd)
        .to_path_buf()
}

fn free_mb(path: &Path) -> Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: statvfs receives a NUL-terminated path and a writable struct.
    ensure!(
        unsafe { libc::statvfs(name.as_ptr(), stats.as_mut_ptr()) } == 0,
        "can't read free disk space at {}: {}",
        path.display(),
        std::io::Error::last_os_error()
    );
    // SAFETY: statvfs succeeded, so it filled the struct.
    let stats = unsafe { stats.assume_init() };
    #[allow(clippy::unnecessary_cast)]
    Ok(stats.f_bavail as u64 * stats.f_frsize as u64 / 1_000_000)
}

pub fn show(json: bool) -> Result<String> {
    let snapshot = observe(UserConfig::load()?.resources)?;
    if json {
        Ok(serde_json::to_string_pretty(&snapshot)?)
    } else {
        Ok(snapshot.to_string())
    }
}

/// Wait only when some limit is on. Reload them on each observation so
/// a waiting Command can be released without restarting it.
pub fn wait() -> Result<()> {
    let mut waiting = crate::poll::WaitingLine::new();
    let mut waited = false;
    loop {
        crate::interrupt::check()?;
        let limits = UserConfig::load()?.resources;
        let blocked = if limits.is_off() {
            None
        } else {
            let snapshot = observe(limits)?;
            (!snapshot.ready).then_some(snapshot)
        };
        let Some(snapshot) = blocked else {
            if waited {
                crate::progress::step("host resources available; starting session");
            }
            return Ok(());
        };
        if waiting.should_report(limits) {
            crate::progress::step(format!(
                "waiting for host resources: {}",
                snapshot.reasons.join("; ")
            ));
        }
        waited = true;
        crate::poll::pause()?;
    }
}

impl std::fmt::Display for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "Available memory: {} / {} MB",
            self.host.available_memory_mb, self.host.total_memory_mb
        )?;
        writeln!(f, "Available disk: {} MB", self.host.available_disk_mb)?;
        writeln!(f, "Logical CPUs: {}", self.host.logical_cpus)?;
        writeln!(
            f,
            "One-minute load: {:.2} ({:.1}% per CPU)",
            self.host.load_one, self.load_per_cpu_percent
        )?;
        writeln!(
            f,
            "Minimum available memory: {} MB; maximum load per CPU: {}%; minimum available disk: {} MB (0 disables each limit)",
            self.limits.min_available_memory_mb,
            self.limits.max_load_per_cpu_percent,
            self.limits.min_available_disk_mb
        )?;
        if self.ready {
            write!(f, "Ready to start another session")
        } else {
            write!(f, "Waiting for host resources: {}", self.reasons.join("; "))
        }
    }
}

impl Limits {
    pub const OFF: Limits = Limits {
        min_available_memory_mb: 0,
        max_load_per_cpu_percent: 0,
        min_available_disk_mb: 0,
    };

    pub fn is_off(self) -> bool {
        self == Limits::OFF
    }

    pub fn assess(self, host: Measurements) -> Snapshot {
        let load_per_cpu_percent = host.load_one / host.logical_cpus as f64 * 100.0;
        let mut reasons = Vec::new();
        if host.available_memory_mb < self.min_available_memory_mb {
            reasons.push(format!(
                "available memory {} MB is below {} MB",
                host.available_memory_mb, self.min_available_memory_mb
            ));
        }
        if self.max_load_per_cpu_percent != 0
            && load_per_cpu_percent > f64::from(self.max_load_per_cpu_percent)
        {
            reasons.push(format!(
                "one-minute load per CPU {load_per_cpu_percent:.1}% exceeds {}%",
                self.max_load_per_cpu_percent
            ));
        }
        if host.available_disk_mb < self.min_available_disk_mb {
            reasons.push(format!(
                "available disk {} MB is below {} MB",
                host.available_disk_mb, self.min_available_disk_mb
            ));
        }
        Snapshot {
            host,
            limits: self,
            load_per_cpu_percent,
            ready: reasons.is_empty(),
            reasons,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST: Measurements = Measurements {
        total_memory_mb: 16000,
        available_memory_mb: 2048,
        logical_cpus: 8,
        load_one: 8.0,
        available_disk_mb: 5120,
    };
    const LIMITS: Limits = Limits {
        min_available_memory_mb: 2048,
        max_load_per_cpu_percent: 100,
        min_available_disk_mb: 5120,
    };

    #[test]
    fn admission_checks_memory_and_load_independently_and_allows_equality() {
        let (host, limits) = (HOST, LIMITS);
        assert!(limits.assess(host).ready);
        let low_memory = Measurements {
            available_memory_mb: 2047,
            ..host
        };
        assert!(!limits.assess(low_memory).ready);
        let high_load = Measurements {
            load_one: 8.1,
            ..host
        };
        assert!(!limits.assess(high_load).ready);
        assert!(Limits::OFF.assess(low_memory).ready);
        assert!(Limits::OFF.assess(high_load).ready);
        let memory_only = Limits {
            max_load_per_cpu_percent: 0,
            ..limits
        };
        assert!(memory_only.assess(high_load).ready);
        let load_only = Limits {
            min_available_memory_mb: 0,
            ..limits
        };
        assert!(load_only.assess(low_memory).ready);
    }

    #[test]
    fn a_disk_below_its_minimum_waits_naming_both_values_and_zero_disables_it() {
        let full_disk = Measurements {
            available_disk_mb: 412,
            ..HOST
        };
        let snapshot = LIMITS.assess(full_disk);
        assert!(!snapshot.ready);
        assert_eq!(snapshot.reasons, ["available disk 412 MB is below 5120 MB"]);
        assert!(LIMITS.assess(HOST).ready, "equality admits");
        let without_disk = Limits {
            min_available_disk_mb: 0,
            ..LIMITS
        };
        let empty_disk = Measurements {
            available_disk_mb: 0,
            ..HOST
        };
        assert!(without_disk.assess(empty_disk).ready);
    }

    #[test]
    fn every_failing_limit_is_listed_together() {
        let starved = Measurements {
            available_memory_mb: 1000,
            load_one: 16.0,
            available_disk_mb: 412,
            ..HOST
        };
        assert_eq!(
            LIMITS.assess(starved).reasons,
            [
                "available memory 1000 MB is below 2048 MB",
                "one-minute load per CPU 200.0% exceeds 100%",
                "available disk 412 MB is below 5120 MB",
            ]
        );
    }

    #[test]
    fn only_all_zero_limits_are_off_and_the_default_waits_for_a_full_disk() {
        assert!(Limits::OFF.is_off());
        assert!(!Limits::default().is_off());
        assert_eq!(Limits::default().min_available_disk_mb, 5120);
        let full_disk = Measurements {
            available_disk_mb: 0,
            ..HOST
        };
        assert!(!Limits::default().assess(full_disk).ready);
    }

    #[test]
    fn worktrees_land_beside_the_repository_that_holds_the_current_directory() {
        let parent = tempfile::tempdir().unwrap();
        let repository = parent.path().join("widgets");
        std::fs::create_dir_all(repository.join(".git")).unwrap();
        std::fs::create_dir_all(repository.join("src/deep")).unwrap();
        assert_eq!(worktree_parent(&repository.join("src/deep")), parent.path());
        assert_eq!(worktree_parent(&repository), parent.path());
        let outside = parent.path().join("plain");
        std::fs::create_dir_all(&outside).unwrap();
        assert_eq!(worktree_parent(&outside), outside);
        assert!(available_disk_mb(&repository).is_ok());
    }
}
