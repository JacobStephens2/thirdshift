use serde_json::{Value, json};

use super::{AdmissionError, Cli, WireDocument};
use crate::logs::reading::Group;

fn wire(commands: Vec<Value>) -> Value {
    json!({
        "version": 2, "hostname": "owner-host", "active_runs": 3,
        "commands": commands, "owner_extension": {"retained": true},
        "github_answer_age_seconds": 300
    })
}

fn legacy() -> Value {
    json!({
        "source": "legacy_log", "repository": "acme/widgets", "kind": "Run",
        "name": "old", "state": "unknown", "started_at": "2026-10-22T11:00:00Z",
        "command_log": "/owner/legacy.log", "peer_extension": {"retained": true}
    })
}

fn record() -> Value {
    json!({
        "source": "command_record", "version": 1, "repository": "acme/widgets", "kind": "Run",
        "name": "ticket", "state": "done", "started_at": "2026-10-22T09:00:00Z",
        "ended_at": "2026-10-22T10:00:00Z", "command_log": "/owner/ticket.log",
        "hostname": "owner-host", "process_id": 0, "process_start": "gone",
        "harness": "codex", "stages": [], "children": [], "dispatched": []
    })
}

fn reservation() -> Value {
    json!({
        "source": "reservation", "repository": "acme/widgets", "kind": "Reservation",
        "selector": "/owner/reservation.json", "state": "held",
        "started_at": "2026-10-21T11:00:00Z", "reservation": {"reference": "origin"},
        "blocking": "Architect run", "owner_description": "Unknown owner",
        "release_command": "thirdshift release-reservation"
    })
}

fn grouped(mut value: Value, group: &str) -> Value {
    value["group"] = json!(group);
    value["landing"] = json!(false);
    value["stopping"] = json!(false);
    value["count"] = json!(1);
    value
}

fn document(value: Value) -> WireDocument {
    serde_json::from_value(value).unwrap()
}

#[test]
fn checked_document_preserves_wire_and_prepares_legacy_rows() {
    let mut command = grouped(legacy(), "needs you");
    command["needs_you_kind"] = json!("slow");
    command["landing"] = json!(true);
    command["dispatched"] = json!([legacy()]);
    command["collapsed"] = json!([]);
    let mut ready = grouped(record(), "ready for review");
    ready["pull_request"] = json!("https://github.com/acme/widgets/pull/7");
    ready["pull_request_title"] = json!("A recorded title");
    ready["end_time_estimated"] = json!(true);
    let held = grouped(reservation(), "needs you");
    let input = wire(vec![command, ready, held]);
    let cli = document(input.clone()).admit_cli("paired-owner").unwrap();
    assert_eq!(cli.commands[0].command().children().count(), 0);
    assert!(matches!(
        cli.commands[2].command().facts,
        Cli::Reservation(_)
    ));
    let document = document(input.clone())
        .admit_console("paired-owner")
        .unwrap();
    let mut rows = document.rows();
    let row = rows.next().unwrap();
    assert!(row.command.facts.named_group == Some(Group::NeedsYou));
    assert_eq!(row.landing_kind, Some("slow"));
    assert_eq!(row.command.selector(), "/owner/legacy.log::old");
    let child = row.command.children().next().unwrap();
    assert_eq!(child.selector(), "/owner/legacy.log::old");
    assert!(child.facts.group.is_none());
    let ready = rows.next().unwrap();
    assert!(ready.stale_github);
    assert_eq!(
        ready.command.selector(),
        "/owner/ticket.command.json::ticket"
    );
    assert_eq!(ready.command.ready_title(), Some("A recorded title"));
    assert!(ready.command.facts.recorded_finish().is_none());
    assert!(
        ready.command.facts.order
            == Group::ReadyForReview.row_order(None, Some("2026-10-22T10:00:00Z"))
    );
    let held = rows.next().unwrap().command;
    assert_eq!(held.selector(), "/owner/reservation.json");
    assert_eq!(held.facts.reservation.as_ref().unwrap().reference, "origin");
    let mut expected = input;
    for root in expected["commands"].as_array_mut().unwrap() {
        root["machine"] = json!("owner-host");
        root["machine_name"] = json!("paired-owner");
    }
    let dispatch = &mut expected["commands"][0]["dispatched"][0];
    dispatch["machine"] = json!("owner-host");
    dispatch["machine_name"] = json!("paired-owner");
    expected["machines"] = json!([]);
    assert_eq!(serde_json::to_value(&document).unwrap(), expected);
}

#[test]
fn structural_cli_and_console_errors_keep_their_precedence() {
    let mut root = grouped(record(), "working");
    root["dispatched"] = json!([{"children": [false]}]);
    let mut malformed_tree = wire(vec![root]);
    malformed_tree["version"] = json!(999);
    assert_eq!(
        document(malformed_tree).admit_console("owner").err(),
        Some(AdmissionError::Tree)
    );
    let mut incompatible = wire(vec![grouped(record(), "working")]);
    incompatible["version"] = json!(999);
    assert_eq!(
        document(incompatible).admit_cli("owner").err(),
        Some(AdmissionError::VersionOrRows)
    );

    let mut console_only = grouped(record(), "working");
    let mut nested = legacy();
    nested["group"] = json!(null);
    console_only["children"] = json!([nested]);
    let valid_cli = wire(vec![console_only.clone()]);
    assert!(document(valid_cli.clone()).admit_cli("owner").is_ok());
    assert_eq!(
        document(valid_cli).admit_console("owner").err(),
        Some(AdmissionError::Presentation)
    );

    let mut invalid_cli = grouped(record(), "working");
    invalid_cli["children"] = json!([{"source": "command_record"}]);
    assert_eq!(
        document(wire(vec![console_only, invalid_cli]))
            .admit_console("owner")
            .err(),
        Some(AdmissionError::VersionOrRows)
    );
}

#[test]
fn omission_edits_both_projections_and_preserves_owner_accounting() {
    let mut parent = grouped(record(), "needs you");
    parent["count"] = json!(2);
    let mut dispatched = legacy();
    dispatched["selector"] = json!("dispatch");
    let mut sibling = legacy();
    sibling["selector"] = json!("sibling");
    parent["dispatched"] = json!([dispatched, sibling]);
    parent["children"] = json!([legacy()]);
    let mut collapsed = record();
    collapsed["selector"] = json!("collapsed");
    collapsed["account"] = json!("other-account");
    parent["collapsed"] = json!([collapsed]);
    let mut document = document(wire(vec![parent, grouped(reservation(), "needs you")]))
        .admit_console("owner")
        .unwrap();
    let checked_parent = document.rows().next().unwrap().command;
    let visible = |command: super::ConsoleCommand<'_>| {
        command
            .children()
            .map(|child| child.selector().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        visible(checked_parent),
        ["dispatch", "sibling", "/owner/legacy.log::old"]
    );
    assert!(!checked_parent.collapsed_are_owned_records());
    let mut expected = serde_json::to_value(&document).unwrap();

    document.omit_command("/owner/legacy.log::old");
    document.omit_command("dispatch");
    document.omit_command("collapsed");
    document.omit_command("/owner/reservation.json");

    assert_eq!(document.active_runs(), 3);
    assert_eq!(document.rows().count(), 1);
    let checked_parent = document.rows().next().unwrap().command;
    assert_eq!(checked_parent.facts.count, 2);
    assert_eq!(visible(checked_parent), ["sibling"]);
    assert_eq!(checked_parent.dispatched_count(), 1);
    assert!(checked_parent.collapsed_are_owned_records());
    let commands = expected["commands"].as_array_mut().unwrap();
    commands.pop();
    commands[0]["dispatched"].as_array_mut().unwrap().remove(0);
    commands[0]["children"] = json!([]);
    commands[0]["collapsed"] = json!([]);
    assert_eq!(serde_json::to_value(document).unwrap(), expected);
}
