//! A read-only host snapshot, using the same admission policy as sessions.

use anyhow::Result;
use askama::Template;
use axum::response::Response;

use crate::config::UserConfig;
use crate::resources::{self, Snapshot};

#[derive(Template)]
#[template(path = "console/resources.html")]
struct ResourcesPage {
    page: super::PageContext,
    snapshot: Snapshot,
}

pub(super) async fn show() -> Response {
    super::page(
        tokio::task::spawn_blocking(|| -> Result<String> {
            let config = UserConfig::load()?;
            Ok(ResourcesPage {
                page: super::PageContext::new(&config),
                snapshot: resources::observe(config.resources)?,
            }
            .render()?)
        })
        .await,
    )
}
