mod support;

use serde_json::json;
use support::Scenario;

#[test]
fn fix_self_merge_requires_review_evidence() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("unreviewed", "main", &["Manually authored feature"]);
    let url = scenario.github_has_pr("unreviewed", "main", "OPEN");
    let head = scenario.origin_git(&["rev-parse", "unreviewed"]);
    let mut state = scenario.gh_state();
    state["checks"][head.trim()] = json!([
        {"name":"test","conclusion":"success","url":"https://ci.example/test"}
    ]);
    scenario.write_gh_state(&state);

    let result = scenario.run(&["fix", &url, "--merge", "--no-email"]);
    let merged = scenario.gh_state()["prs"][0]["state"] == "MERGED";
    assert!(
        !merged || !scenario.claude_calls().is_empty(),
        "ADR-0004: merged a manually authored head without any agent review; exit {:?}: {}",
        result.code,
        result.stderr
    );
}

#[test]
fn an_unreviewed_adopted_head_stays_ready_with_recovery_after_the_hold() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("unreviewed", "main", &["Manually authored feature"]);
    let url = scenario.github_has_pr("unreviewed", "main", "OPEN");
    let result = scenario.run(&["fix", &url, "--merge", "--no-email"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("has no recorded agent review"),
        "{}",
        result.stderr
    );
    assert!(
        result
            .stderr
            .contains(&format!("Recovery: thirdshift fix {url}")),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "OPEN");
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], false);
    assert!(scenario.claude_calls().is_empty());
    assert!(
        scenario
            .launch_git(&["ls-remote", "origin", "refs/thirdshift/reservations/*"])
            .is_empty()
    );
}
