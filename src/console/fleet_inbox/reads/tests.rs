use chrono::{DateTime, Utc};
use serde_json::json;
use std::collections::BTreeSet;

// Use the same interface as the watching Console.
use super::{Answer, InboxReads, Read, ReadResult};
use crate::config::console::fleet::Entry;
use crate::console::Scope;
use crate::logs::status::fleet::Reachability;

fn people(names: &[&str]) -> BTreeSet<Option<String>> {
    names.iter().map(|name| Some((*name).to_owned())).collect()
}

fn pairing(name: &str) -> Entry {
    Entry::link(name.into(), format!("https://{name}.example.test"))
}

fn time(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).unwrap()
}

fn good(selectors: &[&str]) -> ReadResult {
    let commands: Vec<_> = selectors
        .iter()
        .map(|selector| json!({ "selector": selector }))
        .collect();
    let document = serde_json::from_value(json!({
        "version": 2,
        "hostname": "owner",
        "active_runs": 0,
        "commands": commands,
    }))
    .unwrap();
    Ok((document, vec![Scope::Read, Scope::Act]))
}

fn failed() -> ReadResult {
    Err((Reachability::Unreachable, "Inbox read failed".into()))
}

fn selectors(answer: &Answer) -> Vec<String> {
    let document = serde_json::to_value(answer.good.as_deref().unwrap()).unwrap();
    document["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|command| command["selector"].as_str().unwrap().to_owned())
        .collect()
}

fn only_read(mut reads: Vec<Read>) -> Read {
    assert_eq!(reads.len(), 1);
    reads.pop().unwrap()
}

#[test]
fn a_save_fences_overlapping_polls_and_omits_only_the_affected_command() {
    let reads = InboxReads::default();
    let owner = pairing("owner");
    let other = pairing("other");
    let viewers = people(&["actor", "observer"]);
    let entries = [&owner, &other];
    for read in reads.sweep(&viewers, &entries) {
        assert!(reads.complete(read, good(&["target", "unrelated"]), time(1)));
    }
    let old_polls = reads.sweep(&viewers, &entries);
    assert_eq!(old_polls.len(), 4);
    assert!(reads.sweep(&viewers, &entries).is_empty());

    let replacements = reads.dismissal_saved(&owner, Some("actor"), &viewers, "target");
    assert_eq!(replacements.len(), 2);
    for viewer in ["actor", "observer"] {
        let answer = reads.answer(Some(viewer), &owner).unwrap();
        assert_eq!(selectors(&answer), ["unrelated"]);
        assert_eq!(answer.fetched_at, Some(time(1)));
        assert_eq!(
            selectors(&reads.answer(Some(viewer), &other).unwrap()),
            ["target", "unrelated"]
        );
    }
    for read in old_polls {
        if read.entry() == &other {
            assert!(reads.complete(read, good(&["other-machine"]), time(2)));
        } else if read.person() == Some("actor") {
            assert!(!reads.complete(read, good(&["target", "unrelated"]), time(2)));
        } else {
            assert!(!reads.complete(read, failed(), time(2)));
        }
    }
    for read in replacements {
        let answer = reads.answer(read.person(), &owner).unwrap();
        assert_eq!(selectors(&answer), ["unrelated"]);
        assert!(answer.error.is_none());
        let fresh_command = if read.person() == Some("actor") {
            "actor-only"
        } else {
            "observer-only"
        };
        assert!(reads.complete(read, good(&[fresh_command]), time(3)));
    }
    assert_eq!(
        selectors(&reads.answer(Some("actor"), &owner).unwrap()),
        ["actor-only"]
    );
    assert_eq!(
        selectors(&reads.answer(Some("observer"), &owner).unwrap()),
        ["observer-only"]
    );
}

#[test]
fn an_undo_read_can_clear_a_later_polls_failure() {
    let reads = InboxReads::default();
    let owner = pairing("owner");
    let viewers = people(&["actor"]);
    let first = only_read(reads.sweep(&viewers, &[&owner]));
    assert!(reads.complete(first, good(&["dismissed", "unrelated"]), time(1)));

    let undo = only_read(reads.dismissal_saved(&owner, Some("actor"), &viewers, "dismissed"));
    let poll = only_read(reads.sweep(&viewers, &[&owner]));
    assert!(reads.complete(poll, failed(), time(2)));
    let answer = reads.answer(Some("actor"), &owner).unwrap();
    assert_eq!(selectors(&answer), ["unrelated"]);
    assert_eq!(answer.fetched_at, Some(time(1)));
    assert!(matches!(answer.error, Some((Reachability::Unreachable, _))));
    assert!(answer.scopes.is_empty());

    assert!(reads.complete(undo, good(&["restored", "unrelated"]), time(3)));
    let answer = reads.answer(Some("actor"), &owner).unwrap();
    assert_eq!(selectors(&answer), ["restored", "unrelated"]);
    assert_eq!(answer.fetched_at, Some(time(3)));
    assert!(answer.error.is_none());
    assert!(answer.scopes.contains(&Scope::Act));
}

#[test]
fn a_newer_success_fences_an_older_post_save_failure() {
    let reads = InboxReads::default();
    let owner = pairing("owner");
    let viewers = people(&["actor"]);
    let after_save = only_read(reads.dismissal_saved(&owner, Some("actor"), &viewers, "target"));
    let poll = only_read(reads.sweep(&viewers, &[&owner]));
    assert!(reads.complete(poll, good(&["latest"]), time(1)));
    assert!(!reads.complete(after_save, failed(), time(2)));
    let answer = reads.answer(Some("actor"), &owner).unwrap();
    assert_eq!(selectors(&answer), ["latest"]);
    assert_eq!(answer.fetched_at, Some(time(1)));
    assert!(answer.error.is_none());
    assert!(answer.scopes.contains(&Scope::Act));
}

#[test]
fn a_closed_viewers_first_poll_is_fenced_and_kept_until_its_reads_finish() {
    let reads = InboxReads::default();
    let owner = pairing("owner");
    let no_subscribers = BTreeSet::new();
    assert!(reads.sweep(&no_subscribers, &[&owner]).is_empty());
    let first = only_read(reads.sweep(&people(&["closed"]), &[&owner]));
    assert!(reads.answer(Some("closed"), &owner).is_none());
    assert!(reads.sweep(&no_subscribers, &[&owner]).is_empty());

    let replacements = reads.dismissal_saved(&owner, Some("actor"), &people(&["new"]), "target");
    assert_eq!(
        replacements.iter().map(Read::person).collect::<Vec<_>>(),
        [Some("actor"), Some("closed"), Some("new")]
    );
    assert!(!reads.complete(first, good(&["target"]), time(1)));
    assert!(reads.answer(Some("closed"), &owner).is_none());
    assert!(reads.sweep(&no_subscribers, &[&owner]).is_empty());

    for read in replacements {
        assert!(reads.complete(read, good(&["saved"]), time(2)));
    }
    assert_eq!(
        selectors(&reads.answer(Some("closed"), &owner).unwrap()),
        ["saved"]
    );
    assert!(reads.sweep(&no_subscribers, &[&owner]).is_empty());
    assert!(reads.answer(Some("closed"), &owner).is_none());
    assert!(reads.is_empty());
}

#[test]
fn discarded_reads_cannot_restore_an_answer_after_the_exact_pairing_returns() {
    let reads = InboxReads::default();
    let original = pairing("owner");
    let mut replacement = original.clone();
    replacement.address = "https://replacement.example.test".into();
    let viewers = people(&["actor"]);
    let first = only_read(reads.sweep(&viewers, &[&original]));
    assert!(reads.complete(first, good(&["original"]), time(1)));
    let discarded = only_read(reads.sweep(&viewers, &[&original]));
    assert!(reads.answer(Some("actor"), &replacement).is_none());

    let new = only_read(reads.sweep(&viewers, &[&replacement]));
    assert!(reads.complete(new, good(&["replacement"]), time(2)));
    assert!(reads.answer(Some("actor"), &original).is_none());

    let readded = only_read(reads.sweep(&viewers, &[&original]));
    assert!(!reads.complete(discarded, good(&["discarded"]), time(3)));
    assert!(reads.answer(Some("actor"), &original).is_none());
    assert!(reads.answer(Some("actor"), &replacement).is_none());
    assert!(reads.complete(readded, good(&["readded"]), time(4)));
    assert_eq!(
        selectors(&reads.answer(Some("actor"), &original).unwrap()),
        ["readded"]
    );
}
