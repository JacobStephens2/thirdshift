//! Lanes and Stages share the status reader's complete time observations.
use askama::Template;

use super::filters;
use axum::response::Response;

use crate::config::UserConfig;
use crate::logs::status::{Read, time::Views};

#[derive(Template)]
#[template(path = "console/time.html")]
struct TimePage {
    page: super::PageContext,
    title: &'static str,
    hours: String,
    by_repository: bool,
    views: Views,
}

pub(super) async fn show(
    viewer: super::auth::Viewer,
    format: super::api::Format,
    stages: bool,
    query: Option<String>,
) -> Response {
    super::api::read(format, move || {
        let config = UserConfig::load()?;
        let read = Read::capture(&config, false)?;
        let hours = match super::query_value(&query, "hours").as_str() {
            "12" => 12.0,
            "24" => 24.0,
            _ => 3.5,
        };
        let by_repository = super::query_value(&query, "view") == "repository";
        format.render(read.json(&config, true)?, |_| {
            Ok(TimePage {
                page: super::PageContext::new(&config, &viewer.scopes),
                title: if stages { "Stages" } else { "Lanes" },
                hours: hours.to_string(),
                by_repository,
                views: read.times(hours, by_repository)?,
            }
            .render()?)
        })
    })
    .await
}
