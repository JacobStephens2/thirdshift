//! The three ways into User config share reads, edits and Console-log auditing.
use anyhow::Result;
use askama::Template;
use axum::extract::Path;
use axum::extract::RawQuery;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Form};
use serde::Deserialize;

use super::{PageContext, Scope, accounts::Accounts, api, auth::Viewer};
use crate::config::{
    UserConfig,
    settings::{self, Setting, Settings},
};

pub(super) fn change(key: &str, value: &str, actor: &str) -> Result<Setting> {
    let accounts = Accounts::load()?;
    let change = settings::write(key, value).map_err(|error| anyhow::anyhow!("{error:#}"))?;
    if change.changed {
        accounts.record_config(actor, key, change.old, change.setting.value.clone())?;
    }
    Ok(change.setting)
}

pub fn command(key: Option<&str>, value: Option<&str>, json: bool) -> Result<String> {
    let Some(key) = key else {
        let settings = settings::read()?;
        return Ok(if json {
            serde_json::to_string_pretty(&settings)?
        } else {
            settings
                .settings
                .iter()
                .map(text)
                .collect::<Vec<_>>()
                .join("\n")
        });
    };
    let setting = if let Some(value) = value {
        change(key, value, &super::look::cli_actor()?)?
    } else {
        settings::one(settings::read()?, key)?
    };
    Ok(if json {
        serde_json::to_string_pretty(&setting)?
    } else {
        text(&setting)
    })
}

fn text(setting: &Setting) -> String {
    let mut line = format!(
        "{} = {} (default: {})",
        setting.key, setting.value, setting.default
    );
    if let Some(reason) = &setting.reason {
        line.push_str(&format!(" — {reason}"));
    }
    line
}

#[derive(Template)]
#[template(path = "console/user_config.html")]
struct ConfigPage {
    page: PageContext,
    settings: Settings,
    csrf: String,
    admin: bool,
    machine: String,
    suffix: String,
    saved: String,
    targets: Vec<super::fleet_control::Target>,
}

pub(super) async fn show(
    Extension(viewer): Extension<Viewer>,
    Extension(format): Extension<api::Format>,
    RawQuery(query): RawQuery,
) -> Response {
    api::read(format, move || {
        let selection = super::fleet_control::Selection::query(&query);
        let config = UserConfig::load()?;
        let remote = selection.remote(&config, &viewer, format)?;
        let (settings, admin) = if let Some(remote) = &remote {
            let identity = remote.identity()?;
            (
                serde_json::from_value(remote.get("/config")?)?,
                identity.scopes.contains(&Scope::Admin),
            )
        } else {
            (settings::read()?, viewer.scopes.contains(&Scope::Admin))
        };
        format.render(settings, |settings| {
            Ok(ConfigPage {
                page: PageContext::new(&config, &viewer.scopes),
                targets: super::fleet_control::targets(&config, &viewer, Scope::Admin),
                machine: if selection.machine.is_empty() {
                    super::hostname()
                } else {
                    selection.machine.clone()
                },
                suffix: selection.suffix(),
                saved: super::query_value(&query, "saved"),
                settings,
                csrf: viewer.csrf,
                admin,
            }
            .render()?)
        })
    })
    .await
}

pub(super) async fn one(
    Path(key): Path<String>,
    Extension(format): Extension<api::Format>,
) -> Response {
    api::read(format, move || {
        Ok(axum::Json(settings::one(settings::read()?, &key)?).into_response())
    })
    .await
}

#[derive(Deserialize)]
pub(super) struct Edit {
    value: String,
}

pub(super) async fn set(
    Path(key): Path<String>,
    Extension(viewer): Extension<Viewer>,
    Extension(format): Extension<api::Format>,
    RawQuery(query): RawQuery,
    Form(edit): Form<Edit>,
) -> Response {
    let selection = super::fleet_control::Selection::query(&query);
    let location = if format == api::Format::Html {
        let mut parameters = Vec::new();
        if !selection.machine.is_empty() {
            parameters.push(("machine", selection.machine.as_str()));
        }
        parameters.push(("saved", key.as_str()));
        let query = serde_urlencoded::to_string(parameters).expect("config query contains strings");
        format!("/config?{query}#config-{key}")
    } else {
        "/config".to_owned()
    };
    if !selection.machine.is_empty() {
        return api::read(format, move || {
            let config = UserConfig::load()?;
            if let Some(remote) = selection.remote(&config, &viewer, format)? {
                remote.post_form(
                    &super::fleet_control::config_route(&key)?,
                    &[("value", &edit.value)],
                )?;
            } else {
                anyhow::ensure!(
                    viewer.scopes.contains(&Scope::Admin),
                    "Missing scope: admin"
                );
                change(&key, &edit.value, &viewer.caller_label)?;
            }
            Ok(axum::response::Redirect::to(&location).into_response())
        })
        .await;
    }
    if !viewer.scopes.contains(&Scope::Admin) {
        return api::error(axum::http::StatusCode::FORBIDDEN, "Missing scope: admin");
    }
    api::action(format, &location, move || {
        change(&key, &edit.value, &viewer.caller_label)
    })
    .await
}
