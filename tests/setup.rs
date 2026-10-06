//! `thirdshift setup`, wired to the command line and a real terminal: with no
//! terminal it writes the defaults, and a Run with them behaves as one with
//! none; on a terminal, pressing Enter throughout writes the same, an
//! entered key is saved in the Credentials and never shown, and Ctrl-C
//! anywhere in the questions writes nothing. Setup's rules are tested
//! in-process, in the Setup module.

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;

use support::{CTRL_C, Keystrokes, Scenario, TerminalResult};

/// The agent commits its work and opens a PR that closes issue #7, into
/// `main`.
const AGENT_OPENS_PR: &str = r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

const PR_URL: &str = "https://github.com/acme/widgets/pull/1";

fn user_config(scenario: &Scenario) -> Option<String> {
    fs::read_to_string(scenario.path("home/.thirdshift/config.toml")).ok()
}

#[test]
fn with_no_terminal_and_no_user_config_setup_writes_the_defaults() {
    let scenario = Scenario::new();
    scenario.git_email_is(None);

    let result = scenario.run(&["setup"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    let text = user_config(&scenario).expect("no User config written");
    let config: toml::Table = text.parse().unwrap();
    let sections: Vec<&str> = text.lines().filter(|line| line.starts_with('[')).collect();
    assert_eq!(
        sections,
        [
            "[merge]",
            "[base]",
            "[launch]",
            "[email]",
            "[logs]",
            "[activity]",
            "[spec]",
            "[pickup]",
            "[harness]",
            "[harness.claude]",
            "[harness.codex]",
            "[harness.agy]",
            "[harness.grok]",
            "[harness.muse]",
            "[harness.opencode]"
        ],
        "{text}"
    );
    assert_eq!(config["merge"]["always"].as_bool(), Some(false));
    assert_eq!(config["base"]["fix"].as_bool(), Some(false));
    assert_eq!(config["launch"]["pull"].as_bool(), Some(false));
    assert_eq!(config["email"]["always"].as_bool(), Some(false));
    assert_eq!(
        config["email"]["from"].as_str(),
        Some("onboarding@resend.dev")
    );
    assert!(config["email"].get("to").is_none(), "{text}");
    assert_eq!(config["logs"]["dir"].as_str(), Some("~/.thirdshift/logs"));
    assert_eq!(config["activity"]["quiet_skips"].as_bool(), Some(false));
    assert_eq!(config["spec"]["parallel"].as_integer(), Some(3));
    assert_eq!(config["pickup"]["limit"].as_integer(), Some(3));
    assert_eq!(config["harness"]["default"].as_str(), Some("claude"));
    for harness in ["claude", "codex", "agy", "grok", "muse", "opencode"] {
        for key in ["model", "effort"] {
            assert_eq!(config["harness"][harness][key].as_str(), Some(""), "{text}");
        }
    }
    assert!(
        result.stderr.contains(".thirdshift/config.toml"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn a_run_with_the_written_user_config_behaves_as_with_none() {
    let scenario = Scenario::new();
    assert_eq!(scenario.run(&["setup"]).code, Some(0));
    scenario.agent_does(AGENT_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    assert_eq!(
        result.stderr.lines().last(),
        Some(format!("thirdshift: PR {PR_URL} is ready for review").as_str()),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "OPEN");
    assert_eq!(
        scenario
            .entries("home/.thirdshift/logs/acme/widgets/sessions")
            .len(),
        1
    );
}

#[test]
fn setup_with_an_argument_is_an_argument_error_and_writes_nothing() {
    let scenario = Scenario::new();

    for args in [vec!["setup", "extra"], vec!["setup", "--merge", "x"]] {
        let result = scenario.run(&args);

        assert_eq!(result.code, Some(2), "{args:?}: {}", result.stderr);
        assert_eq!(result.stdout, "", "{args:?}");
        assert!(
            result.stderr.starts_with(&format!(
                "thirdshift: unexpected argument after setup: {}\n",
                args[1]
            )),
            "{args:?}: {}",
            result.stderr
        );
        let help = scenario.run(&["help"]).stdout;
        assert!(
            result.stderr.ends_with(&help),
            "{args:?}: {}",
            result.stderr
        );
        assert_eq!(user_config(&scenario), None, "{args:?}");
    }
}

// Setup from a terminal.

const HARNESS: &str = "Harness for every Run's sessions";
const MODEL: &str = "Model for claude";
const EFFORT: &str = "Effort for claude";
const MERGE: &str = "Merge run?";
const BASE_FIX: &str = "Every Run may start a Base fix when the Base branch's CI is red?";
const PULL: &str = "fast-forward";
const NOTIFY: &str = "Run notifications, an email";
const TO: &str = "Send Run notifications to";
const FROM: &str = "Send them from";
const TEST_EMAIL: &str = "test email now?";
const KEY_PROMPT: &str = "Resend API key (input hidden, Enter to skip):";
const KEPT: &str = "Resend API key (input hidden, Enter keeps the saved one):";
const WROTE_CREDENTIALS: &str = "wrote the Credentials";
const KEY: &str = "re_secret_123";

/// Run `thirdshift setup` on a terminal, typing `keystrokes`, and check it
/// succeeded with nothing on stdout.
fn setup_on_terminal(
    scenario: &Scenario,
    env: &[(&str, &str)],
    keystrokes: &[Keystrokes],
) -> TerminalResult {
    let result = scenario.run_on_terminal(&["setup"], env, keystrokes);
    assert_eq!(result.code, Some(0), "terminal: {}", result.stderr);
    assert_eq!(result.stdout, "");
    result
}

/// The keystrokes that turn Run notifications on, to `me@example.com` from
/// the default sender, then `rest`.
fn notifications_on<'a>(rest: &[Keystrokes<'a>]) -> Vec<Keystrokes<'a>> {
    let mut keystrokes = vec![
        (HARNESS, ""),
        (MODEL, ""),
        (EFFORT, ""),
        (MERGE, ""),
        (PULL, ""),
        (NOTIFY, "y"),
        (TO, "me@example.com"),
        (FROM, ""),
    ];
    keystrokes.extend_from_slice(rest);
    keystrokes
}

#[test]
fn on_a_terminal_pressing_enter_throughout_writes_what_setup_with_no_terminal_writes() {
    let unattended = Scenario::new();
    unattended.git_email_is(None);
    assert_eq!(unattended.run(&["setup"]).code, Some(0));
    let scenario = Scenario::new();
    scenario.git_email_is(None);

    let result = setup_on_terminal(
        &scenario,
        &[],
        &[
            (HARNESS, ""),
            (MODEL, ""),
            (EFFORT, ""),
            (MERGE, ""),
            (PULL, ""),
            (NOTIFY, ""),
        ],
    );

    assert_eq!(result.user_config, user_config(&unattended));
    for prompt in [MERGE, PULL, NOTIFY] {
        assert!(
            result.stderr.contains(prompt),
            "terminal: {}",
            result.stderr
        );
    }
}

#[test]
fn on_a_terminal_an_entered_key_is_written_to_the_credentials_and_never_shown() {
    let scenario = Scenario::new();

    let result = setup_on_terminal(
        &scenario,
        &[],
        &notifications_on(&[(KEY_PROMPT, &format!("  {KEY}  ")), (TEST_EMAIL, "")]),
    );

    let path = scenario.path("home/.thirdshift/credentials.toml");
    assert!(
        result
            .stderr
            .contains(&format!("{WROTE_CREDENTIALS} {}", path.display())),
        "terminal: {}",
        result.stderr
    );
    let credentials: toml::Table = scenario.credentials().unwrap().parse().unwrap();
    assert_eq!(credentials["resend"]["key"].as_str(), Some(KEY));
    let mode = fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "{mode:o}");
    for part in [&KEY[..6], &KEY[KEY.len() - 6..]] {
        assert!(!result.stderr.contains(part), "{part}: {}", result.stderr);
    }
    assert!(!result.user_config.unwrap().contains(KEY));
}

#[test]
fn ctrl_c_at_the_key_prompt_writes_neither_file() {
    let scenario = Scenario::new();

    let result =
        scenario.run_on_terminal(&["setup"], &[], &notifications_on(&[(KEY_PROMPT, CTRL_C)]));

    assert_ne!(result.code, Some(0), "terminal: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert_eq!(result.user_config, None);
    assert_eq!(scenario.credentials(), None);
}

#[test]
fn ctrl_c_at_the_key_prompt_leaves_saved_credentials_unchanged() {
    let scenario = Scenario::new();
    let saved = "[resend]\nkey = \"re_saved_456\"\n";
    scenario.credentials_are(saved);

    let result = scenario.run_on_terminal(&["setup"], &[], &notifications_on(&[(KEPT, CTRL_C)]));

    assert_ne!(result.code, Some(0), "terminal: {}", result.stderr);
    assert_eq!(result.user_config, None);
    assert_eq!(scenario.credentials().as_deref(), Some(saved));
}

#[test]
fn ctrl_c_during_the_questions_writes_no_user_config() {
    let scenario = Scenario::new();

    let result = scenario.run_on_terminal(
        &["setup"],
        &[],
        &[
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
}

#[test]
fn ctrl_c_during_the_questions_leaves_an_existing_user_config_unchanged() {
    let scenario = Scenario::new();
    let mine = "[merge]\nalways = true # mine\n";
    scenario.user_config_is(mine);

    let result = scenario.run_on_terminal(
        &["setup"],
        &[],
        &[
            (HARNESS, ""),
            (MODEL, ""),
            (EFFORT, ""),
            (MERGE, "n"),
            (PULL, "y"),
            (NOTIFY, "y"),
            (TO, CTRL_C),
        ],
    );

    assert_ne!(result.code, Some(0), "terminal: {}", result.stderr);
    assert_eq!(result.user_config.as_deref(), Some(mine));
}
