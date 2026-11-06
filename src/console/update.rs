//! Requested updates share the CLI installer; graceful shutdown drains the answer.
use askama::Template;
use axum::{Extension, response::Response};

use super::{
    PageContext,
    accounts::{Accounts, EventKind},
    api,
    auth::Viewer,
};
use crate::config::UserConfig;

#[derive(Template)]
#[template(path = "console/update.html")]
struct UpdatePage {
    page: PageContext,
    csrf: String,
    latest: Option<String>,
    failure: Option<String>,
    result: Option<String>,
}

pub(super) async fn show(
    Extension(viewer): Extension<Viewer>,
    Extension(format): Extension<api::Format>,
) -> Response {
    api::read(format, move || {
        let versions = crate::update::check();
        if format == api::Format::Json {
            return Ok(axum::Json(versions?).into_response());
        }
        let (latest, failure) = match versions {
            Ok(versions) => (Some(versions.latest), None),
            Err(error) => (None, Some(error.to_string())),
        };
        Ok(axum::response::Html(
            UpdatePage {
                page: PageContext::new(&UserConfig::load()?, &viewer.scopes),
                csrf: viewer.csrf,
                latest,
                failure,
                result: None,
            }
            .render()?,
        )
        .into_response())
    })
    .await
}

use axum::response::IntoResponse;

pub(super) async fn run(
    Extension(viewer): Extension<Viewer>,
    Extension(format): Extension<api::Format>,
) -> Response {
    api::read(format, move || {
        // Load before installing so an unavailable audit store refuses the action.
        let accounts = Accounts::load()?;
        let result = crate::update::update()?;
        accounts.record(&viewer.caller_label, EventKind::Updated)?;
        format.render(result, |result| {
            Ok(UpdatePage {
                page: PageContext::new(&UserConfig::load()?, &viewer.scopes),
                csrf: viewer.csrf,
                latest: None,
                failure: None,
                result: Some(result),
            }
            .render()?)
        })
    })
    .await
}
