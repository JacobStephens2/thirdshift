//! The Console's read-only view of the CLI's cron job inventory.
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
}

pub(super) async fn show(axum::Extension(format): axum::Extension<super::api::Format>) -> Response {
    super::api::read(format, move || {
        let config = UserConfig::load()?;
        format.render(cronjobs::load()?, |inventory| {
            Ok(CronJobsPage {
                page: super::PageContext::new(&config),
                inventory,
            }
            .render()?)
        })
    })
    .await
}
