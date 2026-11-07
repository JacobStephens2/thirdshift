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
    machine: String,
    suffix: String,
    current: String,
    targets: Vec<super::fleet_control::Target>,
}

pub(super) async fn show(
    Extension(viewer): Extension<Viewer>,
    Extension(format): Extension<api::Format>,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
) -> Response {
    api::read(format, move || {
        let selection = super::fleet_control::Selection::query(&query);
        let config = UserConfig::load()?;
        if let Some(remote) = selection.remote(&config, &viewer, format)? {
            let identity = remote.identity()?;
            let (latest, failure) = match remote.get("/update") {
                Ok(versions) => (versions["latest"].as_str().map(str::to_owned), None),
                Err(error)
                    if error
                        .downcast_ref::<super::fleet::read::Refusal>()
                        .is_some_and(|refusal| {
                            refusal.status == axum::http::StatusCode::FORBIDDEN
                        }) =>
                {
                    return Err(error);
                }
                Err(error) => (None, Some(error.to_string())),
            };
            return Ok(axum::response::Html(
                UpdatePage {
                    page: PageContext::new(&config, &viewer.scopes),
                    csrf: viewer.csrf.clone(),
                    machine: selection.machine.clone(),
                    suffix: selection.suffix(),
                    current: identity.thirdshift_version,
                    latest,
                    failure,
                    result: None,
                    targets: super::fleet_control::targets(&config, &viewer, super::Scope::Admin),
                }
                .render()?,
            )
            .into_response());
        }
        if !viewer.scopes.contains(&super::Scope::Admin) {
            return Ok(api::error(
                axum::http::StatusCode::FORBIDDEN,
                "Missing scope: admin",
            ));
        }
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
                csrf: viewer.csrf.clone(),
                latest,
                failure,
                result: None,
                machine: super::hostname(),
                suffix: String::new(),
                current: env!("CARGO_PKG_VERSION").into(),
                targets: super::fleet_control::targets(&config, &viewer, super::Scope::Admin),
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
    axum::extract::RawQuery(query): axum::extract::RawQuery,
) -> Response {
    api::read(format, move || {
        let selection = super::fleet_control::Selection::query(&query);
        let config = UserConfig::load()?;
        if let Some(remote) = selection.remote(&config, &viewer, format)? {
            let result = remote.post_form("/update", &[])?;
            return Ok(axum::response::Html(
                UpdatePage {
                    page: PageContext::new(&config, &viewer.scopes),
                    csrf: viewer.csrf.clone(),
                    machine: selection.machine.clone(),
                    suffix: selection.suffix(),
                    current: String::new(),
                    latest: None,
                    failure: None,
                    result: Some(
                        result
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| result.to_string()),
                    ),
                    targets: super::fleet_control::targets(&config, &viewer, super::Scope::Admin),
                }
                .render()?,
            )
            .into_response());
        }
        if !viewer.scopes.contains(&super::Scope::Admin) {
            return Ok(api::error(
                axum::http::StatusCode::FORBIDDEN,
                "Missing scope: admin",
            ));
        }
        // Load before installing so an unavailable audit store refuses the action.
        let accounts = Accounts::load()?;
        let result = crate::update::update()?;
        accounts.record(&viewer.caller_label, EventKind::Updated)?;
        format.render(result, |result| {
            Ok(UpdatePage {
                page: PageContext::new(&UserConfig::load()?, &viewer.scopes),
                csrf: viewer.csrf.clone(),
                latest: None,
                failure: None,
                result: Some(result),
                machine: super::hostname(),
                suffix: String::new(),
                current: env!("CARGO_PKG_VERSION").into(),
                targets: super::fleet_control::targets(&config, &viewer, super::Scope::Admin),
            }
            .render()?)
        })
    })
    .await
}
