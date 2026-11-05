//! One selected Command, using the same retained tree and logs as show/logs.
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

pub(super) async fn show(format: super::api::Format, query: Option<String>) -> Response {
    super::api::read(format, move || {
        let config = UserConfig::load()?;
        let view = spans::load(&config, &super::query_value(&query, "command"))?;
        // JSON is data, never executable markup, even inside a script tag.
        let data = serde_json::to_string(&view.tree)?
            .replace('&', "\\u0026")
            .replace('<', "\\u003c")
            .replace('>', "\\u003e");
        format.render(serde_json::to_value(&view.tree)?, |_| {
            Ok(WaterfallPage {
                page: super::PageContext::new(&config),
                view,
                data,
            }
            .render()?)
        })
    })
    .await
}

pub(super) async fn data(
    axum::Extension(format): axum::Extension<super::api::Format>,
    RawQuery(query): RawQuery,
) -> Response {
    super::api::read(format, move || {
        let value = (|| -> anyhow::Result<serde_json::Value> {
            let view = spans::load(&UserConfig::load()?, &super::query_value(&query, "command"))?;
            let selected = super::query_value(&query, "span");
            Ok(if selected.is_empty() {
                serde_json::to_value(view.tree)?
            } else {
                serde_json::to_value(view.tree.span(&selected)?)?
            })
        })();
        match value {
            Ok(value) => Ok(axum::Json(value).into_response()),
            Err(error) if format == super::api::Format::Html => {
                Ok((axum::http::StatusCode::BAD_REQUEST, error.to_string()).into_response())
            }
            Err(error) => Err(error),
        }
    })
    .await
}
