use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use serde_json::{Value, json};

use super::*;
use crate::logs::reading::{Entry, Snapshot};

const FOLDER: &str = "acme/widgets/commands/issue";
const STAMP: &str = "20261019T110000-0400";

fn now() -> DateTime<Local> {
    DateTime::parse_from_rfc3339("2026-10-19T12:00:00-04:00")
        .unwrap()
        .with_timezone(&Local)
}

fn at(minute: u32) -> String {
    format!("2026-10-19T11:{minute:02}:00-04:00")
}

fn record(kind: &str, number: Option<u64>, minute: u32) -> Value {
    json!({
        "version": 1, "kind": kind, "repository": "acme/widgets",
        "name": number.map_or_else(|| format!("pickup-{STAMP}"), |number| format!("acme/widgets#{number}")),
        "issue": number.map(|number| json!({"number": number, "url": format!("https://github.com/acme/widgets/issues/{number}"), "title": "Widgets"})),
        "hostname": "factory", "process_id": 0, "process_start": "gone",
        "harness": "claude", "model": null, "effort": null,
        "started_at": at(minute), "ended_at": at(59),
        "stages": [], "session_log": null, "outcome": "merged", "cause": null,
        "issue_branch": null, "pull_request": null
    })
}

fn stage(name: &str, minute: u32, session: Option<&Path>) -> Value {
    json!({"name": name, "began_at": at(minute), "session_log": session})
}

fn write(root: &Path, name: &str, value: &Value) -> PathBuf {
    let path = root.join(FOLDER).join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    path
}

/// Inventory orders Sessions by modification time; fix it per fixture.
fn session(root: &Path, name: &str, second: u64) -> PathBuf {
    let path = root.join(FOLDER).join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "").unwrap();
    fs::File::open(&path)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(second))
        .unwrap();
    path
}

fn commands(snapshot: &Snapshot) -> Vec<&Command> {
    snapshot
        .entries()
        .iter()
        .filter_map(|entry| match entry {
            Entry::Record(command) => Some(command.as_ref()),
            Entry::History(_) => None,
        })
        .collect()
}

/// Each node's kind and the record name, stage name or Session file it presents.
fn outline(nodes: &[Node]) -> Vec<String> {
    nodes
        .iter()
        .map(|node| match &node.fact {
            Fact::Command => format!("cmd {}", node.command.record.name),
            Fact::Run => format!("run {}", node.command.record.name),
            Fact::Stage(time) => format!("stage {}", time.stage.name),
            Fact::Session(attachment) => format!(
                "sess {}",
                attachment.path.file_name().unwrap().to_string_lossy()
            ),
        })
        .collect()
}

fn find<'n, 'a>(nodes: &'n [Node<'a>], line: &str) -> &'n Node<'a> {
    let index = outline(nodes)
        .iter()
        .position(|candidate| candidate == line)
        .unwrap_or_else(|| panic!("{line} not in {:?}", outline(nodes)));
    &nodes[index]
}

fn nth<'n, 'a>(nodes: &'n [Node<'a>], line: &str, n: usize) -> &'n Node<'a> {
    let outline = outline(nodes);
    let index = outline
        .iter()
        .enumerate()
        .filter(|(_, candidate)| *candidate == line)
        .nth(n)
        .unwrap_or_else(|| panic!("{line} #{n} not in {outline:?}"))
        .0;
    &nodes[index]
}

#[test]
fn dispatched_and_child_runs_descend_from_the_stage_that_started_them() {
    let dir = tempfile::tempdir().unwrap();
    let mut pickup = record("Pickup run", None, 0);
    pickup["stages"] = json!([
        stage("Architecture review", 0, None),
        stage("dispatched Spec run", 1, None)
    ]);
    let mut spec = record("Spec run", Some(20), 1);
    spec["stages"] = json!([stage("Spec review", 2, None), stage("Tickets", 5, None)]);
    pickup["dispatched"] = json!([spec]);
    let path = write(dir.path(), &format!("{STAMP}~pickup.command.json"), &pickup);
    // Ticket 22 started before any Spec run stage, so it falls back to the Run.
    for (number, minute) in [(21, 6), (22, 1)] {
        let mut ticket = record("Run", Some(number), minute);
        ticket["parent_record"] = json!(path);
        ticket["parent_dispatch"] = json!("https://github.com/acme/widgets/issues/20");
        ticket["stages"] = json!([stage("implement", minute, None)]);
        write(
            dir.path(),
            &format!("{STAMP}~pickup.command.dispatch-20-issue-{number}.child.json"),
            &ticket,
        );
    }
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    let owner = commands(&snapshot)[0];

    let whole = of(owner, owner).unwrap();
    assert_eq!(
        outline(&whole),
        [
            "cmd pickup-20261019T110000-0400",
            "run pickup-20261019T110000-0400",
            "stage Architecture review",
            "stage dispatched Spec run",
            "run acme/widgets#20",
            "stage Spec review",
            "stage Tickets",
            "run acme/widgets#21",
            "stage implement",
            "run acme/widgets#22",
            "stage implement",
        ]
    );
    let id = |line| find(&whole, line).id();
    assert_eq!(
        find(&whole, "run pickup-20261019T110000-0400").parent,
        id("cmd pickup-20261019T110000-0400")
    );
    assert_eq!(
        find(&whole, "run acme/widgets#20").parent,
        id("stage dispatched Spec run")
    );
    assert_eq!(
        find(&whole, "run acme/widgets#21").parent,
        id("stage Tickets")
    );
    assert_eq!(
        find(&whole, "run acme/widgets#22").parent,
        id("run acme/widgets#20")
    );
    assert_eq!(
        nth(&whole, "stage implement", 1).parent,
        id("run acme/widgets#22")
    );

    // A selected Ticket keeps the outer identity but only its own subtree.
    let ticket = &owner.dispatched[0].children[0];
    let selected = of(ticket, owner).unwrap();
    assert_eq!(
        outline(&selected),
        [
            "cmd acme/widgets#21",
            "run acme/widgets#21",
            "stage implement"
        ]
    );
    assert_eq!(selected[0].id(), whole[0].id());
    assert_eq!(selected[0].context.trace, whole[0].context.trace);
    assert_eq!(selected[1].id(), id("run acme/widgets#21"));
    assert_eq!(selected[1].parent, selected[0].id());
    assert_eq!(selected[2].id(), find(&whole, "stage implement").id());
}

#[test]
fn each_session_attaches_once_to_its_descendant_or_first_naming_stage() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join(FOLDER);
    // One Session under its former raw name and its current trace name.
    let current = session(
        dir.path(),
        &format!("20-{STAMP}~spec-implement.logs.jsonl"),
        1,
    );
    let former = folder.join(format!("20-{STAMP}~spec-implement.jsonl"));
    let ticket_session = session(
        dir.path(),
        &format!("21-{STAMP}~spec-implement.logs.jsonl"),
        2,
    );
    let inventory_only = session(
        dir.path(),
        &format!("20-{STAMP}~spec-repair-1.logs.jsonl"),
        3,
    );
    let mut spec = record("Spec run", Some(20), 0);
    spec["stages"] = json!([
        stage("implement", 0, Some(&former)),
        stage("Repair", 20, Some(&ticket_session)),
        stage("pull request", 30, Some(&current))
    ]);
    spec["session_log"] = json!(current);
    let path = write(dir.path(), &format!("20-{STAMP}~spec.command.json"), &spec);
    // The Ticket names no Session; inventory finds the one its ancestor reused.
    let mut ticket = record("Run", Some(21), 10);
    ticket["parent_record"] = json!(path);
    ticket["stages"] = json!([stage("implement", 10, None)]);
    write(
        dir.path(),
        &format!("20-{STAMP}~spec.command.issue-21.child.json"),
        &ticket,
    );
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    let owner = commands(&snapshot)[0];

    let nodes = of(owner, owner).unwrap();
    assert_eq!(
        outline(&nodes),
        [
            "cmd acme/widgets#20",
            "run acme/widgets#20",
            "stage implement",
            "stage Repair",
            "stage pull request",
            "run acme/widgets#21",
            "stage implement",
            "sess 21-20261019T110000-0400~spec-implement.logs.jsonl",
            "sess 20-20261019T110000-0400~spec-implement.logs.jsonl",
            "sess 20-20261019T110000-0400~spec-repair-1.logs.jsonl",
        ]
    );
    let id = |line| find(&nodes, line).id();
    assert_eq!(
        find(&nodes, "run acme/widgets#21").parent,
        id("stage implement")
    );
    let attachment = |node: &Node| match &node.fact {
        Fact::Session(attachment) => (
            attachment.path.clone(),
            attachment.original.clone(),
            attachment
                .stage
                .as_ref()
                .map(|time| time.stage.name.clone()),
        ),
        Fact::Command | Fact::Run | Fact::Stage(_) => panic!("not a Session"),
    };

    // The descendant claims first, so the ancestor's Repair stage loses it.
    let claimed = find(
        &nodes,
        "sess 21-20261019T110000-0400~spec-implement.logs.jsonl",
    );
    assert_eq!(claimed.parent, id("run acme/widgets#21"));
    assert_eq!(claimed.context.run, id("run acme/widgets#21").unwrap());
    assert_eq!(
        attachment(claimed),
        (
            ticket_session.clone(),
            folder.join(format!("21-{STAMP}~spec-implement.jsonl")),
            None
        )
    );
    // Both names normalize to one Session, owned by the first naming stage.
    let reused = find(
        &nodes,
        "sess 20-20261019T110000-0400~spec-implement.logs.jsonl",
    );
    assert_eq!(reused.parent, id("stage implement"));
    assert_eq!(
        attachment(reused),
        (current.clone(), former.clone(), Some("implement".into()))
    );
    let fallback = find(
        &nodes,
        "sess 20-20261019T110000-0400~spec-repair-1.logs.jsonl",
    );
    assert_eq!(fallback.parent, id("run acme/widgets#20"));
    assert_eq!(attachment(fallback).0, inventory_only);
    assert_eq!(attachment(fallback).2, None);

    // A selected Ticket still attaches the Session it owns.
    let selected = of(&owner.children[0], owner).unwrap();
    assert_eq!(
        outline(&selected),
        [
            "cmd acme/widgets#21",
            "run acme/widgets#21",
            "stage implement",
            "sess 21-20261019T110000-0400~spec-implement.logs.jsonl",
        ]
    );
    assert_eq!(selected[3].parent, claimed.parent);
}

#[test]
fn distinct_commands_with_equal_start_stamps_have_distinct_identities() {
    let dir = tempfile::tempdir().unwrap();
    let mut spec = record("Spec run", Some(20), 0);
    spec["stages"] = json!([stage("implement", 0, None)]);
    for name in ["first", "second"] {
        write(
            dir.path(),
            &format!("20-{STAMP}~{name}.command.json"),
            &spec,
        );
    }
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    let commands = commands(&snapshot);
    assert_eq!(commands.len(), 2);
    let [first, second] = [commands[0], commands[1]].map(|command| of(command, command).unwrap());
    assert_eq!(outline(&first), outline(&second));
    assert_ne!(first[0].context.trace, second[0].context.trace);
    for (first, second) in first.iter().zip(&second) {
        assert_ne!(first.id(), second.id());
    }
}
