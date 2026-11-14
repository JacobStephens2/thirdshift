//! Fleet Plan usage: this machine's view beside each Fleet machine's own
//! (`/plan-usage?local=true`, so no machine relays another's), with shared
//! subscription logins grouped across machines by account identity. The
//! last view read from each peer is saved so an offline machine still shows,
//! and a machine that cannot be reached inbound pushes its own view after
//! each Command under a `report` token, saved in its reported slot and shown
//! read-only. Routing never reads any of this: it stays machine-local
//! (ADR 0031).

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::store::State;
use super::view::{self, ConfiguredRoute, Row, View};
use crate::config::{UserConfig, console::fleet::Entry};
use crate::console::fleet::read::{Failure, Remote};
pub(crate) use crate::console::fleet::snapshot::Status;
use crate::console::fleet::snapshot::{Held, Slot};
use crate::harness::Harness;
use crate::resend_key::Credentials;

/// The per-machine peer snapshot cache, under the watching account's logs.
const CACHE: &str = "fleet-plan-usage";
/// What a Fleet machine is asked: its own view alone.
pub(crate) const PEER_ROUTE: &str = "/plan-usage?local=true";
/// The longest string a peer's view may hold. Every fact Plan usage keeps is
/// a short token, label or time.
const LONGEST: usize = 256;

/// Plan usage across the Fleet, or one Fleet machine's.
#[derive(Serialize)]
pub struct Fleet {
    pub version: u8,
    /// The one machine asked for; missing for the whole Fleet.
    pub machine: Option<String>,
    pub machines: Vec<Machine>,
    /// One row per login: a shared subscription once, whatever machines use
    /// it, and every login or Harness of unknown identity on its own.
    pub harnesses: Vec<Login>,
    #[serde(skip)]
    pub now: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct Machine {
    pub name: String,
    pub address: Option<String>,
    pub local: bool,
    pub status: Status,
    /// When this machine's view was last read: now when live, the saved
    /// snapshot's time when cached or reported.
    pub last_seen: Option<DateTime<Utc>>,
    /// When the view shown was pushed by the machine itself, if it was.
    pub pushed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

impl Machine {
    fn new(name: &str, address: Option<&str>, local: bool) -> Self {
        Self {
            name: name.to_owned(),
            address: address.map(str::to_owned),
            local,
            status: Status::Ok,
            last_seen: None,
            pushed_at: None,
            error: None,
        }
    }

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

    fn note(&mut self, text: &str) {
        self.error = Some(match self.error.take() {
            Some(error) => format!("{error}; {text}"),
            None => text.to_owned(),
        });
    }

    /// Whether a person looking at this machine sees a live answer.
    pub fn live(&self) -> bool {
        self.status == Status::Ok
    }
}

/// One subscription login's Plan usage across the machines using it.
#[derive(Serialize)]
pub struct Login {
    #[serde(serialize_with = "super::route::harness_name")]
    pub harness: Harness,
    /// The account identity the Harness reports, which machines share;
    /// missing for a Harness that names none, shown per machine.
    pub identity: Option<String>,
    /// Every machine using this login, in Fleet order.
    pub machines: Vec<String>,
    /// The machine whose observation shows: the freshest.
    pub observed_on: String,
    /// Whether the CLI is on PATH where the observation shows.
    pub installed: bool,
    /// The machines whose factory sessions would start on it now.
    pub active_on: Vec<String>,
    /// Each machine's User config routes to this Harness, with their
    /// availability there.
    pub routes: Vec<MachineRoute>,
    #[serde(flatten)]
    pub state: State,
}

#[derive(Serialize)]
pub struct MachineRoute {
    pub machine: String,
    #[serde(flatten)]
    pub route: ConfiguredRoute,
}

/// The account identity that makes two machines' rows one login: Claude's
/// organization and email, Codex's account id, Muse's account label. Agy
/// and Grok name no account, so their rows stay per machine.
fn identity(row: &Row) -> Option<String> {
    let account = row.state.account.as_ref()?;
    match row.harness {
        Harness::Claude => {
            let parts: Vec<String> = account
                .organization
                .iter()
                .map(|organization| format!("organization {organization}"))
                .chain(account.label.clone())
                .collect();
            (!parts.is_empty()).then(|| parts.join(" · "))
        }
        Harness::Codex => account.id.as_ref().map(|id| format!("account {id}")),
        Harness::Muse => account.label.clone(),
        Harness::Agy | Harness::Grok | Harness::OpenCode | Harness::Kimi => None,
    }
}

/// When a row's state was last reported, by a session or a read.
fn freshness(state: &State) -> (Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
    (state.observed_at, state.read_at)
}

/// Each machine's rows, a login once: grouped by Harness and account
/// identity, showing the freshest observation and naming every machine on
/// it. A row without an identity stays its machine's own.
pub(crate) fn group(sources: &[(String, View)]) -> Vec<Login> {
    let mut logins: Vec<Login> = Vec::new();
    for (machine, view) in sources {
        for row in &view.harnesses {
            let identity = identity(row);
            let found = logins.iter().position(|kept| {
                kept.harness == row.harness
                    && match (&identity, &kept.identity) {
                        (Some(identity), Some(kept)) => identity == kept,
                        (None, None) => kept.machines == [machine.clone()],
                        _ => false,
                    }
            });
            let index = match found {
                Some(index) => index,
                None => {
                    logins.push(Login {
                        harness: row.harness,
                        identity,
                        machines: Vec::new(),
                        observed_on: machine.clone(),
                        installed: row.installed,
                        active_on: Vec::new(),
                        routes: Vec::new(),
                        state: row.state.clone(),
                    });
                    logins.len() - 1
                }
            };
            let kept = &mut logins[index];
            if !kept.machines.contains(machine) {
                kept.machines.push(machine.clone());
            }
            if freshness(&row.state) > freshness(&kept.state) {
                kept.state = row.state.clone();
                kept.observed_on = machine.clone();
                kept.installed = row.installed;
            }
            if row.active {
                kept.active_on.push(machine.clone());
            }
            kept.routes
                .extend(row.routes.iter().map(|route| MachineRoute {
                    machine: machine.clone(),
                    route: route.clone(),
                }));
        }
    }
    logins
}

/// A saved peer snapshot: the last view read from a machine for one person,
/// or the last one the machine pushed itself.
#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    machine: String,
    #[serde(default)]
    person: Option<String>,
    last_seen: DateTime<Utc>,
    #[serde(default)]
    pushed_at: Option<DateTime<Utc>>,
    view: View,
}

impl Held for Saved {
    fn machine(&self) -> &str {
        &self.machine
    }

    fn person(&self) -> Option<&str> {
        self.person.as_deref()
    }

    fn pushed(&self) -> bool {
        self.pushed_at.is_some()
    }
}

fn read_saved(path: &Path) -> Option<Saved> {
    // Through a Value: a flattened state's utilization cannot be read
    // straight from the bytes.
    let value: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    serde_json::from_value(value)
        .ok()
        .filter(|saved: &Saved| saved.version == 1)
}

/// `name`'s snapshot in `slot`, if one is saved there.
fn exact(logs_dir: &Path, name: &str, slot: Slot<'_>) -> Option<Saved> {
    read_saved(&slot.path(logs_dir, CACHE, name)).filter(|saved| slot.holds(saved, name))
}

/// Every machine's newest snapshot `person` may see: their own reads, or
/// what a machine reported.
fn discovered(logs_dir: &Path, person: Option<&str>) -> Vec<Saved> {
    let Ok(entries) = std::fs::read_dir(logs_dir.join(CACHE)) else {
        return Vec::new();
    };
    let mut newest: Vec<Saved> = Vec::new();
    for path in entries.flatten().map(|entry| entry.path()) {
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let Some(saved) = read_saved(&path).filter(|saved| {
            Slot::Read(person).holds(saved, &saved.machine)
                || Slot::Reported.holds(saved, &saved.machine)
        }) else {
            continue;
        };
        match newest.iter_mut().find(|kept| kept.machine == saved.machine) {
            Some(kept) if kept.last_seen < saved.last_seen => *kept = saved,
            Some(_) => {}
            None => newest.push(saved),
        }
    }
    newest.sort_by(|a, b| a.machine.cmp(&b.machine));
    newest
}

/// Replace `name`'s snapshot in `slot` in one rename, so a reader sees the
/// old file or the new one, never part of either.
fn save(logs_dir: &Path, name: &str, slot: Slot<'_>, view: View, at: DateTime<Utc>) -> Result<()> {
    let path: PathBuf = slot.path(logs_dir, CACHE, name);
    let directory = path.parent().expect("cache file has a directory");
    std::fs::create_dir_all(directory)?;
    let saved = Saved {
        version: 1,
        machine: name.to_owned(),
        person: slot.person().map(str::to_owned),
        last_seen: at,
        pushed_at: slot.reported().then_some(at),
        view,
    };
    let mut file = tempfile::Builder::new()
        .prefix(".fleet-plan-usage-")
        .tempfile_in(directory)?;
    serde_json::to_writer(&mut file, &saved)?;
    file.persist(&path)?;
    Ok(())
}

/// Another machine's view, as it answered or pushed it. Its strings are
/// shown in the terminal and on the page, so any that is long or holds a
/// control character makes the whole view unreadable.
fn peer_view(value: Value) -> Result<View> {
    fn check(value: &Value) -> Result<()> {
        match value {
            Value::String(text) => {
                if text.len() > LONGEST || text.chars().any(char::is_control) {
                    bail!("incompatible Plan usage view: unexpected text");
                }
                Ok(())
            }
            Value::Array(items) => items.iter().try_for_each(check),
            Value::Object(fields) => fields.iter().try_for_each(|(key, value)| {
                check(&Value::String(key.clone()))?;
                check(value)
            }),
            Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
        }
    }
    check(&value)?;
    let view: View = serde_json::from_value(value).context("incompatible Plan usage view")?;
    if view.version != 1 {
        bail!(
            "incompatible Plan usage view: unsupported version {}",
            view.version
        );
    }
    Ok(view)
}

/// What one Plan usage operation knows of the Fleet before reading any
/// machine: whether one is paired, and every saved view `person` may see.
/// Taken once, so the default view, the machine choices and the view shown
/// agree however reports land meanwhile.
pub(crate) struct Capture<'a> {
    config: &'a UserConfig,
    person: Option<&'a str>,
    local: String,
    paired: bool,
    discovered: Vec<Saved>,
}

/// Which Fleet view to load.
pub(crate) enum Reach<'a> {
    /// Every machine, led by this machine's own view.
    Fleet(View),
    /// One other machine, by its Fleet name.
    Machine(&'a str),
}

impl<'a> Capture<'a> {
    pub(crate) fn new(config: &'a UserConfig, person: Option<&'a str>) -> Self {
        let credentials = Credentials::read().ok();
        let paired = config.console.fleet.iter().any(|entry| {
            credentials
                .as_ref()
                .is_some_and(|credentials| credentials.fleet_token(&entry.name).is_some())
        });
        Self {
            config,
            person,
            local: crate::console::fleet_control::local_name(config),
            paired,
            discovered: discovered(&config.logs_dir, person),
        }
    }

    /// This machine's name, distinct from every Fleet entry's.
    pub(crate) fn local(&self) -> &str {
        &self.local
    }

    /// Whether Fleet is the default Plan usage view: a machine is paired,
    /// or a saved view of another machine exists.
    pub(crate) fn default_view(&self) -> bool {
        self.paired
            || self
                .discovered
                .iter()
                .any(|saved| saved.machine != self.local)
    }

    /// Every machine Plan usage can be scoped to besides this one: the
    /// Fleet in config order, then retired or reporting machines with a
    /// saved view.
    pub(crate) fn others(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .config
            .console
            .fleet
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        for saved in &self.discovered {
            if saved.machine != self.local && !names.contains(&saved.machine) {
                names.push(saved.machine.clone());
            }
        }
        names
    }

    /// Fleet Plan usage, or one Fleet machine's.
    pub(crate) fn load(&self, reach: Reach<'_>) -> Result<Fleet> {
        let config = self.config;
        let person = self.person;
        let now = Utc::now();
        let mut sources: Vec<(Machine, Option<View>)> = Vec::new();
        let chosen = match reach {
            Reach::Fleet(view) => {
                let mut machine = Machine::new(&self.local, None, true);
                machine.last_seen = Some(now);
                sources.push((machine, Some(view)));
                None
            }
            Reach::Machine(name) => {
                if !self.others().iter().any(|known| known == name) {
                    bail!("unknown Fleet machine: {name}");
                }
                Some(name)
            }
        };
        let entries: Vec<&Entry> = config
            .console
            .fleet
            .iter()
            .filter(|entry| chosen.is_none_or(|name| name == entry.name))
            .collect();
        // Independent requests, each bounded by the Fleet transport's three
        // seconds, so one stalled peer cannot hold the view.
        sources.extend(std::thread::scope(|threads| {
            let handles: Vec<_> = entries
                .iter()
                .map(|entry| threads.spawn(|| remote(config, person, entry, now)))
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("Fleet Plan usage reader panicked"))
                .collect::<Vec<_>>()
        }));
        for saved in &self.discovered {
            if saved.machine == self.local
                || config
                    .console
                    .fleet
                    .iter()
                    .any(|entry| entry.name == saved.machine)
                || chosen.is_some_and(|name| name != saved.machine)
            {
                continue;
            }
            let mut machine = Machine::new(&saved.machine, None, false);
            machine.stands_in(saved);
            if machine.status == Status::Cached {
                machine.note("no longer in this Console's Fleet");
            }
            sources.push((machine, Some(saved.view.clone())));
        }
        let views: Vec<(String, View)> = sources
            .iter_mut()
            .filter_map(|(machine, view)| Some((machine.name.clone(), view.take()?)))
            .collect();
        Ok(Fleet {
            version: 1,
            machine: chosen.map(str::to_owned),
            machines: sources.into_iter().map(|(machine, _)| machine).collect(),
            harnesses: group(&views),
            now,
        })
    }
}

fn remote(
    config: &UserConfig,
    person: Option<&str>,
    entry: &Entry,
    now: DateTime<Utc>,
) -> (Machine, Option<View>) {
    let mut machine = Machine::new(&entry.name, Some(&entry.address), false);
    let read = || -> Result<Option<View>> {
        // A link never sends even an identity probe. The transport
        // revalidates paired destinations before attaching any saved token.
        if Credentials::read()?.fleet_token(&entry.name).is_none() {
            return Ok(None);
        }
        let remote = Remote::select(config, &entry.name, person.map(str::to_owned))?
            .context("expected a Fleet machine")?;
        Ok(Some(peer_view(remote.get(PEER_ROUTE)?)?))
    };
    match read() {
        // A link-only machine may still push its own view here.
        Ok(None) => match exact(&config.logs_dir, &entry.name, Slot::Reported) {
            Some(saved) => {
                machine.stands_in(&saved);
                (machine, Some(saved.view))
            }
            None => {
                machine.status = Status::LinkOnly;
                (machine, None)
            }
        },
        Ok(Some(view)) => {
            machine.last_seen = Some(now);
            // The saved snapshot is a convenience; failing to keep it must
            // not cost the live view.
            let kept = save(
                &config.logs_dir,
                &entry.name,
                Slot::Read(person),
                view.clone(),
                now,
            );
            if let Err(error) = kept {
                machine.note(&format!("snapshot not saved: {error:#}"));
            }
            (machine, Some(view))
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
            let saved = exact(&config.logs_dir, &entry.name, Slot::Read(person))
                .into_iter()
                .chain(exact(&config.logs_dir, &entry.name, Slot::Reported))
                .max_by_key(|saved| saved.last_seen);
            match saved {
                Some(saved) => {
                    machine.stands_in(&saved);
                    (machine, Some(saved.view))
                }
                None => (machine, None),
            }
        }
    }
}

/// What a machine pushes under its `report` token: its own Plan usage view,
/// named for the machine the token was issued to.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Report {
    pub(crate) version: u32,
    pub(crate) machine: String,
    pub(crate) view: Value,
}

/// Why a pushed report was refused.
#[derive(Debug)]
pub(crate) enum Refusal {
    /// The report names a machine other than the token's holder.
    Mismatch,
    /// The report is not one this Console can read.
    Invalid(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mismatch => {
                f.write_str("a report token uploads only its holder's own Plan usage")
            }
            Self::Invalid(why) => write!(f, "invalid Plan usage report: {why}"),
        }
    }
}

impl std::error::Error for Refusal {}

impl Report {
    /// This machine's saved Plan usage, as `thirdshift plan-usage --local`
    /// would show it without running any read: the sessions that just ran
    /// keep it fresh. Its `machine` is left blank: the sender names, per
    /// destination, the holder that destination issued its token to.
    pub(crate) fn local(config: &UserConfig) -> Result<Self> {
        Ok(Self {
            version: 1,
            machine: String::new(),
            view: serde_json::to_value(super::saved_view(config)?)?,
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
        let view = peer_view(self.view).map_err(|error| Refusal::Invalid(format!("{error:#}")))?;
        save(logs_dir, holder, Slot::Reported, view, now)
    }
}

impl fmt::Display for Machine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)?;
        if self.local {
            f.write_str(" (this machine)")?;
        }
        write!(f, " · status: {}", self.status)?;
        for (label, at) in [("last seen", self.last_seen), ("pushed", self.pushed_at)] {
            if let Some(at) = at.filter(|_| !self.live()) {
                write!(f, " · {label} {}", view::local(at))?;
            }
        }
        if let Some(address) = &self.address {
            write!(f, " · address: {address}")?;
        }
        if let Some(error) = &self.error {
            write!(f, " · {error}")?;
        }
        Ok(())
    }
}

impl fmt::Display for Fleet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Machines")?;
        for machine in &self.machines {
            writeln!(f, "  {machine}")?;
        }
        writeln!(f)?;
        if self.harnesses.is_empty() {
            return f.write_str(view::EMPTY);
        }
        for (index, login) in self.harnesses.iter().enumerate() {
            if index > 0 {
                writeln!(f)?;
            }
            write!(f, "{}", login.harness.name())?;
            if login.identity.is_some() && login.machines.len() > 1 {
                write!(f, ", shared by {}", login.machines.join(", "))?;
            } else {
                write!(f, " on {}", login.machines.join(", "))?;
            }
            if !login.active_on.is_empty() {
                write!(f, " (active on {})", login.active_on.join(", "))?;
            }
            writeln!(f)?;
            if let Some(identity) = &login.identity {
                writeln!(f, "  login: {identity}")?;
            }
            for route in &login.routes {
                writeln!(f, "  route on {}: {}", route.machine, route.route)?;
            }
            if login.machines.len() > 1 {
                writeln!(f, "  freshest on {}", login.observed_on)?;
            }
            view::write_state(f, &login.state, login.installed, self.now)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
