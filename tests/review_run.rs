//! Review runs through the CLI, real Git and fake GitHub and Harnesses.

mod support;

use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;

use serde_json::json;
use support::Scenario;

fn pull_request(scenario: &Scenario) -> String {
    scenario.origin_has_branch("colleague", "main", &["Colleague feature"]);
    scenario.github_has_pr("colleague", "main", "OPEN")
}

#[test]
fn advancing_base_during_review_delivers_fixes_and_posts_findings() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario.origin_has_branch("unrelated", "main", &["Unrelated change"]);
    scenario.agent_does(
        r#"
echo review-fix > review-fix.txt
git add review-fix.txt
git commit -qm 'Address review finding'
git fetch -q origin unrelated
git push -q origin FETCH_HEAD:refs/heads/main
printf 'Standards: fixed export. Spec: no findings.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );

    let result = scenario.run(&["review", &url]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        scenario
            .origin_file("colleague", "review-fix.txt")
            .as_deref(),
        Some("review-fix\n")
    );
    let state = scenario.gh_state();
    let comment = state["prs"][0]["postedComments"][0].as_str().unwrap();
    assert!(comment.contains("fixed export"), "{comment}");
    assert!(comment.contains("pushed_head"), "{comment}");
    let (_, notification) = support::resend::saved_notification(&scenario, "review");
    assert!(notification.contains("ready for review"), "{notification}");
}

#[test]
fn advancing_base_into_conflict_keeps_review_fixes_local() {
    let scenario = Scenario::new();
    scenario.origin_has_commit("main", "shared.txt", "original\n", "Shared file");
    let url = pull_request(&scenario);
    scenario.origin_has_commit("colleague", "shared.txt", "author\n", "Author change");
    scenario.origin_has_branch("base-update", "main", &[]);
    scenario.origin_has_commit(
        "base-update",
        "shared.txt",
        "upstream\n",
        "Conflicting change",
    );
    scenario.agent_does(
        r#"
echo review-fix > review-fix.txt
git add review-fix.txt
git commit -qm 'Address review finding'
git fetch -q origin base-update
git push -q origin FETCH_HEAD:refs/heads/main
gh fake pr colleague mergeable '"CONFLICTING"'
printf 'Standards: fixed export. Spec: no findings.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );

    let result = scenario.run(&["review", &url]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("needs updating before merge"),
        "{}",
        result.stderr
    );
    assert!(
        scenario
            .origin_file("colleague", "review-fix.txt")
            .is_none()
    );
    let state = scenario.gh_state();
    let comment = state["prs"][0]["postedComments"][0].as_str().unwrap();
    assert!(comment.contains("needs updating before merge"), "{comment}");
    assert!(!comment.contains("pushed_head"), "{comment}");
    let calls = scenario.claude_calls();
    let worktree = Path::new(calls[0]["cwd"].as_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(worktree.join("review-fix.txt")).unwrap(),
        "review-fix\n"
    );
}

#[test]
fn advancing_base_during_recommend_only_posts_findings_without_pushing() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario.origin_has_branch("unrelated", "main", &["Unrelated change"]);
    let before = scenario.origin_log("colleague");
    scenario.agent_does(
        r#"
git fetch -q origin unrelated
git push -q origin FETCH_HEAD:refs/heads/main
printf 'Standards: consider export. Spec: no findings.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );

    let result = scenario.run(&["review", &url, "--recommend-only"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.origin_log("colleague"), before);
    let state = scenario.gh_state();
    let comment = state["prs"][0]["postedComments"][0].as_str().unwrap();
    assert!(comment.contains("consider export"), "{comment}");
    assert!(
        comment.contains("--recommend-only; nothing pushed"),
        "{comment}"
    );
    assert!(!comment.contains("pushed_head"), "{comment}");
}

#[test]
fn advancing_base_during_review_ci_watch_keeps_the_delivered_head_ready() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario.origin_has_branch("unrelated", "main", &["Unrelated change"]);
    scenario.agent_does(
        r#"
echo review-fix > review-fix.txt
git add review-fix.txt
git commit -qm 'Address review finding'
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"success","pending_polls":2}]'
gh fake on-ci-read 1 'git fetch -q origin unrelated; git push -q origin FETCH_HEAD:refs/heads/main'
printf 'Standards: fixed export. Spec: no findings.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );

    let result = scenario.run(&["review", &url]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        scenario
            .origin_file("colleague", "review-fix.txt")
            .as_deref(),
        Some("review-fix\n")
    );
    let state = scenario.gh_state();
    let comment = state["prs"][0]["postedComments"][0].as_str().unwrap();
    assert!(comment.contains("fixed export"), "{comment}");
    assert!(comment.contains("pushed_head"), "{comment}");
    let (_, notification) = support::resend::saved_notification(&scenario, "review");
    assert!(notification.contains("ready for review"), "{notification}");
}

#[test]
fn retry_delivers_commits_retained_after_delivery_validation_failed() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario
        .agent_does("printf 'No findings. Validation passed.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
    scenario.agent_does_in_session(
        1,
        r#"
echo review-fix > review-fix.txt
git add review-fix.txt
git commit -qm 'Address review finding'
printf 'Review finding fixed. Validation passed.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    scenario.agent_does_in_session(
        2,
        r#"
echo security-fix > security-fix.txt
git add security-fix.txt
git commit -qm 'Address security finding'
printf 'Security finding fixed.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    scenario.agent_does_in_session(3, "exit 1");
    let first = scenario.run(&["review", &url, "security-review"]);
    assert_eq!(first.code, Some(1), "{}", first.stderr);
    assert!(
        first.stderr.contains("unpushed commits"),
        "{}",
        first.stderr
    );
    assert!(
        scenario
            .origin_file("colleague", "review-fix.txt")
            .is_none()
    );

    let retry = scenario.run(&["review", &url]);

    assert_eq!(retry.code, Some(0), "{}", retry.stderr);
    assert_eq!(
        scenario
            .origin_file("colleague", "review-fix.txt")
            .as_deref(),
        Some("review-fix\n")
    );
    assert_eq!(
        scenario
            .origin_file("colleague", "security-fix.txt")
            .as_deref(),
        Some("security-fix\n")
    );
    assert_eq!(
        scenario.gh_state()["prs"][0]["postedComments"]
            .as_array()
            .unwrap()
            .len(),
        1
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
fn known_head_failure_is_preserved_when_base_ci_is_unreadable() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("colleague", "main", &["Colleague feature"]);
    let url = scenario.github_has_pr("colleague", "main", "OPEN");
    scenario.agent_does(r#"
base_commit=$(git merge-base HEAD origin/main)
gh fake checks "$(git rev-parse HEAD)" '[{"name":"unique-head-failure","conclusion":"failure","url":"https://ci.example/known-head-failure"}]'
gh fake fails "api --paginate --jq .statuses[] repos/acme/widgets/commits/$base_commit/status?per_page=100"
printf 'Original findings.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#);

    let result = scenario.run(&["review", &url]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    let state = scenario.gh_state();
    let comment = state["prs"][0]["postedComments"][0].as_str().unwrap();
    let (_, notification) = support::resend::saved_notification(&scenario, "review");
    for evidence in [
        "unique-head-failure",
        "https://ci.example/known-head-failure",
    ] {
        assert!(
            comment.contains(evidence) && notification.contains(evidence),
            "missing {evidence}; comment:\n{comment}\nnotification:\n{notification}"
        );
    }
}

#[test]
fn unchanged_review_with_red_ci_never_reports_ready_for_review() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario.agent_does(
        r#"
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"failure","url":"https://ci.example/test"}]'
printf 'No Standards or Spec findings.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );

    let result = scenario.run(&["review", &url]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    let record = scenario.command_record("home/.thirdshift/logs/acme/widgets/commands/review");
    assert_eq!(record["outcome"], "failed", "{record}");
    let (_, notification) = support::resend::saved_notification(&scenario, "review");
    assert!(!notification.contains("ready for review"), "{notification}");
    assert!(
        notification.contains("https://ci.example/test"),
        "{notification}"
    );
    let comment = scenario.gh_state()["prs"][0]["postedComments"][0]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(comment.contains("https://ci.example/test"), "{comment}");
}

#[test]
fn review_repairs_own_ci_and_validates_the_delivered_head() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario.user_config_is("[review.commands_by_repo]\n'acme/widgets' = ['git rev-parse HEAD']\n");
    scenario.agent_does(
        r#"
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"failure","url":"https://ci.example/test"}]'
printf 'Standards: original finding. Spec: no findings.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    scenario.agent_does_in_session(2,
        r#"
echo repaired > ci-fix.txt
git add ci-fix.txt
git commit -qm 'Fix branch CI'
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"success","pending_polls":2,"url":"https://ci.example/test"}]'
printf 'Repair validation: git rev-parse HEAD passed at %s' "$(git rev-parse HEAD)" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );

    let result = scenario.run(&["review", &url]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        scenario.origin_file("colleague", "ci-fix.txt").as_deref(),
        Some("repaired\n")
    );
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 2, "{calls:?}");
    let prompt = calls[1]["prompt"].as_str().unwrap();
    for text in [
        "https://ci.example/test",
        "git rev-parse HEAD",
        "Do not merge or rebase",
        "Do not push",
        "Do not re-run CI",
    ] {
        assert!(prompt.contains(text), "missing {text}: {prompt}");
    }
    let pushed = scenario.launch_git(&["ls-remote", "origin", "refs/heads/colleague"]);
    let head = pushed.split_whitespace().next().unwrap();
    let comment = scenario.gh_state()["prs"][0]["postedComments"][0]
        .as_str()
        .unwrap()
        .to_owned();
    for text in [
        "original finding",
        "Repair validation",
        head,
        "pushed_head",
        "CI passed",
    ] {
        assert!(comment.contains(text), "missing {text}: {comment}");
    }
    let (_, notification) = support::resend::saved_notification(&scenario, "review");
    assert!(notification.contains("ready for review"), "{notification}");
}

#[test]
fn green_review_with_a_final_conflict_requires_an_author_update() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario.agent_does(
        r#"
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"success"}]'
gh fake pr colleague mergeable '"CONFLICTING"'
printf 'No findings.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );

    let result = scenario.run(&["review", &url]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("needs updating before merge"),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 1);
    let (_, notification) = support::resend::saved_notification(&scenario, "review");
    assert!(!notification.contains("ready for review"), "{notification}");
    assert!(
        notification.contains("needs updating before merge"),
        "{notification}"
    );
}

#[test]
fn review_waits_for_final_head_ci_before_notifying() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario.agent_does(&format!(r#"
gh fake checks "$(git rev-parse HEAD)" '[{{"name":"test","conclusion":"success","pending_polls":3}}]'
gh fake on-ci-read 1 'touch "{started}"; while test ! -f "{release}"; do sleep 0.02; done'
printf 'No findings.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#, started = scenario.path("ci-started").display(), release = scenario.path("ci-release").display()));

    let held = scenario.run_until(&["review", &url], &[], "ci-started");
    let folder = "home/.thirdshift/logs/acme/widgets/commands/review";
    let during = scenario.command_record(folder);
    let notifications = scenario.log_files(folder, "txt");
    std::fs::write(scenario.path("ci-release"), "").unwrap();
    let result = held.finish();

    assert!(during["outcome"].is_null(), "{during}");
    assert!(notifications.is_empty(), "{notifications:?}");
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(result.stderr.contains("still running"), "{}", result.stderr);
    let (_, notification) = support::resend::saved_notification(&scenario, "review");
    assert!(notification.contains("ready for review"), "{notification}");
    assert_eq!(scenario.claude_calls().len(), 1);
}

#[test]
fn review_handles_absent_inherited_unreadable_and_status_ci() {
    for case in [
        "absent",
        "inherited",
        "unreadable",
        "status",
        "recommend-only",
    ] {
        let scenario = Scenario::new();
        let url = pull_request(&scenario);
        let setup = match case {
            "inherited" => {
                r#"
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"failure","url":"https://ci.example/head"}]'
gh fake checks "$(git merge-base HEAD origin/main)" '[{"name":"test","conclusion":"failure","url":"https://ci.example/base"}]'
"#
            }
            "unreadable" => "gh fake fails 'api --paginate --jq .statuses[]'",
            "status" | "recommend-only" => {
                r#"gh fake statuses "$(git rev-parse HEAD)" '[{"context":"deploy","state":"failure","url":"https://ci.example/status"}]'"#
            }
            _ => "true",
        };
        scenario.agent_does(&format!(
            "{setup}\nprintf 'No findings.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n"
        ));
        let args = if case == "recommend-only" {
            vec!["review", &url, "--recommend-only"]
        } else {
            vec!["review", &url]
        };
        let result = scenario.run(&args);

        let succeeds = matches!(case, "absent" | "recommend-only");
        assert_eq!(
            result.code,
            Some(if succeeds { 0 } else { 1 }),
            "{case}: {}",
            result.stderr
        );
        assert_eq!(
            scenario.claude_calls().len(),
            if case == "status" { 2 } else { 1 },
            "{case}"
        );
        let (_, notification) = support::resend::saved_notification(&scenario, "review");
        assert_eq!(
            notification.contains("ready for review"),
            succeeds,
            "{case}: {notification}"
        );
        match case {
            "absent" => assert!(
                result.stderr.contains("no CI checks appeared"),
                "{}",
                result.stderr
            ),
            "inherited" => {
                assert!(notification.contains("fix main first"), "{notification}");
                assert!(
                    notification.contains("https://ci.example/base"),
                    "{notification}"
                );
            }
            "unreadable" => assert!(notification.contains("CI watch on"), "{notification}"),
            "status" => assert!(
                notification.contains("https://ci.example/status"),
                "{notification}"
            ),
            "recommend-only" => assert!(
                !scenario
                    .gh_calls()
                    .iter()
                    .flatten()
                    .any(|arg| arg.contains("/commits/"))
            ),
            _ => unreachable!(),
        }
        assert!(scenario.gh_calls_of("run", "rerun").is_empty(), "{case}");
    }
}

#[test]
fn unchanged_review_repair_gets_one_check_rerun_and_no_second_session() {
    for conclusion in ["success", "failure"] {
        let scenario = Scenario::new();
        let url = pull_request(&scenario);
        let before = scenario.origin_log("colleague");
        scenario.agent_does(&format!(r#"
gh fake checks "$(git rev-parse HEAD)" '[{{"name":"test","conclusion":"failure","url":"https://github.com/acme/widgets/actions/runs/900/job/1","rerun":{{"conclusion":"{conclusion}","pending_polls":2}}}}]'
gh fake run "{{\"databaseId\":900,\"workflow_id\":10,\"created_at\":\"2026-10-19T12:00:00Z\",\"headSha\":\"$(git rev-parse HEAD)\"}}"
printf 'No findings.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#));
        scenario.agent_does_in_session(
            2,
            "printf 'No branch changes. Validation passed.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
        );
        let result = scenario.run(&["review", &url]);

        assert_eq!(
            result.code,
            Some(if conclusion == "success" { 0 } else { 1 }),
            "{}",
            result.stderr
        );
        assert_eq!(scenario.claude_calls().len(), 2);
        assert_eq!(scenario.gh_calls_of("run", "rerun").len(), 1);
        assert_eq!(scenario.origin_log("colleague"), before);
        if conclusion == "failure" {
            assert!(
                result.stderr.contains("Repair found nothing to fix"),
                "{}",
                result.stderr
            );
        }
    }
}

#[test]
fn pull_request_custom_delivery_obeys_its_repair_cap() {
    for cap in [0, 2] {
        let scenario = Scenario::new();
        let url = pull_request(&scenario);
        let directory = scenario.path("home/.thirdshift/processes");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("bounded-review.toml"),
            format!(
                "{}\n[limits]\nrepair_cap = {cap}\n",
                include_str!("../processes/review-run.toml")
            ),
        )
        .unwrap();
        scenario.agent_does(r#"
if [[ "$FAKE_CLAUDE_PROMPT" == 'CI failed on pull request'* ]]; then
    echo fix >> ci-fix.txt
    git add ci-fix.txt
    git commit -qm 'Attempt CI fix'
fi
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"failure","url":"https://ci.example/test"}]'
printf 'Validation passed at %s' "$(git rev-parse HEAD)" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#);
        let result = scenario.run(&["process", "bounded-review", "pull-request", &url]);

        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(
            result.stderr.contains("repairs exhausted"),
            "{}",
            result.stderr
        );
        assert_eq!(scenario.claude_calls().len(), cap + 1);
        let (_, notification) = support::resend::saved_notification(&scenario, "process");
        assert!(!notification.contains("ready for review"), "{notification}");
        assert!(
            notification.contains("https://ci.example/test"),
            "{notification}"
        );
        assert!(scenario.gh_calls_of("pr", "merge").is_empty());
    }
}

#[test]
fn concurrent_pr_changes_during_ci_cannot_inherit_the_old_heads_green_verdict() {
    for change in [
        "head", "draft", "closed", "merged", "retarget", "fork", "author",
    ] {
        let scenario = Scenario::new();
        let url = pull_request(&scenario);
        scenario.origin_has_branch("concurrent", "colleague", &["Author work"]);
        let action = match change {
            "head" => {
                "git fetch -q origin concurrent\ngit push -q origin FETCH_HEAD:refs/heads/colleague"
            }
            "draft" => "gh fake pr colleague isDraft true",
            "closed" => "gh fake pr colleague state '\"CLOSED\"'",
            "merged" => "gh fake pr colleague state '\"MERGED\"'",
            "retarget" => "gh fake pr colleague base '\"concurrent\"'",
            "fork" => "gh fake pr colleague isCrossRepository true",
            "author" => "gh fake pr colleague authorAssociation '\"NONE\"'",
            _ => unreachable!(),
        };
        let mut state = scenario.gh_state();
        state["on_ci_read"] = json!({"times":1,"script":action,"seen":[]});
        scenario.write_gh_state(&state);
        scenario.agent_does(
            r#"
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"success","pending_polls":2}]'
printf 'No findings.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
        );
        let result = scenario.run(&["review", &url]);

        assert_eq!(result.code, Some(1), "{change}: {}", result.stderr);
        assert_eq!(scenario.claude_calls().len(), 1, "{change}");
        if change == "head" {
            assert_eq!(
                scenario.origin_log("colleague"),
                scenario.origin_log("concurrent")
            );
            assert!(
                result.stderr.contains("pull request changed"),
                "{}",
                result.stderr
            );
        }
        let (_, notification) = support::resend::saved_notification(&scenario, "review");
        assert!(
            !notification.contains("ready for review"),
            "{change}: {notification}"
        );
        assert!(scenario.gh_calls_of("pr", "merge").is_empty());
    }
}

#[test]
fn author_changes_during_a_ci_repair_leave_proposed_fixes_local() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario.origin_has_branch("concurrent", "colleague", &["Author work"]);
    scenario.agent_does(r#"
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"failure","url":"https://ci.example/test"}]'
printf 'Original findings.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#);
    scenario.agent_does_in_session(
        2,
        r#"
git fetch -q origin concurrent
git push -q origin FETCH_HEAD:refs/heads/colleague
echo proposed > proposed.txt
git add proposed.txt
git commit -qm 'Proposed CI fix'
printf 'Validation passed.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    let result = scenario.run(&["review", &url]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert_eq!(
        scenario.origin_log("colleague"),
        scenario.origin_log("concurrent")
    );
    assert!(scenario.origin_file("colleague", "proposed.txt").is_none());
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 2);
    assert!(
        Path::new(calls[1]["cwd"].as_str().unwrap())
            .join("proposed.txt")
            .is_file()
    );
    let (_, notification) = support::resend::saved_notification(&scenario, "review");
    assert!(!notification.contains("ready for review"), "{notification}");
}

#[test]
fn live_pull_request_processes_identify_their_target_before_the_first_session_ends() {
    for input in ["url", "author", "custom"] {
        let scenario = Scenario::new();
        let url = pull_request(&scenario);
        let (args, kind) = match input {
            "author" => (
                vec!["review", "--author", "colleague", "--recommend-only"],
                "review",
            ),
            "custom" => {
                let shown = scenario.run(&["process", "show", "review-run"]);
                assert_eq!(shown.code, Some(0), "{}", shown.stderr);
                let directory = scenario.path("home/.thirdshift/processes");
                std::fs::create_dir_all(&directory).unwrap();
                std::fs::write(directory.join("colleague-review.toml"), shown.stdout).unwrap();
                (
                    vec!["process", "colleague-review", "pull-request", &url],
                    "process",
                )
            }
            _ => (vec!["review", &url], "review"),
        };
        scenario.agent_waits_for_release(
            "printf '## Standards\\nNo findings.\\n## Spec\\nNo findings.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
        );
        let held = scenario.run_until(&args, &[], "started");
        let folder = format!("home/.thirdshift/logs/acme/widgets/commands/{kind}");
        let record = scenario.command_record(&folder);
        let log_name = scenario.log_files(&folder, "log").pop().unwrap();
        let log = std::fs::read_to_string(scenario.path(&folder).join(log_name)).unwrap();
        let text = scenario.run(&["status"]);
        let status = scenario.run(&["status", "--json"]);
        std::fs::write(scenario.path("release"), "").unwrap();
        let ended = held.finish();

        assert_eq!(
            record["outcome"],
            serde_json::Value::Null,
            "{input}: {record}"
        );
        assert_eq!(
            record["ended_at"],
            serde_json::Value::Null,
            "{input}: {record}"
        );
        assert_eq!(record["pull_request"], url, "{input}: {record}");
        assert!(log.contains(&url), "{input}: {log}");
        assert_eq!(text.code, Some(0), "{}", text.stderr);
        assert!(text.stdout.contains("Working:"), "{}", text.stdout);
        assert!(
            text.stdout.contains(&format!(" · {url} · live ·")),
            "{input}: {}",
            text.stdout
        );
        assert_eq!(status.code, Some(0), "{}", status.stderr);
        let status: serde_json::Value = serde_json::from_str(&status.stdout).unwrap();
        assert_eq!(status["commands"][0]["state"], "live", "{input}: {status}");
        assert_eq!(
            status["commands"][0]["pull_request"], url,
            "{input}: {status}"
        );
        assert_eq!(ended.code, Some(0), "{input}: {}", ended.stderr);
    }
}

/// Commit `branch` so `.thirdshift-review-input.md` is a symlink to `target`.
fn commit_review_input_symlink(scenario: &Scenario, branch: &str, target: &Path) {
    let seed = scenario.path("seed-link");
    let home = scenario.path("home");
    let git = |args: &[&str], dir: &Path| {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("HOME", &home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(
        &[
            "clone",
            "-q",
            &scenario.github_url(),
            seed.to_str().unwrap(),
        ],
        &scenario.path(""),
    );
    git(&["checkout", "-q", branch], &seed);
    symlink(target, seed.join(".thirdshift-review-input.md")).unwrap();
    git(&["add", ".thirdshift-review-input.md"], &seed);
    git(
        &[
            "commit",
            "-q",
            "-m",
            "Point the review input at another file",
        ],
        &seed,
    );
    git(&["push", "-q", "origin", branch], &seed);
    std::fs::remove_dir_all(&seed).unwrap();
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
fn review_addresses_findings_on_the_pr_branch_and_reports_both_heads() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    let reviewed = scenario.launch_git(&["ls-remote", "origin", "refs/heads/colleague"]);
    let reviewed = reviewed.split_whitespace().next().unwrap();
    scenario.agent_does(
        "echo 'fixed widget export' > export.txt\n\
         git add export.txt\n\
         git commit -qm 'Fix widget export'\n\
         printf passed > validation-results.txt\n\
         printf '## Fixed findings\\nFixed widget export.\\n## Unaddressed findings\\n### Standards\\nNone.\\n### Spec\\nNone.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
    );

    let result = scenario.run(&["review", &url]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        scenario.origin_file("colleague", "export.txt").as_deref(),
        Some("fixed widget export\n")
    );
    let pushed = scenario.launch_git(&["ls-remote", "origin", "refs/heads/colleague"]);
    let pushed = pushed.split_whitespace().next().unwrap();
    assert_ne!(reviewed, pushed);
    let state = scenario.gh_state();
    let comments = state["prs"][0]["postedComments"].as_array().unwrap();
    assert_eq!(comments.len(), 1);
    let comment = comments[0].as_str().unwrap();
    for text in [
        "Fixed widget export",
        "Unaddressed findings",
        reviewed,
        pushed,
        "pushed_head",
    ] {
        assert!(comment.contains(text), "missing {text}: {comment}");
    }
    let prompt = scenario.first_prompt();
    for text in [
        "run its test or command as written",
        "fix the code and keep the test",
        "Do not push",
        "Commit only if",
        "Unaddressed findings",
    ] {
        assert!(prompt.contains(text), "missing {text}: {prompt}");
    }
    assert!(!prompt.contains("This is --recommend-only"), "{prompt}");
    assert_eq!(scenario.launch_git(&["status", "--porcelain"]), "");
}

#[test]
fn review_requests_this_repositories_extra_validation_after_fixes_and_posts_results() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario.user_config_is(
        "[review.commands_by_repo]\n'ACME/Widgets' = ['sh slow-suite.sh', 'sh unavailable-suite.sh']\n'other/repo' = ['do-not-run-this']\n",
    );
    scenario.agent_does(
        "printf 'Validation: sh slow-suite.sh passed; sh unavailable-suite.sh: not run against the branch (requires primary checkout).' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
    );

    let result = scenario.run(&["review", &url]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let prompt = scenario.first_prompt();
    for text in [
        "After addressing findings",
        "in this Review run's worktree",
        "sh slow-suite.sh",
        "sh unavailable-suite.sh",
        "not run against the branch",
        "never report it as passing",
        "what ran, what passed, what failed and what could not run",
    ] {
        assert!(prompt.contains(text), "missing {text}: {prompt}");
    }
    assert!(!prompt.contains("do-not-run-this"), "{prompt}");
    assert!(
        prompt.contains(r#"["sh slow-suite.sh","sh unavailable-suite.sh"]"#),
        "{prompt}"
    );
    let state = scenario.gh_state();
    let comment = state["prs"][0]["postedComments"][0].as_str().unwrap();
    assert!(comment.contains("sh slow-suite.sh passed"), "{comment}");
    assert!(comment.contains("not run against the branch"), "{comment}");
}

#[test]
fn reviews_without_extra_commands_still_request_repository_final_validation() {
    for config in [
        "",
        "[review.commands_by_repo]\n'acme/widgets' = []\n",
        "[review.commands_by_repo]\n'other/repo' = ['unrelated-suite']\n",
    ] {
        let scenario = Scenario::new();
        let url = pull_request(&scenario);
        scenario.user_config_is(config);
        scenario.agent_does("printf 'Validation: repository final validation passed.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");

        let result = scenario.run(&["review", &url]);

        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let prompt = scenario.first_prompt();
        assert!(
            prompt.contains(
                "run final validation as required by the repository's agent instructions"
            ),
            "{prompt}"
        );
        assert!(
            prompt.contains("An empty list still requires the repository's final validation"),
            "{prompt}"
        );
        assert!(prompt.contains("in that worktree: []"), "{prompt}");
        assert!(!prompt.contains("unrelated-suite"), "{prompt}");
    }
}

#[test]
fn recommendation_only_requests_extra_validation_without_fixes_or_pushes() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    let before = scenario.origin_log("colleague");
    scenario.user_config_is("[review.commands_by_repo]\n'acme/widgets' = ['sh slow-suite.sh']\n");
    scenario.agent_does(
        "printf 'Validation: sh slow-suite.sh failed (exit 1).' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
    );

    let result = scenario.run(&["review", &url, "--recommend-only"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let prompt = scenario.first_prompt();
    for text in [
        "after reviewing in --recommend-only",
        "sh slow-suite.sh",
        "A failed command must not skip the remaining commands",
        "Do not commit, push, fix findings",
        "disposable validation artifacts",
    ] {
        assert!(prompt.contains(text), "missing {text}: {prompt}");
    }
    assert_eq!(scenario.origin_log("colleague"), before);
    let state = scenario.gh_state();
    let comment = state["prs"][0]["postedComments"][0].as_str().unwrap();
    assert!(
        comment.contains("sh slow-suite.sh failed (exit 1)"),
        "{comment}"
    );
}

#[test]
fn a_receive_time_branch_move_keeps_the_fixes_local_and_posts_the_findings() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario.origin_has_branch("concurrent", "colleague", &["Later author work"]);
    scenario.repo_has_hook(
        &scenario.path("origin.git"),
        "update",
        "#!/bin/sh\nif test \"$1\" = refs/heads/colleague; then\n  echo attempted > review-race\n  git update-ref \"$1\" refs/heads/concurrent\nfi\n",
    );
    scenario.agent_does(
        "echo 'proposed fix' > fix.txt\n\
         git add fix.txt\n\
         git commit -qm 'Fix export'\n\
         printf '## Standards\\nExport fails.\\n## Fixed findings\\nFixed export.\\n## Unaddressed findings\\n### Spec\\nBody explains UTF-8 choice.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
    );

    let result = scenario.run(&["review", &url]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        scenario.path("origin.git/review-race").is_file(),
        "push was never attempted"
    );
    assert_eq!(
        scenario.origin_log("colleague"),
        scenario.origin_log("concurrent")
    );
    assert!(scenario.origin_file("colleague", "fix.txt").is_none());
    let state = scenario.gh_state();
    let comment = state["prs"][0]["postedComments"][0].as_str().unwrap();
    for text in [
        "branch moved",
        "no fixes were pushed",
        "Export fails",
        "Fixed export",
        "Unaddressed findings",
        "Body explains UTF-8 choice",
    ] {
        assert!(comment.contains(text), "missing {text}: {comment}");
    }
    assert!(!comment.contains("pushed_head"), "{comment}");
    let calls = scenario.claude_calls();
    let worktree = Path::new(calls[0]["cwd"].as_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(worktree.join("fix.txt")).unwrap(),
        "proposed fix\n"
    );
}

#[test]
fn a_receive_time_rewind_does_not_republish_removed_history() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario.origin_has_commit(
        "colleague",
        "secret.txt",
        "local-marker-not-a-credential\n",
        "Add a local marker",
    );
    let origin = scenario.origin_dir();
    let marker = scenario.path("rewound");
    support::write_executable(
        &scenario.path("bin/git"),
        &format!(
            "#!/bin/sh\n\
             if [ \"$1\" = push ] && [ \"$2\" = --no-verify ] && [ ! -e '{marker}' ] && \
                 printf '%s\\n' \"$@\" | /usr/bin/grep -q ':refs/heads/colleague$'; then\n\
               touch '{marker}'\n\
               /usr/bin/git --git-dir='{origin}' update-ref refs/heads/colleague refs/heads/colleague^\n\
             fi\n\
             exec /usr/bin/git \"$@\"\n",
            marker = marker.display(),
            origin = origin.display(),
        ),
    );
    scenario.agent_does(
        "echo 'proposed fix' > fix.txt\n\
         git add fix.txt\n\
         git commit -qm 'Fix export'\n\
         printf '## Standards\\nExport fails.\\n## Unaddressed findings\\n### Standards\\nNone.\\n### Spec\\nNone.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
    );

    let result = scenario.run(&["review", &url]);

    assert!(
        marker.is_file(),
        "publish never reached the push that should observe the rewind\n{}",
        result.stderr
    );
    assert!(
        scenario.origin_file("colleague", "secret.txt").is_none(),
        "removed history was republished\n{}",
        result.stderr
    );
    assert!(scenario.origin_file("colleague", "fix.txt").is_none());
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    let state = scenario.gh_state();
    let comment = state["prs"][0]["postedComments"][0].as_str().unwrap();
    assert!(comment.contains("branch moved"), "{comment}");
    assert!(comment.contains("no fixes were pushed"), "{comment}");
    assert!(!comment.contains("pushed_head"), "{comment}");
}

#[test]
fn review_leaves_body_explained_spec_departures_unaddressed_and_commits_nothing_without_fixes() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    let before = scenario.origin_log("colleague");
    let mut state = scenario.gh_state();
    state["bodies"] = json!({"7": "Export ASCII only."});
    state["prs"][0]["closingIssuesReferences"] = json!([{"url": scenario.issue_url(7)}]);
    state["prs"][0]["body"] = json!("Use UTF-8 to preserve widget names.");
    scenario.write_gh_state(&state);
    scenario.agent_does(
        "printf '## Standards\\nNo findings.\\n## Unaddressed findings\\n### Spec\\n- ASCII requirement: PR body explains UTF-8 preserves widget names.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
    );

    let result = scenario.run(&["review", &url]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let prompt = scenario.first_prompt();
    for text in [
        "Linked closing issues are the Spec",
        "do not fix a departure explained only by the pull request body",
        "citing the body's reason, and do not file a `needs-triage` issue for it",
    ] {
        assert!(prompt.contains(text), "missing {text}: {prompt}");
    }
    let state = scenario.gh_state();
    let comment = state["prs"][0]["postedComments"][0].as_str().unwrap();
    assert!(comment.contains("No fixes to push"), "{comment}");
    assert!(comment.contains("PR body explains UTF-8"), "{comment}");
    assert!(!comment.contains("pushed_head"), "{comment}");
    assert_eq!(scenario.origin_log("colleague"), before);
    assert!(scenario.gh_calls_of("issue", "create").is_empty());
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
    for changed in ["head", "retarget", "branch", "closed", "merged", "draft"] {
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
            change_review_target(&scenario, changed);
        }
        std::fs::write(release, "").unwrap();
        let result = held.finish();
        assert_eq!(result.code, Some(1), "{changed}: {}", result.stderr);
        assert!(scenario.gh_state()["prs"][0]["postedComments"].is_null());
        if matches!(changed, "head" | "retarget" | "branch") {
            let diagnostic = match changed {
                "head" => "head moved; expected",
                "retarget" => "base branch moved; expected main, actual colleague",
                "branch" => "branch renamed; expected colleague, actual renamed",
                _ => unreachable!(),
            };
            assert!(result.stderr.contains(diagnostic), "{}", result.stderr);
            if changed == "head" {
                assert_eq!(
                    scenario.origin_file("colleague", "later.txt").as_deref(),
                    Some("Later author work")
                );
            }
        } else {
            assert!(result.stderr.contains(changed), "{}", result.stderr);
        }
    }
}

fn change_review_target(scenario: &Scenario, change: &str) {
    let mut state = scenario.gh_state();
    match change {
        "retarget" => state["prs"][0]["base"] = json!("colleague"),
        "branch" => {
            scenario.origin_has_branch("renamed", "colleague", &[]);
            state["prs"][0]["head"] = json!("renamed");
        }
        "closed" => state["prs"][0]["state"] = json!("CLOSED"),
        "merged" => state["prs"][0]["state"] = json!("MERGED"),
        "draft" => state["prs"][0]["isDraft"] = json!(true),
        _ => unreachable!(),
    }
    scenario.write_gh_state(&state);
}

#[test]
fn address_mode_reports_a_moved_head_but_stops_publication_after_branch_or_eligibility_changes() {
    for changed in ["head", "retarget", "branch", "closed", "merged", "draft"] {
        let scenario = Scenario::new();
        let url = pull_request(&scenario);
        let release = scenario.path("release");
        scenario.agent_does(&format!(
            "echo 'proposed fix' > fix.txt\ngit add fix.txt\ngit commit -qm 'Fix export'\ntouch '{}'\nfor _ in $(seq 2400); do test -f '{}' && break; sleep 0.05; done\ntest -f '{}'\nprintf '## Standards\\nFixed export.\\n## Unaddressed findings\\n### Spec\\nNone.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
            scenario.path("started").display(), release.display(), release.display()
        ));
        let held = scenario.run_until(&["review", &url], &[], "started");
        match changed {
            "head" => scenario.origin_has_commit(
                "colleague",
                "later.txt",
                "Author work",
                "Later author commit",
            ),
            _ => change_review_target(&scenario, changed),
        }
        std::fs::write(release, "").unwrap();
        let result = held.finish();
        assert_eq!(result.code, Some(1), "{changed}: {}", result.stderr);
        assert!(scenario.origin_file("colleague", "fix.txt").is_none());
        let state = scenario.gh_state();
        if changed == "head" {
            let comment = state["prs"][0]["postedComments"][0].as_str().unwrap();
            assert!(comment.contains("branch moved"), "{comment}");
            assert!(comment.contains("Fixed export"), "{comment}");
        } else {
            assert!(state["prs"][0]["postedComments"].is_null(), "{state}");
        }
    }
}

#[test]
fn recommendation_sessions_that_change_tracked_files_are_refused_and_the_work_is_retained() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario.origin_has_commit(
        "colleague",
        "unexpected.txt",
        "original\n",
        "Add tracked fixture",
    );
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
fn recommend_only_posts_validation_results_with_disposable_artifacts() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario.user_config_is(
        "[review.commands_by_repo]\n'acme/widgets' = ['printf passed > validation-results.txt']\n",
    );
    scenario.agent_does("printf passed > validation-results.txt\nprintf 'Validation: printf passed > validation-results.txt passed (exit 0).' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
    let result = scenario.run(&["review", &url, "--recommend-only"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    let comments = state["prs"][0]["postedComments"].as_array().unwrap();
    assert_eq!(comments.len(), 1, "{state}");
    assert!(comments[0].as_str().unwrap().contains("passed (exit 0)"));
}

#[test]
fn disposable_validation_artifacts_do_not_block_the_next_review() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario.user_config_is(
        "[review.commands_by_repo]\n'acme/widgets' = ['printf passed > validation-results.txt']\n",
    );
    scenario.agent_does("printf passed > validation-results.txt\nprintf 'Validation passed (exit 0).' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
    let first = scenario.run(&["review", &url, "--recommend-only"]);
    assert_eq!(first.code, Some(0), "{}", first.stderr);
    let second = scenario.run(&["review", &url, "--recommend-only"]);
    assert_eq!(second.code, Some(0), "{}", second.stderr);
    let state = scenario.gh_state();
    assert_eq!(
        state["prs"][0]["postedComments"].as_array().unwrap().len(),
        2
    );
}

#[test]
fn an_undelivered_review_leaving_only_validation_artifacts_frees_its_worktree_and_branch() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario.agent_does(
        "mkdir .review-logs\nprintf passed > .review-logs/tests.log\nprintf passed > .validation-tests.txt\nexit 1",
    );
    let result = scenario.run(&["review", &url]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        !result.stderr.contains("uncommitted changes"),
        "{}",
        result.stderr
    );
    assert_eq!(
        scenario
            .launch_git(&["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1
    );
    assert!(
        scenario
            .launch_git(&["branch", "--list", "colleague"])
            .is_empty()
    );
}

#[test]
fn an_undelivered_review_preserves_an_untracked_source_file_beside_validation_artifacts() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario.agent_does(
        "mkdir .review-logs\nprintf passed > .review-logs/tests.log\necho fix > review-fix.txt\nexit 1",
    );
    let result = scenario.run(&["review", &url]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("uncommitted changes"),
        "{}",
        result.stderr
    );
    let calls = scenario.claude_calls();
    let worktree = Path::new(calls[0]["cwd"].as_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(worktree.join("review-fix.txt")).unwrap(),
        "fix\n"
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

#[test]
fn review_input_does_not_follow_a_symlink_outside_the_worktree() {
    let scenario = Scenario::new();
    let outside = scenario.path("outside-secret.txt");
    std::fs::write(&outside, "operator secret\n").unwrap();
    let url = pull_request(&scenario);
    commit_review_input_symlink(&scenario, "colleague", &outside);
    let mut state = scenario.gh_state();
    state["prs"][0]["body"] = json!("SYMLINK-PAYLOAD-TOKEN");
    scenario.write_gh_state(&state);
    scenario.agent_does("printf 'session started' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"; exit 1");

    let result = scenario.run(&["review", &url, "--recommend-only"]);

    assert_eq!(
        std::fs::read_to_string(&outside).unwrap(),
        "operator secret\n",
        "review wrote through .thirdshift-review-input.md\n{}",
        result.stderr
    );
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("symlink"),
        "expected a symlink refusal: {}",
        result.stderr
    );
    assert!(scenario.claude_calls().is_empty(), "{}", result.stderr);
    assert!(scenario.gh_state()["prs"][0]["postedComments"].is_null());
    assert_eq!(
        scenario
            .launch_git(&["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1,
        "{}",
        result.stderr
    );
    assert_eq!(scenario.launch_git(&["branch", "--list", "colleague"]), "");
}
