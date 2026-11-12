//! Captured issue and PR titles survive through the owning Command's notification.

mod support;

use serde_json::json;
use support::resend::{Request, ResendStandIn, assert_saved_notification};
use support::{RunResult, Scenario};

const TITLE: &str = "Export widgets with colors";

fn run_emailed(scenario: &Scenario, args: &[&str], interrupt: bool) -> (RunResult, ResendStandIn) {
    let resend = ResendStandIn::replying(200, r#"{"id":"accepted"}"#);
    let mut args = args.to_vec();
    args.extend(["email", "me@example.com"]);
    let env = [
        ("THIRDSHIFT_RESEND_URL", resend.url()),
        ("RESEND_API_KEY", "re_test_titles"),
    ];
    let result = if interrupt {
        scenario.run_and_signal_with_env(&args, &env, "agent-started", "TERM")
    } else {
        scenario.run_with_env(&args, &env)
    };
    (result, resend)
}

fn one_request(resend: &ResendStandIn) -> Request {
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    requests.into_iter().next().unwrap()
}

fn write_process(scenario: &Scenario, source: &str) {
    let dir = scenario.path("home/.thirdshift/processes");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.toml"), source).unwrap();
}

fn assert_pr_notice(resend: &ResendStandIn, command: &str, url: &str, outcome: &str) -> Request {
    let request = one_request(resend);
    assert_eq!(
        request.body["subject"],
        format!("[thirdshift] acme/widgets {command} #1 {TITLE}: {outcome}")
    );
    let text = request.body["text"].as_str().unwrap();
    assert!(
        text.contains(&format!("Command:      {command}\n")),
        "{text}"
    );
    assert!(
        text.contains(&format!("Pull request: {url}\nTitle:        {TITLE}\n")),
        "{text}"
    );
    assert_eq!(
        text.lines()
            .filter(|line| line.starts_with("Pull request:"))
            .count(),
        1,
        "{text}"
    );
    request
}

fn pull_request(scenario: &Scenario) -> String {
    scenario.origin_has_branch("colleague", "main", &["Colleague feature"]);
    let url = scenario.github_has_pr("colleague", "main", "OPEN");
    let mut state = scenario.gh_state();
    state["prs"][0]["title"] = json!(TITLE);
    scenario.write_gh_state(&state);
    url
}

#[test]
fn review_notification_keeps_the_captured_pr_title() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario
        .agent_does("printf 'No findings. Validation passed.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
    let (result, resend) = run_emailed(&scenario, &["review", &url, "--recommend-only"], false);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let request = assert_pr_notice(&resend, "Review run", &url, "ready for review");
    assert_saved_notification(&scenario, "review", &request);
}

#[test]
fn fix_notification_keeps_the_captured_pr_title() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    let (result, resend) = run_emailed(&scenario, &["fix", &url], false);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let request = assert_pr_notice(&resend, "Fix run", &url, "ready for review");
    assert_saved_notification(&scenario, "fix", &request);
}

#[test]
fn addressing_author_selected_and_custom_reviews_keep_titles_after_later_stops() {
    for mode in [
        "address",
        "author",
        "custom",
        "failed",
        "interrupted",
        "custom-failed",
    ] {
        let scenario = Scenario::new();
        let url = pull_request(&scenario);
        let custom = mode.starts_with("custom");
        if custom {
            write_process(
                &scenario,
                r#"version = 1
description = "Review"
[worktrees.review]
kind = "pull-request"
[[steps]]
name = "review"
catalogue = "review-and-address"
worktree = "review"
[[steps]]
name = "deliver"
catalogue = "delivery"
worktree = "review"
"#,
            );
        }
        let interrupt = mode == "interrupted";
        scenario.agent_does(&match mode {
            "failed" | "custom-failed" => "gh fake fails 'pr view'\nexit 1".into(),
            "interrupted" => format!(
                "touch '{}'\nsleep 60",
                scenario.path("agent-started").display()
            ),
            _ => "printf 'No findings. Validation passed.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"".into(),
        });
        let args = if custom {
            vec!["process", "notes", "pull-request", &url]
        } else if mode == "author" {
            vec!["review", "--author", "colleague"]
        } else {
            vec!["review", &url]
        };
        let (result, resend) = run_emailed(&scenario, &args, interrupt);
        let outcome = if interrupt {
            "interrupted"
        } else if mode.ends_with("failed") {
            "failed"
        } else {
            "ready for review"
        };
        assert_eq!(
            result.code,
            Some(if outcome == "ready for review" { 0 } else { 1 }),
            "{mode}: {}",
            result.stderr
        );
        let request = assert_pr_notice(
            &resend,
            if custom {
                "Process notes"
            } else {
                "Review run"
            },
            &url,
            outcome,
        );
        assert_saved_notification(
            &scenario,
            if custom { "process" } else { "review" },
            &request,
        );
    }
}

#[test]
fn fix_retains_its_title_after_later_failure_or_interruption() {
    for interrupt in [false, true] {
        let scenario = Scenario::new();
        let url = pull_request(&scenario);
        let head = scenario.origin_git(&["rev-parse", "colleague"]);
        let mut state = scenario.gh_state();
        state["checks"][head.trim()] =
            json!([{"name":"test","conclusion":"failure","url":"https://ci.example/test"}]);
        scenario.write_gh_state(&state);
        scenario.agent_does(&if interrupt {
            format!(
                "touch '{}'\nsleep 60",
                scenario.path("agent-started").display()
            )
        } else {
            "exit 1".into()
        });
        let (result, resend) = run_emailed(&scenario, &["fix", &url, "no-base-fix"], interrupt);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        let request = assert_pr_notice(
            &resend,
            "Fix run",
            &url,
            if interrupt { "interrupted" } else { "failed" },
        );
        assert_saved_notification(&scenario, "fix", &request);
    }
}

#[test]
fn issue_process_notifications_keep_titles_and_unavailable_title_fallback() {
    for mode in ["success", "failed", "interrupted", "unavailable"] {
        let scenario = Scenario::new();
        scenario.issue_titled(7, TITLE);
        let interrupt = mode == "interrupted";
        let command = if interrupt {
            format!(
                "touch '{}'\nsleep 60",
                scenario.path("agent-started").display()
            )
        } else if mode == "failed" {
            // Completion must use captured metadata even after GitHub becomes unavailable.
            "gh fake fails 'issue view'\nexit 1".into()
        } else {
            "printf done".into()
        };
        write_process(
            &scenario,
            &format!(
                "version = 1\ndescription = \"Notes\"\n[[steps]]\nname = \"notes\"\ncommand = '''{command}'''\n"
            ),
        );
        if mode == "unavailable" {
            scenario.gh_fails("issue view 7 --repo acme/widgets --json title");
        }
        let url = scenario.issue_url(7);
        let (result, resend) =
            run_emailed(&scenario, &["process", "notes", "issue", &url], interrupt);
        let outcome = if interrupt {
            "interrupted"
        } else if mode == "failed" {
            "failed"
        } else {
            "succeeded"
        };
        assert_eq!(
            result.code,
            Some(if outcome == "succeeded" { 0 } else { 1 }),
            "{mode}: {}",
            result.stderr
        );
        let request = one_request(&resend);
        let suffix = if mode == "unavailable" {
            String::new()
        } else {
            format!(" #7 {TITLE}")
        };
        assert_eq!(
            request.body["subject"],
            format!("[thirdshift] acme/widgets Process notes{suffix}: {outcome}")
        );
        let text = request.body["text"].as_str().unwrap();
        assert!(text.contains("Command:      Process notes\n"), "{text}");
        assert!(text.contains(&format!("Issue:        {url}\n")), "{text}");
        assert_eq!(
            text.contains(&format!("Title:        {TITLE}\n")),
            mode != "unavailable",
            "{text}"
        );
        assert_saved_notification(&scenario, "process", &request);
    }
}

#[test]
fn unreadable_pr_metadata_keeps_the_existing_subject_and_url() {
    for command in ["review", "fix"] {
        let scenario = Scenario::new();
        let url = pull_request(&scenario);
        scenario.gh_fails("api repos/acme/widgets/pulls/1");
        let (result, resend) = run_emailed(&scenario, &[command, &url], false);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        let request = one_request(&resend);
        let name = if command == "review" {
            "Review run"
        } else {
            "Fix run"
        };
        assert_eq!(
            request.body["subject"],
            format!("[thirdshift] acme/widgets {name}: failed")
        );
        let text = request.body["text"].as_str().unwrap();
        assert!(!text.contains("Title:"), "{text}");
        // Review already knows its explicit target even if GitHub cannot read it.
        if command == "review" {
            assert!(text.contains(&url), "{text}");
        }
        // These preflight failures precede allocation of a Command log and saved copy.
    }
}

#[test]
fn author_review_conflict_notification_includes_the_snapshot_title() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario.launch_git(&["branch", "colleague", "main"]);

    let (result, resend) = run_emailed(&scenario, &["review", "--author", "colleague"], false);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(scenario.claude_calls().is_empty());
    // A Pass that only encounters a conflict is skipped, without a Command log.
    assert_pr_notice(&resend, "Review run", &url, "failed");
}
