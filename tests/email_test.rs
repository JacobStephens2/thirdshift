//! `thirdshift email-test [<address>]`: one test email through Resend's HTTP
//! API, here a local stand-in, to check the email setup without a Run.

mod support;

use support::resend::{Request, ResendStandIn};
use support::{RunResult, Scenario};

const KEY: &str = "re_test_123";

const ACCEPTED: &str = r#"{"id":"49a3999c-0ce1-4ea6-ab68-afcd6dc2e794"}"#;

/// Run `thirdshift email-test` with `args` against `resend`, with
/// `RESEND_API_KEY` set to `key`, or unset if `None`.
fn email_test(
    scenario: &Scenario,
    resend: &ResendStandIn,
    args: &[&str],
    key: Option<&str>,
) -> RunResult {
    let mut argv = vec!["email-test"];
    argv.extend_from_slice(args);
    let mut env = vec![("THIRDSHIFT_RESEND_URL", resend.url())];
    if let Some(key) = key {
        env.push(("RESEND_API_KEY", key));
    }
    scenario.run_with_env(&argv, &env)
}

/// Assert the command succeeded, having sent exactly one request, and return
/// it.
fn the_one_request(resend: &ResendStandIn, result: &RunResult) -> Request {
    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert!(
        result
            .stderr
            .contains("accepted by Resend; check your inbox"),
        "stderr: {}",
        result.stderr
    );
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    requests.into_iter().next().unwrap()
}

#[test]
fn sends_one_test_email_to_the_address_given() {
    let scenario = Scenario::new();
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = email_test(&scenario, &resend, &["me@example.com"], Some(KEY));

    let request = the_one_request(&resend, &result);
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/emails");
    assert_eq!(request.authorization.as_deref(), Some("Bearer re_test_123"));
    assert_eq!(request.body["to"], "me@example.com");
    assert_eq!(request.body["from"], "onboarding@resend.dev");
    let subject = request.body["subject"].as_str().unwrap();
    assert!(subject.contains("test"), "subject: {subject}");
    let text = request.body["text"].as_str().unwrap();
    for part in ["Host:", "Time:", "Sender: onboarding@resend.dev"] {
        assert!(text.contains(part), "body lacks {part:?}: {text}");
    }
    assert_eq!(result.stdout, "");
}

#[test]
fn without_an_argument_it_sends_to_email_to() {
    let scenario = Scenario::new();
    scenario.user_config_is("[email]\nto = \"config@example.com\"\n");
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = email_test(&scenario, &resend, &[], Some(KEY));

    let request = the_one_request(&resend, &result);
    assert_eq!(request.body["to"], "config@example.com");
}

#[test]
fn the_argument_wins_over_email_to() {
    let scenario = Scenario::new();
    scenario.user_config_is("[email]\nto = \"config@example.com\"\n");
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = email_test(&scenario, &resend, &["arg@example.com"], Some(KEY));

    let request = the_one_request(&resend, &result);
    assert_eq!(request.body["to"], "arg@example.com");
}

#[test]
fn email_from_replaces_the_default_sender() {
    let scenario = Scenario::new();
    scenario.user_config_is("[email]\nfrom = \"thirdshift@acme.dev\"\n");
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = email_test(&scenario, &resend, &["me@example.com"], Some(KEY));

    let request = the_one_request(&resend, &result);
    assert_eq!(request.body["from"], "thirdshift@acme.dev");
    let text = request.body["text"].as_str().unwrap();
    assert!(text.contains("Sender: thirdshift@acme.dev"), "body: {text}");
}

#[test]
fn with_no_address_known_it_sends_nothing_and_names_the_address() {
    for config in [None, Some("[email]\nfrom = \"thirdshift@acme.dev\"\n")] {
        let scenario = Scenario::new();
        if let Some(config) = config {
            scenario.user_config_is(config);
        }
        let resend = ResendStandIn::replying(200, ACCEPTED);

        let result = email_test(&scenario, &resend, &[], Some(KEY));

        assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
        assert!(
            result.stderr.contains("email.to"),
            "stderr: {}",
            result.stderr
        );
        assert!(resend.requests().is_empty());
    }
}

#[test]
fn without_resend_api_key_it_sends_nothing_and_names_it() {
    for key in [None, Some("")] {
        let scenario = Scenario::new();
        let resend = ResendStandIn::replying(200, ACCEPTED);

        let result = email_test(&scenario, &resend, &["me@example.com"], key);

        assert_eq!(result.code, Some(1), "{key:?}: {}", result.stderr);
        assert!(
            result.stderr.contains("RESEND_API_KEY"),
            "{key:?}: {}",
            result.stderr
        );
        assert!(resend.requests().is_empty(), "{key:?}");
    }
}

#[test]
fn resends_error_text_is_printed_word_for_word() {
    for (status, reply, text) in [
        (
            401,
            r#"{"statusCode":401,"message":"API key is invalid","name":"validation_error"}"#,
            "API key is invalid",
        ),
        (
            500,
            "Internal Server Error: upstream went away",
            "Internal Server Error: upstream went away",
        ),
    ] {
        let scenario = Scenario::new();
        let resend = ResendStandIn::replying(status, reply);

        let result = email_test(&scenario, &resend, &["me@example.com"], Some(KEY));

        assert_eq!(result.code, Some(1), "{status}: {}", result.stderr);
        assert!(result.stderr.contains(text), "{status}: {}", result.stderr);
        assert!(
            !result.stderr.contains("accepted by Resend"),
            "{status}: {}",
            result.stderr
        );
        assert_eq!(resend.requests().len(), 1, "{status}");
    }
}

#[test]
fn an_unknown_key_in_email_is_rejected_naming_it_and_the_file() {
    let scenario = Scenario::new();
    let path = scenario.user_config_is("[email]\nadress = \"me@example.com\"\n");
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = email_test(&scenario, &resend, &["me@example.com"], Some(KEY));

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    for named in ["email.adress", &path.display().to_string()] {
        assert!(result.stderr.contains(named), "stderr: {}", result.stderr);
    }
    assert!(resend.requests().is_empty());
}

#[test]
fn a_second_address_is_an_argument_error() {
    let scenario = Scenario::new();
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = email_test(
        &scenario,
        &resend,
        &["me@example.com", "you@example.com"],
        Some(KEY),
    );

    assert_eq!(result.code, Some(2), "stderr: {}", result.stderr);
    assert!(
        result.stderr.contains("you@example.com"),
        "stderr: {}",
        result.stderr
    );
    assert!(resend.requests().is_empty());
}
