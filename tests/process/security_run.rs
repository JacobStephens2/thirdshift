use super::*;

#[test]
fn the_compiled_security_run_is_listed_shown_and_passes_check() {
    let scenario = Scenario::new();
    assert_compiled_process(
        &scenario,
        "security-run",
        &[
            "security-audit",
            "reproduce-findings",
            "fix-findings",
            "close-findings",
        ],
        None,
    );
}
