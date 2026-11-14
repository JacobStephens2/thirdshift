//! The User config's ordered routes and utilization reserves: `[harness]`
//! `routes` and `max_utilization`, and `[security]` `routes`.

use anyhow::{Context, Result, bail};
use serde::Serialize;
use toml::Value;

use crate::harness::{self, Choice, ChosenBy, Harness, Settings};

/// The ceiling a route uses unless it, or `harness.max_utilization`, sets
/// one: proactive switching happens only at confirmed exhaustion.
pub const FULL: u8 = 100;

/// One entry of a `routes` list: a Harness, with the Model and Effort it
/// runs, each none to inherit `[harness.<name>]`'s.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Route {
    #[serde(serialize_with = "harness_name")]
    pub harness: Harness,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Its own utilization ceiling, ahead of `harness.max_utilization`.
    pub max_utilization: Option<u8>,
    /// Whether it may spend credit or overage balances once its included
    /// Plan windows are used up.
    pub allow_credits: bool,
}

pub(super) fn harness_name<S: serde::Serializer>(
    harness: &Harness,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(harness.name())
}

impl Route {
    /// A route written as the Harness's name alone.
    pub fn named(harness: Harness) -> Self {
        Self {
            harness,
            model: None,
            effort: None,
            max_utilization: None,
            allow_credits: false,
        }
    }

    /// What a session on this route runs on: its Harness, with its own
    /// Model and Effort, else those `[harness.<name>]` in `settings` sets.
    pub fn choice(&self, settings: &Settings, chosen_by: ChosenBy) -> Choice {
        let set = settings.of(self.harness);
        Choice {
            harness: self.harness,
            model: self.model.clone().or_else(|| set.model.clone()),
            effort: self.effort.clone().or_else(|| set.effort.clone()),
            chosen_by,
        }
    }

    /// Whether a session on `choice` runs as this route does, given the
    /// `[harness.<name>]` Model and Effort it inherits from `settings`.
    fn runs_as(&self, choice: &Choice, settings: &Settings) -> bool {
        let set = settings.of(self.harness);
        self.harness == choice.harness
            && self.model.as_ref().or(set.model.as_ref()) == choice.model.as_ref()
            && self.effort.as_ref().or(set.effort.as_ref()) == choice.effort.as_ref()
    }
}

/// The routing settings of the User config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routing {
    /// `harness.routes`, in order; empty keeps `harness.default` alone.
    pub routes: Vec<Route>,
    /// `harness.max_utilization`, from 1 to 100.
    pub max_utilization: u8,
    /// `security.routes`, in order.
    pub security_routes: Vec<Route>,
}

impl Default for Routing {
    fn default() -> Self {
        Self {
            routes: Vec::new(),
            max_utilization: FULL,
            security_routes: Vec::new(),
        }
    }
}

impl Routing {
    /// What a Command that asks for no Harness, Model or Effort runs on
    /// when `harness.routes` is set: its first route, which Plan usage may
    /// move each session Step away from. None with no routes, when
    /// `harness.default` decides alone.
    pub fn first(&self, settings: &Settings) -> Option<Choice> {
        self.routes
            .first()
            .map(|route| route.choice(settings, ChosenBy::Routes))
    }

    /// The ceiling `route` switches away at, as a percentage.
    pub fn ceiling(&self, route: &Route) -> u8 {
        route.max_utilization.unwrap_or(self.max_utilization)
    }

    /// Whether a session on `choice` may spend credits: only when the User
    /// config chose it and the configured route it runs as allows them. A
    /// Harness named by a command, a Step or `harness.default` alone never
    /// spends them.
    pub fn allows_credits(&self, choice: &Choice, settings: &Settings) -> bool {
        matches!(
            choice.chosen_by,
            ChosenBy::UserConfig | ChosenBy::Routes | ChosenBy::SecurityConfig
        ) && self
            .routes
            .iter()
            .chain(&self.security_routes)
            .find(|route| route.runs_as(choice, settings))
            .is_some_and(|route| route.allow_credits)
    }
}

/// `value`, the setting `key` of `file`, as a list of routes.
pub fn parse_routes(value: &Value, key: &str, file: &str) -> Result<Vec<Route>> {
    let Value::Array(entries) = value else {
        bail!("{key} must be a list of Harness names or route tables in {file}");
    };
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            parse_route(entry).with_context(|| format!("{key} entry {} in {file}", index + 1))
        })
        .collect()
}

fn parse_route(entry: &Value) -> Result<Route> {
    match entry {
        Value::String(name) => named(name),
        Value::Table(table) => {
            let mut route = match table.get("harness") {
                Some(Value::String(name)) => named(name)?,
                Some(_) => bail!("harness must be a quoted name"),
                None => bail!("a route table must name its harness"),
            };
            for (key, value) in table {
                match (key.as_str(), value) {
                    ("harness", _) => {}
                    ("model", Value::String(model)) => {
                        route.model = Some(model.clone()).filter(|model| !model.is_empty())
                    }
                    ("effort", Value::String(effort)) => {
                        route.effort = Some(effort.clone()).filter(|effort| !effort.is_empty())
                    }
                    ("model" | "effort", _) => {
                        bail!("{key} must be a quoted string, blank for [harness.<name>]'s")
                    }
                    ("max_utilization", value) => {
                        route.max_utilization = Some(percentage(value, key)?)
                    }
                    ("allow_credits", Value::Boolean(allow)) => route.allow_credits = *allow,
                    ("allow_credits", _) => bail!("allow_credits must be true or false"),
                    _ => bail!(
                        "unknown key {key}; a route takes harness, model, effort, max_utilization and allow_credits"
                    ),
                }
            }
            Ok(route)
        }
        Value::Integer(_)
        | Value::Float(_)
        | Value::Boolean(_)
        | Value::Datetime(_)
        | Value::Array(_) => bail!("a route must be a Harness name or an inline table"),
    }
}

fn named(name: &str) -> Result<Route> {
    Harness::named(name.trim())
        .map(Route::named)
        .with_context(|| format!("harness must be {}, not {name:?}", harness::names()))
}

/// `value` as a utilization percentage from 1 to 100.
pub fn percentage(value: &Value, key: &str) -> Result<u8> {
    value
        .as_integer()
        .filter(|percent| (1..=i64::from(FULL)).contains(percent))
        .and_then(|percent| u8::try_from(percent).ok())
        .with_context(|| format!("{key} must be a whole percentage from 1 to 100"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::ModelAndEffort;

    fn routes(text: &str) -> Result<Vec<Route>> {
        let table: toml::Table = text.parse().unwrap();
        parse_routes(&table["routes"], "harness.routes", "the file")
    }

    #[test]
    fn names_and_tables_parse_in_order_with_inherited_settings_left_unset() {
        let parsed = routes(
            r#"routes = ["claude", { harness = "agy", model = "gemini-3.8-flash", effort = "", max_utilization = 90, allow_credits = true }]"#,
        )
        .unwrap();
        assert_eq!(
            parsed,
            [
                Route::named(Harness::Claude),
                Route {
                    harness: Harness::Agy,
                    model: Some("gemini-3.8-flash".into()),
                    effort: None,
                    max_utilization: Some(90),
                    allow_credits: true,
                },
            ]
        );
    }

    #[test]
    fn malformed_routes_name_the_entry_and_the_problem() {
        for (text, expected) in [
            (r#"routes = "claude""#, "must be a list"),
            (r#"routes = ["gemini"]"#, "entry 1"),
            (r#"routes = ["claude", 3]"#, "entry 2"),
            (r#"routes = [{ model = "x" }]"#, "must name its harness"),
            (
                r#"routes = [{ harness = "codex", max_utilization = 0 }]"#,
                "from 1 to 100",
            ),
            (
                r#"routes = [{ harness = "codex", max_utilization = 101 }]"#,
                "from 1 to 100",
            ),
            (
                r#"routes = [{ harness = "codex", allow_credits = "yes" }]"#,
                "true or false",
            ),
            (
                r#"routes = [{ harness = "codex", credits = true }]"#,
                "unknown key credits",
            ),
        ] {
            let error = format!("{:#}", routes(text).unwrap_err());
            assert!(error.contains(expected), "{text}: {error}");
        }
    }

    #[test]
    fn only_a_user_config_route_that_allows_credits_spends_them() {
        let settings = Settings {
            agy: ModelAndEffort {
                model: Some("gemini-3.8-pro".into()),
                effort: None,
            },
            ..Settings::default()
        };
        let routing = Routing {
            routes: vec![
                Route::named(Harness::Claude),
                Route {
                    allow_credits: true,
                    ..Route::named(Harness::Agy)
                },
            ],
            ..Routing::default()
        };
        let choice = |chosen_by, model: &str| Choice {
            harness: Harness::Agy,
            model: Some(model.into()),
            effort: None,
            chosen_by,
        };
        assert!(routing.allows_credits(&choice(ChosenBy::UserConfig, "gemini-3.8-pro"), &settings));
        assert!(!routing.allows_credits(&choice(ChosenBy::Command, "gemini-3.8-pro"), &settings));
        assert!(!routing.allows_credits(&choice(ChosenBy::Step, "gemini-3.8-pro"), &settings));
        assert!(
            !routing.allows_credits(&choice(ChosenBy::UserConfig, "gemini-3.8-flash"), &settings)
        );
        assert!(
            !Routing::default()
                .allows_credits(&choice(ChosenBy::UserConfig, "gemini-3.8-pro"), &settings)
        );
    }
}
