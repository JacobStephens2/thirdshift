//! `thirdshift email-test [<address>]`: one test email through Resend's HTTP
//! API, here a local stand-in, to check the email setup without a Run.

mod support;

use std::os::unix::fs::PermissionsExt;

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

/// Credentials holding the key `re_file_456`.
const CREDENTIALS: &str = "[resend]\nkey = \"re_file_456\"\n";

#[test]
fn without_resend_api_key_it_sends_with_the_key_in_the_credentials() {
    for key in [None, Some("")] {
        let scenario = Scenario::new();
        scenario.credentials_are(CREDENTIALS);
        let resend = ResendStandIn::replying(200, ACCEPTED);

        let result = email_test(&scenario, &resend, &["me@example.com"], key);

        let request = the_one_request(&resend, &result);
        assert_eq!(
            request.authorization.as_deref(),
            Some("Bearer re_file_456"),
            "{key:?}"
        );
        assert!(!result.stderr.contains("warning:"), "{}", result.stderr);
    }
}

#[test]
fn resend_api_key_wins_over_the_credentials() {
    let scenario = Scenario::new();
    scenario.credentials_are(CREDENTIALS);
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = email_test(&scenario, &resend, &["me@example.com"], Some(KEY));

    let request = the_one_request(&resend, &result);
    assert_eq!(request.authorization.as_deref(), Some("Bearer re_test_123"));
}

#[test]
fn with_resend_api_key_set_broken_credentials_are_never_read() {
    let scenario = Scenario::new();
    scenario.credentials_are("[resend]\nkye = \"re_file_456\"\n");
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = email_test(&scenario, &resend, &["me@example.com"], Some(KEY));

    let request = the_one_request(&resend, &result);
    assert_eq!(request.authorization.as_deref(), Some("Bearer re_test_123"));
}

#[test]
fn with_no_key_anywhere_it_lists_every_way_to_give_one() {
    for credentials in [None, Some("[resend]\n"), Some("# no key yet\n")] {
        let scenario = Scenario::new();
        if let Some(credentials) = credentials {
            scenario.credentials_are(credentials);
        }
        let resend = ResendStandIn::replying(200, ACCEPTED);

        let result = email_test(&scenario, &resend, &["me@example.com"], None);

        assert_eq!(result.code, Some(1), "{credentials:?}: {}", result.stderr);
        let path = scenario.path("home/.thirdshift/credentials.toml");
        for part in [
            "no Resend API key. Either:",
            "run `thirdshift setup`",
            &format!("add it to {} (mode 600):", path.display()),
            "[resend]",
            "key = \"re_...\"",
            "set RESEND_API_KEY in the environment the Run starts from",
            "(a crontab line, CI secret, or a shell profile the Run's shell reads)",
        ] {
            assert!(
                result.stderr.contains(part),
                "{credentials:?}: expected {part:?} in: {}",
                result.stderr
            );
        }
        assert!(
            !result.stderr.contains("unset or empty"),
            "{}",
            result.stderr
        );
        assert!(resend.requests().is_empty(), "{credentials:?}");
    }
}

#[test]
fn broken_credentials_stop_it_naming_the_file_and_the_key() {
    for (credentials, named) in [
        ("[resend\nkey = \"re_file_456\"\n", "can't parse"),
        ("[resend]\nkye = \"re_file_456\"\n", "resend.kye"),
        ("[resnd]\nkey = \"re_file_456\"\n", "[resnd]"),
        ("key = \"re_file_456\"\n", "key"),
        ("[resend]\nkey = 456\n", "resend.key"),
    ] {
        let scenario = Scenario::new();
        let path = scenario.credentials_are(credentials);
        let resend = ResendStandIn::replying(200, ACCEPTED);

        let result = email_test(&scenario, &resend, &["me@example.com"], None);

        assert_eq!(result.code, Some(1), "{credentials:?}: {}", result.stderr);
        for part in [named, &path.display().to_string()] {
            assert!(
                result.stderr.contains(part),
                "{credentials:?}: expected {part:?} in: {}",
                result.stderr
            );
        }
        assert!(resend.requests().is_empty(), "{credentials:?}");
    }
}

#[test]
fn credentials_others_can_read_are_used_with_a_warning() {
    for mode in [0o640, 0o604, 0o644] {
        let scenario = Scenario::new();
        let path = scenario.credentials_are(CREDENTIALS);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        let resend = ResendStandIn::replying(200, ACCEPTED);

        let result = email_test(&scenario, &resend, &["me@example.com"], None);

        let request = the_one_request(&resend, &result);
        assert_eq!(request.authorization.as_deref(), Some("Bearer re_file_456"));
        let warning = result
            .stderr
            .lines()
            .find(|line| line.contains("warning:"))
            .unwrap_or_else(|| panic!("{mode:o}: no warning in: {}", result.stderr));
        let chmod = format!("chmod 600 {}", path.display());
        assert!(warning.contains(&chmod), "{mode:o}: {warning}");
    }
}

#[test]
fn a_refusal_says_where_the_key_came_from() {
    for from_file in [false, true] {
        let scenario = Scenario::new();
        let path = scenario.credentials_are(CREDENTIALS);
        let resend = ResendStandIn::replying(
            401,
            r#"{"statusCode":401,"message":"API key is invalid","name":"validation_error"}"#,
        );
        let key = if from_file { None } else { Some(KEY) };

        let result = email_test(&scenario, &resend, &["me@example.com"], key);

        assert_eq!(result.code, Some(1), "{from_file}: {}", result.stderr);
        let source = if from_file {
            format!("the Credentials {}", path.display())
        } else {
            "RESEND_API_KEY".to_string()
        };
        assert!(
            result
                .stderr
                .contains(&format!("the key came from {source}")),
            "{from_file}: {}",
            result.stderr
        );
    }
}
