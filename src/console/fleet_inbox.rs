//! Subscription-owned Fleet reads. Answers and errors live only in this process.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use tokio::sync::watch;

use crate::config::{UserConfig, console::fleet::Entry};
use crate::logs::status::{
    self,
    fleet::{Document, Reachability},
};

type AnswerKey = (Option<String>, String);
type PersonAnswers = BTreeMap<AnswerKey, Answer>;
type ReadResult = Result<(Document, Vec<super::Scope>), (Reachability, String)>;

#[derive(Clone)]
pub(super) struct Fleet {
    answers: Arc<Mutex<PersonAnswers>>,
    pages: watch::Sender<BTreeMap<Option<String>, usize>>,
    notifier: super::changes::Notifier,
}

struct Answer {
    entry: Entry,
    requested_at: Instant,
    good: Option<Document>,
    fetched_at: Option<DateTime<Utc>>,
    error: Option<(Reachability, String)>,
    scopes: Vec<super::Scope>,
}

impl Answer {
    fn new(entry: Entry, requested_at: Instant) -> Self {
        Self {
            entry,
            requested_at,
            good: None,
            fetched_at: None,
            error: None,
            scopes: Vec::new(),
        }
    }
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
        .timeout(Duration::from_secs(3))
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
            answers: Arc::default(),
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

    fn keep(&self, key: AnswerKey, entry: Entry, requested_at: Instant, result: ReadResult) {
        let mut answers = self.answers.lock().unwrap();
        let answer = answers
            .entry(key)
            .or_insert_with(|| Answer::new(entry.clone(), requested_at));
        if requested_at < answer.requested_at {
            return;
        }
        if answer.entry != entry {
            *answer = Answer::new(entry, requested_at);
        }
        answer.requested_at = requested_at;
        match result {
            Ok((document, scopes)) => {
                answer.good = Some(document);
                answer.scopes = scopes;
                answer.fetched_at = Some(Utc::now());
                answer.error = None;
            }
            Err(error) => {
                answer.error = Some(error);
                answer.scopes.clear();
            }
        }
        self.notifier.commands();
    }

    /// Re-read the owning Inbox after a successful Dismiss or Undo. Until it
    /// answers, the cached row for that Command is known to be out of date.
    pub fn dismissal_saved(
        &self,
        destination: &super::fleet::PairedMachine,
        person: Option<&str>,
        command: &str,
    ) {
        let entry = &destination.entry;
        let key = (person.map(str::to_owned), entry.name.clone());
        let requested_at = {
            let mut answers = self.answers.lock().unwrap();
            let requested_at = Instant::now();
            let answer = answers
                .entry(key.clone())
                .or_insert_with(|| Answer::new(entry.clone(), requested_at));
            if answer.entry != *entry {
                return;
            }
            // A poll that began before the save can no longer replace this answer.
            answer.requested_at = requested_at;
            if let Some(document) = &mut answer.good {
                document.omit_command(command);
            }
            requested_at
        };
        let result = read(entry, &destination.token, person);
        self.keep(key, entry.clone(), requested_at, result);
    }

    pub fn follow(&self) -> tokio::task::JoinHandle<()> {
        let fleet = self.clone();
        let mut pages = self.pages.subscribe();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(timing("THIRDSHIFT_FLEET_INTERVAL_MS", 15));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut jobs = tokio::task::JoinSet::<(AnswerKey, Entry, Instant, ReadResult)>::new();
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
                        if let Ok((key, entry, requested_at, result)) = result {
                            fetching.remove(&key);
                            fleet.keep(key, entry, requested_at, result);
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
                        let key = (person.clone(), entry.name.clone());
                        if !fetching.insert(key.clone()) {
                            continue;
                        }
                        let requested_at = Instant::now();
                        jobs.spawn_blocking(move || {
                            let result = read(&entry, &token, key.0.as_deref());
                            (key, entry, requested_at, result)
                        });
                    }
                }
                // Ages and the unreachable threshold advance even without good answers.
                if !fleet.answers.lock().unwrap().is_empty() {
                    fleet.notifier.commands();
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
