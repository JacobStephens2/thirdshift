use super::*;

#[test]
fn the_compiled_spec_run_is_listed_shown_and_passes_check() {
    let scenario = Scenario::new();
    scenario.spec_has_tickets(7, &[(8, &[])]);
    let shown = scenario.run(&["process", "show", "spec-run"]);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    assert!(shown.stdout.contains("kind = \"spec-branch\""));
    let mut remaining = shown.stdout.as_str();
    for step in [
        "land-tickets",
        "spec-review",
        "security-review",
        "delivery",
        "self-merge",
    ] {
        remaining = remaining
            .split_once(&format!("catalogue = \"{step}\""))
            .unwrap_or_else(|| panic!("missing or out-of-order Step {step}: {}", shown.stdout))
            .1;
    }
    for choice in ["harness =", "model =", "effort ="] {
        assert!(!shown.stdout.contains(choice), "{}", shown.stdout);
    }
    let listed = scenario.run(&["process", "list"]);
    assert_eq!(listed.code, Some(0), "{}", listed.stderr);
    assert!(listed.stdout.contains("spec-run (built-in):"));
    let checked = scenario.run(&["process", "check", "spec-run", "issue", "7"]);
    assert_eq!(checked.code, Some(0), "{}", checked.stderr);
    for step in [
        "land-tickets",
        "spec-review",
        "security-review",
        "delivery",
        "self-merge",
    ] {
        assert!(
            checked.stdout.contains(&format!("{step}: {step}")),
            "{}",
            checked.stdout
        );
    }
    assert!(scenario.claude_calls().is_empty());
}
