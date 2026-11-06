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
    owning_console: Option<String>,
}

fn security_refusal(
    viewer: &super::auth::Viewer,
    config: &UserConfig,
    command: &str,
) -> anyhow::Result<Option<Response>> {
    Ok((!viewer.scopes.contains(&super::Scope::Security)
        && super::api::security_command(config, command)?)
    .then(|| super::api::error(axum::http::StatusCode::FORBIDDEN, "Missing scope: security")))
}

fn span_view(
    viewer: &super::auth::Viewer,
    config: &UserConfig,
    query: &Option<String>,
) -> anyhow::Result<View> {
    let command = super::query_value(query, "command");
    let machine = super::query_value(query, "machine");
    if let Some(remote) = super::fleet::read::Remote::select(
        config,
        &machine,
        viewer.fleet_person().map(str::to_owned),
    )? {
        let value = remote.get(&crate::logs::status::inspect::url("/spans", &command, None))?;
        let mut tree: spans::Tree = serde_json::from_value(value)?;
        for span in &mut tree.spans {
            span.command_url = remote.command_url("/peek", &span.command, None);
            span.conversation_url = span.conversation_url.as_ref().and_then(|url| {
                let url = reqwest::Url::parse("http://localhost")
                    .ok()?
                    .join(url)
                    .ok()?;
                let session = url
                    .query_pairs()
                    .find(|(key, _)| key == "session")?
                    .1
                    .into_owned();
                Some(remote.command_url(
                    "/log",
                    &span.command,
                    Some(std::path::Path::new(&session)),
                ))
            });
        }
        let choice = spans::Choice {
            subject: tree
                .spans
                .first()
                .map(|span| span.name.clone())
                .unwrap_or_default(),
            title: None,
            state: "remote".into(),
            selected: true,
            url: remote.command_url("/waterfall", &tree.command, None),
        };
        return Ok(View {
            choices: vec![choice],
            tree,
        });
    }
    if viewer.scopes.contains(&super::Scope::Security) {
        spans::load(config, &command)
    } else {
        spans::load_without_security_sessions(config, &command)
    }
}

pub(super) async fn show(
    viewer: super::auth::Viewer,
    format: super::api::Format,
    query: Option<String>,
) -> Response {
    if let Some(response) = format.owning_machine(&query) {
        return response;
    }
    super::api::read(format, move || {
        let config = UserConfig::load()?;
        let view = span_view(&viewer, &config, &query)?;
        if super::machine_is_local(&query)
            && let Some(response) = security_refusal(&viewer, &config, &view.tree.command)?
        {
            return Ok(response);
        }
        // JSON is data, never executable markup, even inside a script tag.
        let data = serde_json::to_string(&view.tree)?
            .replace('&', "\\u0026")
            .replace('<', "\\u003c")
            .replace('>', "\\u003e");
        format.render(serde_json::to_value(&view.tree)?, |_| {
            Ok(WaterfallPage {
                page: super::PageContext::new(&config, &viewer.scopes),
                view,
                data,
                owning_console: owning_console(&config, &query),
            }
            .render()?)
        })
    })
    .await
}

pub(super) async fn data(
    axum::Extension(viewer): axum::Extension<super::auth::Viewer>,
    axum::Extension(format): axum::Extension<super::api::Format>,
    RawQuery(query): RawQuery,
) -> Response {
    if let Some(response) = format.owning_machine(&query) {
        return response;
    }
    super::api::read(format, move || {
        let value = (|| -> anyhow::Result<Response> {
            let config = UserConfig::load()?;
            let view = span_view(&viewer, &config, &query)?;
            if super::machine_is_local(&query)
                && let Some(response) = security_refusal(&viewer, &config, &view.tree.command)?
            {
                return Ok(response);
            }
            let selected = super::query_value(&query, "span");
            let value = if selected.is_empty() {
                serde_json::to_value(view.tree)?
            } else {
                serde_json::to_value(view.tree.span(&selected)?)?
            };
            Ok(axum::Json(value).into_response())
        })();
        match value {
            Ok(response) => Ok(response),
            Err(error) if error.is::<super::fleet::read::Refusal>() => Err(error),
            Err(error) if format == super::api::Format::Html => {
                Ok((axum::http::StatusCode::BAD_REQUEST, error.to_string()).into_response())
            }
            Err(error) => Err(error),
        }
    })
    .await
}

fn owning_console(config: &UserConfig, query: &Option<String>) -> Option<String> {
    if super::machine_is_local(query) {
        return None;
    }
    let machine = super::query_value(query, "machine");
    config
        .console
        .fleet
        .iter()
        .find(|entry| entry.name == machine)
        .map(|entry| {
            super::fleet::read::owning_url(
                entry,
                &crate::logs::status::inspect::url(
                    "/waterfall",
                    &super::query_value(query, "command"),
                    None,
                ),
            )
        })
}
