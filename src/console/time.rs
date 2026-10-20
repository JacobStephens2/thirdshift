//! Lanes and Stages share the status reader's complete time observations.
use anyhow::Result;
use askama::Template;
use axum::response::Response;

use crate::config::UserConfig;
use crate::logs::status::time::{self, Views};

#[derive(Template)]
#[template(path = "console/time.html")]
struct TimePage {
    page: super::PageContext,
    title: &'static str,
    hours: String,
    views: Views,
}

pub(super) async fn show(stages: bool, query: Option<String>) -> Response {
    super::page(
        tokio::task::spawn_blocking(move || -> Result<String> {
            let config = UserConfig::load()?;
            let hours = match super::query_value(&query, "hours").as_str() {
                "12" => 12.0,
                "24" => 24.0,
                _ => 3.5,
            };
            Ok(TimePage {
                page: super::PageContext::new(&config),
                title: if stages { "Stages" } else { "Lanes" },
                hours: hours.to_string(),
                views: time::load(&config, hours)?,
            }
            .render()?)
        })
        .await,
    )
}
