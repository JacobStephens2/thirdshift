//! Review reproductions for issue #2278.
mod support;

use support::Scenario;

#[test]
fn recommending_triage_does_not_discard_a_failed_runs_retained_scratch() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "printf '%s' \"$TMPDIR\" > '{}'\nexit 3\n",
        scenario.path("failed-tmp").display(),
    ));
    let failed = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(failed.code, Some(1), "{}", failed.stderr);
    assert!(failed.stderr.contains("Recovery:"), "{}", failed.stderr);
    let retained =
        std::path::PathBuf::from(std::fs::read_to_string(scenario.path("failed-tmp")).unwrap());
    assert!(retained.is_dir(), "failure did not retain scratch");

    scenario.repo_has_labels(&["bug"]);
    let recommendation = r###"{"category":"bug","state":"ready-for-agent","issues":[{"title":"Handle an empty cache","body":"## Agent Brief\n\nReproduce with an empty cache."}],"investigation":"A recommendation only; no implementation was continued."}"###;
    scenario.agent_does(&format!(
        "printf '%s\\n' 'Triage recommendation: {recommendation}' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n",
    ));
    let triage = scenario.run(&["triage", "7", "--recommend-only"]);
    assert_eq!(triage.code, Some(0), "{}", triage.stderr);
    assert!(
        retained.is_dir(),
        "recommendation-only Triage deleted the failed run's retained scratch: {}\n{}",
        retained.display(),
        triage.stderr,
    );
}
