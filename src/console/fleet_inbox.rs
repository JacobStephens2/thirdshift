//! Subscription-owned Fleet reads. Answers and errors live only in this process.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::watch;

use crate::config::{UserConfig, console::fleet::Entry};
use crate::logs::status::{
    self,
    fleet::{Document, Reachability},
};

#[derive(Clone)]
pub(super) struct Fleet {
    answers: Arc<Mutex<BTreeMap<String, Answer>>>,
    pages: watch::Sender<usize>,
}

struct Answer {
    entry: Entry,
    good: Option<Document>,
    fetched_at: Option<DateTime<Utc>>,
    error: Option<(Reachability, String)>,
}

pub(super) struct Lease(watch::Sender<usize>);
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.send_modify(|pages| *pages -= 1);
    }
}

pub(super) struct Machine {
    pub name: String,
    pub address: String,
    pub age: String,
    pub reachability: &'static str,
    pub active_runs: usize,
    pub error: String,
}

fn timing(variable: &str, seconds: u64) -> Duration {
    std::env::var(variable)
        .ok()
        .filter(|_| cfg!(debug_assertions))
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_secs(seconds))
}

impl Fleet {
    pub fn new() -> Self {
        let (pages, _) = watch::channel(0);
        Self {
            answers: Arc::default(),
            pages,
        }
    }

    pub fn subscribe(&self) -> Lease {
        self.pages.send_modify(|pages| *pages += 1);
        Lease(self.pages.clone())
    }

    pub fn follow(&self, notifier: super::changes::Notifier) -> tokio::task::JoinHandle<()> {
        let fleet = self.clone();
        let mut pages = self.pages.subscribe();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(timing("THIRDSHIFT_FLEET_INTERVAL_MS", 15));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut jobs =
                tokio::task::JoinSet::<(Entry, Result<Document, (Reachability, String)>)>::new();
            let mut fetching = BTreeSet::new();
            loop {
                tokio::select! {
                    _ = tick.tick() => {},
                    result = pages.changed() => {
                        if result.is_err() { break; }
                        // Catch up as soon as the first page opens.
                        if *pages.borrow_and_update() != 1 { continue; }
                    },
                    Some(result) = jobs.join_next(), if !jobs.is_empty() => {
                        if let Ok((entry, result)) = result {
                            fetching.remove(&entry.name);
                            let mut answers = fleet.answers.lock().unwrap();
                            let answer = answers.entry(entry.name.clone()).or_insert_with(|| Answer {
                                entry: entry.clone(), good: None, fetched_at: None, error: None,
                            });
                            if answer.entry != entry {
                                *answer = Answer { entry, good: None, fetched_at: None, error: None };
                            }
                            match result {
                                Ok(document) => {
                                    answer.good = Some(document);
                                    answer.fetched_at = Some(Utc::now());
                                    answer.error = None;
                                },
                                Err(error) => answer.error = Some(error),
                            }
                            notifier.commands();
                        }
                        continue;
                    },
                }
                if *pages.borrow() == 0 {
                    continue;
                }
                let live = tokio::task::spawn_blocking(|| -> anyhow::Result<_> {
                    let config = UserConfig::load()?;
                    let credentials = crate::resend_key::Credentials::read()?;
                    Ok(config
                        .console
                        .fleet
                        .into_iter()
                        .filter_map(|entry| {
                            let token = credentials.fleet_token(&entry.name)?.to_owned();
                            Some((entry, token))
                        })
                        .collect::<Vec<_>>())
                })
                .await;
                let Ok(Ok(live)) = live else {
                    continue;
                };
                fleet.answers.lock().unwrap().retain(|name, answer| {
                    live.iter()
                        .any(|(entry, _)| entry.name == *name && *entry == answer.entry)
                });
                for (entry, token) in live {
                    if !fetching.insert(entry.name.clone()) {
                        continue;
                    }
                    jobs.spawn_blocking(move || {
                        let result = reqwest::blocking::Client::builder()
                            .timeout(Duration::from_secs(3))
                            .redirect(reqwest::redirect::Policy::none())
                            .build()
                            .map_err(|_| {
                                (
                                    Reachability::Unreachable,
                                    "Can't create Inbox client".into(),
                                )
                            })
                            .and_then(|client| status::fleet::fetch(&client, &entry, &token, false))
                            .and_then(|document| {
                                status::validate_remote(&document).map_err(|_| {
                                    (
                                        Reachability::NeedsUpdate,
                                        "incompatible Inbox presentation".into(),
                                    )
                                })?;
                                Ok(document)
                            });
                        (entry, result)
                    });
                }
                // Ages and the unreachable threshold advance even without good answers.
                if !fleet.answers.lock().unwrap().is_empty() {
                    notifier.commands();
                }
            }
        })
    }

    pub fn merge(
        &self,
        config: &UserConfig,
        inbox: &mut status::Inbox,
        selected: &str,
    ) -> anyhow::Result<Vec<Machine>> {
        let hostname = crate::host::name().unwrap_or_else(|| "unknown".into());
        let local_count = inbox.active_runs;
        for section in &mut inbox.sections {
            if !selected.is_empty() && selected != hostname {
                section.rows.clear();
                section.references.clear();
            } else {
                for row in &mut section.rows {
                    row.tag_machine(&hostname, "", false);
                }
            }
        }
        let mut machines = Vec::new();
        if selected.is_empty() || selected == hostname {
            machines.push(Machine {
                name: hostname,
                address: "/".into(),
                age: "live".into(),
                reachability: "ok",
                active_runs: local_count,
                error: String::new(),
            });
        } else {
            inbox.active_runs = 0;
            inbox.idle_passes.clear();
            inbox.pass_starts.clear();
        }
        let answers = self.answers.lock().unwrap();
        for entry in &config.console.fleet {
            if !selected.is_empty() && selected != entry.name {
                continue;
            }
            let Some(answer) = answers
                .get(&entry.name)
                .filter(|answer| answer.entry == *entry)
            else {
                continue;
            };
            let age = answer
                .fetched_at
                .map(|time| (Utc::now() - time).to_std().unwrap_or_default());
            let stale = age.is_none_or(|age| age >= timing("THIRDSHIFT_FLEET_STALE_MS", 60));
            let reachability = match answer.error.as_ref().map(|(state, _)| state) {
                Some(Reachability::Refused) => "refused",
                Some(Reachability::NeedsUpdate) => "needs update",
                _ if stale => "unreachable",
                _ => "ok",
            };
            let count = answer.good.as_ref().map_or(0, |good| good.active_runs);
            if let Some(good) = &answer.good {
                inbox.merge_remote(good, entry, stale)?;
            }
            inbox.active_runs += count;
            machines.push(Machine {
                name: entry.name.clone(),
                address: entry.address.clone(),
                age: age.map_or_else(
                    || "no good answer".into(),
                    |age| format!("{}s ago", age.as_secs()),
                ),
                reachability,
                active_runs: count,
                error: answer
                    .error
                    .as_ref()
                    .map(|(_, error)| error.clone())
                    .unwrap_or_default(),
            });
        }
        Ok(machines)
    }
}
