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

type PersonAnswers = BTreeMap<(Option<String>, String), Answer>;

#[derive(Clone)]
pub(super) struct Fleet {
    answers: Arc<Mutex<PersonAnswers>>,
    pages: watch::Sender<BTreeMap<Option<String>, usize>>,
}

struct Answer {
    entry: Entry,
    good: Option<Document>,
    fetched_at: Option<DateTime<Utc>>,
    error: Option<(Reachability, String)>,
    scopes: Vec<super::Scope>,
}

pub(super) struct Lease(
    watch::Sender<BTreeMap<Option<String>, usize>>,
    Option<String>,
);
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.send_modify(|pages| {
            if let Some(count) = pages.get_mut(&self.1) {
                *count -= 1;
                if *count == 0 {
                    pages.remove(&self.1);
                }
            }
        });
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
        let (pages, _) = watch::channel(BTreeMap::new());
        Self {
            answers: Arc::default(),
            pages,
        }
    }

    pub fn subscribe(&self, person: Option<&str>) -> Lease {
        let person = person.map(str::to_owned);
        self.pages
            .send_modify(|pages| *pages.entry(person.clone()).or_default() += 1);
        Lease(self.pages.clone(), person)
    }

    pub fn follow(&self, notifier: super::changes::Notifier) -> tokio::task::JoinHandle<()> {
        let fleet = self.clone();
        let mut pages = self.pages.subscribe();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(timing("THIRDSHIFT_FLEET_INTERVAL_MS", 15));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut jobs = tokio::task::JoinSet::<(
                Option<String>,
                Entry,
                Result<(Document, Vec<super::Scope>), (Reachability, String)>,
            )>::new();
            let mut fetching = BTreeSet::new();
            loop {
                tokio::select! {
                    _ = tick.tick() => {},
                    result = pages.changed() => {
                        if result.is_err() { break; }
                        // Catch up as soon as the first page opens.
                        if pages.borrow_and_update().is_empty() { continue; }
                    },
                    Some(result) = jobs.join_next(), if !jobs.is_empty() => {
                        if let Ok((person, entry, result)) = result {
                            fetching.remove(&(person.clone(), entry.name.clone()));
                            let mut answers = fleet.answers.lock().unwrap();
                            let answer = answers.entry((person, entry.name.clone())).or_insert_with(|| Answer {
                                entry: entry.clone(), good: None, fetched_at: None, error: None, scopes: Vec::new(),
                            });
                            if answer.entry != entry {
                                *answer = Answer { entry, good: None, fetched_at: None, error: None, scopes: Vec::new() };
                            }
                            match result {
                                Ok((document, scopes)) => {
                                    answer.good = Some(document);
                                    answer.scopes = scopes;
                                    answer.fetched_at = Some(Utc::now());
                                    answer.error = None;
                                },
                                Err(error) => { answer.error = Some(error); answer.scopes.clear(); },
                            }
                            notifier.commands();
                        }
                        continue;
                    },
                }
                if pages.borrow().is_empty() {
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
                let people = pages.borrow().keys().cloned().collect::<Vec<_>>();
                fleet
                    .answers
                    .lock()
                    .unwrap()
                    .retain(|(person, name), answer| {
                        people.contains(person)
                            && live
                                .iter()
                                .any(|(entry, _)| entry.name == *name && *entry == answer.entry)
                    });
                for person in people {
                    for (entry, token) in live.iter().cloned() {
                        if !fetching.insert((person.clone(), entry.name.clone())) {
                            continue;
                        }
                        let person = person.clone();
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
                                .and_then(|client| {
                                    let scopes = super::fleet::effective_scopes(
                                        &client,
                                        &entry,
                                        &token,
                                        person.as_deref(),
                                    )?;
                                    let document = status::fleet::fetch_for_person(
                                        &client,
                                        &entry,
                                        &token,
                                        false,
                                        person.as_deref(),
                                    )?;
                                    status::validate_remote(&document).map_err(|_| {
                                        (
                                            Reachability::NeedsUpdate,
                                            "incompatible Inbox presentation".into(),
                                        )
                                    })?;
                                    Ok((document, scopes))
                                });
                            (person, entry, result)
                        });
                    }
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
        person: Option<&str>,
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
                .get(&(person.map(str::to_owned), entry.name.clone()))
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
                inbox.remote_actions(
                    &entry.name,
                    !stale && answer.error.is_none() && answer.scopes.contains(&super::Scope::Act),
                );
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
