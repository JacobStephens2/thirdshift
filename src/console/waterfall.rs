//! One selected Command, using the same retained tree and logs as show/logs.
use anyhow::Result;
use askama::Template;
use axum::response::Response;

use crate::config::UserConfig;
use crate::logs::status::waterfall::{self, View};

#[derive(Template)]
#[template(path = "console/waterfall.html")]
struct WaterfallPage {
    hostname: String,
    consoles: Vec<String>,
    theme: String,
    view: View,
}

pub(super) async fn show(query: Option<String>) -> Response {
    super::page(
        tokio::task::spawn_blocking(move || -> Result<String> {
            let config = UserConfig::load()?;
            Ok(WaterfallPage {
                hostname: crate::host::name().unwrap_or_else(|| "unknown".into()),
                consoles: config.console.other_consoles.clone(),
                theme: config.console.theme.clone(),
                view: waterfall::load(&config, &super::query_value(&query, "command"))?,
            }
            .render()?)
        })
        .await,
    )
}
