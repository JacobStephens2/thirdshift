//! Run notifications: `email` / `--email`, optionally followed by an address,
//! makes a Run send exactly one email through Resend, here a local stand-in,
//! when it ends, whatever the outcome.

mod support;

use std::process::Command;

use support::resend::{Request, ResendStandIn};
use support::{RunResult, Scenario};

const KEY: &str = "re_test_123";

const ACCEPTED: &str = r#"{"id":"49a3999c-0ce1-4ea6-ab68-afcd6dc2e794"}"#;

const PR_URL: &str = "https://github.com/acme/widgets/pull/1";

/// The agent commits its work and opens a PR that closes issue #7.
const AGENT_OPENS_PR: &str = r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

/// Run thirdshift with `args` against `resend`, with `RESEND_API_KEY` set to
/// `key`, or unset if `None`.
fn run(scenario: &Scenario, resend: &ResendStandIn, args: &[&str], key: Option<&str>) -> RunResult {
    let mut env = vec![("THIRDSHIFT_RESEND_URL", resend.url())];
    if let Some(key) = key {
        env.push(("RESEND_API_KEY", key));
    }
    scenario.run_with_env(args, &env)
}

/// The one request the stand-in received.
fn the_one_request(resend: &ResendStandIn) -> Request {
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    requests.into_iter().next().unwrap()
}

fn subject(request: &Request) -> &str {
    request.body["subject"].as_str().unwrap()
}

fn text(request: &Request) -> &str {
    request.body["text"].as_str().unwrap()
}

/// The path of the one session log the Run wrote.
fn the_one_log(scenario: &Scenario) -> String {
    let logs = scenario.entries("home/.thirdshift/logs");
    assert_eq!(logs.len(), 1, "logs: {logs:?}");
    let log = scenario.path("home/.thirdshift/logs").join(&logs[0]);
    log.to_str().unwrap().to_string()
}

fn hostname() -> String {
    let output = Command::new("hostname").output().unwrap();
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn assert_contains(text: &str, part: &str) {
    assert!(text.contains(part), "expected {part:?} in: {text}");
}

#[test]
fn a_run_ready_for_review_sends_one_notification_to_the_address_given() {
    let scenario = Scenario::new();
    scenario.issue_titled(7, "Add export button");
    scenario.agent_does(AGENT_OPENS_PR);
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run(
        &scenario,
        &resend,
        &[&scenario.issue_url(7), "--email", "me@example.com"],
        Some(KEY),
    );

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    let request = the_one_request(&resend);
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/emails");
    assert_eq!(request.authorization.as_deref(), Some("Bearer re_test_123"));
    assert_eq!(request.body["to"], "me@example.com");
    assert_eq!(
        subject(&request),
        "[thirdshift] acme/widgets#7 Add export button: ready for review"
    );
    let text = text(&request);
    assert_contains(text, PR_URL);
    assert_contains(text, &the_one_log(&scenario));
    assert_contains(text, &hostname());
    assert_contains(text, "Took:");
}

#[test]
fn a_merge_run_sends_one_notification_that_it_merged() {
    let scenario = Scenario::new();
    scenario.issue_titled(7, "Add export button");
    scenario.agent_does(AGENT_OPENS_PR);
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run(
        &scenario,
        &resend,
        &["email", "me@example.com", "merge", &scenario.issue_url(7)],
        Some(KEY),
    );

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let request = the_one_request(&resend);
    assert_eq!(
        subject(&request),
        "[thirdshift] acme/widgets#7 Add export button: merged"
    );
    assert_contains(text(&request), PR_URL);
}

#[test]
fn a_failed_run_sends_one_notification_with_the_cause() {
    let scenario = Scenario::new();
    scenario.issue_titled(7, "Add export button");
    scenario.agent_does("echo 'half done' > wip.txt\nexit 3\n");
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run(
        &scenario,
        &resend,
        &["--email", "me@example.com", &scenario.issue_url(7)],
        Some(KEY),
    );

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    let request = the_one_request(&resend);
    assert_eq!(
        subject(&request),
        "[thirdshift] acme/widgets#7 Add export button: failed"
    );
    let text = text(&request);
    assert_contains(text, "claude exited 3");
    assert_contains(text, &the_one_log(&scenario));
}

#[test]
fn an_interrupted_run_sends_one_notification_that_it_was_interrupted() {
    let scenario = Scenario::new();
    let started = scenario.path("agent-started");
    scenario.agent_does(&format!("touch {}\nsleep 30\n", started.display()));
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let child = scenario.run_and_signal_with_env(
        &[&scenario.issue_url(7), "--email", "me@example.com"],
        &[
            ("THIRDSHIFT_RESEND_URL", resend.url()),
            ("RESEND_API_KEY", KEY),
        ],
        "agent-started",
        "TERM",
    );

    assert_eq!(child.code, Some(1), "stderr: {}", child.stderr);
    let request = the_one_request(&resend);
    assert!(
        subject(&request).ends_with(": interrupted"),
        "subject: {}",
        subject(&request)
    );
}

#[test]
fn a_failure_before_the_issue_is_read_sends_a_notification_without_a_title() {
    let scenario = Scenario::new();
    let resend = ResendStandIn::replying(200, ACCEPTED);
    let url = "https://github.com/other/widgets/issues/7";

    let result = run(
        &scenario,
        &resend,
        &["--email", "me@example.com", url],
        Some(KEY),
    );

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    let request = the_one_request(&resend);
    assert_eq!(subject(&request), "[thirdshift] other/widgets#7: failed");
    assert_contains(text(&request), "origin mismatch");
}

#[test]
fn a_bare_flag_sends_to_email_to_and_an_address_after_the_flag_wins() {
    for (args, to) in [
        (vec!["--email"], "config@example.com"),
        (vec!["email"], "config@example.com"),
        (vec!["--email", "flag@example.com"], "flag@example.com"),
    ] {
        let scenario = Scenario::new();
        scenario.user_config_is("[email]\nto = \"config@example.com\"\n");
        scenario.agent_does(AGENT_OPENS_PR);
        let resend = ResendStandIn::replying(200, ACCEPTED);
        let url = scenario.issue_url(7);
        let mut argv = vec![url.as_str()];
        argv.extend(args.iter().copied());

        let result = run(&scenario, &resend, &argv, Some(KEY));

        assert_eq!(result.code, Some(0), "{args:?}: {}", result.stderr);
        assert_eq!(the_one_request(&resend).body["to"], to, "{args:?}");
    }
}

#[test]
fn the_flag_works_in_any_position_and_never_takes_the_issue_url_as_its_address() {
    let url = Scenario::new().issue_url(7);
    for args in [
        vec!["--email", url.as_str()],
        vec!["email", url.as_str(), "merge"],
        vec!["merge", "--email", url.as_str()],
        vec!["--email", "me@example.com", url.as_str(), "merge"],
        vec![url.as_str(), "merge", "email"],
        vec![url.as_str(), "--email", "merge"],
    ] {
        let scenario = Scenario::new();
        scenario.user_config_is("[email]\nto = \"me@example.com\"\n");
        scenario.agent_does(AGENT_OPENS_PR);
        let resend = ResendStandIn::replying(200, ACCEPTED);

        let result = run(&scenario, &resend, &args, Some(KEY));

        assert_eq!(result.code, Some(0), "{args:?}: {}", result.stderr);
        let request = the_one_request(&resend);
        assert_eq!(request.body["to"], "me@example.com", "{args:?}");
        let outcome = if args.contains(&"merge") {
            "merged"
        } else {
            "ready for review"
        };
        assert!(
            subject(&request).ends_with(&format!(": {outcome}")),
            "{args:?}: {}",
            subject(&request)
        );
    }
}

#[test]
fn with_no_address_known_the_run_stops_before_any_work_and_sends_nothing() {
    let scenario = Scenario::new();
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run(
        &scenario,
        &resend,
        &["--email", &scenario.issue_url(7)],
        Some(KEY),
    );

    scenario.assert_rejected_before_any_work(&result, "no email address");
    assert!(scenario.gh_calls().is_empty(), "{:?}", scenario.gh_calls());
    assert!(resend.requests().is_empty());
}

#[test]
fn without_resend_api_key_the_run_stops_before_any_work_and_sends_nothing() {
    for key in [None, Some("")] {
        let scenario = Scenario::new();
        let resend = ResendStandIn::replying(200, ACCEPTED);

        let result = run(
            &scenario,
            &resend,
            &[&scenario.issue_url(7), "email", "me@example.com"],
            key,
        );

        scenario.assert_rejected_before_any_work(&result, "RESEND_API_KEY");
        assert!(scenario.gh_calls().is_empty(), "{:?}", scenario.gh_calls());
        assert!(resend.requests().is_empty());
    }
}

#[test]
fn a_run_without_the_flag_sends_nothing() {
    let scenario = Scenario::new();
    scenario.user_config_is("[email]\nto = \"me@example.com\"\n");
    scenario.agent_does(AGENT_OPENS_PR);
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run(&scenario, &resend, &[&scenario.issue_url(7)], Some(KEY));

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert!(resend.requests().is_empty());
}

#[test]
fn a_failed_send_is_a_warning_that_changes_neither_the_exit_code_nor_stdout() {
    for (status, reply, message) in [
        (
            401,
            r#"{"statusCode":401,"message":"API key is invalid","name":"validation_error"}"#,
            "API key is invalid",
        ),
        (500, "upstream exploded", "upstream exploded"),
    ] {
        for (agent, code, stdout) in [
            (AGENT_OPENS_PR, 0, format!("{PR_URL}\n")),
            ("exit 3", 1, String::new()),
        ] {
            let scenario = Scenario::new();
            scenario.agent_does(agent);
            let resend = ResendStandIn::replying(status, reply);

            let result = run(
                &scenario,
                &resend,
                &[&scenario.issue_url(7), "--email", "me@example.com"],
                Some(KEY),
            );

            assert_eq!(result.code, Some(code), "{status}: {}", result.stderr);
            assert_eq!(result.stdout, stdout, "{status}");
            assert_eq!(resend.requests().len(), 1, "{status}");
            let warning = result
                .stderr
                .lines()
                .find(|line| line.contains("warning:"))
                .unwrap_or_else(|| panic!("no warning in stderr: {}", result.stderr));
            assert_contains(warning, message);
        }
    }
}
