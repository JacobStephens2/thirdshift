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
}

pub(super) async fn show(
    axum::Extension(viewer): axum::Extension<super::auth::Viewer>,
    axum::Extension(format): axum::Extension<super::api::Format>,
) -> Response {
    super::api::read(format, move || {
        let config = UserConfig::load()?;
        format.render(cronjobs::load()?, |inventory| {
            let (passes, start_error) = if viewer.scopes.contains(&super::Scope::Run) {
                match super::start_controls::passes(&config, &inventory.jobs) {
                    Ok(passes) => (passes, None),
                    Err(error) => (Vec::new(), Some(error.to_string())),
                }
            } else {
                (Vec::new(), None)
            };
            Ok(CronJobsPage {
                page: super::PageContext::new(&config, &viewer.scopes),
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
