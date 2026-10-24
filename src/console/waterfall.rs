//! One selected Command, using the same retained tree and logs as show/logs.
use anyhow::Result;
use askama::Template;
use axum::extract::RawQuery;
use axum::response::{IntoResponse, Response};

use crate::config::UserConfig;
use crate::logs::spans::{self, View};

#[derive(Template)]
#[template(path = "console/waterfall.html")]
struct WaterfallPage {
    page: super::PageContext,
    view: View,
    data: String,
}

pub(super) async fn show(query: Option<String>) -> Response {
    super::page(
        tokio::task::spawn_blocking(move || -> Result<String> {
            let config = UserConfig::load()?;
            let view = spans::load(&config, &super::query_value(&query, "command"))?;
            // JSON is data, never executable markup, even inside a script tag.
            let data = serde_json::to_string(&view.tree)?
                .replace('&', "\\u0026")
                .replace('<', "\\u003c")
                .replace('>', "\\u003e");
            Ok(WaterfallPage {
                page: super::PageContext::new(&config),
                view,
                data,
            }
            .render()?)
        })
        .await,
    )
}

pub(super) async fn data(RawQuery(query): RawQuery) -> Response {
    match tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        let view = spans::load(&UserConfig::load()?, &super::query_value(&query, "command"))?;
        let selected = super::query_value(&query, "span");
        Ok(if selected.is_empty() {
            serde_json::to_value(view.tree)?
        } else {
            serde_json::to_value(view.tree.span(&selected)?)?
        })
    })
    .await
    {
        Ok(Ok(value)) => axum::Json(value).into_response(),
        Ok(Err(error)) => (axum::http::StatusCode::BAD_REQUEST, error.to_string()).into_response(),
        Err(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
