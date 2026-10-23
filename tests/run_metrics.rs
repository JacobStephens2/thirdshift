mod support;

use std::fs;

use serde_json::Value;
use support::Scenario;

const LOGS: &str = "home/.thirdshift/logs/acme/widgets/commands/issue";

#[test]
fn a_run_records_session_trace_signals_at_completion() {
    let scenario = Scenario::new();
    scenario.agent_does(r#"
echo '{"type":"assistant","message":{"id":"m1","model":"reported-model","content":[],"usage":{"input_tokens":8,"output_tokens":4,"cache_read_input_tokens":2,"cache_creation_input_tokens":3}}}'
echo work > work.txt
git add work.txt
git commit -qm 'Make widgets'
gh pr create --base main --head issue-7 --title 'Make widgets' --body 'Closes #7'
"#);
    let ended = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let saved = scenario.command_record(LOGS);
    let summary = &saved["run_metrics"]["session_signals"]["implement"];
    assert_eq!(summary["observed_sessions"], 1);
    assert_eq!(summary["missing_sessions"], 0);
    assert_eq!(
        summary["signals"]["tokens_by_model"],
        serde_json::json!([{
            "model":"reported-model", "input_tokens":13, "output_tokens":4,
            "cache_read_tokens":2, "cache_write_tokens":3
        }])
    );
    for name in scenario.entries(LOGS) {
        if name.ends_with(".logs.jsonl") || name.ends_with(".spans.jsonl") {
            fs::remove_file(scenario.path(LOGS).join(name)).unwrap();
        }
    }
    let shown = scenario.run(&["metrics", "widgets#7", "--json"]);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    assert_eq!(shown["command"]["metrics"], saved["run_metrics"]);
}

#[test]
fn spec_review_nested_evidence_heading_preserves_the_findings_axis() {
    let scenario = Scenario::new();
    scenario.agent_does(
        r#"
echo work > work.txt
git add work.txt
git commit -qm 'Make widgets'
gh pr create --base main --head issue-7 --title 'Make widgets' --body 'Closes #7

## Unaddressed findings
### Standards
- Naming: retained because the glossary defines it.

  #### Evidence
  - The glossary uses the same term.

- Duplication: the adapters have different contracts.

### Spec
- None.
### Security
- None.'
"#,
    );
    let ended = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    assert_eq!(
        scenario.command_record(LOGS)["run_metrics"]["unaddressed_findings"],
        serde_json::json!({"standards": 2, "spec": 0, "security": 0})
    );
}

#[test]
fn a_run_counts_unaddressed_findings_by_kind_from_its_final_pr_body() {
    let scenario = Scenario::new();
    scenario.agent_does(
        r#"
echo work > work.txt
git add work.txt
git commit -qm 'Make widgets'
gh pr create --base main --head issue-7 --title 'Make widgets' --body 'Closes #7

## Unaddressed findings

### Standards
- Naming: the glossary uses this name.
  - Evidence: see the glossary.
- Duplication: each adapter has a different contract.

### Spec
1. Deferred behavior: see the linked triage issue.

### Security
- Input bound: the command exercises the bound.

## Evidence
- This is evidence, not a finding.'
"#,
    );

    let ended = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let record = scenario.command_record(LOGS);
    let counts = &record["run_metrics"]["unaddressed_findings"];
    assert_eq!(
        *counts,
        serde_json::json!({"standards": 2, "spec": 1, "security": 1})
    );

    let mut state = scenario.gh_state();
    state["prs"][0]["body"] = "The Day shift edited this later.".into();
    scenario.write_gh_state(&state);
    let shown = scenario.run(&["show", "widgets#7", "--json"]);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    assert_eq!(
        shown["command"]["run_metrics"]["unaddressed_findings"],
        *counts
    );
}

#[test]
fn findings_accept_markdown_axis_labels_and_do_not_count_none_as_a_finding() {
    for axes in [
        "### **Standards**:\n- Naming: glossary evidence.\n### Spec\n- None.\n### Security\n- Input bound: focused run.",
        "**Standards**\n- Naming: glossary evidence.\n**Spec**\nNone.\n**Security**\n- Input bound: focused run.",
        "- **Standards**\n  - Naming: glossary evidence.\n    - Nested evidence.\n- **Spec**\n  - None.\n- **Security**\n  - Input bound: focused run.",
    ] {
        let scenario = Scenario::new();
        scenario.agent_does(&format!(
            "echo work > work.txt\ngit add work.txt\ngit commit -qm 'Make widgets'\ngh pr create --base main --head issue-7 --title 'Make widgets' --body 'Closes #7\n\n## Unaddressed findings\n{axes}'\n"
        ));
        let ended = scenario.run(&[&scenario.issue_url(7)]);
        assert_eq!(ended.code, Some(0), "{}", ended.stderr);
        let record = scenario.command_record(LOGS);
        assert_eq!(
            record["run_metrics"]["unaddressed_findings"],
            serde_json::json!({"standards": 1, "spec": 0, "security": 1}),
            "{axes}"
        );
    }
}

#[test]
fn a_fake_review_keeps_its_finding_counts_in_the_runs_metrics() {
    let scenario = Scenario::new();
    scenario.agent_does(
        r#"
reports=$(printf '%s\n' "$FAKE_CLAUDE_PROMPT" | sed -n 's/.*`\([^`]*\)\/standards\.md`.*/\1/p')
printf '{"standards":2,"spec":0}\n' > "$reports/summary.json"
echo work > work.txt
git add work.txt
git commit -qm 'Make widgets'
gh pr create --base main --head issue-7 --title 'Make widgets' --body 'Closes #7'
"#,
    );

    let ended = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let shown = scenario.run(&["show", "widgets#7", "--json"]);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    assert_eq!(
        shown["command"]["run_metrics"]["stages"][0]["review_findings"],
        serde_json::json!({"standards": 2, "spec": 0})
    );
    let metrics = &shown["command"]["run_metrics"];
    assert_eq!(
        metrics["stages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|stage| !stage["review_findings"].is_null())
            .count(),
        1,
        "Delivery stages must not duplicate the same Session's counts"
    );
    for name in scenario.entries(LOGS) {
        if name.ends_with(".review-summary.json") {
            fs::remove_file(scenario.path(LOGS).join(name)).unwrap();
        }
    }
    let later = scenario.run(&["show", "widgets#7", "--json"]);
    assert_eq!(later.code, Some(0), "{}", later.stderr);
    let later: Value = serde_json::from_str(&later.stdout).unwrap();
    assert_eq!(later["command"]["run_metrics"], *metrics);
}

#[test]
fn absent_or_invalid_review_summaries_keep_counts_missing() {
    for summary in [None, Some("not JSON"), Some(r#"{"standards":-1,"spec":0}"#)] {
        let scenario = Scenario::new();
        let write_summary = summary.map_or_else(String::new, |summary| {
            format!("printf '%s' '{summary}' > \"$reports/summary.json\"\n")
        });
        scenario.agent_does(&format!(
            r#"
reports=$(printf '%s\n' "$FAKE_CLAUDE_PROMPT" | sed -n 's/.*`\([^`]*\)\/standards\.md`.*/\1/p')
printf 'Standards: 99 findings\n' > "$reports/standards.md"
printf 'Spec: 88 findings\n' > "$reports/spec.md"
{write_summary}
echo work > work.txt
git add work.txt
git commit -qm 'Make widgets'
gh pr create --base main --head issue-7 --title 'Make widgets' --body 'Closes #7'
"#
        ));
        let ended = scenario.run(&[&scenario.issue_url(7)]);
        assert_eq!(ended.code, Some(0), "{}", ended.stderr);
        let record = scenario.command_record(LOGS);
        assert_eq!(
            record["run_metrics"]["stages"][0].get("review_findings"),
            Some(&Value::Null),
            "summary {summary:?} must not become zero counts or parse report prose"
        );
    }
}

#[test]
fn a_run_keeps_its_metrics_after_a_ci_fix_repair_and_show_returns_them() {
    let scenario = Scenario::new();
    scenario.agent_does(
        r#"
echo work > work.txt
git add work.txt
git commit -qm 'Make widgets'
gh pr create --base main --head issue-7 --title 'Make widgets' --body 'Closes #7'
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"failure"}]'
"#,
    );
    scenario.agent_does_in_session(
        2,
        r#"
echo fixed > work.txt
git add work.txt
git commit -qm 'Fix widgets'
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"success"}]'
gh api --method PATCH repos/acme/widgets/pulls/1 -f body='Closes #7

## Unaddressed findings
### Spec
- Deliberately deferred behavior; see the triage issue.'
"#,
    );

    let ended = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let record = scenario.command_record(LOGS);
    assert_eq!(record["version"], 2);
    let metrics = &record["run_metrics"];
    assert_eq!(metrics["outcome"], "ready for review");
    assert_eq!(metrics["repairs"], 1);
    assert_eq!(metrics["ci_rounds"], 2);
    assert_eq!(
        metrics["unaddressed_findings"],
        serde_json::json!({"standards": 0, "spec": 1, "security": 0})
    );
    let stages = metrics["stages"].as_array().expect("Run stage metrics");
    assert_eq!(stages.len(), record["stages"].as_array().unwrap().len());
    for stage in stages {
        assert!(stage["seconds"].as_f64().unwrap() >= 0.0);
        assert_eq!(stage["minutes"], stage["seconds"].as_f64().unwrap() / 60.0);
        assert_eq!(stage["position"], "too_few_runs");
        assert_eq!(stage["usual_range"], Value::Null);
    }
    assert!(metrics["total_minutes"].as_f64().unwrap() >= 0.0);

    let shown = scenario.run(&["show", "widgets#7", "--json"]);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    assert_eq!(shown["command"]["run_metrics"], *metrics, "{shown}");
}

#[test]
fn a_spec_run_and_each_ticket_keep_their_own_completion_metrics() {
    let scenario = Scenario::new();
    scenario.spec_has_tickets(20, &[(21, &[]), (22, &[21])]);
    scenario.agent_does_for(20, "true\n");
    for ticket in [21, 22] {
        scenario.agent_does_for(ticket, &format!(
            "echo {ticket} > ticket-{ticket}.txt\ngit add .\ngit commit -qm 'Ticket {ticket}'\ngh pr create --base issue-20 --head issue-{ticket} --title 'Ticket {ticket}' --body 'Closes #{ticket}'\n"
        ));
    }
    let ended = scenario.run(&[&scenario.issue_url(20)]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let spec = scenario.command_record(LOGS);
    assert_eq!(spec["run_metrics"]["outcome"], "ready for review");
    assert_eq!(spec["run_metrics"]["repairs"], 0);
    assert_eq!(spec["run_metrics"]["ci_rounds"], 1);
    assert_eq!(
        spec["run_metrics"]["unaddressed_findings"],
        serde_json::json!({"standards": 0, "spec": 0, "security": 0})
    );
    let children: Vec<Value> = scenario
        .entries(LOGS)
        .iter()
        .filter(|name| name.ends_with(".child.json"))
        .map(|name| {
            serde_json::from_slice(&fs::read(scenario.path(LOGS).join(name)).unwrap()).unwrap()
        })
        .collect();
    assert_eq!(children.len(), 2);
    for child in &children {
        assert_eq!(child["kind"], "Run");
        assert_eq!(child["run_metrics"]["outcome"], "merged");
        assert_eq!(child["run_metrics"]["repairs"], 0);
        assert_eq!(child["run_metrics"]["ci_rounds"], 1);
        assert!(child["run_metrics"]["total_minutes"].is_number());
        assert_eq!(
            child["run_metrics"]["unaddressed_findings"],
            serde_json::json!({"standards": 0, "spec": 0, "security": 0})
        );
        let shown = scenario.run(&[
            "show",
            &format!("widgets#{}", child["issue"]["number"]),
            "--json",
        ]);
        assert_eq!(shown.code, Some(0), "{}", shown.stderr);
        let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
        assert_eq!(shown["command"]["run_metrics"], child["run_metrics"]);
    }
    let shown = scenario.run(&["show", "widgets#20", "--json"]);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    assert_eq!(shown["command"]["run_metrics"], spec["run_metrics"]);
    assert_eq!(shown["command"]["children"].as_array().unwrap().len(), 2);
}

#[test]
fn readers_accept_older_command_records_without_metrics() {
    let scenario = Scenario::new();
    scenario.agent_does("exit 1\n");
    let ended = scenario.run(&[&scenario.issue_url(7)]);
    assert_ne!(ended.code, Some(0));
    let path = scenario
        .entries(LOGS)
        .into_iter()
        .find(|name| name.ends_with(".command.json"))
        .unwrap();
    let path = scenario.path(LOGS).join(path);
    let mut record = scenario.command_record(LOGS);
    assert_eq!(record["run_metrics"]["outcome"], "failed");
    assert_eq!(record["run_metrics"]["unaddressed_findings"], Value::Null);
    assert!(
        record["run_metrics"]
            .as_object()
            .unwrap()
            .contains_key("unaddressed_findings")
    );
    record["run_metrics"]
        .as_object_mut()
        .unwrap()
        .remove("unaddressed_findings");
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let shown = scenario.run(&["show", "widgets#7", "--json"]);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    assert_eq!(
        shown["command"]["run_metrics"]["unaddressed_findings"],
        Value::Null
    );
    record["version"] = 1.into();
    record.as_object_mut().unwrap().remove("run_metrics");
    let original = serde_json::to_vec(&record).unwrap();
    fs::write(&path, &original).unwrap();

    let shown = scenario.run(&["show", "widgets#7", "--json"]);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    assert_eq!(shown["command"]["source"], "command_record");
    assert_eq!(shown["command"]["outcome"], "failed");
    assert_eq!(shown["command"]["run_metrics"], Value::Null);
    assert_eq!(
        fs::read(&path).unwrap(),
        original,
        "readers must not write metrics"
    );
    fs::remove_file(path).unwrap();

    // Older records still supply Usual ranges to a later Run. The later Run
    // keeps the range it saw even when that history subsequently changes.
    record["started_at"] = "2026-10-20T12:00:00Z".into();
    record["ended_at"] = "2026-10-20T12:10:00Z".into();
    record["stages"] = serde_json::json!([{
        "name": "implement", "began_at": "2026-10-20T12:00:00Z", "session_log": null
    }]);
    for number in 1..=3 {
        fs::write(
            scenario
                .path(LOGS)
                .join(format!("{number}-history.command.json")),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
    }
    scenario.issue_titled(8, "Another Run");
    scenario.issue_is(8, "OPEN");
    let ended = scenario.run(&[&scenario.issue_url(8)]);
    assert_ne!(ended.code, Some(0));
    let shown = scenario.run(&["show", "widgets#8", "--json"]);
    assert_eq!(
        shown.code,
        Some(0),
        "{}\nRun: {}",
        shown.stderr,
        ended.stderr
    );
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    let metrics = &shown["command"]["run_metrics"];
    assert_eq!(metrics["outcome"], "failed");
    assert_eq!(metrics["stages"][0]["position"], "below");
    assert_eq!(
        metrics["stages"][0]["usual_range"],
        serde_json::json!({
            "lower_seconds": 600.0, "upper_seconds": 600.0
        })
    );
    for number in 1..=3 {
        fs::remove_file(
            scenario
                .path(LOGS)
                .join(format!("{number}-history.command.json")),
        )
        .unwrap();
    }
    let later = scenario.run(&["show", "widgets#8", "--json"]);
    assert_eq!(later.code, Some(0), "{}", later.stderr);
    let later: Value = serde_json::from_str(&later.stdout).unwrap();
    assert_eq!(later["command"]["run_metrics"], *metrics);
}
