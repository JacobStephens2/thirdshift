use super::*;

#[test]
fn ticket_run_is_listed_and_shown_as_a_compiled_process() {
    let scenario = Scenario::new();
    let listed = scenario.run(&["process", "list"]);
    assert_eq!(listed.code, Some(0), "{}", listed.stderr);
    assert!(
        listed.stdout.contains("ticket-run (built-in):"),
        "{}",
        listed.stdout
    );
    let shown = scenario.run(&["process", "show", "ticket-run"]);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    let mut remaining = shown.stdout.as_str();
    for step in ["implement", "security-review", "delivery", "self-merge"] {
        remaining = remaining
            .split_once(&format!("catalogue = \"{step}\""))
            .unwrap_or_else(|| panic!("missing or out-of-order Step {step}: {}", shown.stdout))
            .1;
    }
    for choice in ["harness =", "model =", "effort ="] {
        assert!(!shown.stdout.contains(choice), "{}", shown.stdout);
    }
    assert!(scenario.claude_calls().is_empty());
}

#[test]
fn the_compiled_ticket_run_passes_check_and_its_printed_copy_runs_as_custom() {
    let scenario = Scenario::new();
    let checked = scenario.run(&["process", "check", "ticket-run", "issue", "7"]);
    assert_eq!(checked.code, Some(0), "{}", checked.stderr);
    for step in ["implement", "security-review", "delivery", "self-merge"] {
        assert!(
            checked.stdout.contains(&format!("{step}: {step}")),
            "{}",
            checked.stdout
        );
    }
    assert!(scenario.claude_calls().is_empty());
    let shown = scenario.run(&["process", "show", "ticket-run"]);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    write_process(&scenario, "my-ticket", &shown.stdout);
    scenario.agent_does_for_in_session(7, 1, COMMIT_FEATURE);
    let result = scenario.run(&["process", "my-ticket", "issue", "7"]);
    assert_left_ready(&scenario, &result);
    assert_eq!(
        process_command(&status_json(&scenario))["process_name"],
        "my-ticket"
    );
    scenario.assert_cleaned_up("issue-7");
}
