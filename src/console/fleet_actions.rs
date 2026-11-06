//! Owning-machine transport shared by the CLI and signed-in Console actions.
use std::time::Duration;

use anyhow::{Result, bail};
use askama::Template;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use serde_json::Value;

use crate::config::UserConfig;

#[derive(Clone, Copy)]
pub(crate) enum Action {
    Stop { requeue: bool },
    Dismiss { undo: bool },
}

impl Action {
    fn endpoint(&self) -> &'static str {
        match self {
            Self::Stop { .. } => "/stop",
            Self::Dismiss { undo: false } => "/dismiss",
            Self::Dismiss { undo: true } => "/dismiss/undo",
        }
    }
}

pub(super) struct Answer {
    status: StatusCode,
    value: Value,
}

pub(super) fn request(
    machine: &str,
    person: Option<&str>,
    command: &str,
    action: Action,
) -> Result<Answer> {
    let config = UserConfig::load()?;
    let destination = super::fleet::PairedMachine::load(&config, machine)?;
    let address = destination.entry.address.trim_end_matches('/');
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut request = client
        .post(format!("{address}/api/v1{}", action.endpoint()))
        .bearer_auth(destination.token);
    if let Some(person) = person {
        request = request.header(super::auth::PERSON_HEADER, person);
    }
    let requeue = matches!(action, Action::Stop { requeue: true });
    let response = request
        .form(&[
            ("command", command),
            ("requeue", if requeue { "true" } else { "false" }),
        ])
        .send()
        .map_err(|_| anyhow::anyhow!("Fleet action request failed or timed out"))?;
    let status = response.status();
    let value = response.json().map_err(|_| {
        anyhow::anyhow!("Fleet action returned an invalid JSON answer (HTTP {status})")
    })?;
    Ok(Answer { status, value })
}

#[derive(Template)]
#[template(path = "console/fleet_action.html")]
pub(super) struct Page {
    pub page: super::PageContext,
    pub csrf: String,
    pub machine: String,
    pub command: String,
    pub confirm: bool,
    pub undo: bool,
    pub answer: String,
}

pub(super) async fn submit(
    viewer: super::auth::Viewer,
    machine: String,
    command: String,
    action: Action,
) -> Response {
    let person = viewer.fleet_person().map(str::to_owned);
    let undo = matches!(action, Action::Dismiss { undo: false });
    let result = tokio::task::spawn_blocking(move || -> Result<_> {
        let answer = request(&machine, person.as_deref(), &command, action)?;
        let config = UserConfig::load()?;
        let html = Page {
            page: super::PageContext::new(&config, &viewer.scopes),
            csrf: viewer.csrf,
            machine,
            command,
            confirm: false,
            undo: undo && answer.status.is_success(),
            answer: serde_json::to_string_pretty(&answer.value)?,
        }
        .render()?;
        Ok((answer.status, html))
    })
    .await;
    match result {
        Ok(Ok((status, html))) => (status, Html(html)).into_response(),
        Ok(Err(error)) => (StatusCode::BAD_GATEWAY, error.to_string()).into_response(),
        Err(error) => super::page(Err(error)),
    }
}

pub(crate) fn cli(machine: &str, command: &str, action: Action, json: bool) -> Result<()> {
    if crate::host::name().is_some_and(|hostname| hostname == machine) {
        return match action {
            Action::Stop { requeue } => crate::logs::stop::stop(command, requeue, json),
            Action::Dismiss { undo } => crate::logs::dismiss::dismiss(command, undo, json),
        };
    }
    let answer = request(machine, None, command, action)?;
    if !answer.status.is_success() {
        bail!(
            "Fleet machine {machine} returned HTTP {}: {}",
            answer.status,
            answer.value
        );
    }
    match action {
        Action::Stop { .. } => crate::logs::stop::print_remote(answer.value, json),
        Action::Dismiss { .. } => crate::logs::dismiss::print_remote(answer.value, json),
    }
}
