//! Per-Step routing across the User config's routes by machine-local Plan
//! usage: before a session Step starts, its first route that is installed,
//! out of cooldown and below its utilization ceiling; and, when a session
//! stops on structured Plan-usage evidence, the next route not yet tried in
//! that Step. Routing reads only this machine's snapshot and its own
//! non-billed reads, never a Fleet peer.

use std::collections::BTreeMap;

use chrono::{DateTime, TimeDelta, Utc};

use super::route::{Route, Routing};
use super::store::{State, Store};
use super::view::Availability;
use crate::harness::{Asked, Choice, ChosenBy, Harness, Settings};

/// How old a Codex or Muse snapshot may grow before routing refreshes it
/// with the Harness's non-billed read.
const FRESH_MINUTES: i64 = 5;

/// One route a session Step may take: the configured entry, for its
/// ceiling and credits, and what a session on it runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub route: Route,
    pub choice: Choice,
}

impl Candidate {
    fn of(route: &Route, settings: &Settings, chosen_by: ChosenBy) -> Self {
        Self {
            route: route.clone(),
            choice: route.choice(settings, chosen_by),
        }
    }

    /// A choice made elsewhere, as a route of its own: a Harness named by
    /// `security.harness` or `harness.default`, with the Model and Effort
    /// its section sets.
    fn chosen(choice: &Choice) -> Self {
        Self {
            route: Route {
                model: choice.model.clone(),
                effort: choice.effort.clone(),
                ..Route::named(choice.harness)
            },
            choice: choice.clone(),
        }
    }
}

/// The routes a Command's session Steps may take, in order: ordinary
/// sessions', and Security runs' and Security reviews'. A list with one
/// route, or none, is pinned.
#[derive(Debug, Clone, Default)]
pub struct Router {
    ordinary: Vec<Candidate>,
    security: Vec<Candidate>,
    routing: Routing,
}

/// A route picked for a Step, with why each earlier untried route was
/// passed over, as "claude is in cooldown".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picked {
    pub index: usize,
    pub passed: Vec<String>,
}

impl Router {
    /// No routing: every session runs on the choice it was given.
    pub fn pinned() -> Self {
        Self::default()
    }

    /// The routes for sessions on `harness`, and for Security sessions on
    /// `security_review`, else `harness`, given the User config's
    /// `settings` and `routing`. Only a choice the User config's routes made
    /// routes: a Harness or Model named by a command or a Step stays pinned.
    /// A Security session walks `security.routes` after `security.harness`,
    /// else `harness.routes`, else `security_default` (`harness.default`).
    pub fn of(
        settings: &Settings,
        routing: &Routing,
        harness: &Choice,
        security_review: Option<&Choice>,
        security_default: Option<&Choice>,
    ) -> Self {
        let ordinary = if harness.chosen_by == ChosenBy::Routes {
            routing
                .routes
                .iter()
                .map(|route| Candidate::of(route, settings, ChosenBy::Routes))
                .collect()
        } else {
            Vec::new()
        };
        let review = security_review.unwrap_or(harness);
        let default = security_default
            .cloned()
            .unwrap_or_else(|| Choice::of(&Asked::default(), settings));
        let mut security = Vec::new();
        match review.chosen_by {
            ChosenBy::SecurityConfig | ChosenBy::Routes => {
                if review.chosen_by == ChosenBy::SecurityConfig {
                    security.push(Candidate::chosen(review));
                }
                if !routing.security_routes.is_empty() {
                    security.extend(
                        routing
                            .security_routes
                            .iter()
                            .map(|route| Candidate::of(route, settings, ChosenBy::SecurityConfig)),
                    );
                } else if !routing.routes.is_empty() {
                    security.extend(
                        routing
                            .routes
                            .iter()
                            .map(|route| Candidate::of(route, settings, ChosenBy::Routes)),
                    );
                } else {
                    security.push(Candidate::chosen(&default));
                }
            }
            // Pinned: a Security review keeps only its own fallback to
            // `harness.default` when it can't start.
            ChosenBy::Command | ChosenBy::UserConfig | ChosenBy::Default | ChosenBy::Step => {}
        }
        // A route that runs as an earlier one adds nothing to try.
        let mut distinct: Vec<Candidate> = Vec::new();
        for candidate in security {
            if !distinct
                .iter()
                .any(|kept| kept.choice.runs_as(&candidate.choice))
            {
                distinct.push(candidate);
            }
        }
        Self {
            ordinary,
            security: distinct,
            routing: routing.clone(),
        }
    }

    /// The routes sessions of this kind walk, in order: Security sessions'
    /// when `security`.
    pub fn candidates(&self, security: bool) -> &[Candidate] {
        if security {
            &self.security
        } else {
            &self.ordinary
        }
    }

    /// Every Harness a route names, so their Factory skills can be linked
    /// before a Step switches to one.
    pub fn harnesses(&self) -> Vec<Harness> {
        let mut all: Vec<Harness> = Vec::new();
        for candidate in self.ordinary.iter().chain(&self.security) {
            if !all.contains(&candidate.choice.harness) {
                all.push(candidate.choice.harness);
            }
        }
        all
    }

    /// The first route of this kind not in `tried` that could take a
    /// session now, by this machine's saved Plan usage, refreshing a stale
    /// Codex or Muse snapshot with its non-billed read first. None when
    /// every untried route is unavailable, or there are fewer than two.
    pub fn pick(&self, security: bool, tried: &[usize]) -> Option<Picked> {
        let candidates = self.candidates(security);
        if candidates.len() < 2 {
            return None;
        }
        let saved = if cfg!(test) {
            BTreeMap::new()
        } else {
            let harnesses: Vec<Harness> = candidates
                .iter()
                .enumerate()
                .filter(|(index, _)| !tried.contains(index))
                .map(|(_, candidate)| candidate.choice.harness)
                .collect();
            Store::machine()
                .and_then(|store| {
                    super::refresh_for_routing(&store, &harnesses)?;
                    store.read()
                })
                .unwrap_or_default()
        };
        select(
            candidates,
            tried,
            &self.routing,
            &saved,
            Harness::installed,
            Utc::now(),
        )
    }
}

/// The first of `candidates` not in `tried` whose route could take a
/// session now, by the `saved` Plan usage and which Harnesses are
/// `installed`. A route Plan usage does not watch is usable when installed.
fn select(
    candidates: &[Candidate],
    tried: &[usize],
    routing: &Routing,
    saved: &BTreeMap<String, State>,
    installed: impl Fn(Harness) -> bool,
    now: DateTime<Utc>,
) -> Option<Picked> {
    let mut passed = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        if tried.contains(&index) {
            continue;
        }
        let harness = candidate.choice.harness;
        let installed = installed(harness);
        let availability = if super::observation::subscription(harness) {
            saved
                .get(harness.name())
                .cloned()
                .unwrap_or_default()
                .availability(&candidate.route, routing, installed, now)
        } else if installed {
            Availability::Available
        } else {
            Availability::NotInstalled
        };
        if availability.usable() {
            return Some(Picked { index, passed });
        }
        passed.push(format!("{} is {availability}", harness.name()));
    }
    None
}

/// Whether routing should refresh `state` with a non-billed read before
/// judging it: nothing saved, the last observation more than five minutes
/// old, or a window or cooldown past its reset; but never twice a minute.
pub(super) fn stale(state: Option<&State>, now: DateTime<Utc>) -> bool {
    let Some(state) = state else {
        return true;
    };
    if state
        .read_at
        .is_some_and(|read| read <= now && now - read < TimeDelta::minutes(1))
    {
        return false;
    }
    state.observed_at.is_none_or(|observed| {
        observed > now || now - observed >= TimeDelta::minutes(FRESH_MINUTES)
    }) || state.windows.iter().any(|window| window.reset_by(now))
        || state
            .cooldown
            .as_ref()
            .is_some_and(|cooldown| cooldown.until <= now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_usage::observation::{Exhausted, Observation, Window};
    use crate::plan_usage::store::Cooldown;

    fn routes(routes: &[Route]) -> Routing {
        Routing {
            routes: routes.to_vec(),
            ..Routing::default()
        }
    }

    fn routed(harness: Harness) -> Choice {
        Choice {
            harness,
            model: None,
            effort: None,
            chosen_by: ChosenBy::Routes,
        }
    }

    fn harnesses(candidates: &[Candidate]) -> Vec<Harness> {
        candidates.iter().map(|c| c.choice.harness).collect()
    }

    fn window(used: f64, resets_at: DateTime<Utc>) -> State {
        State {
            observed_at: Some(Utc::now()),
            windows: vec![Window {
                used_percent: Some(used),
                resets_at: Some(resets_at),
                ..Window::named("five_hour").unwrap()
            }],
            ..State::default()
        }
    }

    fn picked(
        router: &Router,
        security: bool,
        tried: &[usize],
        saved: &BTreeMap<String, State>,
        now: DateTime<Utc>,
    ) -> Option<usize> {
        select(
            router.candidates(security),
            tried,
            &router.routing,
            saved,
            |_| true,
            now,
        )
        .map(|picked| picked.index)
    }

    #[test]
    fn only_a_choice_the_routes_made_routes_ordinary_sessions() {
        let routing = routes(&[Route::named(Harness::Claude), Route::named(Harness::Codex)]);
        let settings = Settings::default();
        let router = Router::of(&settings, &routing, &routed(Harness::Claude), None, None);
        assert_eq!(
            harnesses(router.candidates(false)),
            [Harness::Claude, Harness::Codex]
        );
        for chosen_by in [
            ChosenBy::Command,
            ChosenBy::Step,
            ChosenBy::UserConfig,
            ChosenBy::Default,
        ] {
            let pinned = Choice {
                chosen_by,
                ..routed(Harness::Claude)
            };
            let router = Router::of(&settings, &routing, &pinned, None, None);
            assert!(router.candidates(false).is_empty(), "{chosen_by:?}");
            assert_eq!(router.pick(false, &[]), None, "{chosen_by:?}");
        }
    }

    #[test]
    fn a_reserve_ceiling_or_cooldown_routes_away_and_a_lapsed_cooldown_revalidates() {
        let now = Utc::now();
        let routing = Routing {
            max_utilization: 85,
            ..routes(&[Route::named(Harness::Claude), Route::named(Harness::Codex)])
        };
        let router = Router::of(
            &Settings::default(),
            &routing,
            &routed(Harness::Claude),
            None,
            None,
        );
        let mut saved = BTreeMap::new();
        assert_eq!(picked(&router, false, &[], &saved, now), Some(0));
        saved.insert("claude".into(), window(90.0, now + TimeDelta::hours(1)));
        assert_eq!(picked(&router, false, &[], &saved, now), Some(1));
        // The window reset since: its utilization says nothing now.
        saved.insert("claude".into(), window(90.0, now - TimeDelta::minutes(1)));
        assert_eq!(picked(&router, false, &[], &saved, now), Some(0));
        let mut cooled = State::default();
        cooled.observe(
            &Observation {
                harness: Harness::Claude,
                windows: Vec::new(),
                exhausted: Some(Exhausted::default()),
            },
            "session",
            now,
        );
        saved.insert("claude".into(), cooled.clone());
        assert_eq!(picked(&router, false, &[], &saved, now), Some(1));
        // Each route is tried at most once in a Step.
        assert_eq!(picked(&router, false, &[1], &saved, now), None);
        // Past the backoff the next Step may try it once again.
        let later = cooled.cooldown.as_ref().unwrap().until + TimeDelta::minutes(1);
        assert_eq!(picked(&router, false, &[], &saved, later), Some(0));
    }

    #[test]
    fn credits_hold_a_used_up_window_unless_the_route_allows_them() {
        let now = Utc::now();
        let mut codex = window(100.0, now + TimeDelta::days(3));
        codex.credits = Some(crate::plan_usage::store::Credits {
            has_credits: Some(true),
            balance: Some("100".into()),
        });
        let saved = BTreeMap::from([("codex".to_string(), codex)]);
        for (allow_credits, expected) in [(false, Some(1)), (true, Some(0))] {
            let routing = routes(&[
                Route {
                    allow_credits,
                    ..Route::named(Harness::Codex)
                },
                Route::named(Harness::Claude),
            ]);
            let router = Router::of(
                &Settings::default(),
                &routing,
                &routed(Harness::Codex),
                None,
                None,
            );
            assert_eq!(
                picked(&router, false, &[], &saved, now),
                expected,
                "{allow_credits}"
            );
        }
    }

    #[test]
    fn security_sessions_walk_security_routes_else_harness_routes_else_the_default() {
        let settings = Settings {
            default: Some(Harness::Claude),
            ..Settings::default()
        };
        let security = Choice {
            chosen_by: ChosenBy::SecurityConfig,
            ..routed(Harness::Codex)
        };
        let default = Choice {
            chosen_by: ChosenBy::UserConfig,
            ..routed(Harness::Claude)
        };
        let with_security_routes = Routing {
            security_routes: vec![Route::named(Harness::Muse), Route::named(Harness::Agy)],
            ..routes(&[Route::named(Harness::Claude), Route::named(Harness::Grok)])
        };
        let only_routes = routes(&[Route::named(Harness::Claude), Route::named(Harness::Grok)]);
        for (routing, expected) in [
            (
                &with_security_routes,
                vec![Harness::Codex, Harness::Muse, Harness::Agy],
            ),
            (
                &only_routes,
                vec![Harness::Codex, Harness::Claude, Harness::Grok],
            ),
            (&Routing::default(), vec![Harness::Codex, Harness::Claude]),
        ] {
            let router = Router::of(
                &settings,
                routing,
                &routed(Harness::Claude),
                Some(&security),
                Some(&default),
            );
            assert_eq!(harnesses(router.candidates(true)), expected);
        }
        // A pinned Command's Security sessions don't route; a Security
        // review keeps its own fallback to the default.
        let pinned = Choice {
            chosen_by: ChosenBy::Command,
            ..routed(Harness::Grok)
        };
        let router = Router::of(&settings, &only_routes, &pinned, None, Some(&default));
        assert!(router.candidates(true).is_empty());
        assert!(router.candidates(false).is_empty());
    }

    #[test]
    fn stale_snapshots_are_refreshed_at_most_once_a_minute() {
        let now = Utc::now();
        assert!(stale(None, now));
        let fresh = window(10.0, now + TimeDelta::hours(1));
        assert!(!stale(Some(&fresh), now));
        let old = State {
            observed_at: Some(now - TimeDelta::minutes(6)),
            ..fresh.clone()
        };
        assert!(stale(Some(&old), now));
        let reset = window(10.0, now - TimeDelta::seconds(1));
        assert!(stale(Some(&reset), now));
        let lapsed = State {
            cooldown: Some(Cooldown {
                since: now - TimeDelta::hours(2),
                until: now - TimeDelta::hours(1),
                window: None,
                overage: None,
                backoff_minutes: Some(60),
            }),
            ..fresh.clone()
        };
        assert!(stale(Some(&lapsed), now));
        let just_read = State {
            read_at: Some(now - TimeDelta::seconds(10)),
            ..old
        };
        assert!(!stale(Some(&just_read), now));
    }
}
