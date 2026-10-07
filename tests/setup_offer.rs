//! The first Run's offer of Setup, wired into a Run: a Run from a terminal
//! with no User config asks, on stderr, whether to set the defaults now,
//! before any work, and carries on with the answers. With no terminal, a Run
//! offers nothing and writes nothing; no other command offers it; and Ctrl-C
//! at the offer or during the questions ends the command with no work done.
//! The offer's rules are tested in-process, in the Setup module.

mod support;

use support::resend::ResendStandIn;
use support::{CTRL_C, Scenario, TerminalResult, TerminalStep};

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
const HARNESS: &str = "Harness for every Run's sessions";
const MODEL: &str = "Model for claude";
const EFFORT: &str = "Effort for claude";
const MERGE: &str = "Merge run?";
const BASE_FIX: &str = "Every Run may start a Base fix when the Base branch's CI is red?";
const PULL: &str = "fast-forward";
const NOTIFY: &str = "Run notifications, an email";
const KEY: &str = "re_secret_123";

/// The line a Run prints as it starts its work, after the one saying it is
/// starting and when.
const FIRST_STEP: &str = "thirdshift: creating worktree ";

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
        &[
            (OFFER, ""),
            (HARNESS, ""),
            (MODEL, ""),
            (EFFORT, ""),
            (MERGE, "y"),
            (BASE_FIX, ""),
            (PULL, ""),
            (NOTIFY, ""),
        ],
    );

    assert_ended(&scenario, &result, "merged", "MERGED");
    let config: toml::Table = result.user_config.unwrap().parse().unwrap();
    assert_eq!(config["merge"]["always"].as_bool(), Some(true));
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
    assert_eq!(scenario.credentials(), None);
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "OPEN");
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
        &[
            (OFFER, "y"),
            (HARNESS, ""),
            (MODEL, ""),
            (EFFORT, ""),
            (MERGE, "y"),
            (BASE_FIX, ""),
            (PULL, CTRL_C),
        ],
    );

    assert_ne!(result.code, Some(0), "terminal: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert_eq!(result.user_config, None);
    assert!(scenario.claude_calls().is_empty(), "the Run started");
}

#[test]
fn cancelling_offered_setup_catalogs_and_minimal_calls_stops_before_factory_work() {
    use std::time::{Duration, Instant};
    use support::check::{DuringCheck, OwnedCheck};
    for (harness, model, signal) in [
        ("claude", Some("opus"), libc::SIGINT),
        ("codex", None, libc::SIGTERM),
        ("agy", None, libc::SIGHUP),
        ("grok", None, libc::SIGINT),
        ("muse", Some("muse-spark-1.3"), libc::SIGTERM),
        ("opencode", Some("provider/model"), libc::SIGHUP),
    ] {
        let scenario = Scenario::new();
        let credentials = "[resend]\nkey = \"re_saved\"\n";
        scenario.credentials_are(credentials);
        let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
        let check = OwnedCheck::new(&scenario, DuringCheck::AfterExit);
        let model_prompt = format!("Model for {harness}");
        let effort_prompt = format!("Effort for {harness}");
        let mut keys = vec![
            TerminalStep::line(OFFER, "y"),
            TerminalStep::line(HARNESS, harness),
        ];
        if let Some(model) = model {
            keys.extend([
                TerminalStep::line(&model_prompt, model),
                TerminalStep::line(&effort_prompt, ""),
            ]);
        }
        keys.push(TerminalStep::interrupt_check(signal));
        let mut env = check.env();
        env.push(("THIRDSHIFT_RESEND_URL", resend.url()));
        let started = Instant::now();
        let result = scenario.run_terminal(
            &["--email", "me@example.com", &scenario.issue_url(7)],
            &env,
            &keys,
        );

        assert_eq!(result.code, Some(1), "{harness}: {}", result.stderr);
        assert!(result.stderr.contains("interrupted"), "{}", result.stderr);
        assert_eq!(
            result.stderr.matches("interrupted").count(),
            1,
            "{}",
            result.stderr
        );
        assert!(started.elapsed() < Duration::from_secs(3));
        check.assert_stopped();
        assert_eq!(result.user_config, None);
        assert_eq!(scenario.credentials().as_deref(), Some(credentials));
        assert!(result.terminal_restored);
        assert!(scenario.issue_labels(7).is_empty(), "the Claim was made");
        assert_eq!(scenario.entries("work"), ["widgets"]);
        assert!(
            !scenario
                .path("home/.thirdshift/logs/acme/widgets/commands")
                .exists()
        );
        assert!(resend.requests().is_empty(), "a Run notification went");
    }
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
