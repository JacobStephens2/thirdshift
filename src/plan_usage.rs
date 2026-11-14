//! Plan usage: how much of each subscription Harness's allowance is used,
//! saved per machine. Running sessions update it passively from structured
//! wire frames; `thirdshift plan-usage` and the Console's `/plan-usage`
//! refresh it on demand with official non-billed reads, at most once a
//! minute per Harness, and never on a background timer. Both show the
//! Fleet by default once it has one, grouping shared logins.

use std::time::Duration;

use anyhow::{Result, bail};
use chrono::{TimeDelta, Utc};

pub(crate) mod fleet;
pub(crate) mod observation;
mod read;
pub mod route;
mod store;
mod view;

pub use observation::Observation;
pub use route::Routing;
use store::Store;
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
    let fresh = TimeDelta::from_std(FRESH).expect("a minute fits");
    let stale: Vec<Harness> = read::READABLE
        .into_iter()
        .filter(|harness| harness.installed())
        .filter(|harness| {
            saved
                .get(harness.name())
                .and_then(|state| state.read_at)
                .is_none_or(|read| now - read >= fresh || read > now)
        })
        .collect();
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

/// The view `selection` asks for, read on behalf of `person` when a
/// Console reads Fleet machines for someone signed in.
pub(crate) fn show(
    config: &UserConfig,
    selection: &Selection,
    person: Option<&str>,
) -> Result<Shown> {
    let local = crate::console::fleet_control::local_name(config);
    let chosen = match selection {
        Selection::Local => return Ok(Shown::Local(local_view(config)?)),
        Selection::Machine(name) if *name == local => {
            return Ok(Shown::Local(local_view(config)?));
        }
        Selection::Machine(name) => Some(name.as_str()),
        Selection::Fleet => None,
        Selection::Default if fleet::default_view(config, person) => None,
        Selection::Default => return Ok(Shown::Local(local_view(config)?)),
    };
    let view = match chosen {
        Some(_) => None,
        None => Some(local_view(config)?),
    };
    Ok(Shown::Fleet(fleet::load(config, person, chosen, view)?))
}

/// Forget recorded cooldowns on this machine, and say which Harnesses'.
pub(crate) fn clear(harness: Option<Harness>) -> Result<Vec<&'static str>> {
    Store::machine()?.clear(harness)?;
    Ok(match harness {
        Some(harness) => vec![harness.name()],
        None => observation::SUBSCRIPTION.map(Harness::name).to_vec(),
    })
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
            let shown = show(&UserConfig::load()?, &selection, None)?;
            if json {
                Ok(serde_json::to_string_pretty(&shown.json()?)?)
            } else {
                Ok(shown.to_string())
            }
        }
    }
}
