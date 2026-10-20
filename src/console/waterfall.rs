//! One selected Command, using the same retained tree and logs as show/logs.
use anyhow::Result;
use askama::Template;
use axum::response::Response;

use crate::config::UserConfig;
use crate::logs::status::waterfall::{self, View};

#[derive(Template)]
#[template(path = "console/waterfall.html")]
struct WaterfallPage {
    page: super::PageContext,
    view: View,
}

pub(super) async fn show(query: Option<String>) -> Response {
    super::page(
        tokio::task::spawn_blocking(move || -> Result<String> {
            let config = UserConfig::load()?;
            Ok(WaterfallPage {
                page: super::PageContext::new(&config),
                view: waterfall::load(&config, &super::query_value(&query, "command"))?,
            }
            .render()?)
        })
        .await,
    )
}
