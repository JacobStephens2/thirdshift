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

#[cfg(target_os = "linux")]
use support::write_executable;
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

#[cfg(target_os = "linux")]
#[test]
fn security_sandbox_start_failure_warns_and_still_writes_the_user_config() {
    let scenario = Scenario::new();
    scenario.user_config_is("[security]\nreview = true\n");
    let bwrap = scenario.path("bin/bwrap");
    write_executable(&bwrap, "#!/bin/sh\nexit 1\n");

    let result = scenario.run(&["setup"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("Security sandbox probe: FAIL (sandbox start"),
        "{}",
        result.stderr
    );
    assert!(
        result.stderr.contains("sudo apt install bubblewrap"),
        "{}",
        result.stderr
    );
    assert!(
        result
            .stderr
            .contains("profile bwrap /usr/bin/bwrap flags=(unconfined)"),
        "{}",
        result.stderr
    );
    assert!(
        result
            .stderr
            .contains("sudo apparmor_parser -r /etc/apparmor.d/bwrap"),
        "{}",
        result.stderr
    );
    assert!(user_config(&scenario).unwrap().contains("[harness]"));
}

#[cfg(target_os = "linux")]
#[test]
fn security_sandbox_missing_bwrap_warns_without_losing_settings() {
    let scenario = Scenario::new();
    scenario.user_config_is("[security]\nfix = true\n");
    let result = scenario.run_with_env(&["setup"], &[("PATH", "/thirdshift-no-tools")]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("sandbox start: could not launch bwrap from PATH"),
        "{}",
        result.stderr
    );
    assert!(result.stderr.contains("sudo apt install bubblewrap"));
    assert!(user_config(&scenario).unwrap().contains("fix = true"));
}

#[cfg(target_os = "linux")]
#[test]
fn security_sandbox_runs_only_for_the_saved_security_settings() {
    for (review, fix) in [(false, false), (true, false), (false, true)] {
        let scenario = Scenario::new();
        scenario.user_config_is(&format!("[security]\nreview = {review}\nfix = {fix}\n"));
        let marker = scenario.path("probe-ran");
        let bwrap = scenario.path("bin/bwrap");
        write_executable(
            &bwrap,
            &format!(
                "#!/bin/sh\nprintf probed > '{}'\nexit 1\n",
                marker.display()
            ),
        );
        let result = scenario.run(&["setup"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_eq!(marker.exists(), review || fix);
        assert_eq!(
            result.stderr.contains("Security sandbox probe:"),
            review || fix
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn security_sandbox_hung_bwrap_is_bounded_and_reaped() {
    let scenario = Scenario::new();
    scenario.user_config_is("[security]\nreview = true\n");
    let bwrap = scenario.path("bin/bwrap");
    write_executable(&bwrap, "#!/bin/sh\nexec /bin/sleep 30\n");
    let start = std::time::Instant::now();
    let result = scenario.run(&["setup"]);
    assert!(start.elapsed() < std::time::Duration::from_secs(8));
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result.stderr.contains("sandbox completion: timed out"),
        "{}",
        result.stderr
    );
    assert!(user_config(&scenario).unwrap().contains("[harness]"));
}

#[cfg(target_os = "linux")]
#[test]
fn security_sandbox_checks_real_isolation_and_detects_each_removed_control() {
    let available = std::process::Command::new("/usr/bin/bwrap")
        .args(["--unshare-all", "--ro-bind", "/", "/", "--", "/bin/true"])
        .output()
        .is_ok_and(|result| result.status.success());
    if !available {
        eprintln!("real isolation cases require working /usr/bin/bwrap; stand-in cases still run");
        return;
    }
    for (wrapper, expected) in [
        ("exec /usr/bin/bwrap \"$@\"", "Security sandbox probe: PASS"),
        (
            "first=$1; shift; exec /usr/bin/bwrap \"$first\" --share-net \"$@\"",
            "FAIL (network isolation:",
        ),
        // Substitute only mount option tokens, preserving paths as separate arguments.
        (
            "args=(); for arg; do if [[ $arg == --ro-bind ]]; then args+=(--bind); else args+=(\"$arg\"); fi; done; exec /usr/bin/bwrap \"${args[@]}\"",
            "FAIL (host home read-only:",
        ),
        (
            "args=(); for arg; do if [[ $arg == --bind ]]; then args+=(--ro-bind); else args+=(\"$arg\"); fi; done; exec /usr/bin/bwrap \"${args[@]}\"",
            "FAIL (scratch write:",
        ),
    ] {
        let scenario = Scenario::new();
        scenario.user_config_is("[security]\nreview = true\n");
        let bwrap = scenario.path("bin/bwrap");
        write_executable(&bwrap, &format!("#!/bin/bash\n{wrapper}\n"));
        let before = fs::read_dir(scenario.path("tmp")).unwrap().count();
        let result = scenario.run(&["setup"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert!(
            result.stderr.contains(expected),
            "expected {expected}: {}",
            result.stderr
        );
        assert_eq!(fs::read_dir(scenario.path("tmp")).unwrap().count(), before);
        let config: toml::Table = user_config(&scenario).unwrap().parse().unwrap();
        assert_eq!(config["security"]["review"].as_bool(), Some(true));
    }
}

#[test]
fn setup_probe_scope_tracks_security_skill_requirements() {
    // This contract snapshot intentionally fails if the skill's controls change,
    // requiring a decision about Setup's probe coverage. #608 checks network,
    // read-only host home and scratch writes; it does not certify the remaining
    // environment/resource/credential controls of the target-code runner (#726).
    let skill = include_str!("../skills/thirdshift-security-audit/SKILL.md");
    let requirements = skill
        .split_once("provides all of these controls:\n\n")
        .unwrap()
        .1
        .split_once("\n\n")
        .unwrap()
        .0;
    assert_eq!(requirements, "\
- no external network; use only an isolated loopback namespace when the check needs local client/server traffic;
- an empty environment populated from an explicit allowlist with safe values, with scratch-local `HOME`, temporary directories, and caches;
- a read-only target and toolchain, with the target-controlled process able to write only inside its assigned `scratch/` directory; and
- explicit low CPU, memory, process, file-size, disk, and wall-clock limits.");
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
            "[otlp]",
            "[inbox]",
            "[console]",
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
    assert_eq!(config["logs"]["waiting_line_minutes"].as_integer(), Some(5));
    assert_eq!(config["inbox"]["silent_minutes"].as_integer(), Some(10));
    assert_eq!(config["console"]["port"].as_integer(), Some(8711));
    assert_eq!(config["console"]["theme"].as_str(), Some("focus-auto"));
    assert!(
        config["console"]["other_consoles"]
            .as_array()
            .unwrap()
            .is_empty()
    );
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
            "[otlp]",
            "[inbox]",
            "[console]",
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
fn setup_keeps_existing_security_outside_completed_harness_settings() {
    for later_harness in ["", "\n[harness.agy]\nmodel = ''\neffort = ''\n"] {
        let scenario = Scenario::new();
        scenario.user_config_is(&format!(
            "[harness]\ndefault = 'codex'\n\
             [harness.codex]\nmodel = 'gpt-6.1-sol'\neffort = 'high'\n\n\
             # my Security settings\n[security] # separate area\n\
             fix = true # allow fixes\nreview = true # review changes\n\
             harness = 'codex' # Security Harness\n{later_harness}"
        ));

        let result = scenario.run(&["setup"]);

        assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
        let text = user_config(&scenario).unwrap();
        let sections: Vec<&str> = text.lines().filter(|line| line.starts_with('[')).collect();
        assert_eq!(
            sections.last(),
            Some(&"[security] # separate area"),
            "{text}"
        );
        assert!(
            text.contains(
                "# my Security settings\n[security] # separate area\n\
                 fix = true # allow fixes\nreview = true # review changes\n\
                 harness = 'codex' # Security Harness\n"
            ),
            "{text}"
        );
        let config: toml::Table = text.parse().unwrap();
        assert_eq!(config["security"]["fix"].as_bool(), Some(true));
        assert_eq!(config["security"]["review"].as_bool(), Some(true));
        assert_eq!(config["security"]["harness"].as_str(), Some("codex"));
        assert_eq!(
            config["harness"]["codex"]["model"].as_str(),
            Some("gpt-6.1-sol")
        );
        assert_eq!(config["harness"]["codex"]["effort"].as_str(), Some("high"));

        let again = scenario.run(&["setup"]);
        assert_eq!(again.code, Some(0), "stderr: {}", again.stderr);
        assert_eq!(user_config(&scenario).unwrap(), text);
    }
}

#[test]
fn completing_an_existing_file_replaces_it_atomically_keeps_permissions_and_then_avoids_replacement()
 {
    let scenario = Scenario::new();
    let original = "# my machine\n[logs]\ndir = '/var/log/ts' # custom\n\
                    waiting_line_minutes = 2 # waiting\n\
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
    assert_eq!(config["logs"]["waiting_line_minutes"].as_integer(), Some(2));
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
            .log_files("home/.thirdshift/logs/acme/widgets/commands/issue", "jsonl")
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

const HARNESS: &str = "Harness for every Ticket run's sessions";
const MODEL: &str = "Model for claude";
const EFFORT: &str = "Effort for claude";
const MERGE: &str = "Self-merges?";
const BASE_FIX: &str = "Every Ticket run may start a Base fix when the Base branch's CI is red?";
const PULL: &str = "fast-forward";
const SECURITY_REVIEW: &str = "Ticket runs review their change for Security findings?";
const SECURITY_FIX: &str = "Security runs may fix reproduced findings?";
const SECURITY_CLOSE: &str = "close a finding their fix merged?";
const NOTIFY: &str = "Run notifications, an email";
const CONSOLE: &str = "Run the Console?";
const TO: &str = "Send Run notifications to";
const FROM: &str = "Send them from";
const TEST_EMAIL: &str = "test email now?";
const KEY_PROMPT: &str = "Resend API key (input hidden, Enter to skip):";
const KEPT: &str = "Resend API key (input hidden, Enter keeps the saved one):";
const WROTE_CREDENTIALS: &str = "wrote the Credentials";
const KEY: &str = "re_secret_123";
const CLAUDE_TOKEN: &str = "Claude setup token";

#[test]
fn a_claude_setup_token_is_saved_with_hidden_input_even_without_notifications() {
    let scenario = Scenario::new();
    let token = "sk-ant-oat01-only-for-trials";
    let steps: Vec<_> = [
        (HARNESS, ""),
        (MODEL, ""),
        (EFFORT, ""),
        (MERGE, ""),
        (BASE_FIX, ""),
        (PULL, ""),
        (SECURITY_FIX, ""),
        (SECURITY_REVIEW, ""),
        (NOTIFY, "n"),
        (CLAUDE_TOKEN, token),
        (CONSOLE, "n"),
    ]
    .into_iter()
    .map(TerminalStep::from)
    .collect();
    let result =
        scenario.run_terminal_bounded(&["setup"], &[], &steps, std::time::Duration::from_secs(10));

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(result.terminal_restored);
    assert_eq!(result.stdout, "");
    assert!(!result.stderr.contains(token), "token leaked");
    assert!(result.stderr.contains("input hidden"));
    assert!(!result.user_config.unwrap().contains(token));
    let saved: toml::Table = scenario.credentials().unwrap().parse().unwrap();
    assert_eq!(saved["claude"]["setup_token"].as_str(), Some(token));
    assert_eq!(
        fs::metadata(scenario.path("home/.thirdshift/credentials.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn a_saved_claude_setup_token_is_kept_or_replaced_alongside_a_resend_key() {
    for (entered, expected) in [
        ("", "saved-trial-token"),
        ("new-trial-token", "new-trial-token"),
    ] {
        let scenario = Scenario::new();
        let path = scenario.credentials_are(
            "# My credentials\n[resend]\nkey = 're_old' # email\n\n[claude]\nsetup_token = 'saved-trial-token' # Trials\n",
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        let result = setup_on_terminal(
            &scenario,
            &[],
            &notifications_on(&[
                (KEPT, KEY),
                (TEST_EMAIL, "n"),
                (
                    "Claude setup token from `claude setup-token` (Trials only; input hidden, Enter keeps the saved one):",
                    entered,
                ),
                (CONSOLE, "n"),
            ]),
        );

        let text = scenario.credentials().unwrap();
        let saved: toml::Table = text.parse().unwrap();
        assert_eq!(saved["resend"]["key"].as_str(), Some(KEY));
        assert_eq!(saved["claude"]["setup_token"].as_str(), Some(expected));
        for comment in ["# My credentials", "# email", "# Trials"] {
            assert!(text.contains(comment));
        }
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        for secret in ["saved-trial-token", "new-trial-token", KEY] {
            assert!(!result.stderr.contains(secret), "secret leaked");
            assert!(!result.user_config.as_ref().unwrap().contains(secret));
        }
    }
}

#[test]
fn interruption_at_the_claude_setup_token_prompt_writes_neither_file() {
    let scenario = Scenario::new();
    let saved = "[resend]\nkey = 're_saved'\n[claude]\nsetup_token = 'saved-trial-token'\n";
    scenario.credentials_are(saved);

    let result = scenario.run_on_terminal(
        &["setup"],
        &[],
        &notifications_on(&[(KEPT, KEY), (TEST_EMAIL, "n"), (CLAUDE_TOKEN, CTRL_C)]),
    );

    assert_eq!(result.code, Some(1));
    assert!(result.stderr.contains("interrupted"));
    assert!(result.terminal_restored);
    assert_eq!(result.user_config, None);
    assert_eq!(scenario.credentials().as_deref(), Some(saved));
}

#[test]
fn ordinary_runs_never_use_or_record_the_saved_claude_setup_token() {
    let scenario = Scenario::new();
    let token = "sk-ant-oat01-only-for-trials";
    scenario.credentials_are(&format!("[claude]\nsetup_token = '{token}'\n"));
    scenario.agent_does(&format!(
        "test -z \"${{CLAUDE_CODE_OAUTH_TOKEN+x}}\" || exit 91\n{AGENT_OPENS_PR}"
    ));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 1);
    assert!(!result.stdout.contains(token));
    assert!(!result.stderr.contains(token));
    let mut folders = vec![scenario.path("home/.thirdshift/logs")];
    let mut records = 0;
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else {
                records += usize::from(path.to_string_lossy().ends_with(".command.json"));
                let bytes = fs::read(&path).unwrap();
                assert!(
                    !String::from_utf8_lossy(&bytes).contains(token),
                    "token leaked to {}",
                    path.display()
                );
            }
        }
    }
    assert_eq!(records, 1, "expected one Command record to be checked");
}

#[test]
fn setup_console_questions_use_email_to_keep_comments_and_offer_install() {
    let scenario = Scenario::new();
    scenario.user_config_is("[email]\nto = 'owner@example.com'\n[logs]\nwaiting_line_minutes = 2\n[inbox]\nsilent_minutes = 2\n[console] # my Console\naddress = '' # site\nallowed_emails = [] # people\n");
    let result = setup_on_terminal(
        &scenario,
        &[],
        &[
            (HARNESS, ""),
            (MODEL, ""),
            (EFFORT, ""),
            (MERGE, ""),
            (BASE_FIX, ""),
            (PULL, ""),
            (SECURITY_FIX, ""),
            (SECURITY_REVIEW, ""),
            (NOTIFY, "n"),
            (CLAUDE_TOKEN, ""),
            (CONSOLE, "y"),
            (
                "Console's whole address (e.g. thirdshift.example.com):",
                "https://thirdshift.example.com",
            ),
            ("Allowed emails (comma-separated) [owner@example.com]", ""),
        ],
    );
    let text = result.user_config.unwrap();
    let config: toml::Table = text.parse().unwrap();
    assert_eq!(
        config["console"]["address"].as_str(),
        Some("https://thirdshift.example.com")
    );
    assert_eq!(
        config["console"]["allowed_emails"][0].as_str(),
        Some("owner@example.com")
    );
    for comment in ["[console] # my Console", "# site", "# people"] {
        assert!(text.contains(comment), "{text}");
    }
    assert!(
        result
            .stderr
            .contains("thirdshift console install --output install-console.sh"),
        "{}",
        result.stderr
    );
    assert!(
        result
            .stderr
            .contains("logs.waiting_line_minutes must be less than inbox.silent_minutes"),
        "{}",
        result.stderr
    );
}

#[test]
fn cancelling_the_console_questions_preserves_the_user_config() {
    let scenario = Scenario::new();
    let saved = "[email]\nto = 'owner@example.com'\n";
    scenario.user_config_is(saved);
    let result = scenario.run_on_terminal(
        &["setup"],
        &[],
        &[
            (HARNESS, ""),
            (MODEL, ""),
            (EFFORT, ""),
            (MERGE, ""),
            (BASE_FIX, ""),
            (PULL, ""),
            (SECURITY_FIX, ""),
            (SECURITY_REVIEW, ""),
            (NOTIFY, "n"),
            (CLAUDE_TOKEN, ""),
            (CONSOLE, "y"),
            ("Console's whole address", "thirdshift.example.com"),
            ("Allowed emails", CTRL_C),
        ],
    );
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(result.stderr.contains("interrupted"), "{}", result.stderr);
    assert_eq!(result.user_config.as_deref(), Some(saved));
    assert!(result.terminal_restored);
}

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
                (BASE_FIX, ""),
                (PULL, ""),
                (SECURITY_FIX, ""),
                (SECURITY_REVIEW, answer),
                (NOTIFY, ""),
                (CLAUDE_TOKEN, ""),
                (CONSOLE, "n"),
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
            (BASE_FIX, ""),
            (PULL, ""),
            (SECURITY_FIX, "y"),
            (SECURITY_CLOSE, "y"),
            (SECURITY_REVIEW, ""),
            (NOTIFY, ""),
            (CLAUDE_TOKEN, ""),
            (CONSOLE, "n"),
        ],
    );
    assert_eq!(result.stderr.matches(SECURITY_FIX).count(), 1);
    assert!(
        result.stderr.contains("Security sandbox probe:"),
        "{}",
        result.stderr
    );
    assert!(result.stderr.contains(&format!("{SECURITY_FIX} [y/N]")));
    let text = result.user_config.unwrap();
    let config: toml::Table = text.parse().unwrap();
    assert_eq!(config["security"]["fix"].as_bool(), Some(true));
    assert_eq!(config["security"]["close"].as_bool(), Some(true));
    assert_eq!(result.stderr.matches(SECURITY_CLOSE).count(), 1);
    assert_eq!(config["security"]["harness"].as_str(), Some(""));
    assert!(
        text.contains(
            "# the Harness for Security runs, unless the command names one, and for every Security review; default blank; blank or missing leaves Security runs on harness.default, or Claude Code if that is missing, and Security reviews on their Command's Harness"
        ),
        "{text}"
    );
}

#[test]
fn rerunning_setup_keeps_security_answers_as_defaults_and_can_turn_fixing_off() {
    let scenario = Scenario::new();
    let saved = "# my Security settings\n[security]\nfix = true # keep this choice\nharness = 'codex' # use Codex\n";
    scenario.user_config_is(saved);

    for (answer, was_on, enabled, close) in [
        ("", true, true, Some("")),
        ("n", true, false, None),
        ("", false, false, None),
    ] {
        let mut keystrokes = vec![
            (HARNESS, ""),
            (MODEL, ""),
            (EFFORT, ""),
            (MERGE, ""),
            (BASE_FIX, ""),
            (PULL, ""),
            (SECURITY_FIX, answer),
        ];
        if let Some(close) = close {
            keystrokes.push((SECURITY_CLOSE, close));
        }
        keystrokes.extend_from_slice(&[
            (SECURITY_REVIEW, ""),
            (NOTIFY, ""),
            (CLAUDE_TOKEN, ""),
            (CONSOLE, "n"),
        ]);
        let result = setup_on_terminal(&scenario, &[], &keystrokes);
        assert!(result.stderr.contains(&format!(
            "{SECURITY_FIX} {}",
            if was_on { "[Y/n]" } else { "[y/N]" }
        )));
        assert_eq!(
            result.stderr.contains("Security sandbox probe:"),
            enabled,
            "{}",
            result.stderr
        );
        assert_eq!(result.stderr.matches(SECURITY_FIX).count(), 1);
        let text = result.user_config.unwrap();
        let config: toml::Table = text.parse().unwrap();
        assert_eq!(config["security"]["fix"].as_bool(), Some(enabled));
        assert_eq!(config["security"]["close"].as_bool(), Some(false));
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
                    TerminalStep::line(BASE_FIX, ""),
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
            "  claude  \n\n\n\n\n\n\n\ny\n  café@example.com  \n\n\n\n",
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
            TerminalStep::line(BASE_FIX, ""),
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
        (BASE_FIX, ""),
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
            (BASE_FIX, ""),
            (PULL, ""),
            (SECURITY_FIX, ""),
            (SECURITY_REVIEW, ""),
            (NOTIFY, ""),
            (CLAUDE_TOKEN, ""),
            (CONSOLE, ""),
        ],
    );

    assert_eq!(result.user_config, user_config(&unattended));
    for prompt in [MERGE, BASE_FIX, PULL, SECURITY_FIX, NOTIFY] {
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
        &notifications_on(&[
            (KEY_PROMPT, &format!("  {KEY}  ")),
            (TEST_EMAIL, ""),
            (CLAUDE_TOKEN, ""),
            (CONSOLE, "n"),
        ]),
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
            (BASE_FIX, ""),
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
