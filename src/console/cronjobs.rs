//! The Console's read-only view of the CLI's cron job inventory.
use anyhow::Result;
use askama::Template;
use axum::response::Response;

use crate::config::UserConfig;
use crate::cronjobs::{self, CronJob};

#[derive(Template)]
#[template(path = "console/cronjobs.html")]
struct CronJobsPage {
    page: super::PageContext,
    jobs: Vec<CronJob>,
}

pub(super) async fn show() -> Response {
    super::page(
        tokio::task::spawn_blocking(|| -> Result<String> {
            let config = UserConfig::load()?;
            Ok(CronJobsPage {
                page: super::PageContext::new(&config),
                jobs: cronjobs::load()?,
            }
            .render()?)
        })
        .await,
    )
}
