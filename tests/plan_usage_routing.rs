//! Plan usage routing: a factory Command whose Harness comes from the User
//! config's `harness.routes` starts each session Step on its first route
//! that machine-local Plan usage says can take it, and starts a Step again
//! as a fresh session on the next route when its session stops on a
//! structured Plan-usage limit, keeping commits and discarding the rest.
//! Explicit Harness flags stay pinned.

mod support;

use serde_json::{Value, json};
use support::Scenario;

/// A future Unix time, so a recorded reset is still ahead.
const RESET: i64 = 4_000_000_000;

/// The agent commits its work and opens a PR.
const OPENS_PR: &str = r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

/// Work begins: the agent says something before it stops.
const WORKING: &str =
    r#"echo '{"type":"assistant","message":{"content":[{"type":"text","text":"working"}]}}'"#;

/// Claude Code's own structured refusal of included work.
fn claude_limit() -> String {
    json!({"type": "rate_limit_event", "rate_limit_info": {"status": "rejected",
        "rateLimitType": "five_hour", "resetsAt": RESET}})
    .to_string()
}

/// Save `harnesses` as the machine's Plan usage snapshot.
fn plan_usage_is(scenario: &Scenario, harnesses: Value) {
    let path = scenario.path("home/.thirdshift/plan-usage.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, json!({ "harnesses": harnesses }).to_string()).unwrap();
}

fn saved(scenario: &Scenario) -> Value {
    serde_json::from_slice(
        &std::fs::read(scenario.path("home/.thirdshift/plan-usage.json")).unwrap(),
    )
    .unwrap()
}

/// Claude Code's agent sessions, without its sign-in checks.
fn claude_sessions(scenario: &Scenario) -> Vec<Value> {
    scenario
        .claude_calls()
        .into_iter()
        .filter(|call| {
            call["argv"]
                .as_array()
                .is_some_and(|argv| argv.iter().any(|arg| arg == "--output-format"))
        })
        .collect()
}

/// Claude Code first, then Codex.
const ROUTES: &str = "[harness]\nroutes = [\"claude\", \"codex\"]\n";

#[test]
fn a_step_starts_on_its_first_route_below_its_reserve_ceiling() {
    let scenario = Scenario::new();
    scenario.user_config_is(&format!("{ROUTES}max_utilization = 85\n"));
    plan_usage_is(
        &scenario,
        json!({"claude": {"observed_at": chrono::Utc::now().to_rfc3339(),
            "windows": [{"name": "five_hour", "used_percent": 90.0, "resets_at": "2096-10-02T07:06:40Z"}]}}),
    );
    scenario.agent_does(OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(claude_sessions(&scenario).is_empty(), "{}", result.stderr);
    assert_eq!(scenario.codex_calls().len(), 1);
    assert!(
        result
            .stderr
            .contains("routed by Plan usage: claude is at its ceiling"),
        "{}",
        result.stderr
    );
    // The Command record names what the Step ran on.
    let record = scenario.command_record("issue");
    let stages = record["stages"].as_array().unwrap();
    assert!(
        stages.iter().any(|stage| stage["harness"]
            .as_str()
            .is_some_and(|h| h.starts_with("codex"))),
        "{record:#}"
    );
    // The pull request keeps the Command's single primary Built-with tuple.
    let state = scenario.gh_state();
    let body = state["prs"][0]["body"].as_str().unwrap();
    assert_eq!(
        body.matches("<!-- thirdshift:built-with -->").count(),
        1,
        "{body}"
    );
}

#[test]
fn a_limit_after_work_keeps_commits_discards_the_rest_and_continues_on_the_next_route() {
    let scenario = Scenario::new();
    scenario.user_config_is(ROUTES);
    scenario.agent_does(&format!(
        r#"
if [ -n "${{FAKE_CODEX_ERROR:-}}" ]; then
    test -f kept.txt
    test ! -e scratch.txt
    test "$(cat kept.txt)" = kept
    {OPENS_PR}
    exit 0
fi
echo kept > kept.txt
git add kept.txt
git commit -q -m "Keep this"
echo changed >> kept.txt
echo scratch > scratch.txt
{WORKING}
echo '{limit}'
exit 1
"#,
        limit = claude_limit(),
    ));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(claude_sessions(&scenario).len(), 1);
    assert_eq!(scenario.codex_calls().len(), 1);
    assert!(
        result
            .stderr
            .contains("implement: claude stopped at its Plan usage limit; starting the Step again as a fresh session on codex"),
        "{}",
        result.stderr
    );
    // The commit made before the limit landed, beside the next route's.
    assert_eq!(
        scenario.origin_log("issue-7"),
        Some(vec![
            "Add feature".to_string(),
            "Keep this".to_string(),
            "Initial commit".to_string(),
        ])
    );
    // The next route was given the existing Continuation prompt, unchanged.
    let prompt = scenario.codex_calls()[0]["prompt"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(prompt.contains("Push branch issue-7.\n"), "{prompt}");
    assert!(
        !prompt.contains("Push branch issue-7 and create"),
        "{prompt}"
    );
    for said in ["limit", "Plan usage", "quota", "ran out"] {
        assert!(!prompt.contains(said), "{said}: {prompt}");
    }
    // The limit is recorded for the next Step's proactive choice.
    assert!(saved(&scenario)["harnesses"]["claude"]["cooldown"].is_object());
    let logs = scenario.log_files("home/.thirdshift/logs/acme/widgets/commands/issue", "jsonl");
    assert!(
        logs.iter()
            .any(|name| name.ends_with("-implement-route-2.logs.jsonl")),
        "{logs:?}"
    );
}

#[test]
fn a_conflict_repair_that_stops_at_a_limit_starts_again_from_the_same_conflicted_merge() {
    let scenario = Scenario::new();
    scenario.user_config_is(ROUTES);
    // The implement session pushes its work while main moves on beneath it.
    scenario.agent_does(&format!(
        r#"
if [ -n "${{FAKE_CODEX_ERROR:-}}" ]; then
    git rev-parse -q --verify MERGE_HEAD > /dev/null
    grep -q '<<<<<<<' feature.txt
    test ! -e extra.txt
    printf 'feature\nbase feature\n' > feature.txt
    git add feature.txt
    git commit -q --no-edit
    exit 0
fi
{OPENS_PR}
git push -q origin issue-7
other="$(mktemp -d)"
git clone -q https://github.com/acme/widgets.git "$other"
echo "base feature" > "$other/feature.txt"
git -C "$other" add -A
git -C "$other" commit -q -m "Base moves on"
git -C "$other" push -q origin main
rm -rf "$other"
"#
    ));
    // Claude's conflict Repair half-resolves the merge, then stops.
    scenario.agent_does_in_session(
        2,
        &format!(
            "echo partial > feature.txt\ngit add feature.txt\necho extra > extra.txt\ngit add extra.txt\n{WORKING}\necho '{}'\nexit 1\n",
            claude_limit()
        ),
    );

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(claude_sessions(&scenario).len(), 2);
    assert_eq!(scenario.codex_calls().len(), 1);
    assert_eq!(
        scenario.origin_git(&["log", "--first-parent", "--format=%s", "issue-7"]),
        "Merge remote-tracking branch 'origin/main' into issue-7\n\
         Add feature\n\
         Initial commit\n"
    );
    assert_eq!(
        scenario.origin_git(&["show", "issue-7:feature.txt"]),
        "feature\nbase feature\n"
    );
    assert!(
        scenario
            .origin_git(&["ls-tree", "--name-only", "issue-7"])
            .lines()
            .all(|name| name != "extra.txt")
    );
}

#[test]
fn an_explicit_harness_flag_stays_pinned_through_a_limit() {
    let scenario = Scenario::new();
    scenario.user_config_is(&format!("{ROUTES}max_utilization = 85\n"));
    plan_usage_is(
        &scenario,
        json!({"claude": {"observed_at": chrono::Utc::now().to_rfc3339(),
            "windows": [{"name": "five_hour", "used_percent": 90.0, "resets_at": "2096-10-02T07:06:40Z"}]}}),
    );
    scenario.agent_does(&format!("{WORKING}\necho '{}'\nexit 1\n", claude_limit()));

    let result = scenario.run(&[&scenario.issue_url(7), "--harness", "claude"]);

    assert_ne!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(claude_sessions(&scenario).len(), 1);
    assert!(scenario.codex_calls().is_empty());
    assert!(
        !result.stderr.contains("routed by Plan usage"),
        "{}",
        result.stderr
    );
    assert!(
        !result.stderr.contains("starting the Step again"),
        "{}",
        result.stderr
    );
}

#[test]
fn bare_rate_limits_and_agent_written_limit_text_never_route() {
    for script in [
        // A transient HTTP 429 the Harness reports as a failed turn.
        format!(
            "{WORKING}\necho '{}'\nexit 1\n",
            json!({"type": "result", "subtype": "error_during_execution", "is_error": true,
                "errors": ["429 Too Many Requests: rate limited"]})
        ),
        // The agent itself writes limit text.
        format!(
            "echo '{}'\nexit 1\n",
            json!({"type": "assistant", "message": {"content": [{"type": "text",
                "text": "You've hit your usage limit: quota exhausted, spend limit reached"}]}})
        ),
    ] {
        let scenario = Scenario::new();
        scenario.user_config_is(ROUTES);
        scenario.agent_does(&script);

        let result = scenario.run(&[&scenario.issue_url(7)]);

        assert_ne!(result.code, Some(0), "{}", result.stderr);
        assert!(
            scenario.codex_calls().is_empty(),
            "{script}: {}",
            result.stderr
        );
        assert!(
            !result.stderr.contains("starting the Step again"),
            "{}",
            result.stderr
        );
    }
}

#[test]
fn a_security_review_routes_past_a_plan_limited_security_harness_through_harness_routes() {
    let scenario = Scenario::new();
    scenario.user_config_is(
        "[harness]\nroutes = [\"claude\", \"muse\"]\n[security]\nharness = \"codex\"\n",
    );
    let cooldown = json!({"since": "2026-01-01T00:00:00Z", "until": "2096-10-02T07:06:40Z"});
    plan_usage_is(
        &scenario,
        json!({"codex": {"observed_at": chrono::Utc::now().to_rfc3339(),
            "read_at": chrono::Utc::now().to_rfc3339(), "cooldown": cooldown}}),
    );
    let reviewed = scenario.path("reviewed");
    scenario.agent_does(&format!(
        r#"
if printf '%s' "$FAKE_CLAUDE_PROMPT" | grep -q 'guidance mode'; then
    touch '{reviewed}'
    report=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/^Private report file: `\(.*\)`\.$/\1/p')
    test -n "$report"
    printf '%s\n' '[]' > "$report"
    printf '%s\n' 'Security review: {{"unaddressed_count":0,"findings":[],"pre_existing_count":0}}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
    exit 0
fi
{OPENS_PR}
"#,
        reviewed = reviewed.display()
    ));

    let result = scenario.run(&[&scenario.issue_url(7), "security-review", "merge"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(reviewed.exists(), "{}", result.stderr);
    // Codex, in cooldown, never ran; the review ran on the first of
    // harness.routes, Claude Code, after the implement session.
    assert!(scenario.codex_calls().is_empty(), "{}", result.stderr);
    assert_eq!(claude_sessions(&scenario).len(), 2);
    assert!(
        result.stderr.contains("security-review: on claude"),
        "{}",
        result.stderr
    );
    assert!(
        result.stderr.contains("codex is in cooldown"),
        "{}",
        result.stderr
    );
}

#[test]
fn a_spend_cap_that_still_rejects_after_its_backoff_backs_off_twice_as_long() {
    let scenario = Scenario::new();
    scenario.user_config_is(ROUTES);
    // A backoff of an hour that lapsed a minute ago: Claude is revalidating.
    let lapsed = chrono::Utc::now() - chrono::TimeDelta::minutes(1);
    plan_usage_is(
        &scenario,
        json!({"claude": {"observed_at": lapsed.to_rfc3339(), "cooldown": {
            "since": (lapsed - chrono::TimeDelta::hours(1)).to_rfc3339(),
            "until": lapsed.to_rfc3339(),
            "backoff_minutes": 60
        }}}),
    );
    let spend_cap = json!({"type": "result", "subtype": "error_during_execution", "is_error": true,
        "api_error": "usage_limit", "api_error_params": {"rate_limit_info": {"overageStatus": "rejected"}}});
    scenario.agent_does(&format!(
        r#"
if [ -n "${{FAKE_CODEX_ERROR:-}}" ]; then
    {OPENS_PR}
    exit 0
fi
echo '{spend_cap}'
exit 1
"#
    ));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    // Claude was tried once, rejected before its first turn, and the Step
    // went on on Codex with no reset.
    assert_eq!(claude_sessions(&scenario).len(), 1);
    assert_eq!(scenario.codex_calls().len(), 1);
    assert!(
        !result.stderr.contains("discarded its uncommitted changes"),
        "{}",
        result.stderr
    );
    let cooldown = &saved(&scenario)["harnesses"]["claude"]["cooldown"];
    assert_eq!(cooldown["backoff_minutes"], json!(120), "{cooldown:#}");
}
