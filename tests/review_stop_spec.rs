mod support;

use support::Scenario;

#[test]
fn stopping_a_fresh_numbered_branch_does_not_keep_a_claim_for_a_finished_pr() {
    for previous in ["MERGED", "CLOSED"] {
        let scenario = Scenario::new();
        scenario.issue_labelled(7, &["bug", "ready-for-agent"]);
        scenario.github_has_pr("issue-7", "main", previous);
        scenario.agent_does(&format!(
            "touch '{}'\nwhile true; do sleep 0.05; done\n",
            scenario.path("started").display()
        ));
        let mut held = scenario.run_until(&[&scenario.issue_url(7)], &[], "started");
        let stopped = scenario.run(&["stop", "widgets#7"]);
        if stopped.code != Some(0) {
            held.signal("TERM");
        }
        let ended = held.finish();
        assert_eq!(stopped.code, Some(0), "{}", stopped.stderr);
        assert_eq!(ended.code, Some(1), "{}", ended.stderr);
        assert_eq!(scenario.claude_calls()[0]["branch"], "issue-7-branch-2");
        assert!(scenario.origin_log("issue-7").is_none());
        assert!(scenario.origin_log("issue-7-branch-2").is_none());
        assert_eq!(
            scenario.issue_labels(7),
            ["bug", "needs-triage"],
            "previous PR state: {previous}"
        );
    }
}
