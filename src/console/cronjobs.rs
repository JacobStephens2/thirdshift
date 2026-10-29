//! The Console's read-only view of the CLI's cron job inventory.
use anyhow::Result;
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

pub(super) async fn show() -> Response {
    super::page(
        tokio::task::spawn_blocking(|| -> Result<String> {
            let config = UserConfig::load()?;
            Ok(CronJobsPage {
                page: super::PageContext::new(&config),
                inventory: cronjobs::load()?,
            }
            .render()?)
        })
        .await,
    )
}
