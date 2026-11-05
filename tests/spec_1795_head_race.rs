mod support;

use serde_json::json;
use support::Scenario;

#[test]
fn fix_review_evidence_covers_the_head_actually_adopted() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("colleague", "main", &["Reviewed feature"]);
    let url = scenario.github_has_pr("colleague", "main", "OPEN");
    let reviewed = scenario.origin_git(&["rev-parse", "colleague"]);
    scenario.origin_has_branch("concurrent", "colleague", &["Unreviewed follow-up"]);
    let unreviewed = scenario.origin_git(&["rev-parse", "concurrent"]);
    let mut state = scenario.gh_state();
    state["prs"][0]["comments"] = json!([{
        "body": format!("<!-- thirdshift-review {} -->", json!({"reviewed_head":reviewed.trim()})),
        "author_association":"COLLABORATOR", "user":{"login":"runner"}
    }]);
    state["checks"][unreviewed.trim()] = json!([
        {"name":"test","conclusion":"success","url":"https://ci.example/test"}
    ]);
    scenario.write_gh_state(&state);
    let advanced = scenario.path("advanced-at-first-fetch");
    support::write_executable(
        &scenario.path("bin/git"),
        &format!(
            "#!/bin/sh\n\
             if [ \"$1\" = fetch ] && [ ! -e '{advanced}' ]; then\n\
               touch '{advanced}'\n\
               /usr/bin/git --git-dir='{origin}' update-ref refs/heads/colleague {unreviewed}\n\
             fi\n\
             exec /usr/bin/git \"$@\"\n",
            advanced = advanced.display(),
            origin = scenario.path("origin.git").display(),
            unreviewed = unreviewed.trim(),
        ),
    );

    let result = scenario.run(&["fix", &url, "--merge", "--no-email"]);

    assert!(advanced.exists(), "the concurrent push did not run");
    let merged = scenario.gh_state()["prs"][0]["state"] == "MERGED";
    assert!(
        !merged || !scenario.claude_calls().is_empty(),
        "merged unreviewed adopted head {} using evidence for {} without an agent session; exit {:?}: {}",
        unreviewed.trim(),
        reviewed.trim(),
        result.code,
        result.stderr
    );
}
