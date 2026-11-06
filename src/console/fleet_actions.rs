//! Owning-machine transport shared by the CLI and signed-in Console actions.
use std::time::Duration;

use anyhow::{Context, Result, bail};
use askama::Template;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use serde_json::Value;

use crate::{config::UserConfig, resend_key::Credentials};

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

fn destination(machine: &str) -> Result<(String, String)> {
    let config = UserConfig::load()?;
    let credentials = Credentials::read()?;
    let entry = config
        .console
        .fleet
        .iter()
        .find(|entry| entry.name == machine)
        .context("unknown Fleet machine")?;
    // Recheck edited config before transmitting the saved bearer token.
    super::fleet::pairing_address(&entry.address)?;
    let token = credentials
        .fleet_token(machine)
        .context("link-only Fleet machine")?;
    Ok((entry.address.trim_end_matches('/').into(), token.into()))
}

pub(super) fn request(
    machine: &str,
    person: Option<&str>,
    command: &str,
    action: Action,
) -> Result<Answer> {
    let (address, token) = destination(machine)?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut request = client
        .post(format!("{address}/api/v1{}", action.endpoint()))
        .bearer_auth(token);
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
    let answer = request(machine, None, command, action)?;
    if !answer.status.is_success() {
        bail!(
            "Fleet machine {machine} returned HTTP {}: {}",
            answer.status,
            answer.value
        );
    }
    if json {
        println!("{}", answer.value);
    } else {
        let name = answer.value["name"].as_str().unwrap_or(command);
        match action {
            Action::Stop { .. } => {
                println!(
                    "{name}: {}",
                    answer.value["outcome"].as_str().unwrap_or("unknown")
                );
                for (key, label) in [
                    ("issue_branches", "Issue branch left"),
                    ("pull_requests", "Pull request left"),
                ] {
                    for value in answer.value[key].as_array().into_iter().flatten() {
                        if let Some(value) = value.as_str() {
                            println!("{label}: {value}");
                        }
                    }
                }
            }
            Action::Dismiss { undo: true } => println!("took back the dismissal of {name}"),
            Action::Dismiss { undo: false } => println!(
                "dismissed {name} from {}",
                answer.value["group"].as_str().unwrap_or("Inbox")
            ),
        }
    }
    Ok(())
}
