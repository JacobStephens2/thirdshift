//! Cron job inventory with confirmed starts through the Console API.
use askama::Template;
use axum::http::StatusCode;
use axum::response::Response;

use super::filters;

use crate::config::UserConfig;
use crate::cronjobs::{self, CronJobs};

#[derive(Template)]
#[template(path = "console/cronjobs.html")]
struct CronJobsPage {
    page: super::PageContext,
    inventory: CronJobs,
    csrf: String,
    passes: Vec<super::start_controls::Pass>,
    start_error: Option<String>,
    machine: String,
    can_run: bool,
    show_jobs: bool,
    targets: Vec<super::fleet_control::Target>,
}

#[derive(Template)]
#[template(path = "console/cronjobs_fleet.html")]
struct FleetPage {
    page: super::PageContext,
    inventory: cronjobs::fleet::Inventory,
}

pub(super) async fn show(
    axum::Extension(viewer): axum::Extension<super::auth::Viewer>,
    axum::Extension(format): axum::Extension<super::api::Format>,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
) -> Response {
    let fleet = super::query_value(&query, "fleet");
    if format == super::api::Format::Json && !fleet.is_empty() {
        return super::api::error(
            StatusCode::BAD_REQUEST,
            "Fleet schedules are an explicit client view; call each owning machine's API directly",
        );
    }
    if fleet == "true" && !viewer.scopes.contains(&super::Scope::Read) {
        return super::api::error(StatusCode::FORBIDDEN, "Fleet schedules need read scope");
    }
    super::api::read(format, move || {
        let selection = super::fleet_control::Selection::query(&query);
        let config = UserConfig::load()?;
        if fleet == "true" {
            let inventory = cronjobs::fleet::load(&config, viewer.fleet_person());
            return format.render(inventory, |inventory| {
                Ok(FleetPage {
                    page: super::PageContext::new(&config, &viewer.scopes),
                    inventory,
                }
                .render()?)
            });
        }
        let remote = selection.remote(&config, &viewer, format)?;
        let captured = if remote.is_none() {
            Some(crate::launch_directories::Captured::capture(&config)?)
        } else {
            None
        };
        let mut remote_passes = None;
        // Run lists startable Passes. The crontab and last-fired times stay behind read.
        let (inventory, show_jobs) = if let Some(remote) = &remote {
            let identity = remote.identity()?;
            let can_run = identity.scopes.contains(&super::Scope::Run);
            let can_read = identity.scopes.contains(&super::Scope::Read);
            if can_run {
                let options: super::start_controls::Options =
                    serde_json::from_value(remote.get("/start-options")?)?;
                remote_passes = Some(options.passes);
            }
            if can_read || !can_run {
                (serde_json::from_value(remote.get("/cronjobs")?)?, true)
            } else {
                (
                    CronJobs {
                        jobs: Vec::new(),
                        pass_starts: Vec::new(),
                    },
                    false,
                )
            }
        } else {
            (captured.as_ref().unwrap().cronjobs()?.clone(), true)
        };
        format.render(inventory, |inventory| {
            let can_run = remote_passes.is_some()
                || (remote.is_none() && viewer.scopes.contains(&super::Scope::Run));
            let (passes, start_error) = if let Some(passes) = remote_passes {
                (passes, None)
            } else if remote.is_none() && can_run {
                match super::start_controls::cron_passes(&config, captured.as_ref().unwrap()) {
                    Ok(passes) => (passes, None),
                    Err(error) => (Vec::new(), Some(error.to_string())),
                }
            } else {
                (Vec::new(), None)
            };
            Ok(CronJobsPage {
                page: super::PageContext::new(&config, &viewer.scopes),
                targets: super::fleet_control::targets(&config, &viewer, super::Scope::Run),
                machine: selection.machine,
                can_run,
                show_jobs,
                inventory,
                csrf: viewer.csrf,
                passes,
                start_error,
            }
            .render()?)
        })
    })
    .await
}
