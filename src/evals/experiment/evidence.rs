//! Active Trial publication and the retained evidence values readers expose.
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub(crate) struct RunningTrial {
    pub id: String,
    pub started_at: String,
}

#[derive(Serialize)]
pub(crate) struct SessionLog {
    pub id: String,
    pub trial_id: String,
    pub attempt: usize,
    pub trace: bool,
    #[serde(skip)]
    pub path: PathBuf,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Started {
    #[serde(default)]
    pub index: usize,
    pub started_at: String,
}

pub(super) struct Active(PathBuf);

impl Active {
    pub fn start(trial: &Path, attempt: &Path, index: usize) -> Result<Self> {
        let started = Started {
            index,
            started_at: chrono::Local::now().to_rfc3339(),
        };
        super::store::write(&attempt.join("started.json"), &started)?;
        let active = trial.join("active.json");
        super::store::write(&active, &started)?;
        Ok(Self(active))
    }
}

impl Drop for Active {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
