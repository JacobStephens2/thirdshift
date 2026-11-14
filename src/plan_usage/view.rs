//! `thirdshift plan-usage`'s rows: every subscription Harness the User
//! config routes to, in route order, then any other with a saved
//! observation or a signed-in CLI.

use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, Local, Utc};
use serde::Serialize;

use super::observation::SUBSCRIPTION;
use super::route::{Route, Routing};
use super::store::State;
use crate::config::UserConfig;
use crate::harness::Harness;

/// Where the User config names a route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Setting {
    #[serde(rename = "harness.default")]
    Default,
    #[serde(rename = "harness.routes")]
    Routes,
    #[serde(rename = "security.harness")]
    SecurityHarness,
    #[serde(rename = "security.routes")]
    SecurityRoutes,
}

impl fmt::Display for Setting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Default => "harness.default",
            Self::Routes => "harness.routes",
            Self::SecurityHarness => "security.harness",
            Self::SecurityRoutes => "security.routes",
        })
    }
}

/// Whether a route could take a session now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Available,
    /// Its cooldown passed with no sign it works again: the next session
    /// may try it once.
    Revalidating,
    /// A Plan window is at or above the route's ceiling.
    AtCeiling,
    Cooldown,
    SignedOut,
    NotInstalled,
}

impl Availability {
    pub(super) fn usable(self) -> bool {
        matches!(self, Self::Available | Self::Revalidating)
    }
}

impl fmt::Display for Availability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Available => "available",
            Self::Revalidating => "revalidating",
            Self::AtCeiling => "at its ceiling",
            Self::Cooldown => "in cooldown",
            Self::SignedOut => "signed out",
            Self::NotInstalled => "not installed",
        })
    }
}

#[derive(Debug, Serialize)]
pub struct ConfiguredRoute {
    pub setting: Setting,
    /// 1-based position in a `routes` list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<usize>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub max_utilization: u8,
    pub allow_credits: bool,
    pub availability: Availability,
    /// The route a factory session would start on now.
    pub active: bool,
}

#[derive(Debug, Serialize)]
pub struct Row {
    #[serde(serialize_with = "super::route::harness_name")]
    pub harness: Harness,
    pub installed: bool,
    pub active: bool,
    pub routes: Vec<ConfiguredRoute>,
    #[serde(flatten)]
    pub state: State,
}

#[derive(Debug, Serialize)]
pub struct View {
    pub version: u8,
    pub harnesses: Vec<Row>,
    /// When availability was judged, so the text agrees with it.
    #[serde(skip)]
    pub now: DateTime<Utc>,
}

/// Each place the User config names a route, in route order: the default
/// Harness and `harness.routes`, then `security.harness` and
/// `security.routes`.
fn configured(config: &UserConfig) -> Vec<(Setting, Option<usize>, Route)> {
    let routing = &config.routing;
    let default = config.harness.default.unwrap_or(Harness::Claude);
    let mut all = vec![(Setting::Default, None, Route::named(default))];
    all.extend(
        routing
            .routes
            .iter()
            .enumerate()
            .map(|(index, route)| (Setting::Routes, Some(index + 1), route.clone())),
    );
    if let Some(security) = config.security_harness {
        all.push((Setting::SecurityHarness, None, Route::named(security)));
    }
    all.extend(
        routing
            .security_routes
            .iter()
            .enumerate()
            .map(|(index, route)| (Setting::SecurityRoutes, Some(index + 1), route.clone())),
    );
    all
}

impl State {
    /// Whether `route` could take a session on this Harness now.
    pub(super) fn availability(
        &self,
        route: &Route,
        routing: &Routing,
        installed: bool,
        now: DateTime<Utc>,
    ) -> Availability {
        let state = self;
        if !installed {
            return Availability::NotInstalled;
        }
        if state.account.as_ref().and_then(|account| account.signed_in) == Some(false) {
            return Availability::SignedOut;
        }
        let credits = route.allow_credits
            && state.spend_control_reached != Some(true)
            && state
                .credits
                .as_ref()
                .and_then(|credits| credits.has_credits)
                == Some(true);
        if let Some(cooldown) = &state.cooldown {
            if cooldown.until > now {
                if !credits {
                    return Availability::Cooldown;
                }
            } else {
                return Availability::Revalidating;
            }
        }
        let ceiling = f64::from(routing.ceiling(route));
        let full = state.windows.iter().any(|window| {
            !window.reset_by(now) && window.used_percent.is_some_and(|used| used >= ceiling)
        });
        if full && !(credits && ceiling >= 100.0) {
            return Availability::AtCeiling;
        }
        Availability::Available
    }
}

impl View {
    pub fn of(
        config: &UserConfig,
        mut saved: BTreeMap<String, State>,
        installed: impl Fn(Harness) -> bool,
        now: DateTime<Utc>,
    ) -> Self {
        let mut rows: Vec<Row> = Vec::new();
        for (setting, position, route) in configured(config) {
            if !SUBSCRIPTION.contains(&route.harness) {
                continue;
            }
            let index = match rows.iter().position(|row| row.harness == route.harness) {
                Some(index) => index,
                None => {
                    rows.push(Row {
                        harness: route.harness,
                        installed: installed(route.harness),
                        active: false,
                        routes: Vec::new(),
                        state: saved.remove(route.harness.name()).unwrap_or_default(),
                    });
                    rows.len() - 1
                }
            };
            let row = &mut rows[index];
            let set = config.harness.of(route.harness);
            row.routes.push(ConfiguredRoute {
                setting,
                position,
                model: route.model.clone().or(set.model.clone()),
                effort: route.effort.clone().or(set.effort.clone()),
                max_utilization: config.routing.ceiling(&route),
                allow_credits: route.allow_credits,
                availability: row
                    .state
                    .availability(&route, &config.routing, row.installed, now),
                active: false,
            });
        }
        // The primary route list: harness.routes, else harness.default.
        let primary = if config.routing.routes.is_empty() {
            Setting::Default
        } else {
            Setting::Routes
        };
        // Walk it in order, API-key routes included: the first usable route
        // is active, even when it is one Plan usage does not watch.
        let walk: Vec<(Option<usize>, Harness)> = configured(config)
            .into_iter()
            .filter(|(setting, ..)| *setting == primary)
            .map(|(_, position, route)| (position, route.harness))
            .collect();
        for (position, harness) in walk {
            let Some(index) = rows.iter().position(|row| row.harness == harness) else {
                // Not watched: an installed API-key Harness takes the session.
                if installed(harness) {
                    break;
                }
                continue;
            };
            let Some(at) = rows[index]
                .routes
                .iter()
                .position(|route| route.setting == primary && route.position == position)
            else {
                continue;
            };
            if rows[index].routes[at].availability.usable() {
                rows[index].active = true;
                rows[index].routes[at].active = true;
                break;
            }
        }
        for harness in SUBSCRIPTION {
            if rows.iter().any(|row| row.harness == harness) {
                continue;
            }
            let Some(state) = saved.remove(harness.name()) else {
                continue;
            };
            let signed_in = state.account.as_ref().and_then(|account| account.signed_in);
            let installed = installed(harness);
            if state.observed() || (installed && signed_in == Some(true)) {
                rows.push(Row {
                    harness,
                    installed,
                    active: false,
                    routes: Vec::new(),
                    state,
                });
            }
        }
        View {
            version: 1,
            harnesses: rows,
            now,
        }
    }
}

fn local(time: DateTime<Utc>) -> String {
    time.with_timezone(&Local)
        .format("%a %b %-d %H:%M %Z")
        .to_string()
}

impl fmt::Display for View {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.harnesses.is_empty() {
            return f.write_str(
                "No Plan usage to show: no configured subscription Harness, no saved observation and no signed-in CLI",
            );
        }
        let now = self.now;
        for (index, row) in self.harnesses.iter().enumerate() {
            if index > 0 {
                writeln!(f)?;
            }
            write!(f, "{}", row.harness.name())?;
            if row.active {
                f.write_str(" (active)")?;
            }
            writeln!(f)?;
            for route in &row.routes {
                write!(f, "  route: {}", route.setting)?;
                if let Some(position) = route.position {
                    write!(f, " {position}")?;
                }
                if let Some(model) = &route.model {
                    write!(f, ", {model}")?;
                }
                if let Some(effort) = &route.effort {
                    write!(f, ", {effort}")?;
                }
                if route.max_utilization < super::route::FULL {
                    write!(f, ", ceiling {}%", route.max_utilization)?;
                }
                if route.allow_credits {
                    f.write_str(", credits allowed")?;
                }
                write!(f, ": {}", route.availability)?;
                if route.active {
                    f.write_str(", active")?;
                }
                writeln!(f)?;
            }
            let state = &row.state;
            if let Some(account) = &state.account {
                let mut parts = Vec::new();
                match account.signed_in {
                    Some(true) => parts.push("signed in".to_owned()),
                    Some(false) => parts.push("signed out".to_owned()),
                    None => {}
                }
                parts.extend(account.plan.iter().map(|plan| format!("plan {plan}")));
                parts.extend(account.tier.iter().map(|tier| format!("tier {tier}")));
                parts.extend(account.label.clone());
                if !parts.is_empty() {
                    writeln!(f, "  account: {}", parts.join(", "))?;
                }
            }
            for window in &state.windows {
                write!(f, "  {}:", window.name)?;
                match window.used_percent {
                    Some(used) if !window.reset_by(now) => write!(f, " {used:.0}% used")?,
                    Some(_) => f.write_str(" reset since observed")?,
                    None => f.write_str(" utilization not reported")?,
                }
                if let Some(status) = &window.status {
                    write!(f, ", {status}")?;
                }
                if let Some(reset) = window.resets_at.filter(|reset| *reset > now) {
                    write!(f, ", resets {}", local(reset))?;
                }
                writeln!(f)?;
            }
            if let Some(credits) = &state.credits {
                match (&credits.balance, credits.has_credits) {
                    (Some(balance), _) => writeln!(f, "  credits: {balance}")?,
                    (None, Some(has)) => {
                        writeln!(f, "  credits: {}", if has { "some" } else { "none" })?
                    }
                    (None, None) => {}
                }
            }
            if state.ordinary_usage_allowed == Some(false) {
                writeln!(f, "  included usage not allowed")?;
            }
            if state.spend_control_reached == Some(true) {
                writeln!(f, "  spend control reached")?;
            }
            if let Some(cooldown) = &state.cooldown {
                if cooldown.until > now {
                    write!(f, "  cooldown until {}", local(cooldown.until))?;
                } else {
                    write!(f, "  revalidating since {}", local(cooldown.until))?;
                }
                let mut why = Vec::new();
                why.extend(
                    cooldown
                        .window
                        .iter()
                        .map(|window| format!("{window} window")),
                );
                why.extend(
                    cooldown
                        .overage
                        .iter()
                        .map(|overage| format!("overage {overage}")),
                );
                if let Some(minutes) = cooldown.backoff_minutes {
                    why.push(format!("no reset reported, backoff {minutes} minutes"));
                }
                if !why.is_empty() {
                    write!(f, " ({})", why.join(", "))?;
                }
                writeln!(f)?;
            }
            match (state.observed_at, &state.source) {
                (Some(at), Some(source)) => write!(f, "  observed {} by {source}", local(at))?,
                (Some(at), None) => write!(f, "  observed {}", local(at))?,
                (None, _) if !row.installed => f.write_str("  CLI not on PATH")?,
                (None, _) => f.write_str("  nothing observed yet")?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_usage::observation::Window;
    use crate::plan_usage::store::{Account, Cooldown};
    use chrono::TimeDelta;

    fn config(text: &str) -> UserConfig {
        let home = std::path::Path::new("/home/me");
        UserConfig::parse(text, &home.join(".thirdshift/config.toml"), home).unwrap()
    }

    fn names(view: &View) -> Vec<&'static str> {
        view.harnesses
            .iter()
            .map(|row| row.harness.name())
            .collect()
    }

    #[test]
    fn configured_routes_come_first_in_order_and_others_only_when_observed_or_signed_in() {
        let config = config(
            "[harness]\ndefault = \"codex\"\nroutes = [\"codex\", \"opencode\", { harness = \"agy\", max_utilization = 80 }]\n\
             [security]\nharness = \"muse\"\n",
        );
        let now = Utc::now();
        let signed_in = State {
            account: Some(Account {
                signed_in: Some(true),
                ..Account::default()
            }),
            ..State::default()
        };
        let observed = State {
            observed_at: Some(now),
            ..State::default()
        };
        let saved = BTreeMap::from([
            ("claude".to_owned(), signed_in.clone()),
            ("grok".to_owned(), observed),
            ("kimi".to_owned(), signed_in),
        ]);
        let view = View::of(&config, saved, |harness| harness != Harness::Muse, now);
        assert_eq!(names(&view), ["codex", "agy", "muse", "claude", "grok"]);
        let muse = &view.harnesses[2];
        assert_eq!(muse.routes[0].availability, Availability::NotInstalled);
        assert!(view.harnesses[0].active);
        assert_eq!(view.harnesses[1].routes[0].max_utilization, 80);
    }

    #[test]
    fn the_active_route_is_the_first_usable_one_and_a_ceiling_skips_a_route() {
        let config = config("[harness]\nroutes = [\"claude\", \"codex\"]\nmax_utilization = 85\n");
        let now = Utc::now();
        let near = State {
            windows: vec![Window {
                used_percent: Some(90.0),
                resets_at: Some(now + TimeDelta::hours(1)),
                ..Window::named("five_hour").unwrap()
            }],
            observed_at: Some(now),
            ..State::default()
        };
        let cooled = State {
            cooldown: Some(Cooldown {
                since: now,
                until: now + TimeDelta::hours(1),
                window: None,
                overage: None,
                backoff_minutes: Some(60),
            }),
            ..State::default()
        };
        let view = View::of(
            &config,
            BTreeMap::from([("claude".to_owned(), near.clone())]),
            |_| true,
            now,
        );
        assert_eq!(
            view.harnesses[0].routes[1].availability,
            Availability::AtCeiling
        );
        assert!(!view.harnesses[0].active);
        assert!(view.harnesses[1].active);
        let view = View::of(
            &config,
            BTreeMap::from([("claude".to_owned(), near), ("codex".to_owned(), cooled)]),
            |_| true,
            now,
        );
        assert!(view.harnesses.iter().all(|row| !row.active));
        assert_eq!(
            view.harnesses[1].routes[0].availability,
            Availability::Cooldown
        );
    }

    #[test]
    fn an_api_key_route_ahead_leaves_no_subscription_route_active() {
        let config = config("[harness]\nroutes = [\"opencode\", \"claude\"]\n");
        let view = View::of(&config, BTreeMap::new(), |_| true, Utc::now());
        assert!(
            view.harnesses
                .iter()
                .all(|row| !row.active && row.routes.iter().all(|r| !r.active))
        );
    }
}
