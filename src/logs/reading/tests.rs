use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use serde_json::{Value, json};

use super::*;

#[path = "inventory_tests.rs"]
mod inventory_tests;
#[path = "reader_tests.rs"]
mod reader_tests;

fn now() -> DateTime<Local> {
    DateTime::parse_from_rfc3339("2026-10-19T12:00:00-04:00")
        .unwrap()
        .with_timezone(&Local)
}

fn record(number: u64) -> Value {
    json!({
        "version": 1, "kind": "Run", "repository": "acme/widgets",
        "name": format!("acme/widgets#{number}"),
        "issue": {"number": number, "url": format!("https://github.com/acme/widgets/issues/{number}"), "title": "Widgets"},
        "hostname": "factory", "process_id": std::process::id(),
        "process_start": crate::logs::process_identity::start(std::process::id()).unwrap(),
        "harness": "claude", "model": null, "effort": null,
        "started_at": "2026-10-19T11:00:00-04:00", "ended_at": null,
        "stages": [], "session_log": null, "outcome": null, "cause": null,
        "issue_branch": format!("issue-{number}"), "pull_request": null
    })
}

fn write(root: &Path, name: &str, value: &Value) -> PathBuf {
    let path = root.join("acme/widgets/commands/issue").join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    path
}

#[test]
fn direct_run_selection_captures_its_owner_and_reloads_retained_work() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "7-20261019T110000-0400~direct.command.json",
        &record(7),
    );
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    assert_eq!(snapshot.inbox(true, 7).len(), 1);
    let selected = snapshot.resolve("WIDGETS#7").unwrap();
    assert_eq!(selected.owner.unwrap().record_path, path);
    assert!(selected.child_owner.is_none());
    assert_eq!(selected.stop_target("WIDGETS#7").unwrap(), path);
    assert!(!selected.process_exited());
    let mut ended = record(7);
    ended["issue_branch"] = "issue-7-preserved".into();
    ended["pull_request"] = "https://github.com/acme/widgets/pull/9".into();
    fs::write(&path, serde_json::to_vec(&ended).unwrap()).unwrap();
    let retained = retained_work(&path, now()).unwrap();
    assert_eq!(retained.branches, ["issue-7-preserved".to_string()].into());
    assert_eq!(
        retained.pull_requests,
        ["https://github.com/acme/widgets/pull/9".to_string()].into()
    );
}

#[test]
fn pass_dispatches_and_equal_numbered_children_keep_exact_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let stem = "20261019T110000-0400~security";
    let mut pass = record(7);
    pass["kind"] = "Security run".into();
    pass["name"] = "secure-20261019T110000-0400".into();
    pass["issue"] = Value::Null;
    pass["issue_branch"] = "issue-20".into();
    pass["pull_request"] = "https://github.com/acme/widgets/pull/20".into();
    let path = write(dir.path(), &format!("{stem}.command.json"), &pass);
    let mut dispatches = Vec::new();
    for number in [20, 30] {
        let mut dispatch = record(number);
        if number == 20 {
            dispatch["kind"] = "Spec run".into();
        }
        dispatches.push(dispatch);
        let mut child = record(21);
        child["parent_record"] = json!(path);
        child["parent_dispatch"] =
            format!("https://github.com/acme/widgets/issues/{number}").into();
        child["issue_branch"] = format!("dispatch-{number}-ticket-21").into();
        child["pull_request"] = format!("https://github.com/acme/widgets/pull/{number}").into();
        child["outcome"] = "merged".into();
        child["ended_at"] = "2026-10-19T11:30:00-04:00".into();
        if number == 30 {
            child["started_at"] = "2026-10-19T11:10:00-04:00".into();
        }
        write(
            dir.path(),
            &format!("{stem}.command.dispatch-{number}-issue-21.child.json"),
            &child,
        );
    }
    pass["dispatched"] = dispatches.into();
    fs::write(&path, serde_json::to_vec(&pass).unwrap()).unwrap();
    // A concurrent Command with the same subject and stamp owns its own child.
    let separate = write(
        dir.path(),
        "20261019T110000-0400~separate.command.json",
        &record(20),
    );
    let mut unrelated = record(99);
    unrelated["parent_record"] = json!(separate);
    write(dir.path(), "separate-99.child.json", &unrelated);
    // A parent-dispatch link to a dispatch absent from this Command is not adopted.
    unrelated["parent_record"] = json!(path);
    unrelated["parent_dispatch"] = "https://github.com/acme/widgets/issues/999".into();
    write(dir.path(), "unmatched-99.child.json", &unrelated);

    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    let selected = snapshot.resolve("secure-20261019T110000-0400").unwrap();
    let outer = selected.owner.unwrap();
    assert_eq!(outer.record_path, path);
    assert_eq!(outer.children.len(), 0);
    assert_eq!(outer.dispatched.len(), 2);
    for (index, number) in [20, 30].into_iter().enumerate() {
        let dispatch = &outer.dispatched[index];
        assert_eq!(dispatch.record.issue.as_ref().unwrap().number, number);
        assert_eq!(dispatch.children.len(), 1);
        assert_eq!(
            dispatch.children[0].record.issue_branch.as_deref(),
            Some(format!("dispatch-{number}-ticket-21").as_str())
        );
    }
    let dispatched = snapshot.resolve("widgets#30").unwrap();
    assert_eq!(dispatched.stop_target("widgets#30").unwrap(), path);
    assert!(dispatched.child_owner.is_none());
    let child = snapshot
        .resolve("https://github.com/acme/widgets/issues/21")
        .unwrap();
    assert_eq!(child.owner.unwrap().record_path, path);
    assert_eq!(child.child_owner.unwrap().record.name, "acme/widgets#30");
    assert_eq!(
        child.stop_target("widgets#21").unwrap_err().to_string(),
        "cannot stop widgets#21: child Ticket run; stop Ticket run acme/widgets#30 instead"
    );
    // A child's durable outcome does not end following its outer process.
    assert!(!child.process_exited());
    let retained = retained_work(&path, now()).unwrap();
    assert_eq!(
        retained.branches.into_iter().collect::<Vec<_>>(),
        [
            "dispatch-20-ticket-21",
            "dispatch-30-ticket-21",
            "issue-20",
            "issue-30"
        ]
    );
    assert_eq!(
        retained.pull_requests.into_iter().collect::<Vec<_>>(),
        [
            "https://github.com/acme/widgets/pull/20",
            "https://github.com/acme/widgets/pull/30"
        ]
    );
    assert_eq!(
        retained_work(&separate, now())
            .unwrap()
            .branches
            .into_iter()
            .collect::<Vec<_>>(),
        ["issue-20", "issue-99"]
    );
    assert_eq!(fs::read(&path).unwrap(), serde_json::to_vec(&pass).unwrap());
    assert!(!path.with_extension("stop.json").exists());
}

#[test]
fn ticket_session_inventory_includes_stage_current_resume_and_numbered_repairs() {
    let dir = tempfile::tempdir().unwrap();
    let mut spec = record(20);
    spec["kind"] = "Spec run".into();
    let path = write(
        dir.path(),
        "20-20261019T110000-0400~spec.command.json",
        &spec,
    );
    let folder = path.parent().unwrap();
    let implement = folder.join("21-20261019T110000-0400~spec-implement.logs.jsonl");
    let resume = folder.join("21-20261019T110000-0400~spec-implement-resume.logs.jsonl");
    let repair = folder.join("21-20261019T110000-0400~spec-repair-2-resume.logs.jsonl");
    let previous = folder.join("21-20261019T110000-0400~spec-repair-1.logs.jsonl");
    let retained_reference = folder.join("retained-session.logs.jsonl");
    for (index, session) in [&retained_reference, &implement, &resume, &previous, &repair]
        .into_iter()
        .enumerate()
    {
        fs::write(session, "").unwrap();
        let file = fs::File::open(session).unwrap();
        file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(index as u64))
            .unwrap();
    }
    fs::write(
        folder.join("21-20261019T110000-0400~other-repair-9.logs.jsonl"),
        "unrelated",
    )
    .unwrap();
    let mut ticket = record(21);
    ticket["parent_record"] = json!(path);
    ticket["stages"] = json!([
        {"name": "implement", "began_at": "2026-10-19T11:00:00-04:00", "session_log": implement},
        {"name": "Repair", "began_at": "2026-10-19T11:30:00-04:00", "session_log": retained_reference}
    ]);
    ticket["session_log"] = json!(implement);
    let child_path = write(
        dir.path(),
        "20-20261019T110000-0400~spec.command.issue-21.child.json",
        &ticket,
    );
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    let selected = snapshot.resolve("ACME/WIDGETS#21").unwrap();
    assert_eq!(selected.owner.unwrap().record_path, path);
    assert_eq!(selected.child_owner.unwrap().record.name, "acme/widgets#20");
    let Observation::Record(command) = selected.observation else {
        panic!("record-backed Ticket")
    };
    assert_eq!(command.record_path, child_path);
    assert_eq!(command.stage.as_ref().unwrap().age_seconds, 1800);
    assert_eq!(
        selected.observation.sessions().unwrap(),
        [&retained_reference, &implement, &resume, &previous, &repair]
            .into_iter()
            .cloned()
            .collect::<Vec<_>>()
    );
    assert_eq!(selected.observation.session("implement").unwrap(), resume);
    assert_eq!(selected.observation.session("repair").unwrap(), repair);
    assert_eq!(selected.observation.session("repair-1").unwrap(), previous);
    assert_eq!(
        selected.stop_target("widgets#21").unwrap_err().to_string(),
        "cannot stop widgets#21: child Ticket run; stop Spec run acme/widgets#20 instead"
    );
}

#[test]
fn historical_selection_keeps_unknown_facts_and_has_no_process_or_stop_authority() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("acme/widgets/commands/issue");
    fs::create_dir_all(&folder).unwrap();
    let session = folder.join("7-20261019T110000-0400~history-implement.logs.jsonl");
    fs::write(&session, serde_json::json!({"resourceLogs":[{"scopeLogs":[{"logRecords":[{"body":{"stringValue":"not interpreted as a live event"}}]}]}]}).to_string()).unwrap();
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    assert!(snapshot.inbox(false, 7).is_empty());
    assert_eq!(snapshot.inbox(true, 7).len(), 1);
    let selected = snapshot.resolve("widgets#7").unwrap();
    assert!(selected.owner.is_none());
    assert!(selected.child_owner.is_none());
    assert!(selected.process_exited());
    assert_eq!(selected.observation.session("implement").unwrap(), session);
    let details = serde_json::to_value(selected.observation.details().unwrap()).unwrap();
    assert_eq!(details["state"], "unknown");
    assert_eq!(details["stages"], json!([]));
    assert_eq!(details["stage"], Value::Null);
    assert_eq!(details["harness"], Value::Null);
    assert_eq!(details["model"], Value::Null);
    assert_eq!(details["source"], "session_log");
    assert_eq!(
        selected.stop_target("widgets#7").unwrap_err().to_string(),
        "cannot stop widgets#7: Command unknown"
    );
}

#[test]
fn outcome_ends_display_but_not_the_captured_process_for_following() {
    let dir = tempfile::tempdir().unwrap();
    let mut ended = record(7);
    ended["outcome"] = "ready for review".into();
    ended["ended_at"] = "2026-10-19T11:30:00-04:00".into();
    let path = write(
        dir.path(),
        "7-20261019T110000-0400~ended.command.json",
        &ended,
    );
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    let selected = snapshot.resolve("widgets#7").unwrap();
    assert_eq!(selected.owner.unwrap().state, "ended");
    assert!(!selected.process_exited());
    assert_eq!(
        selected.stop_target("widgets#7").unwrap_err().to_string(),
        "cannot stop widgets#7: Command ended: ready for review"
    );
    // A later record cannot retarget this captured identity.
    ended["process_start"] = "reused-process-identity".into();
    fs::write(&path, serde_json::to_vec(&ended).unwrap()).unwrap();
    assert!(!selected.process_exited());
    let refreshed = Snapshot::load(dir.path(), now()).unwrap();
    assert!(refreshed.resolve("widgets#7").unwrap().process_exited());
}

fn ended(mut record: Value, outcome: &str) -> Value {
    record["outcome"] = outcome.into();
    record["ended_at"] = "2026-10-19T11:30:00-04:00".into();
    record
}

fn died(mut record: Value) -> Value {
    record["process_start"] = "gone".into();
    record
}

#[test]
fn a_spec_run_counts_once_with_its_parallel_tickets_and_another_ticket_run_adds_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut spec = record(20);
    spec["kind"] = "Spec run".into();
    let path = write(dir.path(), "20-spec.command.json", &spec);
    for number in [21, 22] {
        let mut ticket = record(number);
        ticket["parent_record"] = json!(path);
        write(
            dir.path(),
            &format!("20-spec.command.issue-{number}.child.json"),
            &ticket,
        );
    }
    assert_eq!(Snapshot::load(dir.path(), now()).unwrap().active_runs(), 1);
    write(dir.path(), "7-direct.command.json", &record(7));
    assert_eq!(Snapshot::load(dir.path(), now()).unwrap().active_runs(), 2);
}

#[test]
fn a_pass_counts_once_with_its_dispatches_their_tickets_and_base_fixes() {
    for kind in ["Pickup run", "Architect run", "Security run"] {
        a_pass_counts_once(kind);
    }
}

fn a_pass_counts_once(kind: &str) {
    let dir = tempfile::tempdir().unwrap();
    let mut pass = record(7);
    pass["kind"] = kind.into();
    pass["name"] = "pass-20261019T110000-0400".into();
    let mut spec = record(20);
    spec["kind"] = "Spec run".into();
    let ready = {
        let mut run = ended(record(30), "ready for review");
        run["pull_request"] = "https://github.com/acme/widgets/pull/30".into();
        run
    };
    pass["dispatched"] = json!([spec, ready]);
    let path = write(dir.path(), "pickup.command.json", &pass);
    for number in [21, 22] {
        let mut ticket = record(number);
        ticket["parent_record"] = json!(path);
        ticket["parent_dispatch"] = "https://github.com/acme/widgets/issues/20".into();
        let ticket_path = write(
            dir.path(),
            &format!("pickup.command.dispatch-20-issue-{number}.child.json"),
            &ticket,
        );
        let mut base_fix = record(40 + number);
        base_fix["kind"] = "Base fix".into();
        base_fix["parent_record"] = json!(ticket_path);
        write(
            dir.path(),
            &format!("pickup.command.dispatch-20-issue-{number}.issue-4{number}.child.json"),
            &base_fix,
        );
    }
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    // The ready dispatched run has its own Inbox row but no separate tree.
    assert_eq!(snapshot.inbox(false, 7).len(), 2, "{kind}");
    assert_eq!(snapshot.active_runs(), 1, "{kind}");
}

#[test]
fn live_work_under_an_ended_or_died_outer_record_counts_and_ended_work_does_not() {
    let dir = tempfile::tempdir().unwrap();
    for number in [1, 2] {
        let mut failed = ended(record(number), "failed");
        failed["cause"] = "the same cause".into();
        write(
            dir.path(),
            &format!("{number}-failed.command.json"),
            &failed,
        );
    }
    let mut ready = ended(record(3), "ready for review");
    ready["pull_request"] = "https://github.com/acme/widgets/pull/3".into();
    write(dir.path(), "3-ready.command.json", &ready);
    let died_spec = write(dir.path(), "4-died.command.json", &died(record(4)));
    let ended_spec = write(
        dir.path(),
        "5-ended.command.json",
        &ended(record(5), "failed"),
    );
    let mut finished = ended(record(51), "merged");
    finished["parent_record"] = json!(ended_spec);
    write(dir.path(), "5-ended.command.issue-51.child.json", &finished);
    let repo = dir.path().join("acme/widgets");
    fs::write(
        repo.join("activity.log"),
        "2026-10-19 11:00:00 Pickup run skipped: no Ready issue\n",
    )
    .unwrap();
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    assert_eq!(snapshot.idle_passes.len(), 1);
    assert_eq!(snapshot.active_runs(), 0);

    for (parent, name) in [(&died_spec, "4-died"), (&ended_spec, "5-ended")] {
        let mut ticket = record(60);
        ticket["parent_record"] = json!(parent);
        write(
            dir.path(),
            &format!("{name}.command.issue-60.child.json"),
            &ticket,
        );
    }
    assert_eq!(Snapshot::load(dir.path(), now()).unwrap().active_runs(), 2);
}
