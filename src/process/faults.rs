//! Per-command faults, only compiled into the unit-test executable. Fixtures
//! still call the owning interface rather than private workers or handles.

use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};

#[derive(Clone)]
pub(super) struct Fault {
    point: Option<String>,
    gate: Option<PathBuf>,
}

impl Fault {
    pub fn from(command: &Command) -> Self {
        let env = |key| {
            command
                .get_envs()
                .find(|(name, _)| *name == key)
                .and_then(|(_, value)| value)
        };
        Self {
            point: env("THIRDSHIFT_EXECUTION_TEST_FAULT")
                .map(|value| value.to_string_lossy().into_owned()),
            gate: env("THIRDSHIFT_EXECUTION_TEST_GATE").map(PathBuf::from),
        }
    }

    pub fn fire(&self, at: &str) -> Result<()> {
        let panic = self.point.as_deref() == Some(&format!("{at}-panic"));
        if self.point.as_deref() != Some(at) && !panic {
            return Ok(());
        }
        if let Some(gate) = &self.gate {
            let deadline = Instant::now() + Duration::from_secs(2);
            while !gate.exists() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
            assert!(gate.exists(), "the fault fixture never reached its gate");
        }
        if panic {
            panic!("injected {at} panic");
        }
        bail!(std::io::Error::other(format!("injected {at} failure")));
    }
}
