//! Retained Bug origins and the CLI's confirm, correct and reject actions.
use anyhow::{Result, bail};
use askama::Template;
use axum::response::Response;
use axum::{Extension, Form};
use serde::Deserialize;

use super::auth::Viewer;
use crate::config::UserConfig;
use crate::scoring::origins;

#[derive(Template)]
#[template(path = "console/bug_origins.html")]
struct BugOriginsPage {
    page: super::PageContext,
    origins: Vec<origins::Proposal>,
    csrf: String,
}

pub(super) async fn show(
    Extension(viewer): Extension<Viewer>,
    Extension(format): Extension<super::api::Format>,
) -> Response {
    super::api::read(format, move || {
        let config = UserConfig::load()?;
        format.render(origins::view()?, |origins| {
            Ok(BugOriginsPage {
                page: super::PageContext::new(&config, &viewer.scopes),
                origins,
                csrf: viewer.csrf,
            }
            .render()?)
        })
    })
    .await
}

#[derive(Deserialize)]
pub(super) struct SettlementForm {
    issue: String,
    action: String,
    #[serde(default)]
    pull_request: String,
}

pub(super) async fn submit(
    Extension(format): Extension<super::api::Format>,
    Form(form): Form<SettlementForm>,
) -> Response {
    super::api::action(
        format,
        "/bug-origins",
        move || -> Result<serde_json::Value> {
            let mut args = vec![form.issue];
            match form.action.as_str() {
                "confirm" => args.push("--confirm".into()),
                "correct" => {
                    args.push("--correct".into());
                    args.push(form.pull_request);
                }
                "reject" => args.push("--none".into()),
                _ => bail!("choose confirm, correct or reject"),
            }
            args.push("--json".into());
            Ok(serde_json::from_str(&origins::apply(origins::parse(
                &args,
            )?)?)?)
        },
    )
    .await
}
