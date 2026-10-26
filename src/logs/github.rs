//! The Inbox's kept GitHub answer: what the last batched issue/pull request read
//! showed, in one file per machine. `status` reuses it while it is under
//! five minutes old and reads GitHub first when it is missing or older; the
//! Console refreshes the same file once a minute. A row that has reached
//! the History page keeps what GitHub last showed and is not read again.
//! The reads never write to GitHub.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, FixedOffset, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use super::reading::Snapshot;
use crate::config::UserConfig;
use crate::github::{GitHub, InboxState, IssueState, PullRequestState, Quotas};

/// How long `status` reuses the kept answer before reading GitHub again.
pub const FRESH_FOR: Duration = Duration::from_secs(5 * 60);

/// One requested subject, with the Triage wait its label observation must follow.
pub(super) struct Subject {
    pub url: String,
    pub triage_since: Option<DateTime<FixedOffset>>,
}

pub(super) struct FindingSubject {
    pub row: String,
    pub repository: String,
    pub urls: Vec<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Findings {
    waits: BTreeMap<String, bool>,
    pub triaged_at: Option<String>,
}

impl Findings {
    pub fn waiting(&self) -> bool {
        self.waits.values().any(|waiting| *waiting)
    }
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    #[serde(default, alias = "pull_requests")]
    subjects: BTreeMap<String, Option<InboxState>>,
    #[serde(default)]
    history: BTreeMap<String, Option<InboxState>>,
    #[serde(default)]
    findings: BTreeMap<String, Findings>,
    /// Observation times are separate from facts: an unchanged read must
    /// refresh these without waking every open Inbox.
    #[serde(default)]
    answered_at: BTreeMap<String, String>,
    #[serde(default)]
    failure: Option<String>,
    #[serde(default)]
    quotas: Option<Quotas>,
    /// The warning at the last refresh, so crossing the age threshold
    /// wakes pages even when the facts and failure are unchanged.
    #[serde(default)]
    warning: Option<String>,
}

/// What the kept answer holds: every issue/pull request read so far, and how old
/// the answer is. `None` age means no answer is kept. A subject GitHub
/// can't resolve keeps `None`: what it last showed is nothing, and a row
/// that has reached History on that is not read again.
#[derive(Default)]
pub struct Kept {
    states: BTreeMap<String, Option<InboxState>>,
    history: BTreeMap<String, Option<InboxState>>,
    findings: BTreeMap<String, Findings>,
    answered_at: BTreeMap<String, String>,
    pub age: Option<Duration>,
    last_answered: Option<DateTime<Utc>>,
    pub failure: Option<String>,
    quotas: Option<Quotas>,
}

/// The exact kept facts for a row, including dispatched work and private
/// finding waits, without exposing any private write-up.
#[derive(Serialize)]
pub(super) struct RowAnswer {
    subjects: BTreeMap<String, SubjectAnswer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    findings: Option<FindingAnswer>,
}

#[derive(Serialize)]
struct SubjectAnswer {
    facts: Option<InboxState>,
    answered_at: Option<String>,
}

#[derive(Serialize)]
struct FindingAnswer {
    #[serde(flatten)]
    facts: Findings,
    answered_at: Option<String>,
}

impl Kept {
    pub(super) fn row_answer(&self, row: &str, urls: Vec<&str>) -> Option<RowAnswer> {
        let subjects: BTreeMap<_, _> = urls
            .into_iter()
            .filter_map(|url| {
                let key = self.subject_key(row, url);
                let facts = self.history.get(&key).or_else(|| self.states.get(url))?;
                Some((
                    url.to_owned(),
                    SubjectAnswer {
                        facts: facts.clone(),
                        answered_at: self.answered_at.get(&key).cloned(),
                    },
                ))
            })
            .collect();
        let findings = self.findings.get(row).map(|facts| FindingAnswer {
            facts: facts.clone(),
            answered_at: self.answered_at.get(row).cloned(),
        });
        (!subjects.is_empty() || findings.is_some()).then_some(RowAnswer { subjects, findings })
    }

    fn subject_key(&self, row: &str, url: &str) -> String {
        let key = format!("{row}::{url}");
        if self.history.contains_key(&key) {
            key
        } else if self.history.contains_key(row) {
            row.to_owned()
        } else {
            url.to_owned()
        }
    }

    pub(super) fn findings(&self, row: &str) -> Option<&Findings> {
        self.findings.get(row)
    }

    pub fn last_answered_at(&self) -> Option<String> {
        self.last_answered
            .map(|at| at.to_rfc3339_opts(SecondsFormat::Secs, true))
    }

    pub fn warning(&self) -> Option<String> {
        let mut reasons = Vec::new();
        if let Some(reason) = self.quotas.as_ref().and_then(Quotas::warning) {
            reasons.push(reason);
        }
        if self.age.is_none_or(|age| age > FRESH_FOR)
            && let Some(failure) = &self.failure
        {
            reasons.push(format!("latest read failed: {failure}"));
        }
        if reasons.is_empty() {
            return None;
        }
        let reason = reasons.join("; ");
        Some(match self.last_answered {
            Some(at) => format!("GitHub last answered at {}; {reason}", local_stamp(at)),
            None => format!("GitHub has not answered yet; {reason}"),
        })
    }

    /// An open Inbox must also learn when a failed answer becomes stale.
    pub fn next_warning_change(&self) -> Option<Duration> {
        let age = self.age?;
        (age <= FRESH_FOR).then(|| FRESH_FOR - age + Duration::from_millis(1))
    }

    fn state(&self, row: &str, url: &str) -> Option<&InboxState> {
        self.history
            .get(&self.subject_key(row, url))
            .or_else(|| self.states.get(url))
            .and_then(Option::as_ref)
    }

    pub fn pull_request(&self, row: &str, url: &str) -> Option<&PullRequestState> {
        match self.state(row, url)? {
            InboxState::PullRequest(state) => Some(state),
            InboxState::Issue(_) => None,
        }
    }

    pub fn issue(&self, row: &str, url: &str) -> Option<&IssueState> {
        match self.state(row, url)? {
            InboxState::Issue(state) => Some(state),
            InboxState::PullRequest(_) => None,
        }
    }

    pub fn has_answer(&self, url: &str) -> bool {
        self.states.contains_key(url)
    }

    pub fn is_frozen(&self, row: &str, url: &str) -> bool {
        self.history.contains_key(row) || self.history.contains_key(&format!("{row}::{url}"))
    }

    /// Preserve a History row's facts before another row refreshes the same
    /// issue or pull request. The selector distinguishes continuations and dispatches.
    pub fn freeze(&mut self, row: String, url: &str) {
        if let Some(state) = self.states.get(url) {
            let key = format!("{row}::{url}");
            if let Some(at) = self.answered_at.get(url).cloned() {
                self.answered_at.entry(key.clone()).or_insert(at);
            }
            self.history.entry(key).or_insert_with(|| state.clone());
        }
    }
}

/// The kept answer's file: one per machine, beside the logs rather than
/// among any Command's files, since it is a copy of GitHub, not something
/// a Command did.
pub fn path(logs_dir: &Path) -> PathBuf {
    logs_dir.join("github-inbox.json")
}

/// Only github-inbox.changed wakes pages. The answer itself also keeps
/// History bookkeeping, which can change without changing an Inbox row.
pub fn bookkeeping(path: &Path, logs_dir: &Path) -> bool {
    path.parent() == Some(logs_dir)
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                matches!(name, "github-inbox.json" | "github-inbox.refreshed")
                    || name.starts_with(".github-inbox-")
            })
}

/// The kept answer, whatever its age. A missing or unreadable file keeps
/// nothing, so the next read starts over.
pub fn load(logs_dir: &Path) -> Kept {
    let path = path(logs_dir);
    let file = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<File>(&bytes).ok())
        .filter(|file| matches!(file.version, 1 | 2));
    match file {
        Some(file) => {
            let modified = std::fs::metadata(path.with_extension("refreshed"))
                // Version 1 used the answer's file time before it had a
                // separate freshness marker. Version 2 may hold only a failure.
                .or_else(|error| {
                    if file.version == 1 {
                        std::fs::metadata(&path)
                    } else {
                        Err(error)
                    }
                })
                .and_then(|metadata| metadata.modified())
                .ok();
            Kept {
                states: file.subjects,
                history: file.history,
                findings: file.findings,
                answered_at: file.answered_at,
                age: modified.and_then(|at| at.elapsed().ok()),
                last_answered: modified.map(DateTime::from),
                failure: file.failure,
                quotas: file.quotas,
            }
        }
        None => Kept::default(),
    }
}

/// Refresh without an age gate, even when no page is open. All this work,
/// including the blocking GitHub adapter, belongs off the Console's async thread.
pub fn poll() -> Result<()> {
    let config = UserConfig::load()?;
    let snapshot = Snapshot::load_with_silence(
        &config.logs_dir,
        chrono::Local::now(),
        config.silent_interval,
    )?;
    refresh(&snapshot, &config, Duration::ZERO)?;
    Ok(())
}

/// Read GitHub unless the kept answer is still fresh, and keep
/// the merged answer. A failed read keeps the last answer, however old.
/// Only reads; nothing here writes to GitHub.
pub(super) fn refresh(
    snapshot: &Snapshot,
    config: &UserConfig,
    fresh_for: Duration,
) -> Result<Kept> {
    let mut kept = load(&config.logs_dir);
    if !snapshot
        .evaluate_inbox(config.inbox_days, &kept)
        .refresh_work()
        .has_remote_reads()
    {
        return Ok(kept);
    }
    // Serialize readers across status and Console processes. A busy reader
    // already owns the refresh; status can show the kept answer immediately.
    let lock = std::fs::File::create(config.logs_dir.join(".github-inbox-lock"))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Ok(kept),
        Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
    }
    kept = load(&config.logs_dir);
    // A pre-lock evaluation is advisory only. Plan from the answer reloaded
    // under the lock; freezes and subjects share these retained row decisions.
    let evaluation = snapshot.evaluate_inbox(config.inbox_days, &kept);
    let work = evaluation.refresh_work();
    // From History on, a row keeps what GitHub last showed and is not read
    // again: only rows with a kept answer are frozen, so the first read
    // still looks up a row that local rules alone would call History.
    for target in &work.freezes {
        kept.freeze(target.row.clone(), &target.url);
    }
    let subjects = &work.subjects;
    let findings = &work.findings;
    if kept.age.is_some_and(|age| age < fresh_for)
        || !work.has_remote_reads()
        || kept.quotas.as_ref().and_then(Quotas::warning).is_some()
    {
        return Ok(kept);
    }
    let github = GitHub::new();
    match github.inbox_quotas() {
        Ok(quotas) => kept.quotas = Some(quotas),
        Err(error) => {
            kept.failure = Some(format!("{error:#}"));
            save(&config.logs_dir, &kept, false)?;
            return Ok(kept);
        }
    }
    let mut answered = false;
    let mut read_failed = false;
    for chunk in subjects.chunks(100) {
        if kept.quotas.as_ref().and_then(Quotas::warning).is_some() {
            break;
        }
        let urls: Vec<_> = chunk.iter().map(|subject| subject.url.clone()).collect();
        match github.inbox_states(&urls) {
            Ok((states, quota)) => {
                let answered_at = Utc::now().to_rfc3339();
                for subject in chunk {
                    let url = &subject.url;
                    let mut state = states.get(url).cloned();
                    if let Some(InboxState::Issue(issue)) = &mut state
                        && !issue.needs_triage
                    {
                        issue.triaged_at = match kept.states.get(url) {
                            Some(Some(InboxState::Issue(previous))) if !previous.needs_triage => {
                                previous.triaged_at.clone().filter(|at| {
                                    subject.triage_since.is_none_or(|since| {
                                        DateTime::parse_from_rfc3339(at).is_ok_and(|at| at >= since)
                                    })
                                })
                            }
                            _ => None,
                        }
                        .or_else(|| Some(Utc::now().to_rfc3339()));
                    }
                    kept.states.insert(url.clone(), state);
                    kept.answered_at.insert(url.clone(), answered_at.clone());
                }
                kept.quotas
                    .as_mut()
                    .expect("quotas read before rows")
                    .graphql = quota;
                answered = true;
            }
            Err(error) => {
                kept.failure = Some(format!("{error:#}"));
                read_failed = true;
                break;
            }
        }
    }
    let mut repositories = BTreeMap::<_, Vec<_>>::new();
    for subject in findings {
        repositories
            .entry(subject.repository.clone())
            .or_default()
            .push(subject);
    }
    for (repository, subjects) in repositories {
        if kept.quotas.as_ref().and_then(Quotas::warning).is_some() {
            break;
        }
        match github.inbox_findings(&repository) {
            Ok(waits) => {
                for subject in subjects {
                    // Older records did not retain finding URLs. They can only
                    // ask whether any private record in their repository waits.
                    let waits = if subject.urls.is_empty() {
                        waits.clone()
                    } else {
                        subject
                            .urls
                            .iter()
                            .cloned()
                            .map(|url| {
                                let waiting = waits.get(&url).copied().unwrap_or(false);
                                (url, waiting)
                            })
                            .collect()
                    };
                    let mut facts = Findings {
                        waits,
                        triaged_at: None,
                    };
                    if !facts.waiting() {
                        facts.triaged_at = kept
                            .findings
                            .get(&subject.row)
                            .and_then(|previous| previous.triaged_at.clone())
                            .or_else(|| Some(Utc::now().to_rfc3339()));
                    }
                    kept.answered_at
                        .insert(subject.row.clone(), Utc::now().to_rfc3339());
                    kept.findings.insert(subject.row.clone(), facts);
                }
                answered = true;
            }
            Err(error) => {
                kept.failure = Some(format!("{error:#}"));
                read_failed = true;
                if let Ok(quotas) = github.inbox_quotas() {
                    kept.quotas = Some(quotas);
                }
                break;
            }
        }
        // Includes every paginated request and private-storage discovery, so
        // another repository cannot start a list on an exhausted reserve.
        match github.inbox_quotas() {
            Ok(quotas) => kept.quotas = Some(quotas),
            Err(error) => {
                kept.failure = Some(format!("{error:#}"));
                read_failed = true;
                break;
            }
        }
    }
    // A successful endpoint cannot erase another endpoint's failure or
    // make its stale facts look freshly read.
    let refreshed = answered && !read_failed;
    if refreshed {
        kept.age = Some(Duration::ZERO);
        kept.last_answered = Some(Utc::now());
        kept.failure = None;
    }
    save(&config.logs_dir, &kept, refreshed)?;
    Ok(kept)
}

fn save(logs_dir: &Path, kept: &Kept, answered: bool) -> Result<()> {
    let path = path(logs_dir);
    let file = File {
        version: 2,
        subjects: kept.states.clone(),
        history: kept.history.clone(),
        findings: kept.findings.clone(),
        answered_at: kept.answered_at.clone(),
        failure: kept.failure.clone(),
        quotas: kept.quotas.clone(),
        warning: kept.warning(),
    };
    let bytes = serde_json::to_vec(&file)?;
    let previous = std::fs::read(&path).ok();
    let facts_changed = previous
        .as_deref()
        .and_then(|bytes| serde_json::from_slice::<File>(bytes).ok())
        .is_none_or(|previous| {
            previous.version != file.version
                || previous.subjects != file.subjects
                || previous.findings != file.findings
                || previous.warning != file.warning
        });
    // Keep History freezes too, without waking pages for bookkeeping.
    if previous.as_deref() != Some(bytes.as_slice()) {
        let mut tmp = tempfile::Builder::new()
            .prefix(".github-inbox-")
            .tempfile_in(logs_dir)?;
        tmp.write_all(&bytes)?;
        tmp.persist(&path)?;
    }
    let freshness_path = path.with_extension("refreshed");
    if (answered || !freshness_path.exists())
        && let Some(at) = kept.last_answered
    {
        // Also migrate a version-1 answer that used its JSON file time.
        // A failed read must preserve that time, rather than erase it or
        // make the old facts look freshly read.
        let freshness = std::fs::File::create(freshness_path)?;
        freshness.set_modified(at.into())?;
    }
    if facts_changed {
        std::fs::write(path.with_extension("changed"), [])?;
    }
    Ok(())
}

/// `at` in this machine's zone, as people read it: `2026-10-26 06:23 EDT`.
fn local_stamp(at: DateTime<Utc>) -> String {
    let local = at.with_timezone(&chrono::Local);
    let zone =
        zone_abbreviation(at.timestamp()).unwrap_or_else(|| local.format("UTC%:z").to_string());
    format!("{} {zone}", local.format("%Y-%m-%d %H:%M"))
}

/// The zone's short name at that instant, such as `EDT` or `UTC`; chrono
/// knows only the offset.
#[cfg(unix)]
fn zone_abbreviation(seconds: i64) -> Option<String> {
    let time = libc::time_t::try_from(seconds).ok()?;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&time, &mut tm) }.is_null() || tm.tm_zone.is_null() {
        return None;
    }
    let name = unsafe { std::ffi::CStr::from_ptr(tm.tm_zone) }
        .to_str()
        .ok()?;
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(not(unix))]
fn zone_abbreviation(_seconds: i64) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_stamp_reads_date_minutes_and_zone_name() {
        let at = DateTime::parse_from_rfc3339("2026-10-26T10:23:28Z")
            .unwrap()
            .with_timezone(&Utc);
        let local = at.with_timezone(&chrono::Local);
        let stamp = local_stamp(at);
        let expected = local.format("%Y-%m-%d %H:%M ").to_string();
        assert!(stamp.starts_with(&expected), "{stamp}");
        let zone = &stamp[expected.len()..];
        assert!(
            !zone.is_empty() && !zone.contains(' '),
            "zone should be one short name, got {stamp}"
        );
    }
}
