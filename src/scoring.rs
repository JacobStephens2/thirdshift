//! The weekly Scoring sample. A single machine-wide lock protects window
//! advancement; an atomic private sample file is the successful checkpoint.
use std::fs::{self, File, TryLockError};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, TimeDelta, Utc};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};

use crate::config::{self, UserConfig};
use crate::github::GitHub;
use crate::issue::Repo;
use crate::{interrupt, logs};

mod sample;
pub(crate) use sample::PullRequest;
pub(crate) mod packet;

pub fn lock() -> Result<Option<File>> {
    let dir = config::home()?.join(".thirdshift");
    fs::create_dir_all(&dir)?;
    let file = File::create(dir.join("scoring.lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => Err(error.into()),
    }
}

#[derive(Serialize, Deserialize)]
struct Week {
    week: String,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    seed: u64,
    sample: Vec<PullRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    packets: Option<String>,
}

fn directory() -> Result<PathBuf> {
    Ok(config::home()?.join(".thirdshift/evals/scoring"))
}

fn read(dir: &Path) -> Result<Vec<Week>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut weeks: Vec<Week> = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            weeks.push(
                serde_json::from_slice(&fs::read(&path)?)
                    .with_context(|| format!("can't read Scoring sample {}", path.display()))?,
            );
        }
    }
    weeks.sort_by_key(|week| week.window_end);
    Ok(weeks)
}

pub fn sample(config: &UserConfig, end: DateTime<Utc>) -> Result<usize> {
    let repo = Repo {
        owner: "JacobStephens2".into(),
        name: "thirdshift".into(),
    };
    logs::started(logs::Work::ScoringRun(&repo), &config.scoring.grader());
    if config.scoring.owners.is_empty() {
        bail!("configure scoring.owners in the User config before starting a Scoring run");
    }
    let dir = directory()?;
    let previous = read(&dir)?;
    let start = previous
        .last()
        .map(|week| week.window_end)
        .unwrap_or(end - TimeDelta::days(7));
    if start > end {
        bail!("the previous Scoring window ends in the future");
    }
    let candidates = GitHub::new().scoring_pull_requests(&config.scoring.owners, start, end)?;
    let seed = OsRng.next_u64();
    let mut week = Week {
        week: end.format("%G-W%V").to_string(),
        window_start: start,
        window_end: end,
        seed,
        sample: sample::draw(candidates, config.scoring.sample_size.get(), seed),
        packets: None,
    };
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;
    let packets = tempfile::Builder::new()
        .prefix("packets-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(&dir)?;
    for (index, pr) in week.sample.iter().enumerate() {
        packet::write(&GitHub::new(), pr, &packets.path().join(index.to_string()))
            .with_context(|| format!("can't build Scoring packet for {}", pr.url))?;
    }
    week.packets = Some(
        packets
            .path()
            .file_name()
            .context("packet directory has no name")?
            .to_string_lossy()
            .into_owned(),
    );
    let mut file = tempfile::NamedTempFile::new_in(&dir)?;
    serde_json::to_writer_pretty(&mut file, &week)?;
    file.flush()?;
    file.as_file().sync_all()?;
    interrupt::check()?;
    let path = dir.join(format!("{}.json", end.format("%Y%m%dT%H%M%SZ")));
    // Runs in the same second share a display stamp. Replacing the first
    // would discard its sample, so keep distinct storage ownership.
    file.persist_noclobber(if path.exists() {
        dir.join(format!(
            "{}-{}.json",
            end.format("%Y%m%dT%H%M%SZ"),
            week.seed
        ))
    } else {
        path
    })?;
    let _ = packets.keep();
    Ok(week.sample.len())
}

pub fn show(json: bool) -> Result<String> {
    let weeks = read(&directory()?)?;
    if json {
        return Ok(serde_json::to_string_pretty(&weeks)?);
    }
    let mut text = String::new();
    for week in weeks {
        text.push_str(&format!(
            "{}  {} → {}  seed {}\n",
            week.week, week.window_start, week.window_end, week.seed
        ));
        for pr in week.sample {
            text.push_str(&format!("  {}  {} · {}\n", pr.url, pr.harness, pr.model));
        }
    }
    Ok(text)
}
