//! Pass author exclusions through the CLI, with GitHub at the fake boundary.
mod support;

use serde_json::json;
use support::Scenario;

#[test]
fn pickup_passes_over_a_skipped_authors_whole_spec() {
    let scenario = Scenario::new();
    scenario.spec_has_tickets(7, &[(8, &[])]);
    scenario.issue_labelled(7, &["ready-for-agent"]);
    let mut state = scenario.gh_state();
    state["issue_authors"] = json!({"7": "Alice", "8": "carol"});
    scenario.write_gh_state(&state);

    let result = scenario.run(&["pickup", "--skip-author", "alice"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("#7 passed over: written by Alice, whom this Pass skips"),
        "{}",
        result.stderr
    );
    assert!(
        result.stderr.contains("no Ready issue"),
        "{}",
        result.stderr
    );
    assert!(scenario.claude_calls().is_empty());
    assert!(scenario.origin_log("issue-7").is_none());
    assert_eq!(scenario.issue_labels(7), ["ready-for-agent"]);
}

#[test]
fn an_explicit_issue_url_runs_an_author_a_prior_pass_skipped() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["ready-for-agent"]);
    let mut state = scenario.gh_state();
    state["issue_authors"] = json!({"7": "Alice"});
    scenario.write_gh_state(&state);
    let skipped = scenario.run(&["pickup", "skip-author", "alice"]);
    assert_eq!(skipped.code, Some(0), "{}", skipped.stderr);
    assert!(
        skipped.stderr.contains("whom this Pass skips"),
        "{}",
        skipped.stderr
    );
    assert!(scenario.claude_calls().is_empty());
    scenario.agent_does(
        r#"
echo work > work.txt
git add work.txt
git commit -qm 'Do the work'
gh pr create --base main --head issue-7 --title Work --body 'Closes #7'
"#,
    );

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(result.stdout.contains("/pull/"), "{}", result.stdout);
}

#[test]
fn pass_exclusions_never_make_untrusted_authors_eligible() {
    for (command, label) in [("pickup", "ready-for-agent"), ("triage", "needs-triage")] {
        for words in [vec![command], vec![command, "skip-author", "carol"]] {
            let scenario = Scenario::new();
            scenario.issue_labelled(7, &[label]);
            let mut state = scenario.gh_state();
            state["issue_authors"] = json!({"7": "outsider"});
            state["author_associations"] = json!({"7": "NONE"});
            scenario.write_gh_state(&state);

            let result = scenario.run(&words);

            assert_eq!(result.code, Some(0), "{}", result.stderr);
            assert!(
                result.stderr.contains("author is not a Trusted account"),
                "{}",
                result.stderr
            );
            assert!(scenario.claude_calls().is_empty());
            assert_eq!(scenario.issue_labels(7), [label]);
        }
    }
}

#[test]
fn skip_author_requires_a_login_and_is_restricted_to_pickup_and_triage_passes() {
    let scenario = Scenario::new();
    for words in [
        vec!["pickup", "skip-author"],
        vec!["triage", "--skip-author"],
        vec!["pickup", "skip-author", "--merge"],
        vec!["triage", "skip-author", ""],
        vec!["secure", "skip-author", "alice"],
        vec!["architect", "--skip-author", "alice"],
        vec!["architect", "skip-author"],
        vec!["fix", "skip-author", "alice"],
        vec!["review", "--skip-author", "alice"],
        vec!["7", "skip-author", "alice"],
        vec!["triage", "7", "skip-author", "alice"],
        vec!["triage", "--file", "report.txt", "skip-author", "alice"],
    ] {
        let result = scenario.run(&words);
        assert_eq!(result.code, Some(2), "{words:?}: {}", result.stderr);
    }
    let result = scenario.run_with_input(&["triage", "skip-author", "alice"], "A Report\n");
    assert_eq!(result.code, Some(2), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("skip-author applies only to a Triage Pass")
    );
    assert!(scenario.claude_calls().is_empty());
}

#[test]
fn pickup_spec_stops_before_a_skipped_authors_ticket_as_for_an_untrusted_author() {
    let scenario = Scenario::new();
    scenario.spec_has_tickets(7, &[(8, &[])]);
    scenario.issue_labelled(7, &["ready-for-agent"]);
    let mut state = scenario.gh_state();
    state["issue_authors"] = json!({"7": "carol", "8": "Alice"});
    scenario.write_gh_state(&state);
    scenario.agent_does("exit 45");

    let result = scenario.run(&["pickup", "skip-author", "alice"]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("Ticket #8: written by Alice, whom this Pass skips"),
        "{}",
        result.stderr
    );
    assert!(scenario.claude_calls().is_empty());
    assert!(scenario.origin_log("issue-8").is_none());
    assert_eq!(scenario.issue_labels(7), ["agent-failed"]);
}

#[test]
fn triage_skips_each_named_author_and_takes_the_next_issue() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["bug"]);
    for issue in [7, 8, 9] {
        scenario.issue_is(issue, "OPEN");
        scenario.issue_labelled(issue, &["needs-triage"]);
    }
    let mut state = scenario.gh_state();
    state["issue_authors"] = json!({"7": "Alice", "8": "bob", "9": "carol"});
    scenario.write_gh_state(&state);
    scenario.agent_does(r###"printf '%s\n' 'Triage recommendation: {"category":"bug","state":"ready-for-agent","issues":[{"title":"Handle an empty cache","body":"## Agent Brief\n\nReproduce with an empty cache."}],"investigation":"The cache read assumes an entry exists."}' > "$FAKE_CLAUDE_FINAL_MESSAGE""###);

    let result = scenario.run(&["triage", "--skip-author", "alice", "skip-author", "BOB"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    for (issue, login) in [(7, "Alice"), (8, "bob")] {
        assert!(
            result.stderr.contains(&format!(
                "#{issue} passed over: written by {login}, whom this Pass skips"
            )),
            "{}",
            result.stderr
        );
        assert_eq!(scenario.issue_labels(issue), ["needs-triage"]);
    }
    assert!(
        result
            .stderr
            .contains(&format!("triaging {}", scenario.issue_url(9))),
        "{}",
        result.stderr
    );
    let mut labels = scenario.issue_labels(9);
    labels.sort();
    assert_eq!(labels, ["bug", "ready-for-human"]);
}

#[test]
fn pickup_skips_each_named_author_case_insensitively_and_takes_the_next_issue() {
    let scenario = Scenario::new();
    for issue in [7, 8, 9] {
        scenario.issue_is(issue, "OPEN");
        scenario.issue_labelled(issue, &["ready-for-agent"]);
    }
    let mut state = scenario.gh_state();
    state["issue_authors"] = json!({"7": "Alice", "8": "bob", "9": "carol"});
    state["author_associations"] = json!({"7": "MEMBER", "8": "COLLABORATOR"});
    scenario.write_gh_state(&state);
    scenario.agent_does_for(
        9,
        r#"
echo work > work.txt
git add work.txt
git commit -qm 'Do the work'
gh pr create --base main --head issue-9 --title Work --body 'Closes #9'
"#,
    );

    let result = scenario.run(&["pickup", "skip-author", "ALICE", "--skip-author", "Bob"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    for (issue, login) in [(7, "Alice"), (8, "bob")] {
        assert!(
            result.stderr.contains(&format!(
                "#{issue} passed over: written by {login}, whom this Pass skips"
            )),
            "{}",
            result.stderr
        );
        assert_eq!(scenario.issue_labels(issue), ["ready-for-agent"]);
        assert!(scenario.origin_log(&format!("issue-{issue}")).is_none());
    }
    assert!(
        result.stderr.contains("taking Ready issue #9"),
        "{}",
        result.stderr
    );
    assert!(result.stdout.contains("/pull/"), "{}", result.stdout);
}
