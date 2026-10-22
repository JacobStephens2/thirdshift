//! Prefer a Harness's stream metadata; otherwise read its CLI version once
//! per owning Command. The locked cache is shared by parallel child Runs.

use std::fs::OpenOptions;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use super::{Adapter, process};

pub(crate) struct Version {
    adapter: &'static dyn Adapter,
    cache: Option<PathBuf>,
    command: Command,
    current: Option<String>,
}

impl Version {
    pub fn new(
        adapter: &'static dyn Adapter,
        command_log: Option<&Path>,
        session: &Command,
    ) -> Self {
        let mut command = Command::new(session.get_program());
        command.arg("--version");
        if let Some(dir) = session.get_current_dir() {
            command.current_dir(dir);
        }
        for (key, value) in session.get_envs() {
            if let Some(value) = value {
                command.env(key, value);
            } else {
                command.env_remove(key);
            }
        }
        Self {
            adapter,
            cache: command_log
                .map(|log| log.with_extension(format!("harness-{}.version", adapter.name()))),
            command,
            current: None,
        }
    }

    /// A lookup failure costs attribution, never the Session. Retain even an
    /// unknown result so a broken version command is not retried per event.
    pub fn observe(&mut self, event: &Value) -> &str {
        if let Some(version) = self.adapter.stream_version(event).and_then(nonempty) {
            if self.current.as_deref() != Some(version) {
                let _ = self.resolve(Some(version));
                self.current = Some(version.into());
            }
        } else if self.current.is_none() {
            self.current = Some(self.resolve(None).unwrap_or_else(|_| "unknown".into()));
        }
        self.current.as_deref().unwrap()
    }

    fn resolve(&mut self, reported: Option<&str>) -> io::Result<String> {
        let Some(path) = &self.cache else {
            return Ok(reported.map(str::to_owned).unwrap_or_else(|| self.query()));
        };
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        file.lock()?;
        let mut cached = String::new();
        file.read_to_string(&mut cached)?;
        if reported.is_none()
            && let Some(cached) = nonempty(&cached)
        {
            return Ok(cached.into());
        }
        let version = reported.map(str::to_owned).unwrap_or_else(|| self.query());
        file.seek(SeekFrom::Start(0))?;
        file.set_len(0)?;
        file.write_all(version.as_bytes())?;
        Ok(version)
    }

    fn query(&mut self) -> String {
        let Ok(output) = process::output(self.adapter, &mut self.command, None) else {
            return "unknown".into();
        };
        if !output.status.success() {
            return "unknown".into();
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        nonempty(&stdout)
            .or_else(|| nonempty(&stderr))
            .unwrap_or("unknown")
            .into()
    }
}

fn nonempty(text: &str) -> Option<&str> {
    let text = text.trim();
    (!text.is_empty()).then_some(text)
}
