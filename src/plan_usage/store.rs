//! The machine's saved Plan usage snapshot, `~/.thirdshift/plan-usage.json`:
//! one entry per subscription Harness, updated by running sessions and by
//! official non-billed reads, with a stable lock across atomic replacements.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use super::observation::{Exhausted, Observation, Window};
use crate::harness::Harness;

/// The first revalidation backoff when a limit gives no reset time.
const FIRST_BACKOFF_MINUTES: i64 = 60;
/// Repeated rejections double the backoff up to this.
const LAST_BACKOFF_MINUTES: i64 = 6 * 60;

/// A recorded limit: the Harness is unavailable until `until`, and after it
/// revalidating until a later observation shows it working again.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cooldown {
    pub since: DateTime<Utc>,
    pub until: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overage: Option<String>,
    /// Minutes of revalidation backoff, when no reset time was reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff_minutes: Option<i64>,
}

/// The account a Harness CLI says it is signed in to.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Account {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed_in: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    /// Who: an email address or a Harness's own account label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
}

/// A credit or overage balance beside the included Plan windows.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Credits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_credits: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance: Option<String>,
}

/// Everything saved about one Harness's Plan usage.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// When a session or read last reported its windows or limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<DateTime<Utc>>,
    /// What reported them: `session`, or the read's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub windows: Vec<Window>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown: Option<Cooldown>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<Account>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits: Option<Credits>,
    /// Codex's own word on whether included work may still run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordinary_usage_allowed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spend_control_reached: Option<bool>,
    /// When thirdshift last ran the Harness's non-billed reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_at: Option<DateTime<Utc>>,
}

impl State {
    /// Whether anything was ever observed here, beyond the reads' attempts.
    pub fn observed(&self) -> bool {
        self.observed_at.is_some() || self.cooldown.is_some()
    }

    /// Take in a session's or read's windows and limit. A report that the
    /// Harness is working clears a cooldown; a limit records one.
    pub fn observe(&mut self, observation: &Observation, source: &str, now: DateTime<Utc>) {
        self.observed_at = Some(now);
        self.source = Some(source.to_owned());
        self.merge_windows(&observation.windows);
        match &observation.exhausted {
            Some(exhausted) => self.exhaust(exhausted, now),
            None => self.cooldown = None,
        }
    }

    /// Replace each reported window's facts, keeping the others'.
    pub fn merge_windows(&mut self, windows: &[Window]) {
        for window in windows {
            match self
                .windows
                .iter_mut()
                .find(|kept| kept.name == window.name)
            {
                Some(kept) => {
                    kept.used_percent = window.used_percent.or(kept.used_percent);
                    kept.status = window.status.clone().or(kept.status.take());
                    kept.resets_at = window.resets_at.or(kept.resets_at);
                    kept.duration_minutes = window.duration_minutes.or(kept.duration_minutes);
                }
                None => self.windows.push(window.clone()),
            }
        }
    }

    /// Hold the Harness until the reported reset, or, with none, for a
    /// revalidation backoff that doubles each time a limit is met again
    /// after the last one passed, up to six hours.
    pub fn exhaust(&mut self, exhausted: &Exhausted, now: DateTime<Utc>) {
        let previous = self.cooldown.take();
        let (until, backoff_minutes) = match exhausted.resets_at.filter(|reset| *reset > now) {
            Some(reset) => (reset, None),
            None => {
                let minutes = match &previous {
                    // A second report of the same limit, as a failed turn's
                    // error after its rejection event, extends nothing.
                    Some(active) if active.until > now => {
                        self.cooldown = previous;
                        return;
                    }
                    Some(Cooldown {
                        backoff_minutes: Some(minutes),
                        ..
                    }) => (minutes * 2).min(LAST_BACKOFF_MINUTES),
                    _ => FIRST_BACKOFF_MINUTES,
                };
                (now + TimeDelta::minutes(minutes), Some(minutes))
            }
        };
        self.cooldown = Some(Cooldown {
            since: now,
            until,
            window: exhausted.window.clone(),
            overage: exhausted.overage.clone(),
            backoff_minutes,
        });
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Record {
    #[serde(default)]
    harnesses: BTreeMap<String, State>,
}

pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn machine() -> Result<Self> {
        Ok(Self::new(
            crate::config::home()?.join(".thirdshift/plan-usage.json"),
        ))
    }

    /// Each subscription Harness's saved state, by name.
    pub fn read(&self) -> Result<BTreeMap<String, State>> {
        Ok(self.record()?.harnesses)
    }

    fn record(&self) -> Result<Record> {
        match fs::read(&self.path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Record::default()),
            Err(error) => Err(error.into()),
        }
    }

    /// Change one Harness's state under the lock.
    #[cfg(test)]
    pub fn update(&self, harness: Harness, change: impl FnOnce(&mut State)) -> Result<()> {
        self.update_all(|harnesses| change(harnesses.entry(harness.name().into()).or_default()))
    }

    pub fn update_all(&self, change: impl FnOnce(&mut BTreeMap<String, State>)) -> Result<()> {
        let dir = self.path.parent().expect("the snapshot has a parent");
        fs::create_dir_all(dir)?;
        let _lock = crate::github::quota::lock_exclusive(&self.path.with_extension("lock"))?;
        let mut record = self.record()?;
        let previous = serde_json::to_vec_pretty(&record)?;
        change(&mut record.harnesses);
        let bytes = serde_json::to_vec_pretty(&record)?;
        if bytes == previous {
            return Ok(());
        }
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        tmp.write_all(&bytes)?;
        tmp.persist(&self.path)?;
        Ok(())
    }

    /// Forget recorded cooldowns and revalidation backoff, for `harness`
    /// or every Harness.
    pub fn clear(&self, harness: Option<Harness>) -> Result<()> {
        self.update_all(|harnesses| {
            for (name, state) in harnesses.iter_mut() {
                if harness.is_none_or(|harness| harness.name() == name) {
                    state.cooldown = None;
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exhausted(resets_at: Option<DateTime<Utc>>) -> Observation {
        Observation {
            harness: Harness::Agy,
            windows: Vec::new(),
            exhausted: Some(Exhausted {
                resets_at,
                ..Exhausted::default()
            }),
        }
    }

    #[test]
    fn a_reported_reset_holds_until_it_and_a_working_report_clears_it() {
        let now = Utc::now();
        let mut state = State::default();
        state.observe(&exhausted(Some(now + TimeDelta::hours(3))), "session", now);
        let cooldown = state.cooldown.clone().unwrap();
        assert_eq!(cooldown.until, now + TimeDelta::hours(3));
        assert_eq!(cooldown.backoff_minutes, None);
        state.observe(
            &Observation {
                exhausted: None,
                ..exhausted(None)
            },
            "session",
            now,
        );
        assert_eq!(state.cooldown, None);
    }

    #[test]
    fn without_a_reset_the_backoff_doubles_after_each_lapse_up_to_six_hours() {
        let mut now = Utc::now();
        let mut state = State::default();
        let mut seen = Vec::new();
        for _ in 0..5 {
            state.observe(&exhausted(None), "session", now);
            // A repeat while it holds changes nothing.
            state.observe(&exhausted(None), "session", now + TimeDelta::minutes(1));
            let cooldown = state.cooldown.clone().unwrap();
            seen.push(cooldown.backoff_minutes.unwrap());
            now = cooldown.until + TimeDelta::minutes(1);
        }
        assert_eq!(seen, [60, 120, 240, 360, 360]);
    }

    #[test]
    fn clearing_one_harness_keeps_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("plan-usage.json"));
        let now = Utc::now();
        for harness in [Harness::Agy, Harness::Grok] {
            store
                .update(harness, |state| {
                    state.observe(&exhausted(None), "session", now)
                })
                .unwrap();
        }
        store.clear(Some(Harness::Agy)).unwrap();
        let saved = store.read().unwrap();
        assert_eq!(saved["agy"].cooldown, None);
        assert!(saved["agy"].observed_at.is_some());
        assert!(saved["grok"].cooldown.is_some());
        store.clear(None).unwrap();
        assert_eq!(store.read().unwrap()["grok"].cooldown, None);
    }

    #[test]
    fn an_unchanged_snapshot_is_not_rewritten() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan-usage.json");
        let store = Store::new(path.clone());
        store
            .update(Harness::Agy, |state| state.read_at = Some(Utc::now()))
            .unwrap();
        let before = std::fs::metadata(&path).unwrap().ino();
        store.clear(None).unwrap(); // nothing to clear
        assert_eq!(std::fs::metadata(&path).unwrap().ino(), before);
    }
}
