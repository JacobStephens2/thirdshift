//! The three ways into User config share reads, edits and Console-log auditing.
use anyhow::Result;
use askama::Template;
use axum::extract::Path;
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
    if let Some(reason) = setting.reason {
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
}

pub(super) async fn show(
    Extension(viewer): Extension<Viewer>,
    Extension(format): Extension<api::Format>,
) -> Response {
    api::read(format, move || {
        format.render(settings::read()?, |settings| {
            Ok(ConfigPage {
                page: PageContext::new(&UserConfig::load()?, &viewer.scopes),
                settings,
                csrf: viewer.csrf,
                admin: viewer.scopes.contains(&Scope::Admin),
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
    Form(edit): Form<Edit>,
) -> Response {
    api::action(format, "/config", move || {
        change(&key, &edit.value, &viewer.caller_label)
    })
    .await
}
