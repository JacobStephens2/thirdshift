//! Bug-origin proposals through the Scoring CLI with real, bounded Git history.
mod support;

use chrono::{TimeDelta, Utc};
use serde_json::{Value, json};
use support::Scenario;

#[path = "bug_origins/settlement.rs"]
mod settlement;

#[test]
fn sandboxed_bug_origin_stops_at_codex_native_plan_limit() {
    if !std::process::Command::new("bwrap")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        println!("SKIPPED (sandboxed Bug-origin native quota): bwrap unavailable");
        return;
    }
    let scenario = scenario();
    scenario.user_config_is("[scoring]\nowners = [\"acme\"]\nharness = \"codex\"\n");
    let codex = scenario.path("bin/codex");
    std::fs::remove_file(&codex).unwrap();
    support::write_executable(
        &codex,
        r#"#!/bin/sh
set -eu
if [ "${1-}" = --version ]; then
    printf '%s\n' 'codex-cli 1.2.3'
    exit 0
fi
printf '%s\n' '{"type":"thread.started","thread_id":"origin-native-fixture"}'
printf '%s\n' '{"type":"turn.started"}'
mkdir -p "$HOME/.codex/sessions/2026/10/24"
native="$HOME/.codex/sessions/2026/10/24/rollout-origin-native-fixture.jsonl"
printf '{"type":"session_meta","payload":{"id":"origin-native-fixture","cwd":"%s"}}\n' "$PWD" > "$native"
sleep 1.1
timestamp=$(date -u +%Y-%m-%dT%H:%M:%SZ)
printf '{"timestamp":"%s","type":"event_msg","payload":{"type":"token_count","rate_limits":{"secondary":{"used_percent":85}}}}\n' "$timestamp" >> "$native"
sleep 2
printf '%s' '{"commit":null,"reason":"No introducing commit is established by the available history."}' > proposal.json
printf '%s\n' '{"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}}'
"#,
    );
    let fix = scenario
        .origin_git(&["rev-parse", "main"])
        .trim()
        .to_owned();
    let mut state = scenario.gh_state();
    state["scoring_bugs"] = json!([bug(8, Some(&fix), 1)]);
    scenario.write_gh_state(&state);
    let run = scenario.run_with_env(&["score"], &[("THIRDSHIFT_TRIAL_NO_SANDBOX", "")]);
    assert_eq!(
        run.code,
        Some(1),
        "Scoring missed the native Codex warning: {}",
        run.stderr
    );
    assert!(
        run.stderr
            .contains("Scoring stopped at a plan-limit warning"),
        "{}",
        run.stderr
    );
    assert!(
        weeks(&scenario).is_empty(),
        "a plan-limit stop advanced the window"
    );
}

fn scenario() -> Scenario {
    let scenario = Scenario::new();
    // A pinned Harness keeps its executable name; the shared fake dispatches
    // by that name, rather than by its temporary canonical file's hash.
    let harness = scenario.path("bin/claude");
    let source = harness.canonicalize().unwrap();
    std::fs::remove_file(&harness).unwrap();
    support::copy_executable(&source, &harness);
    scenario.user_config_is("[scoring]\nowners = [\"acme\"]\n");
    for (name, script) in [
        ("systemctl", "echo running"),
        (
            "systemd-run",
            "while [ \"$1\" != -- ]; do shift; done\nshift\nexec \"$@\"",
        ),
        ("journalctl", "exit 0"),
    ] {
        support::write_executable(
            &scenario.path(&format!("bin/{name}")),
            &format!("#!/bin/sh\n{script}\n"),
        );
    }
    scenario
}

fn score(scenario: &Scenario) -> support::RunResult {
    scenario.run_with_env(&["score"], &[("THIRDSHIFT_TRIAL_NO_SANDBOX", "1")])
}

fn weeks(scenario: &Scenario) -> Vec<Value> {
    let run = scenario.run(&["scores", "--json"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    serde_json::from_str(&run.stdout).unwrap()
}

fn bug(number: u64, fix: Option<&str>, days_ago: i64) -> Value {
    json!({
        "number": number,
        "url": format!("https://github.com/acme/widgets/issues/{number}"),
        "title": "Wrong answer", "body": "The answer should be 42.",
        "closedAt": (Utc::now() - TimeDelta::days(days_ago)).to_rfc3339(),
        "repository": {"nameWithOwner": "acme/widgets"},
        "labels": {"nodes": [{"name": "bug"}], "pageInfo": {"hasNextPage": false}},
        "timelineItems": {"nodes": [{"closer": fix.map(|oid| json!({"oid": oid}))}]}
    })
}

#[test]
fn named_history_commit_maps_through_github_to_the_other_hosts_run() {
    let scenario = scenario();
    scenario.origin_has_commit("main", "answer", "41\n", "Introduce wrong answer");
    let introduced = scenario
        .origin_git(&["rev-parse", "main"])
        .trim()
        .to_string();
    scenario.origin_has_commit("main", "answer", "42\n", "Fix answer");
    let fix = scenario
        .origin_git(&["rev-parse", "main"])
        .trim()
        .to_string();
    scenario.origin_has_commit("main", "future", "must be invisible\n", "Later work");
    let mut state = scenario.gh_state();
    state["scoring_bugs"] = json!([bug(8, Some(&fix), 30)]);
    state["origin_prs"] = json!({introduced.clone(): [{
        "html_url": "https://github.com/acme/widgets/pull/23",
        "merged_at": (Utc::now() - TimeDelta::days(40)).to_rfc3339(),
        "body": "Built with codex 1.2.3 · model · high <!-- thirdshift:built-with -->"
    }]});
    scenario.write_gh_state(&state);
    scenario.agent_does(&format!(r#"
test "$(git -C repository rev-parse HEAD)" = '{fix}'
test ! -e repository/future
test "$(git -C repository show '{introduced}:answer')" = 41
printf '%s' '{{"commit":"{introduced}","reason":"History changes the answer from 42 to 41; the fix restores 42."}}' > proposal.json
"#));
    let run = score(&scenario);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let proposal = &weeks(&scenario)[0]["bug_origins"][0];
    assert_eq!(proposal["issue"], scenario.issue_url(8));
    assert_eq!(proposal["commit"], introduced);
    assert_eq!(proposal["state"], "proposed");
    let mapped = &proposal["pull_requests"][0];
    assert_eq!(mapped["url"], "https://github.com/acme/widgets/pull/23");
    assert_eq!(mapped["run"]["source"], "built_with");
    assert_eq!(mapped["run"]["harness"], "codex");
    assert_eq!(mapped["run"]["version"], "1.2.3");
    assert_eq!(mapped["run"]["model"], "model");
}

#[test]
fn history_without_an_introducing_commit_retains_no_guess() {
    let scenario = scenario();
    let fix = scenario
        .origin_git(&["rev-parse", "main"])
        .trim()
        .to_string();
    let mut state = scenario.gh_state();
    state["scoring_bugs"] = json!([bug(8, Some(&fix), 1)]);
    scenario.write_gh_state(&state);
    scenario.agent_does("printf '%s' '{\"commit\":null,\"reason\":\"The available history already contains the bug; no introducing change is shown.\"}' > proposal.json");
    let run = score(&scenario);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let proposal = &weeks(&scenario)[0]["bug_origins"][0];
    assert!(proposal["commit"].is_null());
    assert_eq!(proposal["pull_requests"], json!([]));
    assert!(
        proposal["reason"]
            .as_str()
            .unwrap()
            .contains("no introducing change")
    );
    assert!(
        !scenario
            .gh_calls()
            .iter()
            .any(|call| call.iter().any(|arg| arg.contains("/pulls")))
    );
}

#[test]
fn security_fixes_get_no_proposal_or_session() {
    let scenario = scenario();
    let fix = scenario
        .origin_git(&["rev-parse", "main"])
        .trim()
        .to_string();
    let mut security = bug(8, Some(&fix), 1);
    security["labels"]["nodes"] = json!([{"name":"bug"}, {"name":"security-fix"}]);
    let mut state = scenario.gh_state();
    state["scoring_bugs"] = json!([security]);
    scenario.write_gh_state(&state);
    let run = score(&scenario);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(weeks(&scenario)[0]["bug_origins"], json!([]));
    assert!(scenario.claude_calls().is_empty());
}

#[test]
fn first_proposals_backfill_all_bugs_then_follow_the_window_across_pages() {
    let scenario = scenario();
    let mut state = scenario.gh_state();
    state["scoring_bugs"] = (8..=62).map(|n| bug(n, None, 400)).collect();
    scenario.write_gh_state(&state);
    let run = score(&scenario);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let first = weeks(&scenario).remove(0);
    assert_eq!(first["bug_origins"].as_array().unwrap().len(), 55);
    let mut next = bug(63, None, 0);
    next["closedAt"] = first["window_end"].clone();
    state["scoring_bugs"].as_array_mut().unwrap().push(next);
    scenario.write_gh_state(&state);
    let run = score(&scenario);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let all = weeks(&scenario);
    assert_eq!(all.len(), 2);
    let later = &all[1]["bug_origins"];
    assert_eq!(later.as_array().unwrap().len(), 1);
    assert_eq!(later[0]["issue"], scenario.issue_url(63));
}

#[test]
fn a_legacy_sample_does_not_skip_the_first_proposal_backfill() {
    let scenario = scenario();
    let run = score(&scenario);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let dir = scenario.path("home/.thirdshift/evals/scoring");
    let path = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "json"))
        .unwrap();
    let mut week: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    week.as_object_mut().unwrap().remove("bug_origins");
    std::fs::write(path, serde_json::to_vec(&week).unwrap()).unwrap();
    let mut state = scenario.gh_state();
    state["scoring_bugs"] = json!([bug(8, None, 400)]);
    scenario.write_gh_state(&state);
    let run = score(&scenario);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(
        weeks(&scenario)[1]["bug_origins"][0]["issue"],
        scenario.issue_url(8)
    );
}

#[test]
fn first_proposal_backfill_covers_more_than_githubs_search_limit() {
    let scenario = scenario();
    let mut state = scenario.gh_state();
    state["scoring_bugs"] = (8..=1008)
        .map(|n| bug(n, None, if n % 2 == 0 { 400 } else { 1 }))
        .collect();
    scenario.write_gh_state(&state);
    let run = score(&scenario);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(
        weeks(&scenario)[0]["bug_origins"].as_array().unwrap().len(),
        1001
    );
}

#[test]
fn a_symlinked_proposal_cannot_import_a_host_file() {
    let scenario = scenario();
    let fix = scenario
        .origin_git(&["rev-parse", "main"])
        .trim()
        .to_string();
    let mut state = scenario.gh_state();
    state["scoring_bugs"] = json!([bug(8, Some(&fix), 1)]);
    scenario.write_gh_state(&state);
    // Outside the session workspace. A symlink here is the session pointing
    // the host's post-session read at a file the sandbox does not contain.
    let outside = scenario.path("outside-proposal.json");
    let reason = "contents of a host file outside the session workspace";
    std::fs::write(
        &outside,
        format!(r#"{{"commit":null,"reason":"{reason}"}}"#),
    )
    .unwrap();
    scenario.agent_does(&format!("ln -s {} proposal.json\n", shell_quote(&outside)));
    let run = score(&scenario);
    assert_eq!(
        run.code,
        Some(1),
        "symlinked proposal was accepted:\n{}",
        run.stderr
    );
    assert!(run.stderr.contains("not a regular file"), "{}", run.stderr);
    assert!(
        weeks(&scenario).is_empty(),
        "a symlinked proposal advanced the window"
    );
}

fn shell_quote(path: &std::path::Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

#[test]
fn a_named_commit_outside_captured_history_is_rejected_without_a_window() {
    let scenario = scenario();
    let fix = scenario
        .origin_git(&["rev-parse", "main"])
        .trim()
        .to_string();
    let mut state = scenario.gh_state();
    state["scoring_bugs"] = json!([bug(8, Some(&fix), 1)]);
    scenario.write_gh_state(&state);
    scenario.agent_does("printf '%s' '{\"commit\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"reason\":\"Unsupported assertion.\"}' > proposal.json");
    let run = score(&scenario);
    assert_eq!(run.code, Some(1), "{}", run.stderr);
    assert!(
        run.stderr.contains("not in pre-fix history"),
        "{}",
        run.stderr
    );
    assert!(weeks(&scenario).is_empty());
}

#[test]
fn plan_limit_warning_stops_before_the_next_bug_and_advances_no_window() {
    let scenario = scenario();
    let fix = scenario
        .origin_git(&["rev-parse", "main"])
        .trim()
        .to_string();
    let mut state = scenario.gh_state();
    state["scoring_bugs"] = json!([bug(8, Some(&fix), 1), bug(9, Some(&fix), 1)]);
    scenario.write_gh_state(&state);
    scenario.agent_does(
        r#"
printf '%s\n' '{"type":"rate_limit_event","rate_limit_info":{"status":"allowed_warning"}}'
sleep 30
"#,
    );
    let start = std::time::Instant::now();
    let run = score(&scenario);
    assert_eq!(run.code, Some(1), "{}", run.stderr);
    assert!(
        run.stderr
            .contains("Scoring stopped at a plan-limit warning"),
        "{}",
        run.stderr
    );
    assert!(
        start.elapsed().as_secs() < 15,
        "warning was not handled live"
    );
    assert!(weeks(&scenario).is_empty());
    assert_eq!(scenario.claude_calls().len(), 1);
    scenario.agent_does("printf '%s' '{\"commit\":null,\"reason\":\"No introducing change is shown.\"}' > proposal.json");
    let retry = score(&scenario);
    assert_eq!(retry.code, Some(0), "{}", retry.stderr);
    assert_eq!(
        weeks(&scenario)[0]["bug_origins"].as_array().unwrap().len(),
        2
    );
}

#[test]
fn introducing_pr_maps_to_this_hosts_retained_run_before_its_built_with_line() {
    let scenario = scenario();
    scenario.agent_does(
        r#"
echo 41 > answer
git add answer
git commit -q -m 'Wrong answer'
gh pr create --base main --head issue-7 --title 'Answer' --body 'Closes #7'
"#,
    );
    let run = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let introduced = scenario
        .origin_git(&["rev-parse", "issue-7"])
        .trim()
        .to_string();
    scenario.origin_git(&["update-ref", "refs/heads/main", &introduced]);
    scenario.origin_has_commit("main", "answer", "42\n", "Fix answer");
    let fix = scenario
        .origin_git(&["rev-parse", "main"])
        .trim()
        .to_string();
    let mut state = scenario.gh_state();
    state["scoring_bugs"] = json!([bug(8, Some(&fix), 0)]);
    state["origin_prs"] = json!({introduced.clone(): [{
        "html_url": "https://github.com/acme/widgets/pull/1",
        "merged_at": (Utc::now() - TimeDelta::days(1)).to_rfc3339(),
        "body": state["prs"][0]["body"],
    }]});
    scenario.write_gh_state(&state);
    scenario.agent_does(&format!("printf '%s' '{{\"commit\":\"{introduced}\",\"reason\":\"The answer was introduced here and corrected by the fix.\"}}' > proposal.json"));
    let run = score(&scenario);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let local = &weeks(&scenario)[0]["bug_origins"][0]["pull_requests"][0]["run"];
    assert_eq!(local["source"], "local");
    let path = local["command_record"].as_str().unwrap();
    let record: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        record["pull_request"],
        "https://github.com/acme/widgets/pull/1"
    );
    assert_eq!(record["name"], local["name"]);
}
