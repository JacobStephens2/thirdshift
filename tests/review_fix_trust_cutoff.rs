mod support;

use serde_json::json;
use std::process::Command;
use support::Scenario;

#[test]
fn slow_worktree_acquisition_keeps_the_fix_run_trust_window() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("colleague", "main", &["Colleague feature"]);
    let url = scenario.github_has_pr("colleague", "main", "OPEN");
    let head = scenario.origin_git(&["rev-parse", "colleague"]);
    let mut state = scenario.gh_state();
    state["checks"][head.trim()] = json!([{"name":"test","conclusion":"failure"}]);
    state["titles"]["7"] = json!("Export widgets");
    state["bodies"]["7"] = json!("Untrusted body written as this Command starts.");
    state["edits"]["7"] = json!([{
        "kind": "body", "by": "outsider", "at": chrono::Utc::now().to_rfc3339()
    }]);
    state["prs"][0]["closingIssuesReferences"] = json!([{"url": scenario.issue_url(7)}]);
    scenario.write_gh_state(&state);
    scenario.agent_does("exit 1");

    let located = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    assert!(located.status.success());
    let real_git = String::from_utf8(located.stdout).unwrap();
    let real_git = real_git.trim();
    let library = scenario.path("fix-clock.so");
    support::clock::compile(&library);
    let offset = scenario.path("fix-clock-offset");
    std::fs::write(&offset, "0").unwrap();
    support::write_executable(
        &scenario.path("bin/git"),
        r#"#!/bin/sh
"$THIRDSHIFT_REVIEW_REAL_GIT" "$@"
result=$?
if [ "$result" -ne 0 ]; then exit "$result"; fi
case " $* " in
  *" worktree add "*) printf 600 > "$THIRDSHIFT_REVIEW_CLOCK_OFFSET" ;;
esac
exit "$result"
"#,
    );
    let result = scenario.run_with_env(
        &["fix", &url, "no-merge", "no-base-fix", "no-email"],
        &[
            ("LD_PRELOAD", library.to_str().unwrap()),
            ("THIRDSHIFT_REVIEW_CLOCK_OFFSET", offset.to_str().unwrap()),
            ("THIRDSHIFT_REVIEW_REAL_GIT", real_git),
        ],
    );
    assert_eq!(std::fs::read_to_string(offset).unwrap(), "600");
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        scenario.claude_calls().is_empty(),
        "an edit inside the Command's trust window reached a Repair: {}",
        result.stderr
    );
    assert!(
        result
            .stderr
            .contains("#7 body edited since the Fix run started"),
        "{}",
        result.stderr
    );
}
