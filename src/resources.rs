//! Host measurements and the policy that admits another agent session.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

use crate::config::UserConfig;

/// Zero disables each limit independently. Memory uses decimal MB.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub min_available_memory_mb: u64,
    pub max_load_per_cpu_percent: u32,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Measurements {
    pub total_memory_mb: u64,
    pub available_memory_mb: u64,
    pub logical_cpus: usize,
    pub load_one: f64,
}

#[derive(Serialize)]
pub struct Snapshot {
    pub host: Measurements,
    pub limits: Limits,
    pub load_per_cpu_percent: f64,
    pub ready: bool,
    pub reasons: Vec<String>,
}

/// Read only RAM and the CPU inventory, without enumerating processes.
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
    Ok(limits.assess(Measurements {
        total_memory_mb: system.total_memory() / 1_000_000,
        available_memory_mb: system.available_memory() / 1_000_000,
        logical_cpus: system.cpus().len(),
        load_one,
    }))
}

pub fn show(json: bool) -> Result<String> {
    let snapshot = observe(UserConfig::load()?.resources)?;
    if json {
        Ok(serde_json::to_string_pretty(&snapshot)?)
    } else {
        Ok(snapshot.to_string())
    }
}

/// Wait only when limits were configured. Reload them on each observation so
/// a waiting Command can be released without restarting it.
pub fn wait() -> Result<()> {
    let mut waiting = crate::poll::WaitingLine::new();
    let mut waited = false;
    loop {
        crate::interrupt::check()?;
        let limits = UserConfig::load()?.resources;
        let blocked = if limits == Limits::default() {
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
        writeln!(f, "Logical CPUs: {}", self.host.logical_cpus)?;
        writeln!(
            f,
            "One-minute load: {:.2} ({:.1}% per CPU)",
            self.host.load_one, self.load_per_cpu_percent
        )?;
        writeln!(
            f,
            "Minimum available memory: {} MB; maximum load per CPU: {}% (0 disables each limit)",
            self.limits.min_available_memory_mb, self.limits.max_load_per_cpu_percent
        )?;
        if self.ready {
            write!(f, "Ready to start another session")
        } else {
            write!(f, "Waiting for host resources: {}", self.reasons.join("; "))
        }
    }
}

impl Limits {
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

    #[test]
    fn admission_checks_memory_and_load_independently_and_allows_equality() {
        let host = Measurements {
            total_memory_mb: 16000,
            available_memory_mb: 2048,
            logical_cpus: 8,
            load_one: 8.0,
        };
        let limits = Limits {
            min_available_memory_mb: 2048,
            max_load_per_cpu_percent: 100,
        };
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
        assert!(Limits::default().assess(low_memory).ready);
        assert!(Limits::default().assess(high_load).ready);
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
}
