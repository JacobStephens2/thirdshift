//! The CLI and authenticated page share the same restricted look writes.
use anyhow::{Context, Result, ensure};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Form, Json};
use serde::Deserialize;
use serde_json::json;

use super::accounts::{Accounts, EventKind};
use super::auth::Viewer;
use crate::config::console::themes::{self, Theme};
use crate::config::{ConsoleSettings, UserConfig, console};

fn save_theme(theme: &Theme, actor: &str) -> Result<()> {
    let accounts = Accounts::load()?;
    theme.save()?;
    accounts.record(actor, EventKind::ThemeSaved)?;
    Ok(())
}

pub fn import_theme(path: &str) -> Result<String> {
    let theme = Theme::parse(&std::fs::read_to_string(path)?)?;
    save_theme(&theme, &cli_actor()?)?;
    Ok(serde_json::to_string_pretty(&theme)?)
}

pub fn export_theme(name: &str) -> Result<String> {
    Ok(serde_json::to_string_pretty(&Theme::export(name)?)?)
}

pub(super) async fn export_document(
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    match tokio::task::spawn_blocking(move || Theme::export(&name)).await {
        Ok(Ok(theme)) => Json(theme).into_response(),
        Ok(Err(error)) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
        error => failed(error),
    }
}

pub(super) async fn import_document(
    Extension(viewer): Extension<Viewer>,
    text: String,
) -> Response {
    let theme = match Theme::parse(&text) {
        Ok(theme) => theme,
        Err(error) => return (StatusCode::BAD_REQUEST, format!("{error:#}")).into_response(),
    };
    match tokio::task::spawn_blocking(move || {
        save_theme(&theme, &viewer.email)?;
        Ok::<_, anyhow::Error>(theme)
    })
    .await
    {
        Ok(Ok(theme)) => Json(theme).into_response(),
        error => failed(error),
    }
}

fn write_theme(name: &str, actor: &str) -> Result<ConsoleSettings> {
    console::validate_theme(name)?;
    let accounts = Accounts::load()?;
    let settings = console::write_look(Some(name), &[], &[])?;
    accounts.record(actor, EventKind::ThemeChanged)?;
    Ok(settings)
}

fn write_sections(hide: &[String], show: &[String], actor: &str) -> Result<ConsoleSettings> {
    console::validate_sections(hide)?;
    console::validate_sections(show)?;
    let accounts = Accounts::load()?;
    let settings = console::write_look(None, hide, show)?;
    accounts.record(actor, EventKind::SectionsChanged)?;
    Ok(settings)
}

fn cli_actor() -> Result<String> {
    let user = std::process::Command::new("id")
        .arg("-un")
        .output()
        .context("could not identify the Unix account changing the Console look")?;
    ensure!(
        user.status.success(),
        "could not identify the Unix account changing the Console look"
    );
    Ok(format!("CLI:{}", String::from_utf8(user.stdout)?.trim()))
}

pub fn theme(name: Option<&str>, json_output: bool) -> Result<String> {
    let Some(name) = name else {
        return Ok(if json_output {
            serde_json::to_string(&themes::names()?)?
        } else {
            themes::names()?.join("\n")
        });
    };
    let settings = write_theme(name, &cli_actor()?)?;
    Ok(if json_output {
        serde_json::to_string(&settings)?
    } else {
        format!("Theme: {}", settings.theme)
    })
}

pub fn sections(hide: &[String], show: &[String], json_output: bool) -> Result<String> {
    let settings = if hide.is_empty() && show.is_empty() {
        UserConfig::load()?.console
    } else {
        write_sections(hide, show, &cli_actor()?)?
    };
    Ok(if json_output {
        serde_json::to_string(&settings)?
    } else {
        format!("Hidden sections: {}", settings.hidden_sections.join(", "))
    })
}

pub(super) async fn get(Extension(viewer): Extension<Viewer>) -> Response {
    match tokio::task::spawn_blocking(|| {
        let config = UserConfig::load()?;
        let revision = super::accounts::digest(&export_theme(&config.console.theme)?);
        Ok::<_, anyhow::Error>((config, themes::names()?, revision))
    })
    .await
    {
        Ok(Ok((config, names, revision))) => Json(json!({
            "theme": config.console.theme,
            "theme_revision": revision,
            "hidden_sections": config.console.hidden_sections,
            "themes": names, "sections": console::SECTIONS,
            "csrf": viewer.csrf,
        }))
        .into_response(),
        error => failed(error),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ThemeForm {
    theme: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SectionsForm {
    #[serde(default)]
    hide: String,
    #[serde(default)]
    show: String,
}

pub(super) async fn set_theme(
    Extension(viewer): Extension<Viewer>,
    Form(form): Form<ThemeForm>,
) -> Response {
    if let Err(error) = console::validate_theme(&form.theme) {
        return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
    }
    written(tokio::task::spawn_blocking(move || write_theme(&form.theme, &viewer.email)).await)
}

pub(super) async fn set_sections(
    Extension(viewer): Extension<Viewer>,
    Form(form): Form<SectionsForm>,
) -> Response {
    let split = |value: &str| {
        value
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let (hide, show) = (split(&form.hide), split(&form.show));
    if let Err(error) =
        console::validate_sections(&hide).and_then(|()| console::validate_sections(&show))
    {
        return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
    }
    written(tokio::task::spawn_blocking(move || write_sections(&hide, &show, &viewer.email)).await)
}

fn written(
    result: std::result::Result<Result<ConsoleSettings>, tokio::task::JoinError>,
) -> Response {
    match result {
        Ok(Ok(settings)) => Json(settings).into_response(),
        error => failed(error),
    }
}

fn failed(error: impl std::fmt::Debug) -> Response {
    eprintln!("thirdshift Console: look operation failed: {error:?}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Can't read or save the Console look.",
    )
        .into_response()
}
