use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use serde_json::{Value, json};

use super::*;

#[path = "attribution_tests.rs"]
mod attribution_tests;

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
fn retained_pull_request_lookup_orders_instants_across_offsets() {
    let dir = tempfile::tempdir().unwrap();
    let url = "https://github.com/acme/widgets/pull/23";
    let mut older = record(7);
    older["pull_request"] = url.into();
    older["started_at"] = "2026-10-19T12:00:00+02:00".into();
    write(dir.path(), "older.command.json", &older);
    let mut newer = record(7);
    newer["pull_request"] = url.into();
    newer["started_at"] = "2026-10-19T11:00:00Z".into();
    let path = write(dir.path(), "newer.command.json", &newer);

    assert_eq!(
        run_for_pull_request(dir.path(), url).unwrap(),
        Some((path, "acme/widgets#7".into()))
    );
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
    assert_eq!(
        snapshot
            .evaluate_inbox(7, &crate::logs::github::Kept::default())
            .into_rows(true)
            .len(),
        1
    );
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
    assert!(
        snapshot
            .evaluate_inbox(7, &crate::logs::github::Kept::default())
            .into_rows(false)
            .is_empty()
    );
    assert_eq!(
        snapshot
            .evaluate_inbox(7, &crate::logs::github::Kept::default())
            .into_rows(true)
            .len(),
        1
    );
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

#[test]
fn review_finding_retention_deadline_includes_triage() {
    let dir = tempfile::tempdir().unwrap();
    let mut audit = record(7);
    audit["kind"] = "Security run".into();
    audit["name"] = "".into();
    audit["issue"] = Value::Null;
    audit["issue_branch"] = Value::Null;
    audit["started_at"] = "2026-09-18T11:00:00-04:00".into();
    audit["ended_at"] = "2026-09-19T12:00:00-04:00".into();
    audit["outcome"] = "findings recorded".into();
    audit["security_findings_waiting"] = true.into();
    let path = write(dir.path(), "7-fixture.command.json", &audit);
    let mut cache = json!({"version": 2, "findings": {}});
    cache["findings"][path.to_str().unwrap()] = json!({
        "waits": {"https://github.com/acme/widgets/security/advisories/GHSA-waiting": false},
        "triaged_at": "2026-10-13T12:00:00-04:00"
    });
    fs::write(
        dir.path().join("github-inbox.json"),
        serde_json::to_vec(&cache).unwrap(),
    )
    .unwrap();
    let kept = crate::logs::github::load(dir.path());
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    let evaluation = snapshot.evaluate_inbox(7, &kept);
    let expected = now() + chrono::Duration::days(1) + chrono::Duration::nanoseconds(1);
    assert_eq!(
        evaluation.next_change(),
        Some(expected),
        "the open Inbox needs the triage+retention deadline to enter History"
    );
    assert!(evaluation.into_rows(true)[0].group == Group::Resolved);
}

#[test]
fn inbox_evaluation_keeps_stop_and_dismissal_facts_until_the_next_capture() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "7-captured.command.json",
        &ended(record(7), "failed"),
    );
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    let selector = snapshot
        .resolve("widgets#7")
        .unwrap()
        .observation
        .selector()
        .unwrap();
    let kept = crate::logs::github::Kept::default();
    let evaluation = snapshot.evaluate_inbox(7, &kept);
    fs::write(
        path.with_extension("stop.json"),
        br#"{"who":"test","requeue":false}"#,
    )
    .unwrap();
    fs::write(
        path.with_extension("dismiss.json"),
        serde_json::to_vec(&json!({"rows": [{
            "selector": selector, "kind": "Triage", "who": "test", "at": "2026-10-19T12:00:00-04:00"
        }]}))
        .unwrap(),
    )
    .unwrap();

    assert_eq!(evaluation.refresh_work().subjects.len(), 1);
    assert!(evaluation.refresh_work().subjects[0].triage_since.is_none());
    assert_eq!(
        evaluation.next_change(),
        Some(now() + chrono::Duration::minutes(1410) + chrono::Duration::nanoseconds(1))
    );
    let rows = evaluation.into_rows(false);
    assert!(rows[0].group == Group::NeedsYou);
    assert!(rows[0].needs_you_kind == Some(inbox::NeedsYouKind::Failed));
    assert!(rows[0].dismissal.is_none());

    let fresh = snapshot.evaluate_inbox(7, &kept);
    assert_eq!(
        fresh.refresh_work().subjects[0].triage_since,
        Some(DateTime::parse_from_rfc3339("2026-10-19T11:30:00-04:00").unwrap())
    );
    assert_eq!(
        fresh.next_change(),
        Some(now() + chrono::Duration::minutes(1410) + chrono::Duration::nanoseconds(1))
    );
    let rows = fresh.into_rows(false);
    assert!(rows[0].group == Group::Done);
    assert_eq!(rows[0].dismissal.as_ref().unwrap().kinds, ["Triage"]);
}

fn ended(mut record: Value, outcome: &str) -> Value {
    record["outcome"] = outcome.into();
    record["ended_at"] = "2026-10-19T11:30:00-04:00".into();
    record
}

#[test]
fn inbox_evaluation_filters_history_before_collapse_but_plans_every_lookup() {
    let dir = tempfile::tempdir().unwrap();
    for (number, dismissed_at) in [
        (7, "2026-10-18T12:00:00-04:00"),
        (8, "2026-10-01T12:00:00-04:00"),
    ] {
        let mut failed = ended(record(number), "failed");
        failed["started_at"] = "2026-09-18T11:00:00-04:00".into();
        failed["ended_at"] = "2026-09-19T12:00:00-04:00".into();
        failed["cause"] = "same failure".into();
        let path = write(
            dir.path(),
            &format!("{number}-collapsed.command.json"),
            &failed,
        );
        fs::write(
            path.with_extension("dismiss.json"),
            serde_json::to_vec(&json!({"rows": [{
                "selector": format!("{}::acme/widgets#{number}", path.display()),
                "kind": "Failed", "who": "test", "at": dismissed_at
            }]}))
            .unwrap(),
        )
        .unwrap();
    }
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    let kept = crate::logs::github::Kept::default();
    let evaluation = snapshot.evaluate_inbox(7, &kept);
    assert_eq!(
        evaluation
            .refresh_work()
            .subjects
            .iter()
            .map(|subject| subject.url.as_str())
            .collect::<Vec<_>>(),
        [
            "https://github.com/acme/widgets/issues/7",
            "https://github.com/acme/widgets/issues/8"
        ]
    );
    assert!(
        evaluation.refresh_work().freezes.is_empty(),
        "unanswered History gets its first lookup"
    );
    let boundary = DateTime::parse_from_rfc3339("2026-10-25T12:00:00-04:00")
        .unwrap()
        .with_timezone(&Local);
    assert_eq!(
        evaluation.next_change(),
        Some(boundary + chrono::Duration::nanoseconds(1))
    );
    let rows = evaluation.into_rows(false);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].group == Group::Resolved);
    assert_eq!(rows[0].count, 1);
    assert!(rows[0].collapsed.is_empty());
    let all = snapshot.evaluate_inbox(7, &kept).into_rows(true);
    assert_eq!(all[0].count, 2);
    assert!(all[0].collapsed[0].group == Group::History);

    for (number, group) in [(7, Group::Resolved), (8, Group::History)] {
        let selector = snapshot
            .resolve(&format!("widgets#{number}"))
            .unwrap()
            .observation
            .selector()
            .unwrap();
        let evaluation = snapshot.evaluate_inbox(7, &kept);
        assert_eq!(evaluation.refresh_work().subjects.len(), 2);
        assert_eq!(
            evaluation.next_change(),
            Some(boundary + chrono::Duration::nanoseconds(1))
        );
        let target = evaluation.into_target(&selector).unwrap();
        assert!(target.group == group);
        assert_eq!(
            target.count, 1,
            "History and visible Dismissal membership stay separate"
        );
        assert!(target.collapsed.is_empty());
    }

    let at_cutoff = Snapshot::load(dir.path(), boundary).unwrap();
    let evaluation = at_cutoff.evaluate_inbox(7, &kept);
    assert_eq!(
        evaluation.next_change(),
        Some(boundary + chrono::Duration::nanoseconds(1))
    );
    assert_eq!(
        evaluation.into_rows(false).len(),
        1,
        "retention includes the cutoff"
    );
    let after = Snapshot::load(dir.path(), boundary + chrono::Duration::nanoseconds(1)).unwrap();
    let evaluation = after.evaluate_inbox(7, &kept);
    assert!(evaluation.next_change().is_none());
    assert_eq!(evaluation.refresh_work().subjects.len(), 2);
    assert!(evaluation.into_rows(false).is_empty());
}

fn kept_answer(root: &Path, answer: Value) -> crate::logs::github::Kept {
    fs::write(
        root.join("github-inbox.json"),
        serde_json::to_vec(&answer).unwrap(),
    )
    .unwrap();
    crate::logs::github::load(root)
}

#[test]
fn inbox_evaluation_preserves_independent_findings_and_dismissal_retention() {
    let dir = tempfile::tempdir().unwrap();
    let finding = "https://github.com/acme/widgets/security/advisories/GHSA-waiting";
    let issue = "https://github.com/acme/widgets/issues/7";
    let mut audit = ended(record(7), "failed");
    audit["kind"] = "Security run".into();
    audit["started_at"] = "2026-09-18T11:00:00-04:00".into();
    audit["ended_at"] = "2026-09-19T12:00:00-04:00".into();
    audit["security_findings_waiting"] = true.into();
    audit["security_findings"] =
        json!([{ "url": finding, "title": "Waiting", "severity": "high" }]);
    let path = write(dir.path(), "7-findings.command.json", &audit);
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    let row = snapshot
        .resolve("widgets#7")
        .unwrap()
        .observation
        .selector()
        .unwrap();
    // Work settlement and Dismissal each expose a Finding independently.
    for (closed, waits, kinds, group, kind, deadline_day) in [
        (
            false,
            true,
            vec!["Failed"],
            Group::NeedsYou,
            Some(inbox::NeedsYouKind::Finding),
            Some(24),
        ),
        (
            true,
            true,
            vec![],
            Group::NeedsYou,
            Some(inbox::NeedsYouKind::Finding),
            None,
        ),
        (true, false, vec![], Group::Resolved, None, Some(25)),
        (
            true,
            true,
            vec!["Failed", "Finding"],
            Group::Resolved,
            None,
            Some(24),
        ),
        (
            true,
            false,
            vec!["Failed", "Finding"],
            Group::Resolved,
            None,
            Some(24),
        ),
    ] {
        fs::write(path.with_extension("dismiss.json"), serde_json::to_vec(&json!({"rows": if kinds.is_empty() { json!([]) } else { json!([{
            "selector": row, "kind": "Failed", "kinds": kinds, "who": "test", "at": "2026-10-17T12:00:00-04:00"
        }]) }})).unwrap()).unwrap();
        let kept = kept_answer(
            dir.path(),
            json!({
                "version": 2,
                "subjects": {issue: {"closed": closed, "closed_at": closed.then_some("2026-09-20T12:00:00-04:00"), "needs_triage": false, "triaged_at": null}},
                "findings": {row.clone(): {"waits": {finding: waits}, "triaged_at": (!waits).then_some("2026-10-18T12:00:00-04:00")}}
            }),
        );
        let evaluation = snapshot.evaluate_inbox(7, &kept);
        assert_eq!(
            evaluation
                .refresh_work()
                .subjects
                .iter()
                .map(|subject| subject.url.as_str())
                .collect::<Vec<_>>(),
            [issue]
        );
        assert_eq!(evaluation.refresh_work().findings.len(), 1);
        assert_eq!(evaluation.refresh_work().findings[0].row, row);
        assert_eq!(evaluation.refresh_work().findings[0].urls, [finding]);
        assert_eq!(
            evaluation.next_change(),
            deadline_day.map(|day| DateTime::parse_from_rfc3339(&format!(
                "2026-10-{day}T12:00:00-04:00"
            ))
            .unwrap()
            .with_timezone(&Local)
                + chrono::Duration::nanoseconds(1))
        );
        let rows = evaluation.into_rows(false);
        assert!(
            rows[0].group == group,
            "closed={closed}, waits={waits}, kinds={kinds:?}"
        );
        assert!(rows[0].needs_you_kind == kind);
        if !waits {
            assert!(
                rows[0]
                    .left_line()
                    .unwrap()
                    .contains("findings triaged at 2026-10-18T12:00:00-04:00")
            );
        }
        if kinds.len() == 2 {
            assert_eq!(
                rows[0].dismissal.as_ref().unwrap().dismissed_at,
                "2026-10-17T12:00:00-04:00"
            );
        }
    }
}

#[test]
fn inbox_evaluation_keeps_history_and_current_rows_independent_on_a_shared_url() {
    let dir = tempfile::tempdir().unwrap();
    let url = "https://github.com/acme/widgets/pull/9";
    for (number, ended_at) in [
        (7, "2026-09-19T12:00:00-04:00"),
        (8, "2026-10-19T11:30:00-04:00"),
    ] {
        let mut ready = ended(record(number), "ready for review");
        ready["issue"] = Value::Null;
        ready["started_at"] = "2026-09-18T11:00:00-04:00".into();
        ready["ended_at"] = ended_at.into();
        ready["pull_request"] = url.into();
        write(dir.path(), &format!("{number}-shared.command.json"), &ready);
    }
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    let history = snapshot
        .resolve(&format!(
            "{}::acme/widgets#7",
            dir.path()
                .join("acme/widgets/commands/issue/7-shared.command.json")
                .display()
        ))
        .unwrap()
        .observation
        .selector()
        .unwrap();
    let merged =
        json!({"state": "Merged", "merged_at": "2026-09-20T12:00:00-04:00", "closed_at": null});
    let mut kept = kept_answer(
        dir.path(),
        json!({"version": 2, "subjects": {url: merged}, "answered_at": {url: "2026-10-18T12:00:00Z"}}),
    );
    let evaluation = snapshot.evaluate_inbox(7, &kept);
    assert_eq!(
        evaluation.refresh_work().subjects.len(),
        1,
        "current row still gets refreshed"
    );
    let freezes = &evaluation.refresh_work().freezes;
    assert_eq!(freezes.len(), 1);
    assert_eq!(freezes[0].row, history);
    assert_eq!(freezes[0].url, url);
    for freeze in freezes {
        kept.freeze(freeze.row.clone(), &freeze.url);
    }
    // The result owns metadata and targets, rather than borrowing the kept answer.
    drop(kept);
    assert_eq!(
        evaluation.next_change(),
        Some(now() + chrono::Duration::minutes(1410) + chrono::Duration::nanoseconds(1))
    );
    let rows = evaluation.into_rows(true);
    assert!(rows[0].group == Group::Done);
    assert!(rows[1].group == Group::History);

    // Exact row/URL freezes, unresolved answers and legacy row keys all
    // override newer shared facts without freezing the current Command.
    for legacy in [false, true] {
        for old_answer in [merged.clone(), Value::Null] {
            let key = if legacy {
                history.clone()
            } else {
                format!("{history}::{url}")
            };
            let kept = kept_answer(
                dir.path(),
                json!({
                    "version": 2,
                    "subjects": {url: {"state": "Open", "merged_at": null, "closed_at": null}},
                    "history": {key.clone(): old_answer},
                    "answered_at": {key: "2026-10-18T12:00:00Z", url: "2026-10-19T16:00:00Z"}
                }),
            );
            let evaluation = snapshot.evaluate_inbox(7, &kept);
            assert_eq!(
                evaluation
                    .refresh_work()
                    .subjects
                    .iter()
                    .map(|subject| subject.url.as_str())
                    .collect::<Vec<_>>(),
                [url]
            );
            assert!(evaluation.refresh_work().freezes.is_empty());
            let rows = evaluation.into_rows(true);
            assert!(rows[0].group == Group::ReadyForReview);
            assert_eq!(rows[0].github_state.as_deref(), Some("open"));
            assert!(rows[1].group == Group::History);
            assert_eq!(
                rows[1].github_state.as_deref(),
                old_answer.is_object().then_some("merged")
            );
            let value = serde_json::to_value(&rows[1]).unwrap();
            assert_eq!(
                value["github"]["subjects"][url]["answered_at"],
                "2026-10-18T12:00:00Z"
            );
        }
    }
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
    assert_eq!(
        snapshot
            .evaluate_inbox(7, &crate::logs::github::Kept::default())
            .into_rows(false)
            .len(),
        2,
        "{kind}"
    );
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
