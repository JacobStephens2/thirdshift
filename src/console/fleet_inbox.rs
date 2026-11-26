//! Subscription-owned Fleet reads. Answers and errors live only in this process.
mod reads;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::watch;

use crate::config::{UserConfig, console::fleet::Entry};
use crate::logs::status::{self, fleet::Reachability};

use reads::{InboxReads, Read, ReadResult};

#[derive(Clone)]
pub(super) struct Fleet {
    reads: Arc<InboxReads>,
    pages: watch::Sender<BTreeMap<Option<String>, usize>>,
    notifier: super::changes::Notifier,
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

fn read(entry: &Entry, token: &str, person: Option<&str>) -> ReadResult {
    let client = reqwest::blocking::Client::builder()
        .timeout(timing("THIRDSHIFT_FLEET_READ_TIMEOUT_MS", 3))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| {
            (
                Reachability::Unreachable,
                "Can't create Inbox client".into(),
            )
        })?;
    let scopes = super::fleet::effective_scopes(&client, entry, token, person)?;
    let document = status::fleet::fetch_for_person(&client, entry, token, false, person)?;
    status::validate_remote(&document).map_err(|_| {
        (
            Reachability::NeedsUpdate,
            "incompatible Inbox presentation".into(),
        )
    })?;
    Ok((document, scopes))
}

impl Fleet {
    pub fn new(notifier: super::changes::Notifier) -> Self {
        let (pages, _) = watch::channel(BTreeMap::new());
        Self {
            reads: Arc::default(),
            pages,
            notifier,
        }
    }

    pub fn subscribe(&self, person: Option<&str>) -> Lease {
        let person = person.map(str::to_owned);
        self.pages
            .send_modify(|pages| *pages.entry(person.clone()).or_default() += 1);
        Lease(self.pages.clone(), person)
    }

    fn complete(&self, request: Read, result: ReadResult) {
        if self.reads.complete(request, result, Utc::now()) {
            self.notifier.commands();
        }
    }

    fn reconcile(&self, request: Read, token: &str) {
        let result = read(request.entry(), token, request.person());
        self.complete(request, result);
    }

    /// Re-read the owning Inbox as each viewer after a successful Dismiss or Undo.
    /// The acting viewer's read completes before the action response is sent.
    pub fn dismissal_saved(
        &self,
        destination: &super::fleet::PairedMachine,
        person: Option<&str>,
        command: &str,
    ) {
        let reads = {
            let pages = self.pages.borrow();
            let people = pages.keys().cloned().collect();
            self.reads
                .dismissal_saved(&destination.entry, person, &people, command)
        };
        self.notifier.commands();
        let mut caller = None;
        for request in reads {
            if request.person() == person {
                caller = Some(request);
            } else {
                let fleet = self.clone();
                let token = destination.token.clone();
                drop(tokio::task::spawn_blocking(move || {
                    fleet.reconcile(request, &token)
                }));
            }
        }
        if let Some(request) = caller {
            self.reconcile(request, &destination.token);
        }
    }

    pub fn follow(&self) -> tokio::task::JoinHandle<()> {
        let fleet = self.clone();
        let mut pages = self.pages.subscribe();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(timing("THIRDSHIFT_FLEET_INTERVAL_MS", 15));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut jobs = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _ = tick.tick() => {},
                    result = pages.changed() => {
                        if result.is_err() { break; }
                        // Catch up as soon as the first page opens.
                        if pages.borrow_and_update().is_empty() { continue; }
                    },
                    Some(_) = jobs.join_next(), if !jobs.is_empty() => continue,
                }
                let had_entries = !fleet.reads.is_empty();
                if pages.borrow().is_empty() && !had_entries {
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
                            Some((entry.name.clone(), (entry, token)))
                        })
                        .collect::<BTreeMap<_, _>>())
                })
                .await;
                let Ok(Ok(live)) = live else {
                    continue;
                };
                let requests = {
                    let pages = pages.borrow();
                    let people = pages.keys().cloned().collect::<BTreeSet<_>>();
                    let entries = live.values().map(|(entry, _)| entry).collect::<Vec<_>>();
                    fleet.reads.sweep(&people, &entries)
                };
                // Ages and the unreachable threshold advance even without good answers.
                if had_entries || !requests.is_empty() {
                    fleet.notifier.commands();
                }
                for request in requests {
                    let fleet = fleet.clone();
                    let entry = request.entry().clone();
                    let person = request.person().map(str::to_owned);
                    let token = live[&entry.name].1.clone();
                    jobs.spawn(async move {
                        let result = tokio::task::spawn_blocking(move || {
                            read(&entry, &token, person.as_deref())
                        })
                        .await
                        .unwrap_or_else(|_| {
                            Err((Reachability::Unreachable, "Inbox read task failed".into()))
                        });
                        fleet.complete(request, result);
                    });
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
        for entry in &config.console.fleet {
            if !selected.is_empty() && selected != entry.name {
                continue;
            }
            let Some(answer) = self.reads.answer(person, entry) else {
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
                let unavailable = stale || answer.error.is_some();
                inbox.merge_remote(good, entry, stale, answer.error.is_some())?;
                inbox.remote_actions(
                    &entry.name,
                    !unavailable && answer.scopes.contains(&super::Scope::Act),
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
