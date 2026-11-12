//! Explicit Fleet metrics client: each owning machine's own `/metrics`, merged
//! with this machine's, and the last summary read from each peer, saved so an
//! offline or retired machine stays visible. Owning-machine APIs stay local.
//! A machine that cannot be reached inbound pushes its own summary instead,
//! under a `report` token: it is saved in that machine's reported slot and
//! shown read-only (#2036 Q4.2).

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

/// Where runs failed. Its totals are counted again from the rows, so a saved
/// snapshot's Commands outside today's window drop out of both.
#[derive(Default, Serialize, Deserialize)]
struct SummaryFailures {
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
    /// The one machine asked for; missing for the whole Fleet.
    pub(crate) machine: Option<String>,
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
    /// When the summary shown was pushed by the machine itself, if it was.
    pub(crate) pushed_at: Option<DateTime<Utc>>,
    pub(crate) error: Option<String>,
}

impl Machine {
    /// Show `saved` in place of a live read: reported when the machine
    /// pushed it, else cached.
    fn stands_in(&mut self, saved: &Saved) {
        self.status = if saved.pushed_at.is_some() {
            Status::Reported
        } else {
            Status::Cached
        };
        self.last_seen = Some(saved.last_seen);
        self.pushed_at = saved.pushed_at;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Status {
    Ok,
    /// Unreachable now, or no longer in the Fleet: its saved snapshot shows.
    Cached,
    /// Not read from here: the summary it last pushed shows, read-only.
    Reported,
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
            Self::Reported => "reported",
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

/// A saved peer snapshot file: the machine's last read views for one
/// person. A peer answers each person by its own sign-in list, so a snapshot
/// stands in only for the person whose read it keeps; missing for this
/// machine's owner. A reported snapshot holds the views the machine pushed
/// itself, under a `report` token this Console issued to it, and stands in
/// for everyone who can read this Console.
#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    machine: String,
    #[serde(default)]
    person: Option<String>,
    last_seen: DateTime<Utc>,
    /// When the machine last pushed; missing on a snapshot read from it.
    #[serde(default)]
    pushed_at: Option<DateTime<Utc>>,
    views: BTreeMap<String, SavedView>,
}

/// Which saved snapshot of a machine: one person's reads, or what the
/// machine reported itself.
#[derive(Clone, Copy)]
enum Slot<'a> {
    Read(Option<&'a str>),
    Reported,
}

impl<'a> Slot<'a> {
    /// Whose reads the slot keeps; a reported slot keeps no person's.
    fn person(self) -> Option<&'a str> {
        match self {
            Self::Read(person) => person,
            Self::Reported => None,
        }
    }

    fn reported(self) -> bool {
        matches!(self, Self::Reported)
    }

    fn holds(self, saved: &Saved, name: &str) -> bool {
        saved.machine == name
            && saved.pushed_at.is_some() == self.reported()
            && saved.person.as_deref() == self.person()
    }
}

/// What a machine pushes under its `report` token: its own summary of one
/// `metrics` view, named for the machine the token was issued to.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MetricsReport {
    pub(crate) version: u32,
    pub(crate) machine: String,
    /// The view's `metrics` query: its window and filters.
    view: String,
    summary: Summary,
}

/// Why a pushed report was refused.
#[derive(Debug)]
pub(crate) enum Refusal {
    /// The report names a machine other than the token's holder.
    Mismatch,
    /// The report is not one this Console can read.
    Invalid(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mismatch => f.write_str("a report token uploads only its holder's own metrics"),
            Self::Invalid(why) => write!(f, "invalid metrics report: {why}"),
        }
    }
}

impl std::error::Error for Refusal {}

impl MetricsReport {
    /// This machine's summary of the view `asked` names. Its `machine` is
    /// left blank: the sender names, per destination, the holder that
    /// destination issued its token to.
    pub(crate) fn local(config: &UserConfig, asked: &MetricsArgs) -> Result<Self> {
        let trends = super::trends(config, asked)?;
        let summary = serde_json::from_value(serde_json::to_value(&trends)?)?;
        Ok(Self {
            version: 1,
            machine: String::new(),
            view: View::of(asked, &trends.window).query,
            summary,
        })
    }

    /// Save a report pushed under `holder`'s `report` token in `holder`'s
    /// reported slot, and nowhere else. `local` is this Console's own name,
    /// which no report may take: it would never be shown.
    pub(crate) fn receive(
        self,
        logs_dir: &Path,
        holder: &str,
        local: &str,
        now: DateTime<Utc>,
    ) -> Result<()> {
        if self.machine != holder {
            return Err(Refusal::Mismatch.into());
        }
        if holder == local {
            return Err(Refusal::Invalid(format!(
                "{holder} is this Console's own name; issue the report token to the machine's own Fleet name"
            ))
            .into());
        }
        if self.version != 1 {
            return Err(Refusal::Invalid(format!("unsupported version {}", self.version)).into());
        }
        let asked = view_arguments(&self.view)
            .map_err(|error| Refusal::Invalid(format!("view: {error:#}")))?;
        let view = View::of(&asked, &Window::of(&asked, now.date_naive()));
        save(logs_dir, holder, Slot::Reported, &view, &self.summary, now)
    }
}

/// A pushed view's query as the `metrics` arguments it names: only a window
/// and filters, never a Command, `--fleet` or `--machine`.
fn view_arguments(query: &str) -> Result<MetricsArgs> {
    let mut arguments = vec!["metrics".to_owned(), "--json".to_owned()];
    for (key, value) in serde_urlencoded::from_str::<Vec<(String, String)>>(query)? {
        if !VIEW_KEYS.contains(&key.as_str()) {
            bail!("unknown metrics argument: {key}");
        }
        arguments.extend([format!("--{key}"), value]);
    }
    let crate::args::Command::Metrics(asked) = crate::args::parse(&arguments)? else {
        bail!("not a metrics view");
    };
    Ok(asked)
}

#[derive(Serialize, Deserialize)]
struct SavedView {
    fetched_at: DateTime<Utc>,
    /// The view's filters, without its window, and whether it was weekly:
    /// what lets it stand in for another window's view.
    #[serde(default)]
    filters: Option<String>,
    #[serde(default)]
    weekly: Option<bool>,
    summary: Value,
}

/// The query keys that are `metrics` window and filters, each a `--<key>`
/// flag: all a view's query may hold.
pub(crate) const VIEW_KEYS: [&str; 10] = [
    "window", "grain", "from", "through", "repo", "kind", "harness", "model", "version", "account",
];

/// What each machine is asked: the `metrics` query in a fixed order, so its
/// saved snapshot is found again by the same text, and the parts that let a
/// snapshot of another window stand in.
pub(crate) struct View {
    pub(crate) query: String,
    filters: String,
    weekly: bool,
}

impl View {
    pub(crate) fn of(asked: &MetricsArgs, window: &Window) -> Self {
        let (windowing, filters) = pairs(asked);
        let encode = |pairs: &[(&str, String)]| {
            serde_urlencoded::to_string(pairs).expect("metrics query pairs encode")
        };
        Self {
            query: encode(&[windowing, filters.clone()].concat()),
            filters: encode(&filters),
            weekly: window.weekly(),
        }
    }
}

/// `metrics` query pairs, each a `--<key>` flag and its value.
type Pairs = Vec<(&'static str, String)>;

/// The window's query pairs, then the filters'.
fn pairs(asked: &MetricsArgs) -> (Pairs, Pairs) {
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
    let mut filters = Vec::new();
    for (key, values) in [
        ("repo", &asked.repositories),
        ("kind", &asked.kinds),
        ("harness", &asked.harnesses),
        ("model", &asked.models),
        ("version", &asked.versions),
        ("account", &asked.accounts),
    ] {
        filters.extend(values.iter().map(|value| (key, value.clone())));
    }
    (pairs, filters)
}

/// Whether Fleet is the Console's default Metrics view: a paired machine, or
/// `person`'s saved snapshot of one or a reported one, exists.
pub(crate) fn default_view(config: &UserConfig, person: Option<&str>) -> bool {
    let credentials = Credentials::read().ok();
    config.console.fleet.iter().any(|entry| {
        credentials
            .as_ref()
            .is_some_and(|credentials| credentials.fleet_token(&entry.name).is_some())
    }) || !saved(&config.logs_dir, person).is_empty()
}

/// Every machine Fleet metrics can be scoped to besides this one: the Fleet
/// in config order, then retired or reporting machines with a snapshot
/// `person` may see.
pub(crate) fn others(config: &UserConfig, person: Option<&str>) -> Vec<String> {
    let local = crate::console::fleet_control::local_name(config);
    let mut names: Vec<_> = config
        .console
        .fleet
        .iter()
        .map(|entry| entry.name.clone())
        .collect();
    let mut retired: Vec<_> = saved(&config.logs_dir, person)
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
    let chosen = asked.machine.as_deref();
    if let Some(name) = chosen
        && !others(config, person).iter().any(|known| known == name)
    {
        bail!("unknown Fleet machine: {name}");
    }
    let mut sources = Vec::new();
    let window = if chosen.is_none() {
        let trends = super::trends(config, asked)?;
        let summary = serde_json::from_value(serde_json::to_value(&trends)?)?;
        sources.push((
            Machine {
                name: local.clone(),
                address: None,
                local: true,
                status: Status::Ok,
                last_seen: Some(now),
                pushed_at: None,
                error: None,
            },
            Some(summary),
        ));
        trends.window
    } else {
        Window::of(asked, now.date_naive())
    };
    let view = View::of(asked, &window);
    let entries: Vec<_> = config
        .console
        .fleet
        .iter()
        .filter(|entry| chosen.is_none_or(|name| name == entry.name))
        .collect();
    // Independent requests, each bounded by the Fleet transport's three
    // seconds, so one stalled peer cannot hold the page.
    sources.extend(std::thread::scope(|threads| {
        let handles: Vec<_> = entries
            .iter()
            .map(|entry| threads.spawn(|| remote(config, entry, person, &view, &window, now)))
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("Fleet metrics reader panicked"))
            .collect::<Vec<_>>()
    }));
    let paired: Vec<_> = config.console.fleet.iter().map(|e| &e.name).collect();
    let mut retired: Vec<_> = saved(&config.logs_dir, person)
        .into_iter()
        .filter(|saved| {
            saved.machine != local
                && !paired.contains(&&saved.machine)
                && chosen.is_none_or(|name| name == saved.machine)
        })
        .collect();
    retired.sort_by(|a, b| a.machine.cmp(&b.machine));
    for saved in retired {
        let mut machine = Machine {
            name: saved.machine.clone(),
            address: None,
            local: false,
            status: Status::Cached,
            last_seen: None,
            pushed_at: None,
            error: None,
        };
        machine.stands_in(&saved);
        if machine.status == Status::Cached {
            note(&mut machine, "no longer in this Console's Fleet");
        }
        let summary = cached(&mut machine, saved, &view, &window);
        sources.push((machine, summary));
    }
    Ok(merge(
        chosen.map(str::to_owned),
        window,
        Filters::of(asked),
        sources,
    ))
}

fn remote(
    config: &UserConfig,
    entry: &Entry,
    person: Option<&str>,
    view: &View,
    window: &Window,
    now: DateTime<Utc>,
) -> (Machine, Option<Summary>) {
    let mut machine = Machine {
        name: entry.name.clone(),
        address: Some(entry.address.clone()),
        local: false,
        status: Status::Ok,
        last_seen: None,
        pushed_at: None,
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
        let route = if view.query.is_empty() {
            "/metrics".to_owned()
        } else {
            format!("/metrics?{}", view.query)
        };
        let summary =
            serde_json::from_value(remote.get(&route)?).context("incompatible metrics summary")?;
        Ok(Some(summary))
    };
    let reported = || {
        read_saved(&path(&config.logs_dir, &entry.name, Slot::Reported))
            .filter(|saved| Slot::Reported.holds(saved, &entry.name))
    };
    match read() {
        // A link-only machine may still push its own summary here.
        Ok(None) => match reported() {
            Some(saved) => {
                machine.stands_in(&saved);
                let summary = cached(&mut machine, saved, view, window);
                (machine, summary)
            }
            None => {
                machine.status = Status::LinkOnly;
                (machine, None)
            }
        },
        Ok(Some(summary)) => {
            machine.last_seen = Some(now);
            // The saved snapshot is a convenience; failing to keep it must
            // not cost the live view.
            if let Err(error) = save(
                &config.logs_dir,
                &entry.name,
                Slot::Read(person),
                view,
                &summary,
                now,
            ) {
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
            // The newer of this person's last read and the machine's own report.
            let Some(saved) = read_saved(&path(&config.logs_dir, &entry.name, Slot::Read(person)))
                .filter(|saved| Slot::Read(person).holds(saved, &entry.name))
                .into_iter()
                .chain(reported())
                .max_by_key(|saved| saved.last_seen)
            else {
                return (machine, None);
            };
            machine.stands_in(&saved);
            let summary = cached(&mut machine, saved, view, window);
            (machine, summary)
        }
    }
}

/// A saved view's summary. Without this view's own, the newest one read with
/// the same filters stands in, its buckets regrouped by week when this view
/// is weekly; a weekly snapshot cannot stand in for a daily view.
fn cached(
    machine: &mut Machine,
    mut saved: Saved,
    view: &View,
    window: &Window,
) -> Option<Summary> {
    if let Some(summary) = saved
        .views
        .remove(&view.query)
        .and_then(|saved| serde_json::from_value(saved.summary).ok())
    {
        return Some(summary);
    }
    let stand_in = saved
        .views
        .into_values()
        .filter(|saved| {
            saved.filters.as_deref() == Some(view.filters.as_str())
                && saved.weekly.is_some_and(|weekly| view.weekly || !weekly)
        })
        .max_by_key(|saved| saved.fetched_at)
        .and_then(|saved| serde_json::from_value::<Summary>(saved.summary).ok());
    let Some(mut summary) = stand_in else {
        note(machine, "no saved snapshot for this view");
        return None;
    };
    note(machine, "showing its snapshot of another window");
    // Its comparison covers that window's dates, never this one's.
    summary.comparison = None;
    let mut buckets = BTreeMap::<_, SummaryBucket>::new();
    for row in summary.outcomes {
        let bucket = window.bucket_on(row.bucket);
        let merged = buckets
            .entry((bucket, row.account.clone(), row.kind.clone()))
            .or_insert_with(|| SummaryBucket {
                bucket,
                account: row.account,
                kind: row.kind,
                tally: Tally::default(),
                failed_commands: Vec::new(),
            });
        merged.tally.merge(&row.tally);
        merged.failed_commands.extend(row.failed_commands);
    }
    summary.outcomes = buckets.into_values().collect();
    Some(summary)
}

fn note(machine: &mut Machine, text: &str) {
    machine.error = Some(match machine.error.take() {
        Some(error) => format!("{error}; {text}"),
        None => text.to_owned(),
    });
}

fn merge(
    machine: Option<String>,
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
            for mut row in summary.where_runs_failed.rows {
                // A cached snapshot may predate today's window, as above.
                row.commands.retain(|command| {
                    match DateTime::parse_from_rfc3339(&command.started_at) {
                        Ok(started) => {
                            window.shows(window.bucket_on(started.with_timezone(&Utc).date_naive()))
                        }
                        // An unreadable start cannot place it outside the window.
                        Err(_) => true,
                    }
                });
                if row.commands.is_empty() {
                    continue;
                }
                failures.failed += row.commands.len();
                if row.recorded {
                    failures.observed += row.commands.len();
                } else {
                    failures.missing += row.commands.len();
                }
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
        machine,
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

/// A name's saved snapshot file in `slot`. Names are printable but may hold
/// any character, so the file is named by a digest; the file names the
/// machine, and the person or that the machine reported it.
fn path(logs_dir: &Path, name: &str, slot: Slot<'_>) -> PathBuf {
    let mut digest = Sha256::new();
    digest.update(name.as_bytes());
    match slot {
        Slot::Read(None) => {}
        Slot::Read(Some(person)) => {
            digest.update([0]);
            digest.update(person.as_bytes());
        }
        // No printable name or person holds a 1 byte.
        Slot::Reported => digest.update([1]),
    }
    let digest = digest.finalize();
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

/// The snapshots `person` may see, one per machine: the newer of their own
/// last read and the machine's own report.
fn saved(logs_dir: &Path, person: Option<&str>) -> Vec<Saved> {
    let Ok(entries) = std::fs::read_dir(logs_dir.join(CACHE)) else {
        return Vec::new();
    };
    let mut newest = BTreeMap::<String, Saved>::new();
    for saved in entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .filter_map(|path| read_saved(&path))
        .filter(|saved| {
            Slot::Read(person).holds(saved, &saved.machine)
                || Slot::Reported.holds(saved, &saved.machine)
        })
    {
        if newest
            .get(&saved.machine)
            .is_none_or(|kept| kept.last_seen < saved.last_seen)
        {
            newest.insert(saved.machine.clone(), saved);
        }
    }
    newest.into_values().collect()
}

/// Replaces the machine's snapshot of this view in one rename, so a reader
/// sees the old file or the new one, never part of either.
fn save(
    logs_dir: &Path,
    name: &str,
    slot: Slot<'_>,
    view: &View,
    summary: &Summary,
    at: DateTime<Utc>,
) -> Result<()> {
    let path = path(logs_dir, name, slot);
    let directory = path.parent().expect("cache file has a directory");
    std::fs::create_dir_all(directory)?;
    // Hold the slot across its read and replacement, so two writes at once
    // each keep their view.
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("lock"))?;
    lock.lock()?;
    let mut saved = read_saved(&path)
        .filter(|saved| slot.holds(saved, name))
        .unwrap_or_else(|| Saved {
            version: 1,
            machine: name.to_owned(),
            person: slot.person().map(str::to_owned),
            last_seen: at,
            pushed_at: None,
            views: BTreeMap::new(),
        });
    saved.last_seen = at;
    if slot.reported() {
        saved.pushed_at = Some(at);
    }
    saved.views.insert(
        view.query.clone(),
        SavedView {
            fetched_at: at,
            filters: Some(view.filters.clone()),
            weekly: Some(view.weekly),
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
