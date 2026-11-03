//! Review runs through the CLI, real Git and fake GitHub and Harnesses.

mod support;

use serde_json::json;
use support::Scenario;

fn pull_request(scenario: &Scenario) -> String {
    scenario.origin_has_branch("colleague", "main", &["Colleague feature"]);
    scenario.github_has_pr("colleague", "main", "OPEN")
}

#[test]
fn refuses_ineligible_pull_requests_before_starting_a_session() {
    for (field, value, reason) in [
        ("isCrossRepository", json!(true), "fork"),
        ("isDraft", json!(true), "draft"),
        ("state", json!("CLOSED"), "closed"),
        ("state", json!("MERGED"), "merged"),
        ("authorAssociation", json!("NONE"), "Trusted account"),
    ] {
        let scenario = Scenario::new();
        let url = pull_request(&scenario);
        let mut state = scenario.gh_state();
        state["prs"][0][field] = value;
        scenario.write_gh_state(&state);

        let result = scenario.run(&["review", &url, "--recommend-only"]);

        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(result.stderr.contains(reason), "{field}: {}", result.stderr);
        assert!(scenario.claude_calls().is_empty());
        assert_eq!(scenario.launch_git(&["status", "--porcelain"]), "");
    }
}

#[test]
fn recommend_only_posts_one_review_of_the_pr_head_and_keeps_it_in_the_inbox() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    let before = scenario.origin_log("colleague");
    scenario.agent_does(
        "test \"$(git branch --show-current)\" = colleague\n\
         test -s .thirdshift-review-input.md\n\
         printf '## Standards\\n\\nPossible duplicated code in feature.\\n\\n## Spec\\n\\nNo findings.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
    );

    let result = scenario.run(&["review", &url, "--recommend-only"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    let comments = state["prs"][0]["postedComments"].as_array().unwrap();
    assert_eq!(comments.len(), 1);
    let comment = comments[0].as_str().unwrap();
    assert!(comment.contains("Possible duplicated code"), "{comment}");
    assert!(comment.contains("pull request body alone"), "{comment}");
    let head = scenario.launch_git(&["ls-remote", "origin", "refs/heads/colleague"]);
    assert!(
        comment.contains(head.split_whitespace().next().unwrap()),
        "{comment}"
    );
    assert!(comment.contains("<!-- thirdshift-review"), "{comment}");
    assert_eq!(scenario.origin_log("colleague"), before);
    assert_eq!(scenario.launch_git(&["status", "--porcelain"]), "");
    let record = scenario.command_record("home/.thirdshift/logs/acme/widgets/commands/review");
    assert_eq!(record["kind"], "Review run");
    assert_eq!(record["process_name"], "review-run");
    assert_eq!(record["process_kind"], "built-in");
    assert_eq!(record["pull_request"], url);
    assert_eq!(record["outcome"], "ready for review");
    let (_, notification) = support::resend::saved_notification(&scenario, "review");
    assert!(notification.contains("Review run"), "{notification}");
    assert!(notification.contains(&url), "{notification}");
    let activity =
        std::fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
            .unwrap();
    assert!(activity.contains("Review run started:"), "{activity}");
    assert!(activity.contains("Review run ended:"), "{activity}");
    let status = scenario.run(&["status", "--json"]);
    assert_eq!(status.code, Some(0), "{}", status.stderr);
    assert!(
        status.stdout.contains("ready for review"),
        "{}",
        status.stdout
    );
    assert!(status.stdout.contains("Review run"), "{}", status.stdout);
}

#[test]
fn review_packet_uses_closing_issues_and_only_trusted_comments() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario.issue_is(8, "OPEN");
    let mut state = scenario.gh_state();
    state["titles"] = json!({"7": "Export widgets", "8": "Keep widget colors"});
    state["bodies"] = json!({"7": "Export CSV.", "8": "Blue widgets stay blue."});
    state["comments"] = json!({"7": ["Use UTF-8.", {"body": "issue outsider", "by": "outsider", "association": "NONE"}]});
    state["prs"][0]["body"] = json!("The author explains the change here.");
    state["prs"][0]["closingIssuesReferences"] = json!([
        {"url": scenario.issue_url(7)}, {"url": scenario.issue_url(8)},
    ]);
    for (field, trusted, outsider) in [
        ("comments", "PR discussion", "discussion outsider"),
        ("reviewComments", "Inline feedback", "inline outsider"),
        ("reviews", "Review summary", "review outsider"),
    ] {
        state["prs"][0][field] = json!([
            {"body": trusted, "author_association": "COLLABORATOR", "user": {"login": "helper"}},
            {"body": outsider, "author_association": "NONE", "user": {"login": "outsider"}},
        ]);
    }
    scenario.write_gh_state(&state);
    let received = scenario.path("received-review.md");
    scenario.agent_does(&format!(
        "cp .thirdshift-review-input.md '{}'\nprintf '## Standards\\nNo findings.\\n## Spec\\nNo findings.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"", received.display()
    ));

    let result = scenario.run(&["review", &url, "--recommend-only"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let packet = std::fs::read_to_string(received).unwrap();
    for kept in [
        "Export CSV.",
        "Blue widgets stay blue.",
        "Use UTF-8.",
        "PR discussion",
        "Inline feedback",
        "Review summary",
        "author explains",
    ] {
        assert!(packet.contains(kept), "missing {kept}: {packet}");
    }
    assert!(!packet.contains("outsider"), "{packet}");
    let state = scenario.gh_state();
    let comment = state["prs"][0]["postedComments"][0].as_str().unwrap();
    assert!(!comment.contains("body alone"), "{comment}");
    let prompt = scenario.first_prompt();
    assert!(
        prompt.contains("Linked closing issues are the Spec"),
        "{prompt}"
    );
}

#[test]
fn review_process_file_can_be_inspected_checked_and_copied() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    let shown = scenario.run(&["process", "show", "review-run"]);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    assert!(shown.stdout.contains("kind = \"pull-request\""));
    assert!(shown.stdout.contains("catalogue = \"review-and-address\""));
    let listed = scenario.run(&["process", "list"]);
    assert_eq!(listed.code, Some(0), "{}", listed.stderr);
    assert!(
        listed.stdout.contains("review-run (built-in)"),
        "{}",
        listed.stdout
    );
    let checked = scenario.run(&["process", "check", "review-run", "pull-request", &url]);
    assert_eq!(checked.code, Some(0), "{}", checked.stderr);
    assert!(checked.stdout.contains("review-and-address"));
    assert!(scenario.claude_calls().is_empty());
    let directory = scenario.path("home/.thirdshift/processes");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("colleague-review.toml"), &shown.stdout).unwrap();
    scenario.agent_does("printf '## Standards\\nNo findings.\\n## Spec\\nNo findings.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");

    let result = scenario.run(&["process", "colleague-review", "pull-request", &url]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        scenario.gh_state()["prs"][0]["postedComments"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let record = scenario.command_record("home/.thirdshift/logs/acme/widgets/commands/process");
    assert_eq!(record["kind"], "Process");
    assert_eq!(record["process_name"], "colleague-review");
    assert_eq!(record["process_kind"], "custom");
    assert_eq!(record["pull_request"], url);
    assert_eq!(record["outcome"], "ready for review");
}

#[test]
fn a_changed_pr_stops_before_posting_recommendations() {
    for changed in ["head", "merged", "draft"] {
        let scenario = Scenario::new();
        let url = pull_request(&scenario);
        let started = scenario.path("started");
        let release = scenario.path("release");
        scenario.agent_does(&format!(
            "touch '{}'\nfor _ in $(seq 2400); do test -f '{}' && break; sleep 0.05; done\ntest -f '{}'\nprintf '## Standards\\nNo findings.\\n## Spec\\nNo findings.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
            started.display(), release.display(), release.display()
        ));
        let held = scenario.run_until(&["review", &url, "--recommend-only"], &[], "started");
        if changed == "head" {
            scenario.origin_has_commit(
                "colleague",
                "later.txt",
                "Later author work",
                "Later author commit",
            );
        } else {
            let mut state = scenario.gh_state();
            if changed == "merged" {
                state["prs"][0]["state"] = json!("MERGED");
            } else {
                state["prs"][0]["isDraft"] = json!(true);
            }
            scenario.write_gh_state(&state);
        }
        std::fs::write(release, "").unwrap();
        let result = held.finish();
        assert_eq!(result.code, Some(1), "{changed}: {}", result.stderr);
        assert!(scenario.gh_state()["prs"][0]["postedComments"].is_null());
        if changed == "head" {
            assert!(
                result.stderr.contains("changed during Review run"),
                "{}",
                result.stderr
            );
            assert_eq!(
                scenario.origin_file("colleague", "later.txt").as_deref(),
                Some("Later author work")
            );
        } else {
            assert!(result.stderr.contains(changed), "{}", result.stderr);
        }
    }
}

#[test]
fn recommendation_sessions_that_change_files_are_refused_and_the_work_is_retained() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    let before = scenario.origin_log("colleague");
    scenario.agent_does("echo 'unexpected edit' > unexpected.txt\nprintf 'No findings.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
    let result = scenario.run(&["review", &url, "--recommend-only"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("changed the branch"),
        "{}",
        result.stderr
    );
    assert!(scenario.gh_state()["prs"][0]["postedComments"].is_null());
    assert_eq!(scenario.origin_log("colleague"), before);
    let calls = scenario.claude_calls();
    let worktree = std::path::Path::new(calls[0]["cwd"].as_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(worktree.join("unexpected.txt")).unwrap(),
        "unexpected edit\n"
    );
}

#[test]
fn recommend_only_preserves_an_existing_local_head_branch() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario.launch_git(&["fetch", "origin", "colleague:colleague"]);
    let before = scenario.launch_git(&["rev-parse", "refs/heads/colleague"]);
    scenario.agent_does("printf '## Standards\\nNo findings.\\n## Spec\\nNo findings.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
    let result = scenario.run(&["review", &url, "--recommend-only"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        scenario.launch_git(&["rev-parse", "refs/heads/colleague"]),
        before
    );
    assert_eq!(
        scenario
            .launch_git(&["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1
    );
}

#[test]
fn custom_pr_steps_keep_results_and_pass_their_paths_to_later_sessions() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    let note = scenario.path("result-directory.txt");
    let process = format!(
        r#"version = 1
description = "Keep Step results"
[worktrees.review]
kind = "pull-request"
[[steps]]
name = "preflight"
command = '''printf '%s' "$THIRDSHIFT_RESULTS_DIR" > '{}'; printf 'preflight output' '''
worktree = "review"
[[steps]]
name = "inspect"
skill = "thirdshift-code-review"
worktree = "review"
[[steps]]
name = "review"
catalogue = "review-and-address"
worktree = "review"
[[steps]]
name = "deliver"
catalogue = "delivery"
worktree = "review"
"#,
        note.display()
    );
    let directory = scenario.path("home/.thirdshift/processes");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("result-review.toml"), process).unwrap();
    scenario.agent_does(
        "printf '## Standards\\nNo findings.\\n## Spec\\nNo findings.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
    );

    let result = scenario.run(&["process", "result-review", "pull-request", &url]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let results = std::fs::read_to_string(note).unwrap();
    let retained = std::path::Path::new(&results)
        .join("preflight.txt")
        .is_file();
    let prompt = scenario.first_prompt();
    let informed = prompt.contains("preflight.txt");
    assert!(
        retained && informed,
        "retained Step result: {retained}; later session received result path: {informed}; results directory: {results}; prompt: {prompt}"
    );
}
