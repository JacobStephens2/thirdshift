//! `thirdshift setup`, wired to the command line and a real terminal: with no
//! terminal it writes the defaults, and a Run with them behaves as one with
//! none; on a terminal, pressing Enter throughout writes the same, an
//! entered key is saved in the Credentials and never shown, and Ctrl-C
//! anywhere in the questions writes nothing. Setup's rules are tested
//! in-process, in the Setup module.

mod support;

use std::fs;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

use support::{CTRL_C, Keystrokes, Scenario, TerminalResult, TerminalStep};

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
            "[harness.opencode]",
            "[security]"
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
    assert_eq!(config["pickup"]["wait_minutes"].as_integer(), Some(30));
    assert_eq!(config["security"]["fix"].as_bool(), Some(false));
    assert_eq!(config["security"]["harness"].as_str(), Some(""));
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
fn upgrading_setup_puts_security_after_all_harness_settings() {
    let scenario = Scenario::new();
    let original = "\
[merge]
always = false
[base]
fix = false
[launch]
pull = false
[email]
always = false
from = 'onboarding@resend.dev'
[logs]
dir = '~/.thirdshift/logs'
[activity]
quiet_skips = false
[spec]
parallel = 3
[pickup]
limit = 3
wait_minutes = 30
[harness]
default = 'codex'
[harness.claude]
model = ''
effort = ''
[harness.codex]
model = 'gpt-6.1-sol' # my Model
effort = 'high' # my Effort
";
    scenario.user_config_is(original);

    let result = scenario.run(&["setup"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let text = user_config(&scenario).unwrap();
    let sections: Vec<&str> = text.lines().filter(|line| line.starts_with('[')).collect();
    let harness = sections
        .iter()
        .position(|section| *section == "[harness]")
        .unwrap();
    assert_eq!(
        &sections[harness..],
        [
            "[harness]",
            "[harness.claude]",
            "[harness.codex]",
            "[harness.agy]",
            "[harness.grok]",
            "[harness.muse]",
            "[harness.opencode]",
            "[security]",
        ],
        "{text}"
    );
    assert!(
        text.contains("model = 'gpt-6.1-sol' # my Model\n"),
        "{text}"
    );
    assert!(text.contains("effort = 'high' # my Effort\n"), "{text}");
    let config: toml::Table = text.parse().unwrap();
    assert_eq!(config["harness"]["default"].as_str(), Some("codex"));
    assert_eq!(config["security"]["fix"].as_bool(), Some(false));
    assert_eq!(config["security"]["review"].as_bool(), Some(false));
    assert_eq!(config["security"]["harness"].as_str(), Some(""));

    let again = scenario.run(&["setup"]);
    assert_eq!(again.code, Some(0), "stderr: {}", again.stderr);
    assert_eq!(user_config(&scenario).unwrap(), text);
}

#[test]
fn completing_an_existing_file_replaces_it_atomically_keeps_permissions_and_then_avoids_replacement()
 {
    let scenario = Scenario::new();
    let original = "# my machine\n[logs]\ndir = '/var/log/ts' # custom\n\
                    [merge]\nalways = true # merge\n\
                    [pickup]\nwait_minutes = 60 # time to attach Tickets\n";
    let path = scenario.user_config_is(original);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let mut previous_file = fs::File::open(&path).unwrap();
    let old_inode = previous_file.metadata().unwrap().ino();

    let result = scenario.run(&["setup"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let text = user_config(&scenario).unwrap();
    assert!(text.starts_with(original), "{text}");
    let config: toml::Table = text.parse().unwrap();
    assert_eq!(config["logs"]["dir"].as_str(), Some("/var/log/ts"));
    assert_eq!(config["pickup"]["wait_minutes"].as_integer(), Some(60));
    assert_eq!(config["harness"]["default"].as_str(), Some("claude"));
    let metadata = fs::metadata(&path).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o777, 0o640);
    assert_ne!(metadata.ino(), old_inode);
    let mut previous_text = String::new();
    previous_file.read_to_string(&mut previous_text).unwrap();
    assert_eq!(previous_text, original);

    let again = scenario.run(&["setup"]);

    assert_eq!(again.code, Some(0), "stderr: {}", again.stderr);
    assert!(
        again.stderr.contains("already lists every setting"),
        "{}",
        again.stderr
    );
    assert_eq!(user_config(&scenario).unwrap(), text);
    assert_eq!(fs::metadata(&path).unwrap().ino(), metadata.ino());
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
const SECURITY_REVIEW: &str = "Runs review their change for Security findings?";
const SECURITY_FIX: &str = "Security runs may fix reproduced findings?";
const NOTIFY: &str = "Run notifications, an email";
const TO: &str = "Send Run notifications to";
const FROM: &str = "Send them from";
const TEST_EMAIL: &str = "test email now?";
const KEY_PROMPT: &str = "Resend API key (input hidden, Enter to skip):";
const KEPT: &str = "Resend API key (input hidden, Enter keeps the saved one):";
const WROTE_CREDENTIALS: &str = "wrote the Credentials";
const KEY: &str = "re_secret_123";

#[test]
fn setup_can_enable_security_review_and_keeps_it_as_the_next_default() {
    let scenario = Scenario::new();
    for (answer, choices) in [("y", "[y/N]"), ("", "[Y/n]")] {
        let result = setup_on_terminal(
            &scenario,
            &[],
            &[
                (HARNESS, ""),
                (MODEL, ""),
                (EFFORT, ""),
                (MERGE, ""),
                (PULL, ""),
                (SECURITY_FIX, ""),
                (SECURITY_REVIEW, answer),
                (NOTIFY, ""),
            ],
        );
        assert!(
            result
                .stderr
                .contains(&format!("{SECURITY_REVIEW} {choices}"))
        );
        let config: toml::Table = result.user_config.unwrap().parse().unwrap();
        assert_eq!(config["security"]["review"].as_bool(), Some(true));
    }
}

#[test]
fn setup_asks_once_about_security_fixing_with_off_as_the_default_and_writes_yes() {
    let scenario = Scenario::new();
    let result = setup_on_terminal(
        &scenario,
        &[],
        &[
            (HARNESS, ""),
            (MODEL, ""),
            (EFFORT, ""),
            (MERGE, ""),
            (PULL, ""),
            (SECURITY_FIX, "y"),
            (SECURITY_REVIEW, ""),
            (NOTIFY, ""),
        ],
    );
    assert_eq!(result.stderr.matches(SECURITY_FIX).count(), 1);
    assert!(result.stderr.contains(&format!("{SECURITY_FIX} [y/N]")));
    let text = result.user_config.unwrap();
    let config: toml::Table = text.parse().unwrap();
    assert_eq!(config["security"]["fix"].as_bool(), Some(true));
    assert_eq!(config["security"]["harness"].as_str(), Some(""));
    assert!(text.contains("# the Harness a Security run uses unless its command names one"));
}

#[test]
fn rerunning_setup_keeps_security_answers_as_defaults_and_can_turn_fixing_off() {
    let scenario = Scenario::new();
    let saved = "# my Security settings\n[security]\nfix = true # keep this choice\nharness = 'codex' # use Codex\n";
    scenario.user_config_is(saved);

    for (answer, enabled, choices) in [
        ("", true, "[Y/n]"),
        ("n", false, "[Y/n]"),
        ("", false, "[y/N]"),
    ] {
        let result = setup_on_terminal(
            &scenario,
            &[],
            &[
                (HARNESS, ""),
                (MODEL, ""),
                (EFFORT, ""),
                (MERGE, ""),
                (PULL, ""),
                (SECURITY_FIX, answer),
                (SECURITY_REVIEW, ""),
                (NOTIFY, ""),
            ],
        );
        assert!(result.stderr.contains(&format!("{SECURITY_FIX} {choices}")));
        assert_eq!(result.stderr.matches(SECURITY_FIX).count(), 1);
        let text = result.user_config.unwrap();
        let config: toml::Table = text.parse().unwrap();
        assert_eq!(config["security"]["fix"].as_bool(), Some(enabled));
        assert_eq!(config["security"]["harness"].as_str(), Some("codex"));
        assert!(text.starts_with("# my Security settings\n[security]\n"));
        assert!(text.contains("# keep this choice\n"));
        assert!(text.contains("harness = 'codex' # use Codex\n"));
    }
}

#[test]
fn cancelling_a_claude_setup_check_cleans_up_without_writing_or_retrying() {
    use std::time::{Duration, Instant};
    use support::check::{DuringCheck, OwnedCheck};

    let scenario = Scenario::new();
    let saved = "[harness.claude]\nmodel = \"opus\"\n";
    scenario.user_config_is(saved);
    let credentials = "[resend]\nkey = \"re_saved\"\n";
    scenario.credentials_are(credentials);
    let check = OwnedCheck::new(&scenario, DuringCheck::Live);
    let started = Instant::now();
    let result = scenario.run_terminal(
        &["setup"],
        &check.env(),
        &[
            TerminalStep::line(HARNESS, "claude"),
            TerminalStep::line(MODEL, ""),
            TerminalStep::line(EFFORT, ""),
            TerminalStep::interrupt_check(libc::SIGINT),
        ],
    );

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(result.stderr.contains("interrupted"), "{}", result.stderr);
    assert!(started.elapsed() < Duration::from_secs(3));
    check.assert_stopped();
    assert_eq!(result.stderr.matches(MODEL).count(), 1, "{}", result.stderr);
    assert_eq!(result.user_config.as_deref(), Some(saved));
    assert_eq!(scenario.credentials().as_deref(), Some(credentials));
    assert!(result.terminal_restored);
}

#[test]
fn cancelling_a_codex_setup_catalog_does_not_keep_settings_as_a_fallback() {
    use support::check::{DuringCheck, OwnedCheck};
    let scenario = Scenario::new();
    let check = OwnedCheck::new(&scenario, DuringCheck::Live);
    let result = scenario.run_terminal(
        &["setup"],
        &check.env(),
        &[
            TerminalStep::line(HARNESS, "codex"),
            TerminalStep::interrupt_check(libc::SIGTERM),
        ],
    );

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(result.stderr.contains("interrupted"), "{}", result.stderr);
    assert!(
        !result.stderr.contains("harness settings stay"),
        "{}",
        result.stderr
    );
    check.assert_stopped();
    assert_eq!(result.user_config, None);
    assert!(result.terminal_restored);
}

#[test]
fn cancelling_the_other_setup_checks_neither_retries_nor_keeps_settings() {
    use support::check::{DuringCheck, OwnedCheck};
    for (harness, model, signal) in [
        ("agy", None, libc::SIGHUP),
        ("grok", None, libc::SIGINT),
        ("muse", Some("muse-spark-1.3"), libc::SIGTERM),
        ("opencode", Some("provider/model"), libc::SIGHUP),
    ] {
        let scenario = Scenario::new();
        let saved = "# keep my answers\n[harness]\ndefault = \"claude\"\n";
        scenario.user_config_is(saved);
        let credentials = "[resend]\nkey = \"re_saved\"\n";
        scenario.credentials_are(credentials);
        let check = OwnedCheck::new(&scenario, DuringCheck::Detached);
        let model_prompt = format!("Model for {harness}");
        let effort_prompt = format!("Effort for {harness}");
        let mut keys = vec![TerminalStep::line(HARNESS, harness)];
        if let Some(model) = model {
            keys.extend([
                TerminalStep::line(&model_prompt, model),
                TerminalStep::line(&effort_prompt, ""),
            ]);
        }
        keys.push(TerminalStep::interrupt_check(signal));
        let result = scenario.run_terminal(&["setup"], &check.env(), &keys);

        assert_eq!(result.code, Some(1), "{harness}: {}", result.stderr);
        assert!(result.stderr.contains("interrupted"), "{}", result.stderr);
        assert_eq!(
            result.stderr.matches("interrupted").count(),
            1,
            "{}",
            result.stderr
        );
        assert!(
            !result.stderr.contains("harness settings stay"),
            "{}",
            result.stderr
        );
        assert_eq!(
            result.stderr.matches(&model_prompt).count(),
            usize::from(model.is_some()),
            "{}",
            result.stderr
        );
        check.assert_stopped();
        assert_eq!(result.user_config.as_deref(), Some(saved));
        assert_eq!(scenario.credentials().as_deref(), Some(credentials));
        assert!(result.terminal_restored);
        assert!(scenario.issue_labels(7).is_empty());
        assert_eq!(scenario.entries("work"), ["widgets"]);
        assert!(
            !scenario
                .path("home/.thirdshift/logs/acme/widgets/commands")
                .exists()
        );
    }
}

#[test]
fn cancelling_later_questions_after_successful_or_repeated_checks_restores_echo_and_preserves_files()
 {
    for repeated in [false, true] {
        for hidden in [false, true] {
            let scenario = Scenario::new();
            let saved = "# keep this\n[harness.claude]\nmodel = \"opus\"\n";
            scenario.user_config_is(saved);
            let credentials = "[resend]\nkey = \"re_saved\"\n";
            scenario.credentials_are(credentials);
            if repeated {
                scenario.agent_does_in_session(1, "echo 'model refused' >&2; exit 1");
            }
            let mut keys = vec![
                TerminalStep::line(HARNESS, "claude"),
                TerminalStep::line(MODEL, ""),
                TerminalStep::line(EFFORT, ""),
            ];
            if repeated {
                keys.extend([
                    TerminalStep::line(MODEL, "opus"),
                    TerminalStep::line(EFFORT, ""),
                ]);
            }
            if hidden {
                keys.extend([
                    TerminalStep::line(MERGE, ""),
                    TerminalStep::line(PULL, ""),
                    TerminalStep::line(SECURITY_FIX, ""),
                    TerminalStep::line(SECURITY_REVIEW, ""),
                    TerminalStep::line(NOTIFY, "y"),
                    TerminalStep::line(TO, "me@example.com"),
                    TerminalStep::line(FROM, ""),
                    TerminalStep::signal(
                        KEPT,
                        if repeated {
                            libc::SIGHUP
                        } else {
                            libc::SIGTERM
                        },
                    ),
                ]);
            } else {
                keys.push(TerminalStep::signal(
                    MERGE,
                    if repeated {
                        libc::SIGTERM
                    } else {
                        libc::SIGINT
                    },
                ));
            }
            let result = scenario.run_terminal(&["setup"], &[], &keys);

            assert_eq!(result.code, Some(1), "{}", result.stderr);
            assert!(result.stderr.contains("interrupted"), "{}", result.stderr);
            assert!(result.terminal_restored);
            assert_eq!(result.stdout, "");
            assert_eq!(result.user_config.as_deref(), Some(saved));
            assert_eq!(scenario.credentials().as_deref(), Some(credentials));
            assert_eq!(scenario.claude_calls().len(), if repeated { 2 } else { 1 });
        }
    }
}

#[test]
fn pasted_answers_are_left_available_to_later_terminal_questions() {
    let scenario = Scenario::new();
    let result = setup_on_terminal(
        &scenario,
        &[],
        &[(
            HARNESS,
            "  claude  \n\n\n\n\n\n\ny\n  café@example.com  \n\n",
        )],
    );
    let config: toml::Table = result.user_config.unwrap().parse().unwrap();
    assert_eq!(config["harness"]["default"].as_str(), Some("claude"));
    assert_eq!(config["merge"]["always"].as_bool(), Some(false));
    assert_eq!(config["email"]["always"].as_bool(), Some(true));
    assert_eq!(config["email"]["to"].as_str(), Some("café@example.com"));
}

#[test]
fn hidden_input_restores_terminal_attributes_on_eof_and_invalid_utf8() {
    for bytes in [b"\x04".as_slice(), &[0xff, b'\n']] {
        let scenario = Scenario::new();
        let mut steps: Vec<_> = notifications_on(&[])
            .into_iter()
            .map(TerminalStep::from)
            .collect();
        steps.push(TerminalStep::bytes(KEY_PROMPT, bytes));
        let result = scenario.run_terminal(&["setup"], &[], &steps);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(result.terminal_restored);
        assert_eq!(result.user_config, None);
        assert_eq!(scenario.credentials(), None);
        if bytes[0] == 0xff {
            assert!(result.stderr.contains("terminal input is not UTF-8"));
        }
    }
}

#[test]
fn a_partial_final_terminal_answer_is_trimmed_and_accepted_before_eof() {
    let scenario = Scenario::new();
    let result = scenario.run_terminal(
        &["setup"],
        &[],
        &[
            TerminalStep::line(HARNESS, ""),
            TerminalStep::line(MODEL, ""),
            TerminalStep::line(EFFORT, ""),
            TerminalStep::line(MERGE, ""),
            TerminalStep::line(PULL, ""),
            TerminalStep::line(SECURITY_FIX, ""),
            TerminalStep::line(SECURITY_REVIEW, ""),
            // Queue EOF for both the partial answer and the next question.
            // macOS can finish both reads before another input action runs.
            TerminalStep::bytes(NOTIFY, b"  y  \x04\x04\x04"),
        ],
    );
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains(TO),
        "the partial answer was lost: {}",
        result.stderr
    );
    assert!(result.terminal_restored);
    assert_eq!(result.user_config, None);
    assert_eq!(scenario.credentials(), None);
}

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
    assert!(result.terminal_restored, "{}", result.stderr);
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
        (SECURITY_FIX, ""),
        (SECURITY_REVIEW, ""),
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
            (SECURITY_FIX, ""),
            (SECURITY_REVIEW, ""),
            (NOTIFY, ""),
        ],
    );

    assert_eq!(result.user_config, user_config(&unattended));
    for prompt in [MERGE, PULL, SECURITY_FIX, NOTIFY] {
        assert!(
            result.stderr.contains(prompt),
            "terminal: {}",
            result.stderr
        );
    }
    assert_eq!(result.stderr.matches(SECURITY_FIX).count(), 1);
    assert!(result.stderr.contains(&format!("{SECURITY_FIX} [y/N]")));
    let config: toml::Table = result.user_config.unwrap().parse().unwrap();
    assert_eq!(config["security"]["fix"].as_bool(), Some(false));
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

    assert_eq!(result.code, Some(1), "terminal: {}", result.stderr);
    assert!(result.stderr.contains("interrupted"));
    assert!(result.terminal_restored);
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

    assert_eq!(result.code, Some(1), "terminal: {}", result.stderr);
    assert!(result.terminal_restored);
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
            (PULL, ""),
            (SECURITY_FIX, CTRL_C),
        ],
    );

    assert_eq!(result.code, Some(1), "terminal: {}", result.stderr);
    assert!(result.terminal_restored);
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
            (SECURITY_FIX, ""),
            (SECURITY_REVIEW, ""),
            (NOTIFY, "y"),
            (TO, CTRL_C),
        ],
    );

    assert_eq!(result.code, Some(1), "terminal: {}", result.stderr);
    assert!(result.terminal_restored);
    assert_eq!(result.user_config.as_deref(), Some(mine));
}
