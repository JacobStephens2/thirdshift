//! Plan usage: how much of each subscription Harness's allowance is used,
//! saved per machine. Running sessions update it passively from structured
//! wire frames; `thirdshift plan-usage` and the Console's `/plan-usage`
//! refresh it on demand with official non-billed reads, at most once a
//! minute per Harness, and never on a background timer. Both show the
//! Fleet by default once it has one, grouping shared logins. A Command
//! routed by the User config's routes reads it before each session Step
//! ([`router`]), refreshing a stale snapshot of a Harness whose read reports
//! Plan windows first, and never asks a Fleet peer.

use std::time::Duration;

use anyhow::{Result, bail};
use chrono::{TimeDelta, Utc};

pub(crate) mod fleet;
pub(crate) mod observation;
mod read;
pub mod route;
pub mod router;
mod store;
mod view;

pub use observation::Observation;
pub use route::Routing;
pub use router::Router;
use store::Store;
pub(crate) use view::EMPTY;
pub use view::View;

use crate::config::UserConfig;
use crate::harness::{Choice, Harness};

/// How long a Harness's non-billed read stays fresh for the view.
const FRESH: Duration = Duration::from_secs(60);

/// Which machines `thirdshift plan-usage` shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Selection {
    /// The Fleet once a machine is paired or saved, else this machine.
    #[default]
    Default,
    Fleet,
    /// One machine by its Fleet name; this machine's own name is `Local`.
    Machine(String),
    /// This machine alone, as a Fleet machine is asked.
    Local,
}

/// What `thirdshift plan-usage` was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Show {
        selection: Selection,
        json: bool,
    },
    /// Forget recorded cooldowns for one Harness, or all with none.
    Clear {
        harness: Option<Harness>,
        json: bool,
    },
    /// Send this machine's saved Plan usage to each report destination, or
    /// the one named.
    Push {
        destination: Option<String>,
    },
}

/// What a Plan usage view shows: one machine's rows, or the Fleet's logins.
pub enum Shown {
    Local(View),
    Fleet(fleet::Fleet),
}

impl Shown {
    pub fn json(&self) -> Result<serde_json::Value> {
        Ok(match self {
            Self::Local(view) => serde_json::to_value(view)?,
            Self::Fleet(fleet) => serde_json::to_value(fleet)?,
        })
    }
}

impl std::fmt::Display for Shown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local(view) => view.fmt(f),
            Self::Fleet(fleet) => fleet.fmt(f),
        }
    }
}

/// Save what running sessions observed. Plan usage is evidence, never the
/// session's outcome, so a snapshot that can't be written costs nothing.
pub(crate) fn record(observations: Vec<Observation>) {
    if observations.is_empty() || cfg!(test) {
        return;
    }
    let Ok(store) = Store::machine() else {
        return;
    };
    let now = Utc::now();
    let _ = store.update_all(|harnesses| {
        for observation in &observations {
            harnesses
                .entry(observation.harness.name().into())
                .or_default()
                .observe(observation, "session", now);
        }
    });
}

/// Whether a headless session on `choice` may spend credit balances once
/// its included Plan windows are used up: only when it runs as a User
/// config route with `allow_credits = true`.
pub(crate) fn credits_allowed(choice: &Choice) -> bool {
    if cfg!(test) {
        return false;
    }
    UserConfig::load().is_ok_and(|config| config.routing.allows_credits(choice, &config.harness))
}

/// `harness` as a subscription Harness for `--clear`.
pub fn subscription_named(name: &str) -> Result<Harness> {
    match Harness::named(name) {
        Some(harness) if observation::subscription(harness) => Ok(harness),
        _ => bail!(
            "{name:?} is not a subscription Harness: Plan usage watches {}",
            observation::SUBSCRIPTION.map(Harness::name).join(", ")
        ),
    }
}

/// Run each installed readable Harness's non-billed read whose last one is
/// more than a minute old, side by side.
fn refresh(store: &Store) -> Result<()> {
    let saved = store.read()?;
    let now = Utc::now();
    let stale: Vec<Harness> = read::READABLE
        .into_iter()
        .filter(|harness| harness.installed())
        .filter(|harness| !read_lately(saved.get(harness.name()), now))
        .collect();
    read_into(store, &stale)
}

/// Whether `state`'s non-billed read ran within the last minute, so it is
/// not run again yet.
fn read_lately(state: Option<&store::State>, now: chrono::DateTime<Utc>) -> bool {
    let fresh = TimeDelta::from_std(FRESH).expect("a minute fits");
    state
        .and_then(|state| state.read_at)
        .is_some_and(|read| read <= now && now - read < fresh)
}

/// Run each of `stale`'s non-billed reads side by side, and take their
/// answers into the snapshot.
fn read_into(store: &Store, stale: &[Harness]) -> Result<()> {
    let read: Vec<(Harness, read::Reading)> = std::thread::scope(|scope| {
        let reads: Vec<_> = stale
            .iter()
            .map(|harness| scope.spawn(move || read::read(*harness).map(|read| (*harness, read))))
            .collect();
        reads
            .into_iter()
            .filter_map(|read| read.join().ok().flatten())
            .collect()
    });
    if read.is_empty() {
        return Ok(());
    }
    // Sessions may have written meanwhile: take the answers into the
    // snapshot as it is now.
    let now = Utc::now();
    store.update_all(|harnesses| {
        for (harness, reading) in read {
            reading.apply(harnesses.entry(harness.name().into()).or_default(), now);
        }
    })
}

/// This machine's view, after refreshing its non-billed reads.
fn local_view(config: &UserConfig) -> Result<View> {
    let store = Store::machine()?;
    refresh(&store)?;
    Ok(View::of(
        config,
        store.read()?,
        Harness::installed,
        Utc::now(),
    ))
}

/// This machine's view from what is saved, running no read: what the
/// Console header and a pushed report show.
pub(crate) fn saved_view(config: &UserConfig) -> Result<View> {
    Ok(View::of(
        config,
        Store::machine()?.read()?,
        Harness::installed,
        Utc::now(),
    ))
}

/// The view `selection` asks for, from what `capture` took of the Fleet:
/// read on behalf of its person when a Console reads for someone signed in.
pub(crate) fn show(
    config: &UserConfig,
    selection: &Selection,
    capture: &fleet::Capture<'_>,
) -> Result<Shown> {
    let reach = match selection {
        Selection::Machine(name) if name != capture.local() => fleet::Reach::Machine(name),
        Selection::Fleet => fleet::Reach::Fleet(local_view(config)?),
        Selection::Default if capture.default_view() => fleet::Reach::Fleet(local_view(config)?),
        Selection::Local | Selection::Machine(_) | Selection::Default => {
            return Ok(Shown::Local(local_view(config)?));
        }
    };
    Ok(Shown::Fleet(capture.load(reach)?))
}

/// Forget recorded cooldowns on this machine, and say which Harnesses'.
pub(crate) fn clear(harness: Option<Harness>) -> Result<Vec<&'static str>> {
    Store::machine()?.clear(harness)?;
    Ok(match harness {
        Some(harness) => vec![harness.name()],
        None => observation::SUBSCRIPTION.map(Harness::name).to_vec(),
    })
}

/// Before routing a session Step, run the non-billed read of each of
/// `harnesses` that reports Plan windows ([`read::WINDOWED`]) and is
/// installed, when its saved snapshot is stale for routing, side by side.
/// Only this machine is read.
fn refresh_for_routing(store: &Store, harnesses: &[Harness]) -> Result<()> {
    let saved = store.read()?;
    let now = Utc::now();
    let mut stale: Vec<Harness> = Vec::new();
    for harness in harnesses {
        if read::WINDOWED.contains(harness)
            && !stale.contains(harness)
            && harness.installed()
            && !read_lately(saved.get(harness.name()), now)
            && router::stale(saved.get(harness.name()), now)
        {
            stale.push(*harness);
        }
    }
    read_into(store, &stale)
}

/// `thirdshift plan-usage`.
pub fn execute(action: Action) -> Result<String> {
    match action {
        Action::Push { destination } => {
            crate::console::report::push_plan_usage_by_hand(destination.as_deref())
        }
        Action::Clear { harness, json } => {
            let cleared = clear(harness)?;
            if json {
                return Ok(serde_json::to_string_pretty(
                    &serde_json::json!({"version": 1, "cleared": cleared}),
                )?);
            }
            Ok(format!(
                "Cleared recorded Plan usage cooldowns and revalidation for {}",
                cleared.join(", ")
            ))
        }
        Action::Show { selection, json } => {
            let config = UserConfig::load()?;
            let shown = show(&config, &selection, &fleet::Capture::new(&config, None))?;
            if json {
                Ok(serde_json::to_string_pretty(&shown.json()?)?)
            } else {
                Ok(shown.to_string())
            }
        }
    }
}
