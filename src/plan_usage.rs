//! Plan usage: how much of each subscription Harness's allowance is used,
//! saved per machine. Running sessions update it passively from structured
//! wire frames; `thirdshift plan-usage` refreshes it on demand with official
//! non-billed reads, at most once a minute per Harness, and never on a
//! background timer. A Command routed by the User config's routes reads it
//! before each session Step ([`router`]), refreshing a stale snapshot of a
//! Harness whose read reports Plan windows first, and never asks a Fleet
//! peer.

use std::time::Duration;

use anyhow::{Result, bail};
use chrono::{TimeDelta, Utc};

pub mod exhaustion;
pub(crate) mod observation;
mod read;
pub mod route;
pub mod router;
mod store;
mod view;

pub use exhaustion::Exhaustion;
pub use observation::Observation;
pub use route::Routing;
pub use router::Router;
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

/// Whether a Pass whose sessions would run on `harness`, a Security run's
/// when `security`, finds every route it may take unavailable by this
/// machine's Plan usage, refreshing a stale snapshot of a Harness whose
/// read reports Plan windows first. Only a choice the User config's routes
/// made walks them; any other is that Harness alone, held only at confirmed
/// exhaustion. Never asks a Fleet peer, and none in unit tests.
pub fn pass_exhaustion(config: &UserConfig, harness: &Choice, security: bool) -> Option<Exhaustion> {
    if cfg!(test) {
        return None;
    }
    let routes = allowed_routes(config, harness, security);
    let store = Store::machine().ok()?;
    let harnesses: Vec<Harness> = routes.iter().map(|route| route.harness).collect();
    let _ = refresh_for_routing(&store, &harnesses);
    let saved = store.read().ok()?;
    exhaustion::judge(&routes, &config.routing, &saved, Harness::installed, Utc::now())
}

/// The routes a Pass on `harness` may take, in order.
fn allowed_routes(
    config: &UserConfig,
    harness: &Choice,
    security: bool,
) -> Vec<route::Route> {
    let router = Router::of(
        &config.harness,
        &config.routing,
        harness,
        Choice::of_security_review(config.security_harness, &config.harness).as_ref(),
        Some(&Choice::of(&crate::harness::Asked::default(), &config.harness)),
    );
    let candidates = router.candidates(security);
    if candidates.len() >= 2 {
        return candidates
            .iter()
            .map(|candidate| candidate.route.clone())
            .collect();
    }
    let pinned = candidates
        .first()
        .map_or(harness.harness, |candidate| candidate.choice.harness);
    vec![exhaustion::pinned(pinned)]
}

/// Whether every route a factory Command started with no Harness flags
/// would take on this machine is unavailable by its saved Plan usage, so
/// scheduled Passes are skipping: `harness.routes`, else `harness.default`
/// alone. Reads only the saved snapshot, never a Harness or a Fleet peer.
pub fn machine_exhaustion(config: &UserConfig) -> Option<Exhaustion> {
    let routes = if config.routing.routes.is_empty() {
        vec![exhaustion::pinned(
            config.harness.default.unwrap_or(Harness::Claude),
        )]
    } else {
        config.routing.routes.clone()
    };
    let saved = Store::machine().ok()?.read().ok()?;
    exhaustion::judge(&routes, &config.routing, &saved, Harness::installed, Utc::now())
}

/// The latest Plan usage limit still holding a Harness on this machine, as
/// a Paused Command's record keeps it: for a Spec run whose Ticket's child
/// Run paused, which reports only that it did. None in unit tests.
pub(crate) fn latest_limit() -> Option<crate::session::limits::Limit> {
    if cfg!(test) {
        return None;
    }
    let saved = Store::machine().ok()?.read().ok()?;
    let now = Utc::now();
    let (name, state) = saved
        .iter()
        .filter(|(_, state)| state.cooldown.as_ref().is_some_and(|c| c.until > now))
        .max_by_key(|(_, state)| state.cooldown.as_ref().map(|c| c.since))?;
    let cooldown = state.cooldown.as_ref()?;
    let used = cooldown.window.as_deref().and_then(|window| {
        state
            .windows
            .iter()
            .find(|kept| kept.name == window)
            .and_then(|kept| kept.used_percent)
    });
    Some(
        crate::session::limits::Limit {
            harness: Some(name.clone()),
            window: cooldown.window.clone(),
            utilization: used.map(|used| used / 100.0),
            resets_at: Some(
                cooldown
                    .until
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            ),
        }
        .accepted(),
    )
}

/// The status banner for a machine whose every configured default route
/// is unavailable by Plan usage: the routes held and the earliest known
/// reset.
pub fn banner(config: &UserConfig) -> Option<String> {
    let exhaustion = machine_exhaustion(config)?;
    let mut line = format!(
        "Plan usage: every configured route is unavailable, so scheduled Passes skip: {}",
        exhaustion.routes()
    );
    if let Some(reset) = exhaustion.earliest_reset() {
        line.push_str(&format!(
            "; earliest reset {}",
            reset.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        ));
    }
    Some(line)
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
