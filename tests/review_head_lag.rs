//! Review Delivery must wait for its own push to appear in GitHub's PR API.

mod support;

use serde_json::json;
use support::Scenario;

fn pull_request(scenario: &Scenario) -> String {
    scenario.origin_has_branch("colleague", "main", &["Colleague feature"]);
    scenario.github_has_pr("colleague", "main", "OPEN")
}

fn lag_head(scenario: &Scenario, reads: i64) {
    let old = scenario.origin_git(&["rev-parse", "colleague"]);
    let mut state = scenario.gh_state();
    state["prs"][0]["headLag"] = json!({"head": old.trim(), "reads": reads});
    scenario.write_gh_state(&state);
}

const FIX: &str = r#"
echo review-fix > review-fix.txt
git add review-fix.txt
git commit -qm 'Address review finding'
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"success"}]'
printf 'Standards: fixed export. Spec: no findings.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#;

#[test]
fn review_delivers_when_github_reports_the_pre_push_head_for_multiple_reads() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    lag_head(&scenario, 2);
    scenario.agent_does(FIX);

    let result = scenario.run_with_env(&["review", &url], &[("THIRDSHIFT_CI_GRACE_MS", "10000")]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.gh_state()["prs"][0]["headLag"]["reads"], 0);
    assert_eq!(
        scenario
            .origin_file("colleague", "review-fix.txt")
            .as_deref(),
        Some("review-fix\n")
    );
    let (_, notification) = support::resend::saved_notification(&scenario, "review");
    assert!(notification.contains("ready for review"), "{notification}");
}

#[test]
fn review_times_out_on_a_head_that_never_reflects_its_push() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    lag_head(&scenario, 1_000_000);
    scenario.agent_does(FIX);

    // Zero grace expires the observation wait immediately, without a timer race.
    let result = scenario.run_with_env(&["review", &url], &[("THIRDSHIFT_CI_GRACE_MS", "0")]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("timed out waiting for GitHub to report pushed head"),
        "{}",
        result.stderr
    );
    let pushed = scenario.origin_git(&["rev-parse", "colleague"]);
    let state = scenario.gh_state();
    let comment = state["prs"][0]["postedComments"][0].as_str().unwrap();
    assert!(
        comment.contains(pushed.trim()) && comment.contains("pushed_head"),
        "{comment}"
    );
    let (_, notification) = support::resend::saved_notification(&scenario, "review");
    assert!(!notification.contains("ready for review"), "{notification}");
}

#[test]
fn a_foreign_push_after_our_push_names_the_expected_and_actual_heads() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario.origin_has_branch("concurrent", "colleague", &["Author work"]);
    let foreign = scenario.origin_git(&["rev-parse", "concurrent"]);
    scenario.repo_has_hook(
        &scenario.path("origin.git"),
        "post-receive",
        "#!/bin/sh\nwhile read old new ref; do\n  if test \"$ref\" = refs/heads/colleague; then\n    git update-ref refs/heads/colleague refs/heads/concurrent\n  fi\ndone\n",
    );
    scenario.agent_does(FIX);

    let result = scenario.run(&["review", &url]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("head moved; expected"),
        "{}",
        result.stderr
    );
    assert!(
        result
            .stderr
            .contains(&format!("actual {}", foreign.trim())),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.origin_git(&["rev-parse", "colleague"]), foreign);
    let (_, notification) = support::resend::saved_notification(&scenario, "review");
    assert!(!notification.contains("ready for review"), "{notification}");
}

#[test]
fn review_waits_for_the_ci_repairs_push_too() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    lag_head(&scenario, 2);
    let head = scenario.origin_git(&["rev-parse", "colleague"]);
    let mut state = scenario.gh_state();
    state["checks"][head.trim()] = json!([{"name":"test","conclusion":"failure"}]);
    scenario.write_gh_state(&state);
    scenario.agent_does("printf 'No findings.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
    scenario.agent_does_in_session(2, FIX);

    let result = scenario.run_with_env(&["review", &url], &[("THIRDSHIFT_CI_GRACE_MS", "10000")]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.gh_state()["prs"][0]["headLag"]["reads"], 0);
    assert_eq!(scenario.claude_calls().len(), 2);
    let state = scenario.gh_state();
    let comment = state["prs"][0]["postedComments"][0].as_str().unwrap();
    assert!(
        comment.contains("Repair 1") && comment.contains("CI passed"),
        "{comment}"
    );
}
