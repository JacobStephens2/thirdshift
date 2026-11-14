//! Plan usage: each subscription login's Plan windows, cooldowns and the
//! routes that use it, across the Fleet by default, the twin of
//! `thirdshift plan-usage`; clearing this machine's cooldowns; receiving a
//! Fleet machine's pushed view; and the header's warning.
use anyhow::{Context, Result};
use askama::Template;
use axum::body::Bytes;
use axum::extract::RawQuery;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Form};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::auth::Viewer;
use crate::config::UserConfig;
use crate::plan_usage::fleet::{self, Login, Machine, Status};
use crate::plan_usage::{Action, Selection, Shown};

/// The header's warning: any configured route on this machine used up or
/// at its ceiling, from what is saved, running no read.
pub(super) struct Warning {
    pub(super) label: String,
    pub(super) detail: String,
}

impl Warning {
    pub(super) fn new(config: &UserConfig) -> Option<Self> {
        let view = crate::plan_usage::saved_view(config).ok()?;
        let limited: Vec<String> = view
            .harnesses
            .iter()
            .flat_map(|row| {
                row.routes
                    .iter()
                    .filter(|route| route.availability.limited())
                    .map(|route| format!("{} {route}", row.harness.name()))
            })
            .collect();
        if limited.is_empty() {
            return None;
        }
        Some(Self {
            label: format!(
                "Plan usage · {} route{} limited",
                limited.len(),
                if limited.len() == 1 { "" } else { "s" }
            ),
            detail: limited.join("; "),
        })
    }
}

/// The machines a Plan usage page can be scoped to, for its selector.
struct Machines {
    /// The machine chosen; missing for the whole Fleet.
    chosen: Option<String>,
    /// Whether the Fleet can be chosen: a machine is paired or saved.
    fleet: bool,
    /// This machine's name, distinct from every Fleet entry's.
    local: String,
    /// Fleet machines and retired or reporting ones with a saved view.
    others: Vec<String>,
}

impl Machines {
    fn is(&self, name: &str) -> bool {
        self.chosen.as_deref() == Some(name)
    }
}

#[derive(Template)]
#[template(path = "console/plan_usage.html")]
struct Page {
    page: super::PageContext,
    csrf: String,
    /// The machines read; empty for this machine's own view.
    machines: Vec<Machine>,
    logins: Vec<Login>,
    selector: Machines,
    harnesses: [&'static str; 5],
    /// The CLI twin of what is shown.
    cli: String,
    now: DateTime<Utc>,
}

impl Page {
    fn age(&self, at: &DateTime<Utc>) -> String {
        super::age(at)
    }

    /// Whether a machine's view is a saved one: offline, retired or
    /// pushed.
    fn saved(&self, machine: &Machine) -> bool {
        matches!(machine.status, Status::Cached | Status::Reported)
    }

    /// Whether `window` reset since it was observed.
    fn reset(&self, window: &crate::plan_usage::observation::Window) -> bool {
        window.reset_by(self.now)
    }

    /// Whether `at` is still ahead.
    fn ahead(&self, at: &DateTime<Utc>) -> bool {
        *at > self.now
    }

    fn local_time(&self, at: &DateTime<Utc>) -> String {
        crate::local_time::readable(&at.to_rfc3339()).into_owned()
    }
}

/// The machine choice as the `plan-usage` arguments it names: `fleet` and
/// `local` take `true`, `machine` a Fleet name. The page's selector sends
/// an empty `machine` for the Fleet, which means the default.
fn selection(query: &Option<String>) -> Result<Selection> {
    let mut arguments = vec!["plan-usage".to_owned(), "--json".to_owned()];
    if let Some(query) = query {
        for (key, value) in serde_urlencoded::from_str::<Vec<(String, String)>>(query)? {
            if value.is_empty() {
                continue;
            }
            match key.as_str() {
                "machine" => arguments.extend(["--machine".to_owned(), value]),
                "fleet" | "local" if value == "true" => arguments.push(format!("--{key}")),
                "fleet" | "local" => anyhow::bail!("{key} takes true"),
                _ => anyhow::bail!("unknown plan-usage argument: {key}"),
            }
        }
    }
    let crate::args::Command::PlanUsage(Action::Show { selection, .. }) =
        crate::args::parse(&arguments)?
    else {
        unreachable!("fixed plan-usage command")
    };
    Ok(selection)
}

pub(super) async fn show(
    Extension(viewer): Extension<Viewer>,
    Extension(format): Extension<super::api::Format>,
    RawQuery(query): RawQuery,
) -> Response {
    super::api::read(format, move || {
        let config = UserConfig::load()?;
        let person = viewer.fleet_person();
        let selection = selection(&query)?;
        let shown = crate::plan_usage::show(&config, &selection, person)?;
        if format == super::api::Format::Json {
            return Ok(axum::Json(shown.json()?).into_response());
        }
        let local = super::fleet_control::local_name(&config);
        let (chosen, machines, logins, now) = match shown {
            Shown::Local(view) => {
                let now = view.now;
                let logins = fleet::group(&[(local.clone(), view)]);
                (Some(local.clone()), Vec::new(), logins, now)
            }
            Shown::Fleet(fleet) => (fleet.machine, fleet.machines, fleet.harnesses, fleet.now),
        };
        let mut cli = vec!["plan-usage".to_owned()];
        match (&selection, &chosen) {
            (Selection::Local, _) => cli.push("--local".into()),
            (_, Some(name)) => cli.extend(["--machine".into(), name.clone()]),
            (_, None) => cli.push("--fleet".into()),
        }
        cli.push("--json".into());
        let selector = Machines {
            chosen,
            fleet: fleet::default_view(&config, person),
            others: fleet::others(&config, person),
            local,
        };
        Ok(axum::response::Html(
            Page {
                page: super::PageContext::new(&config, &viewer.scopes),
                csrf: viewer.csrf,
                machines,
                logins,
                selector,
                harnesses: crate::plan_usage::observation::SUBSCRIPTION.map(|h| h.name()),
                cli: crate::args::shell_command(cli),
                now,
            }
            .render()?,
        )
        .into_response())
    })
    .await
}

#[derive(Deserialize)]
pub(super) struct ClearForm {
    #[serde(default)]
    harness: String,
}

/// `plan-usage --clear [<harness>]`: forget this machine's recorded
/// cooldowns for one subscription Harness, or for all.
pub(super) async fn clear(
    Extension(format): Extension<super::api::Format>,
    Form(form): Form<ClearForm>,
) -> Response {
    super::api::action(
        format,
        "/plan-usage",
        move || -> Result<serde_json::Value> {
            let harness = match form.harness.as_str() {
                "" => None,
                name => Some(crate::plan_usage::subscription_named(name)?),
            };
            let answer = crate::plan_usage::execute(Action::Clear {
                harness,
                json: true,
            })?;
            serde_json::from_str(&answer).context("plan-usage --clear answered without JSON")
        },
    )
    .await
}

/// The largest Plan usage report a machine may push: a handful of
/// Harnesses' windows and routes.
pub(super) const REPORT_LIMIT: usize = 256 * 1024;

/// The receiving end of `plan-usage --push` and the push after each
/// Command: save the view a `report` token's holder pushed in that
/// machine's own reported slot. Any other caller, or a report naming
/// another machine, is refused.
pub(super) async fn receive(Extension(viewer): Extension<Viewer>, body: Bytes) -> Response {
    let super::identity::Identity::Token { holder, .. } = viewer.identity else {
        return super::api::error(StatusCode::FORBIDDEN, "Missing scope: report");
    };
    let report = match serde_json::from_slice::<fleet::Report>(&body) {
        Ok(report) => report,
        Err(error) => {
            return super::api::error(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("invalid Plan usage report: {error}"),
            );
        }
    };
    let result = tokio::task::spawn_blocking(move || -> Result<_> {
        let config = UserConfig::load()?;
        let now = Utc::now();
        let local = super::fleet_control::local_name(&config);
        report.receive(&config.logs_dir, &holder, &local, now)?;
        Ok(serde_json::json!({"machine": holder, "pushed_at": now}))
    })
    .await;
    let error = match result {
        Ok(Ok(receipt)) => return axum::Json(receipt).into_response(),
        Ok(Err(error)) => match error.downcast_ref::<fleet::Refusal>() {
            Some(refusal @ fleet::Refusal::Mismatch) => {
                return super::api::error(StatusCode::FORBIDDEN, refusal);
            }
            Some(refusal) => {
                return super::api::error(StatusCode::UNPROCESSABLE_ENTITY, refusal);
            }
            None => error,
        },
        Err(error) => error.into(),
    };
    eprintln!("thirdshift Console: Plan usage report not saved: {error:?}");
    super::api::error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Could not save the Plan usage report.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_query_names_the_same_selection_as_the_cli() {
        let parse = |query: &str| selection(&Some(query.to_owned()));
        assert_eq!(parse("").unwrap(), Selection::Default);
        assert_eq!(parse("machine=").unwrap(), Selection::Default);
        assert_eq!(parse("fleet=true").unwrap(), Selection::Fleet);
        assert_eq!(parse("local=true").unwrap(), Selection::Local);
        assert_eq!(
            parse("machine=laptop").unwrap(),
            Selection::Machine("laptop".into())
        );
        assert!(parse("fleet=true&local=true").is_err());
        assert!(parse("fleet=yes").is_err());
        assert!(parse("clear=agy").is_err());
    }
}
