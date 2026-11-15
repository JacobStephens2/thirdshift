//! Whether every route a Pass or a session Step may take is unavailable by
//! machine-local Plan usage: in cooldown after a structured refusal, or at
//! its utilization ceiling. Then a scheduled Pass skips and a live factory
//! Command ends as a Paused Command, neither sleeping in-process. A route
//! that is not installed or signed out is unavailable too, but says nothing
//! about Plan usage, so it alone never pauses or skips anything.

use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, SecondsFormat, Utc};

use super::route::{FULL, Route, Routing};
use super::store::State;
use super::view::Availability;
use crate::harness::Harness;
use crate::session::limits::Limit;

/// Every allowed route unavailable, with what holds each route held by
/// Plan usage, in route order.
#[derive(Debug, Clone, PartialEq)]
pub struct Exhaustion {
    pub stops: Vec<Stop>,
}

/// One route Plan usage holds: its Harness, whether it is in cooldown or
/// at its ceiling, and the Plan window, utilization and reset known for it.
#[derive(Debug, Clone, PartialEq)]
pub struct Stop {
    pub harness: Harness,
    pub availability: Availability,
    pub window: Option<String>,
    /// Percentage used, 0 to 100.
    pub used_percent: Option<f64>,
    pub resets_at: Option<DateTime<Utc>>,
}

impl Exhaustion {
    /// The earliest known time a held route may take work again.
    pub fn earliest_reset(&self) -> Option<DateTime<Utc>> {
        self.stops.iter().filter_map(|stop| stop.resets_at).min()
    }

    /// The first held route, as a Paused Command's record keeps it.
    pub fn limit(&self) -> Limit {
        let Some(stop) = self.stops.first() else {
            return Limit::default();
        };
        Limit::of(
            stop.harness,
            stop.window.clone(),
            stop.used_percent,
            stop.resets_at,
        )
    }

    /// The held routes, as "claude in cooldown (five_hour) until
    /// 2026-11-14T18:00:00Z; codex at its ceiling (primary at 92%)".
    /// Stable while the saved Plan usage is, so a skip's Activity log line
    /// repeats only when it changes.
    pub fn routes(&self) -> String {
        self.stops
            .iter()
            .map(Stop::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    }
}

impl fmt::Display for Exhaustion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", crate::session::limits::NO_ROUTE, self.routes())
    }
}

impl fmt::Display for Stop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.harness.name(), self.availability)?;
        match (&self.window, self.used_percent) {
            (Some(window), Some(used)) => write!(f, " ({window} at {used:.0}%)")?,
            (Some(window), None) => write!(f, " ({window})")?,
            (None, Some(used)) => write!(f, " (at {used:.0}%)")?,
            (None, None) => {}
        }
        if let Some(reset) = self.resets_at {
            write!(
                f,
                " until {}",
                reset.to_rfc3339_opts(SecondsFormat::Secs, true)
            )?;
        }
        Ok(())
    }
}

/// Whether every one of `routes` is unavailable by the `saved` Plan usage
/// and which Harnesses are `installed`, with at least one held by Plan
/// usage itself. A route Plan usage does not watch is available when
/// installed. None when any route could take a session, or there are none.
pub(super) fn judge(
    routes: &[Route],
    routing: &Routing,
    saved: &BTreeMap<String, State>,
    installed: impl Fn(Harness) -> bool,
    now: DateTime<Utc>,
) -> Option<Exhaustion> {
    let mut stops = Vec::new();
    for route in routes {
        let harness = route.harness;
        let installed = installed(harness);
        if !super::observation::subscription(harness) {
            if installed {
                return None;
            }
            continue;
        }
        let state = saved.get(harness.name()).cloned().unwrap_or_default();
        let availability = state.availability(route, routing, installed, now);
        if availability.usable() {
            return None;
        }
        if let Some(stop) = Stop::of(harness, &state, route, routing, availability, now) {
            stops.push(stop);
        }
    }
    (!stops.is_empty()).then_some(Exhaustion { stops })
}

impl Stop {
    /// What holds `route` on `harness` by `state`, when Plan usage is what
    /// makes it `availability`: none when it is not installed or signed out.
    fn of(
        harness: Harness,
        state: &State,
        route: &Route,
        routing: &Routing,
        availability: Availability,
        now: DateTime<Utc>,
    ) -> Option<Self> {
        let live = || {
            state
                .windows
                .iter()
                .filter(move |window| !window.reset_by(now))
        };
        match availability {
            Availability::Cooldown => {
                let cooldown = state.cooldown.as_ref()?;
                let used = cooldown
                    .window
                    .as_deref()
                    .and_then(|name| state.used_percent_in(name, now));
                Some(Self {
                    harness,
                    availability,
                    window: cooldown.window.clone(),
                    used_percent: used,
                    resets_at: Some(cooldown.until),
                })
            }
            Availability::AtCeiling => {
                let ceiling = f64::from(routing.ceiling(route));
                let window = live()
                    .filter(|window| window.used_percent.is_some_and(|used| used >= ceiling))
                    .max_by(|a, b| {
                        a.used_percent
                            .unwrap_or_default()
                            .total_cmp(&b.used_percent.unwrap_or_default())
                    })?;
                Some(Self {
                    harness,
                    availability,
                    window: Some(window.name.clone()),
                    used_percent: window.used_percent,
                    resets_at: window.resets_at,
                })
            }
            Availability::Available
            | Availability::Revalidating
            | Availability::SignedOut
            | Availability::NotInstalled => None,
        }
    }
}

/// A Harness chosen outside the User config's routes, as a route of its
/// own: it is held only at confirmed exhaustion, never at a reserve
/// ceiling, so a pinned Command keeps to what it was asked.
pub(super) fn pinned(harness: Harness) -> Route {
    Route {
        max_utilization: Some(FULL),
        ..Route::named(harness)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_usage::observation::Window;
    use crate::plan_usage::store::Cooldown;
    use chrono::TimeDelta;

    fn routing(max_utilization: u8) -> Routing {
        Routing {
            routes: vec![Route::named(Harness::Claude), Route::named(Harness::Codex)],
            max_utilization,
            ..Routing::default()
        }
    }

    fn cooled(until: DateTime<Utc>) -> State {
        State {
            cooldown: Some(Cooldown {
                since: until - TimeDelta::hours(5),
                until,
                window: Some("five_hour".into()),
                overage: None,
                backoff_minutes: None,
            }),
            ..State::default()
        }
    }

    fn used(percent: f64, resets_at: DateTime<Utc>) -> State {
        State {
            observed_at: Some(resets_at - TimeDelta::hours(1)),
            windows: vec![Window {
                used_percent: Some(percent),
                resets_at: Some(resets_at),
                ..Window::named("primary").unwrap()
            }],
            ..State::default()
        }
    }

    #[test]
    fn every_route_held_by_plan_usage_is_an_exhaustion_naming_each_and_the_earliest_reset() {
        let now = Utc::now();
        let routing = routing(85);
        let claude_reset = now + TimeDelta::hours(3);
        let codex_reset = now + TimeDelta::hours(1);
        let saved = BTreeMap::from([
            ("claude".to_string(), cooled(claude_reset)),
            ("codex".to_string(), used(92.0, codex_reset)),
        ]);
        let exhaustion = judge(&routing.routes, &routing, &saved, |_| true, now).unwrap();
        assert_eq!(exhaustion.earliest_reset(), Some(codex_reset));
        let reason = exhaustion.to_string();
        assert!(
            reason.starts_with(
                "no allowed route has available Plan usage: claude in cooldown (five_hour) until "
            ),
            "{reason}"
        );
        assert!(
            reason.contains("; codex at its ceiling (primary at 92%) until "),
            "{reason}"
        );
        let limit = exhaustion.limit();
        assert_eq!(limit.harness.as_deref(), Some("claude"));
        assert_eq!(limit.window.as_deref(), Some("five_hour"));
    }

    #[test]
    fn any_usable_route_or_routes_merely_not_installed_are_no_exhaustion() {
        let now = Utc::now();
        let routing = routing(85);
        let saved = BTreeMap::from([("claude".to_string(), cooled(now + TimeDelta::hours(3)))]);
        // Codex has nothing saved, so it is available.
        assert_eq!(
            judge(&routing.routes, &routing, &saved, |_| true, now),
            None
        );
        // Neither installed: a missing CLI is not Plan usage.
        assert_eq!(
            judge(&routing.routes, &routing, &BTreeMap::new(), |_| false, now),
            None
        );
        // A lapsed cooldown revalidates, so it may be tried.
        let lapsed = BTreeMap::from([
            ("claude".to_string(), cooled(now - TimeDelta::minutes(1))),
            ("codex".to_string(), used(100.0, now + TimeDelta::hours(1))),
        ]);
        assert_eq!(
            judge(&routing.routes, &routing, &lapsed, |_| true, now),
            None
        );
        // An installed API-key Harness is never watched, so always usable.
        let open = Routing {
            routes: vec![
                Route::named(Harness::Claude),
                Route::named(Harness::OpenCode),
            ],
            ..Routing::default()
        };
        assert_eq!(judge(&open.routes, &open, &saved, |_| true, now), None);
    }

    #[test]
    fn a_pinned_harness_is_held_only_at_confirmed_exhaustion_not_a_reserve_ceiling() {
        let now = Utc::now();
        let routing = routing(85);
        let saved = BTreeMap::from([("codex".to_string(), used(92.0, now + TimeDelta::hours(1)))]);
        assert_eq!(
            judge(&[pinned(Harness::Codex)], &routing, &saved, |_| true, now),
            None
        );
        let full = BTreeMap::from([("codex".to_string(), used(100.0, now + TimeDelta::hours(1)))]);
        assert!(judge(&[pinned(Harness::Codex)], &routing, &full, |_| true, now).is_some());
    }
}
