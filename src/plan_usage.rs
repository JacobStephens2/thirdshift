//! Plan usage: how much of each subscription Harness's allowance is used,
//! saved per machine. Running sessions update it passively from structured
//! wire frames; `thirdshift plan-usage` refreshes it on demand with official
//! non-billed reads, at most once a minute per Harness, and never on a
//! background timer.

use std::time::Duration;

use anyhow::{Result, bail};
use chrono::{TimeDelta, Utc};

pub(crate) mod observation;
mod read;
pub mod route;
mod store;
mod view;

pub use observation::Observation;
pub use route::Routing;
use store::Store;

use crate::config::UserConfig;
use crate::harness::{Choice, Harness};

/// How long a Harness's non-billed read stays fresh for the view.
const FRESH: Duration = Duration::from_secs(60);

/// What `thirdshift plan-usage` was asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Show {
        json: bool,
    },
    /// Forget recorded cooldowns for one Harness, or all with none.
    Clear {
        harness: Option<Harness>,
        json: bool,
    },
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

/// `thirdshift plan-usage`.
pub fn execute(action: Action) -> Result<String> {
    let store = Store::machine()?;
    match action {
        Action::Clear { harness, json } => {
            store.clear(harness)?;
            let cleared: Vec<&str> = match harness {
                Some(harness) => vec![harness.name()],
                None => observation::SUBSCRIPTION.map(Harness::name).to_vec(),
            };
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
        Action::Show { json } => {
            let config = UserConfig::load()?;
            refresh(&store)?;
            let view = view::View::of(&config, store.read()?, Harness::installed, Utc::now());
            if json {
                Ok(serde_json::to_string_pretty(&view)?)
            } else {
                Ok(view.to_string())
            }
        }
    }
}
