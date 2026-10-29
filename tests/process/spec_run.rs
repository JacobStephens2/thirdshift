use super::*;

#[test]
fn the_compiled_spec_run_is_listed_shown_and_passes_check() {
    let scenario = Scenario::new();
    scenario.spec_has_tickets(7, &[(8, &[])]);
    let source = assert_compiled_process(
        &scenario,
        "spec-run",
        &[
            "land-tickets",
            "spec-review",
            "security-review",
            "delivery",
            "self-merge",
        ],
        Some("7"),
    );
    assert!(source.contains("kind = \"spec-branch\""));
}
