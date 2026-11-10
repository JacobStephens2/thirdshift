//! Explicit Fleet schedule client. Owning-machine APIs always remain local.
use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::Serialize;

use super::{CronJob, CronJobs};
use crate::config::{UserConfig, console::fleet::Entry};
use crate::console::fleet::read::{Refusal, Remote};
use crate::resend_key::Credentials;

#[derive(Serialize)]
pub(crate) struct Inventory {
    pub repositories: Vec<Repository>,
    pub machines: Vec<Machine>,
}

#[derive(Serialize)]
pub(crate) struct Repository {
    pub repository: Option<String>,
    pub machines: Vec<Schedules>,
}

impl Repository {
    pub fn heading(&self) -> &str {
        self.repository.as_deref().unwrap_or("Other")
    }
}

#[derive(Serialize)]
pub(crate) struct Schedules {
    pub name: String,
    pub jobs: Vec<CronJob>,
}

#[derive(Serialize)]
pub(crate) struct Machine {
    pub name: String,
    pub address: Option<String>,
    pub status: Status,
    pub error: Option<String>,
    pub empty: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Status {
    Ok,
    Unavailable,
    Refused,
    Incompatible,
    LinkOnly,
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Ok => "ok",
            Self::Unavailable => "unavailable",
            Self::Refused => "refused",
            Self::Incompatible => "incompatible",
            Self::LinkOnly => "link-only",
        })
    }
}

impl Machine {
    fn capture(mut self, result: Result<CronJobs>) -> (Self, Vec<CronJob>) {
        match result {
            Ok(inventory) => {
                self.empty = inventory.jobs.is_empty();
                (self, inventory.jobs)
            }
            Err(error) => {
                self.status = if let Some(refusal) = error.downcast_ref::<Refusal>() {
                    match refusal.status.as_u16() {
                        401 | 403 => Status::Refused,
                        404 => Status::Incompatible,
                        _ if refusal.message.contains("incompatible") => Status::Incompatible,
                        _ => Status::Unavailable,
                    }
                } else if error.to_string().contains("incompatible") {
                    Status::Incompatible
                } else {
                    Status::Unavailable
                };
                self.error = Some(format!("{error:#}"));
                (self, Vec::new())
            }
        }
    }
}

fn remote(config: &UserConfig, entry: &Entry, person: Option<&str>) -> (Machine, Vec<CronJob>) {
    let mut machine = Machine {
        name: entry.name.clone(),
        address: Some(entry.address.clone()),
        status: Status::Ok,
        error: None,
        empty: false,
    };
    let read = || -> Result<Option<CronJobs>> {
        // A link never sends even an identity probe. The transport revalidates
        // paired destinations before attaching any saved token.
        if Credentials::read()?.fleet_token(&entry.name).is_none() {
            return Ok(None);
        }
        let remote = Remote::select(config, &entry.name, person.map(str::to_owned))?
            .context("expected a Fleet machine")?;
        let inventory = serde_json::from_value(remote.get("/cronjobs")?)
            .context("incompatible cron inventory")?;
        Ok(Some(inventory))
    };
    match read() {
        Ok(None) => {
            machine.status = Status::LinkOnly;
            (machine, Vec::new())
        }
        Ok(Some(inventory)) => machine.capture(Ok(inventory)),
        Err(error) => machine.capture(Err(error)),
    }
}

pub(crate) fn load(config: &UserConfig, person: Option<&str>) -> Inventory {
    let local = Machine {
        name: crate::console::fleet_control::local_name(config),
        address: None,
        status: Status::Ok,
        error: None,
        empty: false,
    }
    .capture(super::load());
    let mut snapshots = vec![local];
    // Independent requests share no cache and each uses the Fleet transport's
    // three-second bound and no-redirect policy.
    snapshots.extend(std::thread::scope(|threads| {
        let handles: Vec<_> = config
            .console
            .fleet
            .iter()
            .map(|entry| threads.spawn(move || remote(config, entry, person)))
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("Fleet schedule reader panicked"))
            .collect::<Vec<_>>()
    }));
    let mut grouped: BTreeMap<(bool, Option<String>), BTreeMap<String, Vec<CronJob>>> =
        BTreeMap::new();
    let mut machines = Vec::new();
    for (machine, jobs) in snapshots {
        for job in jobs {
            grouped
                .entry((job.repository.is_none(), job.repository.clone()))
                .or_default()
                .entry(machine.name.clone())
                .or_default()
                .push(job);
        }
        machines.push(machine);
    }
    Inventory {
        repositories: grouped
            .into_iter()
            .map(|((_, repository), machines)| Repository {
                repository,
                machines: machines
                    .into_iter()
                    .map(|(name, jobs)| Schedules { name, jobs })
                    .collect(),
            })
            .collect(),
        machines,
    }
}

pub(crate) fn show(json: bool) -> Result<String> {
    let inventory = load(&UserConfig::load()?, None);
    if json {
        return Ok(serde_json::to_string_pretty(&inventory)?);
    }
    let mut lines = Vec::new();
    for machine in &inventory.machines {
        let mut line = format!("{}: {}", machine.name, machine.status);
        if let Some(address) = &machine.address {
            line.push_str(&format!(" ({address})"));
        }
        if let Some(error) = &machine.error {
            line.push_str(&format!(" — {error}"));
        } else if machine.empty {
            line.push_str(" — No thirdshift cron jobs in this account's crontab.");
        }
        lines.push(line);
    }
    for repository in &inventory.repositories {
        lines.push(repository.heading().into());
        for machine in &repository.machines {
            lines.push(format!("  {}", machine.name));
            for job in &machine.jobs {
                let kind = job
                    .command_kind
                    .map(|kind| kind.to_string())
                    .unwrap_or_else(|| "Unknown Command kind".into());
                lines.push(format!(
                    "    {kind}: {}  {}",
                    job.schedule,
                    job.command.replace(['\r', '\n'], " ")
                ));
            }
        }
    }
    Ok(lines.join("\n"))
}
