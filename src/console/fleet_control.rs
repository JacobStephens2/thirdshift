//! Fleet run/admin selection. Only page handlers forward a signed-in person;
//! API endpoints remain local and CLI requests use the token holder.
use anyhow::{Context, Result, bail, ensure};
use axum::{
    Extension, Json,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};

use super::{Scope, api, auth::Viewer, fleet::read::Remote};
use crate::{args, config::UserConfig};

#[derive(Default)]
pub(super) struct Selection {
    pub machine: String,
}

impl Selection {
    pub fn query(query: &Option<String>) -> Self {
        Self {
            machine: super::query_value(query, "machine"),
        }
    }
    pub fn remote(
        &self,
        config: &UserConfig,
        viewer: &Viewer,
        format: api::Format,
    ) -> Result<Option<Remote>> {
        if local_machine(config, &self.machine) {
            return Ok(None);
        }
        ensure!(
            format == api::Format::Html,
            "Call the owning machine's API directly"
        );
        Remote::select(
            config,
            &self.machine,
            viewer.fleet_person().map(str::to_owned),
        )
    }

    pub fn suffix(&self) -> String {
        if self.machine.is_empty() {
            String::new()
        } else {
            format!(
                "?{}",
                serde_urlencoded::to_string([("machine", &self.machine)]).unwrap()
            )
        }
    }
}

fn local_machine(config: &UserConfig, name: &str) -> bool {
    name.is_empty()
        || (name == super::hostname()
            && !config.console.fleet.iter().any(|entry| entry.name == name))
}

pub(crate) fn local_name(config: &UserConfig) -> String {
    let hostname = super::hostname();
    if config
        .console
        .fleet
        .iter()
        .any(|entry| entry.name == hostname)
    {
        format!("{hostname} (local)")
    } else {
        hostname
    }
}

pub(super) struct Target {
    pub machine: String,
    pub name: String,
    pub suffix: String,
    pub run: bool,
    pub admin: bool,
}

pub(super) fn targets(config: &UserConfig, viewer: &Viewer, scope: Scope) -> Vec<Target> {
    choices(config, viewer, &[scope])
}

pub(super) fn controls(config: &UserConfig, viewer: &Viewer) -> Vec<Target> {
    choices(config, viewer, &[Scope::Run, Scope::Admin])
}

fn choices(config: &UserConfig, viewer: &Viewer, required: &[Scope]) -> Vec<Target> {
    let mut targets = Vec::new();
    if required.iter().any(|scope| viewer.scopes.contains(scope)) {
        targets.push(Target {
            machine: String::new(),
            name: local_name(config),
            suffix: String::new(),
            run: viewer.scopes.contains(&Scope::Run),
            admin: viewer.scopes.contains(&Scope::Admin),
        });
    }
    // Each destination is independent: a refused or unavailable machine cannot
    // hide choices on another one. No pairing-time scopes authorize controls.
    let remote = std::thread::scope(|threads| {
        let handles: Vec<_> = config
            .console
            .fleet
            .iter()
            .map(|entry| {
                threads.spawn(move || {
                    let remote = Remote::select(
                        config,
                        &entry.name,
                        viewer.fleet_person().map(str::to_owned),
                    )
                    .ok()??;
                    let scopes = remote.identity().ok()?.scopes;
                    required
                        .iter()
                        .any(|scope| scopes.contains(scope))
                        .then(|| Target {
                            machine: entry.name.clone(),
                            name: entry.name.clone(),
                            suffix: Selection {
                                machine: entry.name.clone(),
                            }
                            .suffix(),
                            run: scopes.contains(&Scope::Run),
                            admin: scopes.contains(&Scope::Admin),
                        })
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|handle| handle.join().ok().flatten())
            .collect::<Vec<_>>()
    });
    targets.extend(remote);
    targets
}

pub(super) async fn submit(
    Extension(viewer): Extension<Viewer>,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
    Json(request): Json<Value>,
) -> Response {
    api::read(api::Format::Json, move || {
        let selection = Selection::query(&query);
        let config = UserConfig::load()?;
        let remote = selection
            .remote(&config, &viewer, api::Format::Html)?
            .context("select a Fleet machine")?;
        Ok(Json(remote.post_json("/runs", &request)?).into_response())
    })
    .await
}

pub(crate) struct Cli {
    machine: Option<String>,
    operation: Operation,
}

enum Operation {
    Options,
    Start(Value),
    Config {
        key: Option<String>,
        value: Option<String>,
        json: bool,
    },
    Update {
        check: bool,
        json: bool,
    },
}

pub(super) fn config_route(key: &str) -> Result<String> {
    ensure!(
        key.split('.').all(|part| !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')),
        "invalid User config key: {key}"
    );
    Ok(format!("/config/{key}"))
}

impl Cli {
    pub fn parse(arguments: &[String]) -> Result<Self> {
        let mut words = Vec::new();
        let mut machine = None;
        let mut iter = arguments.iter();
        while let Some(word) = iter.next() {
            if word == "--machine" {
                ensure!(machine.is_none(), "--machine may be given only once");
                machine = Some(
                    iter.next()
                        .filter(|name| !name.starts_with('-'))
                        .context("--machine needs a Fleet name")?
                        .clone(),
                );
            } else {
                words.push(word.clone());
            }
        }
        if words.first().is_some_and(|word| word == "start") {
            if words[1..]
                .iter()
                .all(|word| matches!(word.as_str(), "--options" | "--json"))
                && words.iter().any(|word| word == "--options")
            {
                return Ok(Self {
                    machine,
                    operation: Operation::Options,
                });
            }
            ensure!(
                words.len() >= 3,
                "usage: thirdshift start <owner/repo> <issue> [options] [--machine <name>] | start --options --json"
            );
            let repository = &words[1];
            let given: Vec<_> = words[2..]
                .iter()
                .filter(|word| word.as_str() != "--json")
                .cloned()
                .collect();
            let args::Command::Run(run) = args::parse(&given)? else {
                bail!("start needs an issue and shared options")
            };
            let issue = match run.issue {
                crate::issue::IssueInput::Url(url) => json!(url.url),
                crate::issue::IssueInput::Number(number) => json!(number.get()),
            };
            let mut request = json!({"repository":repository,"issue":issue});
            super::start_controls::add_flags(&mut request, &run.flags);
            let parsed: super::start::Request = serde_json::from_value(request.clone())?;
            parsed.arguments()?;
            return Ok(Self {
                machine,
                operation: Operation::Start(request),
            });
        }
        let command = args::parse(&words)?;
        let operation = if let args::Command::Config { key, value, json } = command {
            Operation::Config { key, value, json }
        } else if let args::Command::Update { check, json } = command {
            Operation::Update { check, json }
        } else {
            bail!("--machine supports start, config and update")
        };
        Ok(Self { machine, operation })
    }

    pub fn run(self) -> Result<String> {
        let config = UserConfig::load()?;
        let machine = self.machine.filter(|name| !local_machine(&config, name));
        let remote = machine
            .as_deref()
            .map(|name| Remote::select(&config, name, None)?.context("select a Fleet machine"))
            .transpose()?;
        match self.operation {
            Operation::Options => {
                let value = match remote {
                    Some(remote) => remote.get("/start-options")?,
                    None => serde_json::to_value(super::start_controls::Options::load(&config)?)?,
                };
                Ok(serde_json::to_string_pretty(&value)?)
            }
            Operation::Start(request) => {
                let value = match remote {
                    Some(remote) => remote.post_json("/runs", &request)?,
                    None => super::start::cli(request)?,
                };
                Ok(serde_json::to_string_pretty(&value)?)
            }
            Operation::Config { key, value, json } => {
                let Some(remote) = remote else {
                    return super::user_config::command(key.as_deref(), value.as_deref(), json);
                };
                let route = key
                    .as_deref()
                    .map(config_route)
                    .transpose()?
                    .unwrap_or("/config".into());
                let value = if let Some(value) = value {
                    remote.post_form(&route, &[("value", value.as_str())])?
                } else {
                    remote.get(&route)?
                };
                Ok(serde_json::to_string_pretty(&value)?)
            }
            Operation::Update { check, json } => match remote {
                Some(remote) => Ok(serde_json::to_string_pretty(&if check {
                    remote.get("/update")?
                } else {
                    remote.post_form("/update", &[])?
                })?),
                None => crate::update::command(check, json),
            },
        }
    }
}
