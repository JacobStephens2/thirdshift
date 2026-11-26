//! In-process read ordering for the watching Console's Fleet Inbox.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

use crate::config::console::fleet::Entry;
use crate::console::Scope;
use crate::logs::status::fleet::{Document, Reachability};

type Person = Option<String>;
pub(super) type ReadResult = Result<(Document, Vec<Scope>), (Reachability, String)>;

#[derive(Default)]
pub(super) struct InboxReads(Mutex<State>);

/// Only an admitted read can complete, even if the exact entry is added again.
pub(super) struct Read {
    person: Person,
    entry: Entry,
    order: u64,
}

impl Read {
    pub fn person(&self) -> Option<&str> {
        self.person.as_deref()
    }

    pub fn entry(&self) -> &Entry {
        &self.entry
    }
}

#[derive(Clone, Default)]
pub(super) struct Answer {
    pub good: Option<Arc<Document>>,
    pub fetched_at: Option<DateTime<Utc>>,
    pub error: Option<(Reachability, String)>,
    pub scopes: Vec<Scope>,
}

#[derive(Default)]
struct State {
    last_order: u64,
    viewers: BTreeMap<Person, Vec<Pairing>>,
}

struct Pairing {
    entry: Entry,
    cutoff: u64,
    pending: BTreeMap<u64, Source>,
    answer: Option<Answer>,
}

#[derive(PartialEq, Eq)]
enum Source {
    Poll,
    SavedDismissal,
}

enum Reason<'a> {
    Poll,
    SavedDismissal(&'a str),
}

impl State {
    fn admit(&mut self, person: &Person, entry: &Entry, reason: Reason<'_>) -> Option<Read> {
        let pairings = self.viewers.entry(person.clone()).or_default();
        let index = pairings.iter().position(|pairing| pairing.entry == *entry);
        let pairing = if let Some(index) = index {
            &mut pairings[index]
        } else {
            pairings.push(Pairing {
                entry: entry.clone(),
                cutoff: 0,
                pending: BTreeMap::new(),
                answer: None,
            });
            pairings.last_mut().unwrap()
        };
        let source = match reason {
            Reason::Poll => {
                if pairing
                    .pending
                    .values()
                    .any(|source| *source == Source::Poll)
                {
                    return None;
                }
                Source::Poll
            }
            Reason::SavedDismissal(command) => {
                pairing.cutoff = self.last_order;
                if let Some(document) = pairing
                    .answer
                    .as_mut()
                    .and_then(|answer| answer.good.as_mut())
                {
                    Arc::make_mut(document).omit_command(command);
                }
                Source::SavedDismissal
            }
        };
        self.last_order += 1;
        pairing.pending.insert(self.last_order, source);
        Some(Read {
            person: person.clone(),
            entry: entry.clone(),
            order: self.last_order,
        })
    }
}

impl InboxReads {
    pub fn is_empty(&self) -> bool {
        self.0.lock().unwrap().viewers.is_empty()
    }

    /// Discard retired pairings and idle viewers before admitting subscribed polls.
    pub fn sweep(&self, subscribed: &BTreeSet<Person>, current: &[&Entry]) -> Vec<Read> {
        let mut state = self.0.lock().unwrap();
        state.viewers.retain(|person, pairings| {
            pairings.retain(|pairing| {
                current.contains(&&pairing.entry)
                    && (subscribed.contains(person) || !pairing.pending.is_empty())
            });
            !pairings.is_empty()
        });
        let mut reads = Vec::new();
        for person in subscribed {
            for entry in current {
                reads.extend(state.admit(person, entry, Reason::Poll));
            }
        }
        reads
    }

    /// Dismiss and Undo fence outstanding reads and admit replacements atomically.
    pub fn dismissal_saved(
        &self,
        entry: &Entry,
        actor: Option<&str>,
        subscribed: &BTreeSet<Person>,
        command: &str,
    ) -> Vec<Read> {
        let mut state = self.0.lock().unwrap();
        let mut people = subscribed.clone();
        people.insert(actor.map(str::to_owned));
        people.extend(state.viewers.iter().filter_map(|(person, pairings)| {
            pairings
                .iter()
                .any(|pairing| pairing.entry == *entry)
                .then(|| person.clone())
        }));
        people
            .iter()
            .filter_map(|person| state.admit(person, entry, Reason::SavedDismissal(command)))
            .collect()
    }

    /// Return whether this completion changed the answer available to the Inbox.
    pub fn complete(&self, read: Read, result: ReadResult, completed_at: DateTime<Utc>) -> bool {
        let mut state = self.0.lock().unwrap();
        let Some(pairing) = state.viewers.get_mut(&read.person).and_then(|pairings| {
            pairings
                .iter_mut()
                .find(|pairing| pairing.entry == read.entry)
        }) else {
            return false;
        };
        if pairing.pending.remove(&read.order).is_none() || read.order <= pairing.cutoff {
            return false;
        }
        let answer = pairing.answer.get_or_insert_with(Answer::default);
        match result {
            Ok((document, scopes)) => {
                pairing.cutoff = read.order;
                *answer = Answer {
                    good: Some(Arc::new(document)),
                    fetched_at: Some(completed_at),
                    error: None,
                    scopes,
                };
            }
            Err(error) => {
                // A newer failure leaves an earlier post-save success eligible.
                answer.error = Some(error);
                answer.scopes.clear();
            }
        }
        true
    }

    pub fn answer(&self, person: Option<&str>, entry: &Entry) -> Option<Answer> {
        self.0
            .lock()
            .unwrap()
            .viewers
            .get(&person.map(str::to_owned))?
            .iter()
            .find(|pairing| pairing.entry == *entry)?
            .answer
            .clone()
    }
}

#[cfg(test)]
mod tests;
