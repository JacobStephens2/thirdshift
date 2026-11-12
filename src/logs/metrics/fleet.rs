//! Explicit Fleet metrics client: each owning machine's own `/metrics`, merged
//! with this machine's, and the last summary read from each peer, saved so an
//! offline or retired machine stays visible. Owning-machine APIs stay local.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::outcomes::{Delta, Tally};
use super::scope::{Filters, Span, Window};
use crate::args::MetricsArgs;
use crate::config::{UserConfig, console::fleet::Entry};
use crate::console::fleet::read::{Failure, Remote};
use crate::resend_key::Credentials;

/// The per-machine peer-metrics cache, under the watching account's logs.
const CACHE: &str = "fleet-metrics";
/// Views kept per machine, the most recently read first.
const VIEWS: usize = 16;

/// What one machine's `metrics --json` trends say that Fleet metrics merge,
/// and all a saved peer snapshot keeps.
#[derive(Serialize, Deserialize)]
struct Summary {
    outcomes: Vec<SummaryBucket>,
    #[serde(default)]
    comparison: Option<SummaryComparison>,
    #[serde(default)]
    where_runs_failed: SummaryFailures,
}

/// One machine's bucket for one OS account and Command kind.
#[derive(Serialize, Deserialize)]
pub(crate) struct SummaryBucket {
    pub(crate) bucket: NaiveDate,
    pub(crate) account: Option<String>,
    pub(crate) kind: String,
    #[serde(flatten)]
    pub(crate) tally: Tally,
    #[serde(default)]
    pub(crate) failed_commands: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct SummaryComparison {
    current: Span,
    prior: Span,
    rows: Vec<SummaryChange>,
}

#[derive(Serialize, Deserialize)]
struct SummaryChange {
    account: Option<String>,
    kind: String,
    current: Tally,
    prior: Tally,
}

#[derive(Default, Serialize, Deserialize)]
struct SummaryFailures {
    failed: usize,
    observed: usize,
    missing: usize,
    rows: Vec<SummaryStopped>,
}

#[derive(Serialize, Deserialize)]
struct SummaryStopped {
    stopped_at: Option<String>,
    category: String,
    recorded: bool,
    commands: Vec<SummaryCommand>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SummaryCommand {
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) account: Option<String>,
    pub(crate) started_at: String,
    pub(crate) selector: String,
}

/// Outcome stacks, preceding-window changes and Where runs failed across the
/// Fleet. Each Command is counted once, on the machine it ran on.
#[derive(Serialize)]
pub(crate) struct Fleet {
    pub(crate) version: u32,
    /// `fleet`, or the one machine asked for.
    pub(crate) scope: String,
    pub(crate) window: Window,
    pub(crate) filters: Filters,
    pub(crate) machines: Vec<Machine>,
    pub(crate) outcomes: Vec<Bucket>,
    pub(crate) comparison: Option<Comparison>,
    pub(crate) where_runs_failed: WhereRunsFailed,
}

#[derive(Serialize)]
pub(crate) struct Machine {
    pub(crate) name: String,
    pub(crate) address: Option<String>,
    pub(crate) local: bool,
    pub(crate) status: Status,
    /// When this machine's metrics were last read: now when live, the saved
    /// snapshot's time when cached.
    pub(crate) last_seen: Option<DateTime<Utc>>,
    pub(crate) error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Status {
    Ok,
    /// Unreachable now, or no longer in the Fleet: its saved snapshot shows.
    Cached,
    Unavailable,
    Refused,
    Incompatible,
    LinkOnly,
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Ok => "ok",
            Self::Cached => "cached",
            Self::Unavailable => "unavailable",
            Self::Refused => "refused",
            Self::Incompatible => "incompatible",
            Self::LinkOnly => "link-only",
        })
    }
}

#[derive(Serialize)]
pub(crate) struct Bucket {
    pub(crate) machine: String,
    #[serde(flatten)]
    pub(crate) row: SummaryBucket,
}

/// The window against the one before it: Fleet totals per Command kind, and
/// each machine's own rows.
#[derive(Serialize)]
pub(crate) struct Comparison {
    pub(crate) current: Span,
    pub(crate) prior: Span,
    pub(crate) fleet: Vec<Change>,
    pub(crate) rows: Vec<Change>,
}

#[derive(Serialize)]
pub(crate) struct Change {
    /// Missing on a Fleet total.
    pub(crate) machine: Option<String>,
    pub(crate) account: Option<String>,
    pub(crate) kind: String,
    pub(crate) current: Tally,
    pub(crate) prior: Tally,
    pub(crate) delta: Delta,
}

impl Change {
    fn of(machine: Option<String>, row: SummaryChange) -> Self {
        Self {
            machine,
            account: row.account,
            kind: row.kind,
            delta: Delta::of(&row.current, &row.prior),
            current: row.current,
            prior: row.prior,
        }
    }
}

#[derive(Default, Serialize)]
pub(crate) struct WhereRunsFailed {
    pub(crate) failed: usize,
    pub(crate) observed: usize,
    pub(crate) missing: usize,
    pub(crate) rows: Vec<Stopped>,
}

/// The Fleet's Failed Commands that stopped at one stage in one way.
#[derive(Serialize)]
pub(crate) struct Stopped {
    pub(crate) stopped_at: Option<String>,
    pub(crate) category: String,
    pub(crate) recorded: bool,
    pub(crate) failed: usize,
    /// How many of them each machine holds.
    pub(crate) machines: BTreeMap<String, usize>,
    pub(crate) commands: Vec<FailedCommand>,
}

#[derive(Serialize)]
pub(crate) struct FailedCommand {
    pub(crate) machine: String,
    #[serde(flatten)]
    pub(crate) command: SummaryCommand,
}

/// A saved peer snapshot file: the machine's last read views.
#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    machine: String,
    last_seen: DateTime<Utc>,
    views: BTreeMap<String, SavedView>,
}

#[derive(Serialize, Deserialize)]
struct SavedView {
    fetched_at: DateTime<Utc>,
    summary: Value,
}

/// The `metrics` query an owning machine is asked, in a fixed order, so a
/// view's saved snapshot is found again by the same text.
pub(crate) fn query(asked: &MetricsArgs) -> String {
    let mut pairs: Vec<(&str, String)> = Vec::new();
    if let Some(window) = asked.window {
        pairs.push(("window", window.name().into()));
    }
    if let Some(grain) = asked.grain {
        pairs.push(("grain", grain.name().into()));
    }
    for (key, date) in [("from", asked.from), ("through", asked.through)] {
        if let Some(date) = date {
            pairs.push((key, date.to_string()));
        }
    }
    for (key, values) in [
        ("repo", &asked.repositories),
        ("kind", &asked.kinds),
        ("harness", &asked.harnesses),
        ("model", &asked.models),
        ("version", &asked.versions),
        ("account", &asked.accounts),
    ] {
        pairs.extend(values.iter().map(|value| (key, value.clone())));
    }
    serde_urlencoded::to_string(pairs).expect("metrics query pairs encode")
}

/// Whether Fleet is the Console's default Metrics view: a paired machine, or
/// a saved snapshot of one, exists.
pub(crate) fn default_view(config: &UserConfig) -> bool {
    let credentials = Credentials::read().ok();
    config.console.fleet.iter().any(|entry| {
        credentials
            .as_ref()
            .is_some_and(|credentials| credentials.fleet_token(&entry.name).is_some())
    }) || !saved(&config.logs_dir).is_empty()
}

/// Every machine Fleet metrics can be scoped to besides this one: the Fleet
/// in config order, then retired machines with a saved snapshot.
pub(crate) fn names(config: &UserConfig) -> Vec<String> {
    let local = crate::console::fleet_control::local_name(config);
    let mut names: Vec<_> = config
        .console
        .fleet
        .iter()
        .map(|entry| entry.name.clone())
        .collect();
    let mut retired: Vec<_> = saved(&config.logs_dir)
        .into_iter()
        .map(|saved| saved.machine)
        .filter(|name| *name != local && !names.contains(name))
        .collect();
    retired.sort();
    names.extend(retired);
    names
}

/// Fleet metrics, or one machine's when `asked.machine` names a peer. A
/// caller handles this machine's own name as plain `metrics`.
pub(crate) fn load(
    config: &UserConfig,
    asked: &MetricsArgs,
    person: Option<&str>,
) -> Result<Fleet> {
    let now = Utc::now();
    let local = crate::console::fleet_control::local_name(config);
    let one = asked.machine.as_deref();
    if let Some(name) = one
        && !names(config).iter().any(|known| known == name)
    {
        bail!("unknown Fleet machine: {name}");
    }
    let mut sources = Vec::new();
    let window = if one.is_none() {
        let trends = super::trends(config, asked)?;
        let summary = serde_json::from_value(serde_json::to_value(&trends)?)?;
        sources.push((
            Machine {
                name: local.clone(),
                address: None,
                local: true,
                status: Status::Ok,
                last_seen: Some(now),
                error: None,
            },
            Some(summary),
        ));
        trends.window
    } else {
        Window::of(asked, now.date_naive())
    };
    let query = query(asked);
    let entries: Vec<_> = config
        .console
        .fleet
        .iter()
        .filter(|entry| one.is_none_or(|name| name == entry.name))
        .collect();
    // Independent requests, each bounded by the Fleet transport's three
    // seconds, so one stalled peer cannot hold the page.
    sources.extend(std::thread::scope(|threads| {
        let handles: Vec<_> = entries
            .iter()
            .map(|entry| threads.spawn(|| remote(config, entry, person, &query, now)))
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("Fleet metrics reader panicked"))
            .collect::<Vec<_>>()
    }));
    let paired: Vec<_> = config.console.fleet.iter().map(|e| &e.name).collect();
    let mut retired: Vec<_> = saved(&config.logs_dir)
        .into_iter()
        .filter(|saved| {
            saved.machine != local
                && !paired.contains(&&saved.machine)
                && one.is_none_or(|name| name == saved.machine)
        })
        .collect();
    retired.sort_by(|a, b| a.machine.cmp(&b.machine));
    for saved in retired {
        let mut machine = Machine {
            name: saved.machine.clone(),
            address: None,
            local: false,
            status: Status::Cached,
            last_seen: Some(saved.last_seen),
            error: Some("no longer in this Console's Fleet".into()),
        };
        let summary = cached(&mut machine, saved, &query);
        sources.push((machine, summary));
    }
    Ok(merge(
        one.map_or_else(|| "fleet".to_owned(), str::to_owned),
        window,
        Filters::of(asked),
        sources,
    ))
}

fn remote(
    config: &UserConfig,
    entry: &Entry,
    person: Option<&str>,
    query: &str,
    now: DateTime<Utc>,
) -> (Machine, Option<Summary>) {
    let mut machine = Machine {
        name: entry.name.clone(),
        address: Some(entry.address.clone()),
        local: false,
        status: Status::Ok,
        last_seen: None,
        error: None,
    };
    let read = || -> Result<Option<Summary>> {
        // A link never sends even an identity probe. The transport revalidates
        // paired destinations before attaching any saved token.
        if Credentials::read()?.fleet_token(&entry.name).is_none() {
            return Ok(None);
        }
        let remote = Remote::select(config, &entry.name, person.map(str::to_owned))?
            .context("expected a Fleet machine")?;
        let route = if query.is_empty() {
            "/metrics".to_owned()
        } else {
            format!("/metrics?{query}")
        };
        let summary =
            serde_json::from_value(remote.get(&route)?).context("incompatible metrics summary")?;
        Ok(Some(summary))
    };
    match read() {
        Ok(None) => {
            machine.status = Status::LinkOnly;
            (machine, None)
        }
        Ok(Some(summary)) => {
            machine.last_seen = Some(now);
            // The saved snapshot is a convenience; failing to keep it must
            // not cost the live view.
            if let Err(error) = save(&config.logs_dir, &entry.name, query, &summary, now) {
                machine.error = Some(format!("snapshot not saved: {error:#}"));
            }
            (machine, Some(summary))
        }
        Err(error) => {
            machine.error = Some(format!("{error:#}"));
            machine.status = match Failure::of(&error) {
                Failure::Refused => Status::Refused,
                Failure::Incompatible => Status::Incompatible,
                Failure::Unavailable => Status::Unavailable,
            };
            // Only an unreachable machine falls back: a refusal withdraws access.
            if machine.status != Status::Unavailable {
                return (machine, None);
            }
            let Some(saved) = read_saved(&path(&config.logs_dir, &entry.name))
                .filter(|saved| saved.machine == entry.name)
            else {
                return (machine, None);
            };
            machine.status = Status::Cached;
            machine.last_seen = Some(saved.last_seen);
            let summary = cached(&mut machine, saved, query);
            (machine, summary)
        }
    }
}

/// A saved view's summary, noting on the machine when this view was never read.
fn cached(machine: &mut Machine, mut saved: Saved, query: &str) -> Option<Summary> {
    let summary = saved
        .views
        .remove(query)
        .and_then(|view| serde_json::from_value(view.summary).ok());
    if summary.is_none() {
        note(machine, "no saved snapshot for this view");
    }
    summary
}

fn note(machine: &mut Machine, text: &str) {
    machine.error = Some(match machine.error.take() {
        Some(error) => format!("{error}; {text}"),
        None => text.to_owned(),
    });
}

fn merge(
    scope: String,
    window: Window,
    filters: Filters,
    sources: Vec<(Machine, Option<Summary>)>,
) -> Fleet {
    let spans = window.current().zip(window.prior);
    let mut outcomes = Vec::new();
    let mut totals = BTreeMap::<String, (Tally, Tally)>::new();
    let mut rows = Vec::new();
    let mut failures = WhereRunsFailed::default();
    let mut stopped = BTreeMap::<(Option<String>, String, bool), Stopped>::new();
    let mut machines = Vec::new();
    for (mut machine, summary) in sources {
        if let Some(summary) = summary {
            let name = machine.name.clone();
            // A cached snapshot may predate today's window; show only its
            // buckets that are still in it.
            outcomes.extend(
                summary
                    .outcomes
                    .into_iter()
                    .filter(|row| window.shows(row.bucket))
                    .map(|row| Bucket {
                        machine: name.clone(),
                        row,
                    }),
            );
            match (spans, summary.comparison) {
                (Some((current, prior)), Some(compared))
                    if compared.current == current && compared.prior == prior =>
                {
                    for row in compared.rows {
                        let (now, before) = totals.entry(row.kind.clone()).or_default();
                        now.merge(&row.current);
                        before.merge(&row.prior);
                        rows.push(Change::of(Some(name.clone()), row));
                    }
                }
                (Some(_), Some(_)) => {
                    note(&mut machine, "its saved comparison covers other dates");
                }
                _ => {}
            }
            let failed = summary.where_runs_failed;
            failures.failed += failed.failed;
            failures.observed += failed.observed;
            failures.missing += failed.missing;
            for row in failed.rows {
                let entry = stopped
                    .entry((row.stopped_at.clone(), row.category.clone(), row.recorded))
                    .or_insert_with(|| Stopped {
                        stopped_at: row.stopped_at,
                        category: row.category,
                        recorded: row.recorded,
                        failed: 0,
                        machines: BTreeMap::new(),
                        commands: Vec::new(),
                    });
                entry.failed += row.commands.len();
                *entry.machines.entry(name.clone()).or_default() += row.commands.len();
                entry
                    .commands
                    .extend(row.commands.into_iter().map(|command| FailedCommand {
                        machine: name.clone(),
                        command,
                    }));
            }
        }
        machines.push(machine);
    }
    // Buckets in order; within one, machines keep their listed order.
    outcomes.sort_by_key(|bucket| bucket.row.bucket);
    failures.rows = stopped.into_values().collect();
    // The largest cluster first; ties keep stage and category order.
    failures
        .rows
        .sort_by_key(|row| std::cmp::Reverse(row.failed));
    Fleet {
        version: 1,
        scope,
        window,
        filters,
        machines,
        outcomes,
        comparison: spans.map(|(current, prior)| Comparison {
            current,
            prior,
            fleet: totals
                .into_iter()
                .map(|(kind, (current, prior))| {
                    Change::of(
                        None,
                        SummaryChange {
                            account: None,
                            kind,
                            current,
                            prior,
                        },
                    )
                })
                .collect(),
            rows,
        }),
        where_runs_failed: failures,
    }
}

/// A name's saved snapshot file. Names are printable but may hold any
/// character, so the file is named by a digest; the file names the machine.
fn path(logs_dir: &Path, name: &str) -> PathBuf {
    let digest = Sha256::digest(name.as_bytes());
    let hex: String = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    logs_dir.join(CACHE).join(format!("{hex}.json"))
}

fn read_saved(path: &Path) -> Option<Saved> {
    serde_json::from_slice(&std::fs::read(path).ok()?)
        .ok()
        .filter(|saved: &Saved| saved.version == 1)
}

fn saved(logs_dir: &Path) -> Vec<Saved> {
    let Ok(entries) = std::fs::read_dir(logs_dir.join(CACHE)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .filter_map(|path| read_saved(&path))
        .collect()
}

/// Replaces the machine's snapshot of this view in one rename, so a reader
/// sees the old file or the new one, never part of either.
fn save(
    logs_dir: &Path,
    name: &str,
    query: &str,
    summary: &Summary,
    at: DateTime<Utc>,
) -> Result<()> {
    let path = path(logs_dir, name);
    let directory = path.parent().expect("cache file has a directory");
    std::fs::create_dir_all(directory)?;
    let mut saved = read_saved(&path)
        .filter(|saved| saved.machine == name)
        .unwrap_or_else(|| Saved {
            version: 1,
            machine: name.to_owned(),
            last_seen: at,
            views: BTreeMap::new(),
        });
    saved.last_seen = at;
    saved.views.insert(
        query.to_owned(),
        SavedView {
            fetched_at: at,
            summary: serde_json::to_value(summary)?,
        },
    );
    while saved.views.len() > VIEWS {
        let oldest = saved
            .views
            .iter()
            .min_by_key(|(_, view)| view.fetched_at)
            .map(|(key, _)| key.clone())
            .expect("more views than the limit");
        saved.views.remove(&oldest);
    }
    let mut file = tempfile::Builder::new()
        .prefix(".fleet-metrics-")
        .tempfile_in(directory)?;
    serde_json::to_writer(&mut file, &saved)?;
    file.persist(&path)?;
    Ok(())
}

#[cfg(test)]
mod tests;
