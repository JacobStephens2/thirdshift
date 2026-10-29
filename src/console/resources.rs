//! A read-only host snapshot, using the same admission policy as sessions.

use anyhow::Result;
use askama::Template;
use axum::response::Response;

use crate::config::UserConfig;
use crate::resources::{self, Snapshot};

/// The compact projection of the session admission policy shared by all headers.
pub(super) struct Status {
    pub(super) state: &'static str,
    pub(super) label: &'static str,
    pub(super) detail: String,
}

impl Status {
    pub(super) fn new(config: &UserConfig) -> Self {
        if config.resources == resources::Limits::default() {
            return Self {
                state: "off",
                label: "Host limits off",
                detail: "Memory and load limits are disabled; capacity is not checked.".into(),
            };
        }
        match resources::observe(config.resources) {
            Ok(snapshot) => Self {
                state: if snapshot.ready { "ready" } else { "busy" },
                label: if snapshot.ready {
                    "Host ready for another session"
                } else {
                    "Host busy · new sessions wait"
                },
                detail: format!(
                    "{} MB available · {:.1}% load per CPU. {}",
                    snapshot.host.available_memory_mb,
                    snapshot.load_per_cpu_percent,
                    if snapshot.ready {
                        "Within configured limits; capacity is not reserved.".into()
                    } else {
                        snapshot.reasons.join("; ")
                    }
                ),
            },
            Err(error) => Self {
                state: "unknown",
                label: "Host resources unavailable",
                detail: error.to_string(),
            },
        }
    }
}

#[derive(Template)]
#[template(path = "console/host-resources.html")]
struct StatusFragment {
    status: Status,
}

pub(super) async fn status() -> Response {
    super::page(
        tokio::task::spawn_blocking(|| -> Result<String> {
            Ok(StatusFragment {
                status: Status::new(&UserConfig::load()?),
            }
            .render()?)
        })
        .await,
    )
}

#[derive(Template)]
#[template(path = "console/resources.html")]
struct ResourcesPage {
    page: super::PageContext,
    snapshot: Snapshot,
}

pub(super) async fn show() -> Response {
    super::page(
        tokio::task::spawn_blocking(|| -> Result<String> {
            let config = UserConfig::load()?;
            Ok(ResourcesPage {
                page: super::PageContext::new(&config),
                snapshot: resources::observe(config.resources)?,
            }
            .render()?)
        })
        .await,
    )
}
