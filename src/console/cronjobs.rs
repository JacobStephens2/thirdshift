//! Cron job inventory with confirmed starts through the Console API.
use askama::Template;
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
    targets: Vec<super::fleet_control::Target>,
}

pub(super) async fn show(
    axum::Extension(viewer): axum::Extension<super::auth::Viewer>,
    axum::Extension(format): axum::Extension<super::api::Format>,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
) -> Response {
    super::api::read(format, move || {
        let selection = super::fleet_control::Selection::query(&query);
        let config = UserConfig::load()?;
        let remote = selection.remote(&config, &viewer, format)?;
        let mut remote_options = None;
        let inventory = if let Some(remote) = &remote {
            let identity = remote.identity()?;
            if identity.scopes.contains(&super::Scope::Run) {
                let options: super::start_controls::Options =
                    serde_json::from_value(remote.get("/start-options")?)?;
                remote_options = Some(options.passes);
                options.inventory
            } else {
                serde_json::from_value(remote.get("/cronjobs")?)?
            }
        } else {
            cronjobs::load()?
        };
        format.render(inventory, |inventory| {
            let can_run = remote_options.is_some()
                || (remote.is_none() && viewer.scopes.contains(&super::Scope::Run));
            let (passes, start_error) = if let Some(passes) = remote_options {
                (passes, None)
            } else if remote.is_none() && can_run {
                match super::start_controls::passes(&config, &inventory.jobs) {
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
