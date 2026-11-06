mod support;

use serde_json::json;
use support::Scenario;

#[test]
fn fix_checks_review_evidence_for_the_fetched_head() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("colleague", "main", &["Reviewed feature"]);
    let url = scenario.github_has_pr("colleague", "main", "OPEN");
    let original = scenario.origin_git(&["rev-parse", "colleague"]);
    scenario.origin_has_branch("concurrent", "colleague", &["Unreviewed follow-up"]);
    let foreign = scenario.origin_git(&["rev-parse", "concurrent"]);
    let mut state = scenario.gh_state();
    state["prs"][0]["comments"] = json!([{
        "body": format!("<!-- thirdshift-review {} -->", json!({
            "reviewed_head": original.trim()
        })),
        "author_association": "COLLABORATOR",
        "user": {"login": "runner"}
    }]);
    for head in [original.trim(), foreign.trim()] {
        state["checks"][head] = json!([{
            "name": "test", "conclusion": "success", "url": "https://ci.example/test"
        }]);
    }
    scenario.write_gh_state(&state);
    scenario.agent_does("printf 'No findings.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
    let marker = scenario.path("advanced-at-fetch");
    support::write_executable(
        &scenario.path("bin/git"),
        &format!(
            r#"#!/bin/sh
if [ "$1" = fetch ] && [ ! -e '{marker}' ]; then
  /usr/bin/git --git-dir='{origin}' update-ref refs/heads/colleague '{foreign}' || exit 1
  touch '{marker}'
fi
exec /usr/bin/git "$@"
"#,
            marker = marker.display(),
            origin = scenario.origin_dir().display(),
            foreign = foreign.trim(),
        ),
    );

    let result = scenario.run(&["fix", &url, "--merge", "--no-email"]);
    assert!(
        marker.is_file(),
        "acquisition race did not run: {}",
        result.stderr
    );
    let merged = scenario.gh_state()["prs"][0]["state"] == "MERGED";
    let reviewed_foreign = scenario.claude_calls().iter().any(|call| {
        call["prompt"]
            .as_str()
            .unwrap_or("")
            .contains("/thirdshift-code-review")
    });
    assert!(
        !merged || reviewed_foreign,
        "ADR-0004: merged a fetched head covered only by an older head's review; {}",
        result.stderr
    );
}

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
