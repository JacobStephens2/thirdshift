//! Focused issue #1796 review reproductions through the public Fix Pass.
mod support;

use serde_json::json;
use support::Scenario;

#[test]
fn failed_push_with_no_jobs_is_repaired_despite_an_unrelated_green_status() {
    let scenario = Scenario::new();
    let head = scenario
        .origin_git(&["rev-parse", "main"])
        .trim()
        .to_string();
    let mut state = scenario.gh_state();
    state["runs"] = json!([{
        "id": 900, "head_branch": "main", "event": "push", "head_sha": head,
        "html_url": "https://github.com/acme/widgets/actions/runs/900",
        "status": "completed", "conclusion": "failure", "updated_at": "2020-01-01T00:00:00Z",
        "run_attempt": 1, "jobs": []
    }]);
    state["statuses"][&head] = json!([{
        "context": "external-lint", "state": "success", "url": "https://ci.example/passed"
    }]);
    scenario.write_gh_state(&state);
    scenario.agent_does(
        "echo fixed > ci-fix.txt\ngit add ci-fix.txt\ngit commit -qm 'Fix invalid workflow'",
    );

    let result = scenario.run(&["fix", "no-merge", "no-email"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        scenario.claude_calls().len(),
        1,
        "failed push workflow must start repair despite a passing external status: {}",
        result.stderr
    );
    assert_eq!(scenario.gh_state()["prs"][0]["base"], "main");
}
