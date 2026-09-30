//! The first Run's offer of Setup: a Run from a terminal with no User config
//! asks, on stderr, whether to set the defaults now, before any work. Yes runs
//! the Setup questions and the Run carries on with the answers; no writes the
//! all-defaults User config. With no terminal, a Run offers nothing and writes
//! nothing.

mod support;

use std::fs;

use support::resend::ResendStandIn;
use support::{CTRL_C, Scenario, TerminalResult};

/// The agent commits its work and opens a PR that closes issue #7, into
/// `main`.
const AGENT_OPENS_PR: &str = r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

const PR_URL: &str = "https://github.com/acme/widgets/pull/1";

const OFFER: &str = "Set your defaults now? [Y/n]";
const MERGE: &str = "Merge run?";
const PULL: &str = "fast-forward";
const NOTIFY: &str = "Run notifications, an email";
const TO: &str = "Send Run notifications to";
const FROM: &str = "Send them from";
const TEST_EMAIL: &str = "test email now?";
const KEY: &str = "re_secret_123";

/// The first line a Run prints once it starts its work.
const FIRST_STEP: &str = "thirdshift: ";

/// Assert the Run ended ready for review or merged, as `outcome` says, with
/// only the PR's URL on stdout.
fn assert_ended(scenario: &Scenario, result: &TerminalResult, outcome: &str, state: &str) {
    assert_eq!(result.code, Some(0), "terminal: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    assert!(
        result
            .stderr
            .trim_end()
            .ends_with(&format!("thirdshift: PR {PR_URL} is {outcome}")),
        "terminal: {}",
        result.stderr
    );
    assert_eq!(scenario.gh_state()["prs"][0]["state"], state);
}

#[test]
fn a_first_run_from_a_terminal_offers_setup_before_any_work() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_OPENS_PR);

    let result = scenario.run_on_terminal(&[&scenario.issue_url(7)], &[], &[(OFFER, "n")]);

    assert_ended(&scenario, &result, "ready for review", "OPEN");
    let path = scenario.path("home/.thirdshift/config.toml");
    let offer = format!("No User config at {}. {OFFER}", path.display());
    let offered = result.stderr.find(&offer).expect(&result.stderr);
    let first_step = result.stderr.find(FIRST_STEP).unwrap();
    assert!(offered < first_step, "terminal: {}", result.stderr);
}

#[test]
fn accepting_and_choosing_merge_always_makes_that_run_a_merge_run() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_OPENS_PR);

    let result = scenario.run_on_terminal(
        &[&scenario.issue_url(7)],
        &[],
        &[(OFFER, ""), (MERGE, "y"), (PULL, ""), (NOTIFY, "")],
    );

    assert_ended(&scenario, &result, "merged", "MERGED");
    let config: toml::Table = result.user_config.unwrap().parse().unwrap();
    assert_eq!(config["merge"]["always"].as_bool(), Some(true));
}

#[test]
fn no_merge_in_the_command_wins_over_a_fresh_merge_always() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_OPENS_PR);

    let result = scenario.run_on_terminal(
        &["--no-merge", &scenario.issue_url(7)],
        &[],
        &[(OFFER, "y"), (MERGE, "y"), (PULL, ""), (NOTIFY, "")],
    );

    assert_ended(&scenario, &result, "ready for review", "OPEN");
    let config: toml::Table = result.user_config.unwrap().parse().unwrap();
    assert_eq!(config["merge"]["always"].as_bool(), Some(true));
}

#[test]
fn no_email_in_the_command_wins_over_fresh_run_notifications() {
    let scenario = Scenario::new();
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
    scenario.agent_does(AGENT_OPENS_PR);

    let result = scenario.run_on_terminal(
        &["--no-email", &scenario.issue_url(7)],
        &[
            ("RESEND_API_KEY", KEY),
            ("THIRDSHIFT_RESEND_URL", resend.url()),
        ],
        &[
            (OFFER, "y"),
            (MERGE, ""),
            (PULL, ""),
            (NOTIFY, "y"),
            (TO, "me@example.com"),
            (FROM, ""),
            (TEST_EMAIL, "n"),
        ],
    );

    assert_ended(&scenario, &result, "ready for review", "OPEN");
    let config: toml::Table = result.user_config.unwrap().parse().unwrap();
    assert_eq!(config["email"]["always"].as_bool(), Some(true));
    assert!(resend.requests().is_empty(), "{:?}", resend.requests());
}

#[test]
fn declining_writes_the_defaults_and_a_second_run_does_not_offer() {
    let unattended = Scenario::new();
    assert_eq!(unattended.run(&["setup"]).code, Some(0));
    let defaults = fs::read_to_string(unattended.path("home/.thirdshift/config.toml")).unwrap();
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_OPENS_PR);

    let first = scenario.run_on_terminal(&[&scenario.issue_url(7)], &[], &[(OFFER, "n")]);

    assert_ended(&scenario, &first, "ready for review", "OPEN");
    assert_eq!(first.user_config.as_deref(), Some(defaults.as_str()));
    assert!(
        first.stderr.contains("thirdshift setup"),
        "terminal: {}",
        first.stderr
    );
    assert!(!first.stderr.contains(MERGE), "terminal: {}", first.stderr);

    scenario.issue_is(8, "OPEN");
    let second = scenario.run_on_terminal(&[&scenario.issue_url(8)], &[], &[]);

    assert!(
        !second.stderr.contains(OFFER),
        "terminal: {}",
        second.stderr
    );
    assert_eq!(second.user_config.as_deref(), Some(defaults.as_str()));
}

#[test]
fn a_bad_command_exits_2_with_no_offer_and_no_file() {
    let scenario = Scenario::new();

    for args in [vec!["--merge"], vec!["not-a-url"], vec!["merge", "merge"]] {
        let result = scenario.run_on_terminal(&args, &[], &[]);

        assert_eq!(result.code, Some(2), "{args:?}: {}", result.stderr);
        assert!(
            !result.stderr.contains(OFFER),
            "{args:?}: {}",
            result.stderr
        );
        assert_eq!(result.user_config, None, "{args:?}");
    }
}

#[test]
fn with_no_terminal_a_run_offers_nothing_and_writes_nothing() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    assert!(!result.stderr.contains(OFFER), "stderr: {}", result.stderr);
    assert!(!scenario.path("home/.thirdshift/config.toml").exists());
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "OPEN");
}

#[test]
fn an_unwritable_user_config_is_a_warning_and_the_run_completes_on_the_defaults() {
    use std::os::unix::fs::PermissionsExt;

    let scenario = Scenario::new();
    scenario.agent_does(AGENT_OPENS_PR);
    let dir = scenario.path("home/.thirdshift");
    fs::create_dir_all(dir.join("logs")).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();

    let result = scenario.run_on_terminal(
        &[&scenario.issue_url(7)],
        &[],
        &[(OFFER, "y"), (MERGE, "y"), (PULL, ""), (NOTIFY, "")],
    );

    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert_ended(&scenario, &result, "ready for review", "OPEN");
    assert!(
        result.stderr.contains("thirdshift: warning: "),
        "terminal: {}",
        result.stderr
    );
    assert_eq!(result.user_config, None);
}

#[test]
fn ctrl_c_at_the_offer_writes_no_file_and_does_no_work() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_OPENS_PR);

    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);

    let result = scenario.run_on_terminal(
        &["--email", "me@example.com", &scenario.issue_url(7)],
        &[
            ("RESEND_API_KEY", KEY),
            ("THIRDSHIFT_RESEND_URL", resend.url()),
        ],
        &[(OFFER, CTRL_C)],
    );

    assert_ne!(result.code, Some(0), "terminal: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert_eq!(result.user_config, None);
    assert!(scenario.claude_calls().is_empty(), "the Run started");
    assert!(
        scenario.gh_calls().is_empty(),
        "thirdshift asked GitHub: {:?}",
        scenario.gh_calls()
    );
    assert!(resend.requests().is_empty(), "a Run notification went");
}

#[test]
fn ctrl_c_during_the_questions_writes_no_file_and_does_no_work() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_OPENS_PR);

    let result = scenario.run_on_terminal(
        &[&scenario.issue_url(7)],
        &[],
        &[(OFFER, "y"), (MERGE, "y"), (PULL, CTRL_C)],
    );

    assert_ne!(result.code, Some(0), "terminal: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert_eq!(result.user_config, None);
    assert!(scenario.claude_calls().is_empty(), "the Run started");
}

#[test]
fn help_version_update_and_email_test_never_offer_setup() {
    let scenario = Scenario::new();

    for args in [
        vec!["help"],
        vec!["version"],
        vec!["update"],
        vec!["email-test", "me@example.com"],
    ] {
        let result = scenario.run_on_terminal(&args, &[], &[]);

        assert!(
            !result.stderr.contains(OFFER),
            "{args:?}: {}",
            result.stderr
        );
        assert_eq!(result.user_config, None, "{args:?}");
    }
}
