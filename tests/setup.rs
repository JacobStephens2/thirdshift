//! `thirdshift setup`: Setup writes a complete User config. With no terminal
//! and no User config, it writes every setting at its default without asking,
//! and `email.to` as the GitHub email it suggests, if it finds one; over an
//! existing one, it keeps its values and comments and adds the keys it lacks.
//! From a terminal, it first asks the Setup questions on stderr, each with the
//! current value as its default answer. Base fixes are asked about only
//! with every Run a Merge run. The Harness question lists both Harnesses,
//! refusing one that isn't installed, and the Model and Effort are asked for
//! the Harness chosen: a Claude Model checked with a test call, and a Codex
//! Model and Effort chosen from Codex's catalog and written as Codex names
//! them.

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;

use support::resend::ResendStandIn;
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

/// The `email.to` the User config sets, if it sets one.
fn email_to(scenario: &Scenario) -> Option<String> {
    let config: toml::Table = user_config(scenario)?.parse().unwrap();
    Some(config["email"].get("to")?.as_str()?.to_string())
}

/// The lines of `text` that set a key, commented out or not, as
/// `(section, line)`.
fn key_lines(text: &str) -> Vec<(String, String)> {
    let mut section = String::new();
    let mut keys = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name.to_string();
        } else if line.trim_start_matches("# ").contains(" = ") && !section.is_empty() {
            keys.push((section.clone(), line.to_string()));
        }
    }
    keys
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
            "[harness.codex]"
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
    for harness in ["claude", "codex"] {
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
fn every_key_is_written_with_a_comment_giving_what_it_does_and_its_default() {
    let scenario = Scenario::new();
    scenario.git_email_is(None);

    scenario.run(&["setup"]);

    let text = user_config(&scenario).unwrap();
    let keys = key_lines(&text);
    let names: Vec<String> = keys
        .iter()
        .map(|(section, line)| {
            let key = line.trim_start_matches("# ").split(' ').next().unwrap();
            format!("{section}.{key}")
        })
        .collect();
    assert_eq!(
        names,
        [
            "merge.always",
            "base.fix",
            "launch.pull",
            "email.always",
            "email.to",
            "email.from",
            "logs.dir",
            "activity.quiet_skips",
            "spec.parallel",
            "pickup.limit",
            "harness.default",
            "harness.claude.model",
            "harness.claude.effort",
            "harness.codex.model",
            "harness.codex.effort"
        ],
        "{text}"
    );
    for (section, line) in &keys {
        let Some((_, comment)) = line.trim_start_matches("# ").split_once(" # ") else {
            panic!("no trailing comment on [{section}] {line:?}");
        };
        assert!(
            comment.contains("default"),
            "the comment on [{section}] {line:?} doesn't give the default"
        );
    }
    let commented_out: Vec<&String> = keys
        .iter()
        .filter(|(_, line)| line.starts_with('#'))
        .map(|(_, line)| line)
        .collect();
    assert_eq!(commented_out.len(), 1, "{text}");
    assert!(commented_out[0].starts_with("# to = "), "{text}");
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
fn a_run_with_the_written_user_config_still_needs_an_address_for_email() {
    let scenario = Scenario::new();
    scenario.git_email_is(Some("123+runner@users.noreply.github.com"));
    assert_eq!(scenario.run(&["setup"]).code, Some(0));
    assert!(email_to(&scenario).is_none());
    let setup_calls = scenario.gh_calls().len();

    let result = scenario.run_with_env(
        &["--email", &scenario.issue_url(7)],
        &[("RESEND_API_KEY", "re_test_123")],
    );

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert!(
        result.stderr.contains("email.to"),
        "stderr: {}",
        result.stderr
    );
    assert!(scenario.claude_calls().is_empty(), "the Run started");
    let run_calls = &scenario.gh_calls()[setup_calls..];
    assert!(run_calls.is_empty(), "the Run asked GitHub: {run_calls:?}");
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

/// Every key this version knows, as `section.key`, sorted.
const EVERY_KEY: [&str; 15] = [
    "activity.quiet_skips",
    "base.fix",
    "email.always",
    "email.from",
    "email.to",
    "harness.claude.effort",
    "harness.claude.model",
    "harness.codex.effort",
    "harness.codex.model",
    "harness.default",
    "launch.pull",
    "logs.dir",
    "merge.always",
    "pickup.limit",
    "spec.parallel",
];

/// The `section.key` names of the keys `text` sets, commented out or not, in
/// the order they appear.
fn key_names(text: &str) -> Vec<String> {
    key_lines(text)
        .iter()
        .map(|(section, line)| {
            let key = line.trim_start_matches("# ").split(' ').next().unwrap();
            format!("{section}.{key}")
        })
        .collect()
}

#[test]
fn setup_over_a_partial_user_config_keeps_it_and_adds_each_missing_key_with_its_comment() {
    let scenario = Scenario::new();
    let partial = "\
# My machine: always merge.
[merge]
always = true   # I trust the factory

[email]
to = \"me@example.com\"  # my inbox
";
    scenario.user_config_is(partial);

    let result = scenario.run(&["setup"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert!(
        result.stderr.contains(".thirdshift/config.toml"),
        "stderr: {}",
        result.stderr
    );
    let text = user_config(&scenario).unwrap();
    assert!(
        text.starts_with(
            "# My machine: always merge.\n[merge]\nalways = true   # I trust the factory\n"
        ),
        "{text}"
    );
    assert!(
        text.contains("to = \"me@example.com\"  # my inbox\n"),
        "{text}"
    );
    let config: toml::Table = text.parse().unwrap();
    assert_eq!(config["merge"]["always"].as_bool(), Some(true), "{text}");
    assert_eq!(config["email"]["to"].as_str(), Some("me@example.com"));
    assert_eq!(config["launch"]["pull"].as_bool(), Some(false), "{text}");
    assert_eq!(config["email"]["always"].as_bool(), Some(false), "{text}");
    assert_eq!(
        config["email"]["from"].as_str(),
        Some("onboarding@resend.dev")
    );
    assert_eq!(config["logs"]["dir"].as_str(), Some("~/.thirdshift/logs"));
    let mut names = key_names(&text);
    names.sort();
    assert_eq!(names, EVERY_KEY, "{text}");
    for (section, line) in key_lines(&text) {
        let Some((_, comment)) = line.split_once(" # ") else {
            panic!("no trailing comment on [{section}] {line:?}");
        };
        // The two lines from the partial User config carry the user's own
        // comments; every added key's comment gives its default.
        if !line.contains("me@example.com") && !line.starts_with("always = true") {
            assert!(
                comment.contains("default"),
                "the comment on [{section}] {line:?} doesn't give the default"
            );
        }
    }
}

#[test]
fn setup_over_a_user_config_with_no_email_to_adds_it_commented_out_once() {
    let scenario = Scenario::new();
    scenario.user_config_is("[launch]\npull = true\n");

    for _ in 0..2 {
        let result = scenario.run(&["setup"]);
        assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    }

    let text = user_config(&scenario).unwrap();
    let config: toml::Table = text.parse().unwrap();
    assert_eq!(config["launch"]["pull"].as_bool(), Some(true), "{text}");
    assert!(config["email"].get("to").is_none(), "{text}");
    let mut names = key_names(&text);
    names.sort();
    assert_eq!(names, EVERY_KEY, "{text}");
}

#[test]
fn setup_writes_pickup_limit_at_3_with_its_comment_and_keeps_one_already_there() {
    let scenario = Scenario::new();
    scenario.user_config_is("[launch]\npull = true\n");

    let result = scenario.run(&["setup"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let text = user_config(&scenario).unwrap();
    let written = "\n[pickup]\nlimit = 3   # how many open issues labelled in-progress stop a \
                   Pickup run taking another; default 3\n";
    assert!(text.contains(&format!("{written}\n[harness]\n")), "{text}");

    let mine = "[pickup]\nlimit = 7 # I review quickly\n";
    scenario.user_config_is(mine);

    let result = scenario.run(&["setup"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let text = user_config(&scenario).unwrap();
    assert!(text.starts_with(mine), "{text}");
    assert_eq!(text.matches("limit =").count(), 1, "{text}");
}

#[test]
fn setup_over_a_complete_user_config_leaves_it_as_it_was() {
    let scenario = Scenario::new();
    assert_eq!(scenario.run(&["setup"]).code, Some(0));
    let edited = user_config(&scenario)
        .unwrap()
        .replace("always = false   #", "always = true    #")
        .replace("\"~/.thirdshift/logs\"", "\"/var/log/thirdshift\"")
        + "# hand-written at the end\n";
    scenario.user_config_is(&edited);

    let result = scenario.run(&["setup"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert_eq!(user_config(&scenario).as_deref(), Some(edited.as_str()));
}

#[test]
fn setup_refuses_a_user_config_a_run_would_refuse_and_leaves_it_alone() {
    for (broken, named) in [
        ("[merge]\nalway = true\n", "unknown key merge.alway"),
        ("[merge\nalways = true\n", "can't parse the User config"),
        (
            "# mine\n[launch]\npull = \"yes\"\n",
            "launch.pull must be true or false",
        ),
    ] {
        let scenario = Scenario::new();
        scenario.user_config_is(broken);
        let run = scenario.run(&[&scenario.issue_url(7)]);

        let result = scenario.run(&["setup"]);

        assert_eq!(result.code, Some(1), "{broken:?}: {}", result.stderr);
        assert_eq!(result.stdout, "");
        assert!(result.stderr.contains(named), "stderr: {}", result.stderr);
        assert!(
            result.stderr.contains(".thirdshift/config.toml"),
            "stderr: {}",
            result.stderr
        );
        // The Run's first line says it is starting; the rest is the same.
        let (_, refused) = run.stderr.split_once('\n').unwrap();
        assert_eq!(result.stderr, refused, "{broken:?}");
        assert_eq!(user_config(&scenario).as_deref(), Some(broken));
    }
}

#[test]
fn a_run_reads_the_completed_user_config_with_the_settings_it_had_before() {
    let scenario = Scenario::new();
    scenario.user_config_is("[merge]\nalways = true\n\n[logs]\ndir = \"~/elsewhere/logs\"\n");
    assert_eq!(scenario.run(&["setup"]).code, Some(0));
    scenario.agent_does(AGENT_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    assert_eq!(
        result.stderr.lines().last(),
        Some(format!("thirdshift: PR {PR_URL} is merged").as_str()),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(
        scenario
            .entries("home/elsewhere/logs/acme/widgets/sessions")
            .len(),
        1
    );
    assert!(!scenario.path("home/.thirdshift/logs").exists());
}

#[test]
fn setup_writes_the_public_github_email_as_email_to() {
    let scenario = Scenario::new();
    scenario.github_email_is(Some("octo@example.com"));

    let result = scenario.run(&["setup"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(email_to(&scenario).as_deref(), Some("octo@example.com"));
    let text = user_config(&scenario).unwrap();
    assert!(!text.contains("# to = "), "{text}");
    let to = key_lines(&text)
        .into_iter()
        .find(|(_, line)| line.starts_with("to = "))
        .unwrap()
        .1;
    assert!(to.contains(" # ") && to.contains("default"), "{to}");
}

#[test]
fn with_no_public_github_email_setup_writes_the_git_email() {
    let scenario = Scenario::new();
    scenario.github_email_is(None);
    scenario.git_email_is(Some("me@example.org"));

    assert_eq!(scenario.run(&["setup"]).code, Some(0));

    assert_eq!(email_to(&scenario).as_deref(), Some("me@example.org"));
}

#[test]
fn when_gh_api_user_fails_setup_writes_the_git_email() {
    let scenario = Scenario::new();
    scenario.github_profile_fails();
    scenario.git_email_is(Some("me@example.org"));

    let result = scenario.run(&["setup"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(email_to(&scenario).as_deref(), Some("me@example.org"));
    assert!(
        scenario
            .gh_calls()
            .iter()
            .any(|call| call[..2] == ["api", "user"]),
        "{:?}",
        scenario.gh_calls()
    );
}

#[test]
fn a_noreply_git_email_is_never_written() {
    let scenario = Scenario::new();
    scenario.git_email_is(Some("123+octo@users.noreply.github.com"));

    assert_eq!(scenario.run(&["setup"]).code, Some(0));

    let text = user_config(&scenario).unwrap();
    assert!(!text.contains("noreply"), "{text}");
    assert!(text.contains("\n# to = "), "{text}");
}

#[test]
fn with_neither_email_setup_writes_email_to_commented_out() {
    let scenario = Scenario::new();
    scenario.github_profile_fails();
    scenario.git_email_is(None);

    assert_eq!(scenario.run(&["setup"]).code, Some(0));

    let text = user_config(&scenario).unwrap();
    assert_eq!(email_to(&scenario), None, "{text}");
    assert!(text.contains("\n# to = \"you@example.com\""), "{text}");
}

#[test]
fn setup_never_replaces_an_existing_email_to() {
    let scenario = Scenario::new();
    scenario.github_email_is(Some("octo@example.com"));
    let mine = "[email]\nto = \"mine@example.net\"\n";
    scenario.user_config_is(mine);

    let result = scenario.run(&["setup"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(email_to(&scenario).as_deref(), Some("mine@example.net"));
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
const KEY_QUESTION: &str = "Resend API key (input hidden";
const KEY_PROMPT: &str = "Resend API key (input hidden, Enter to skip):";
const KEPT: &str = "Resend API key (input hidden, Enter keeps the saved one):";
const SKIPPED: &str = "No Resend API key, so no email can go yet.";
const OLD_KEY_HINT: &str = "export RESEND_API_KEY=";
const WROTE_CREDENTIALS: &str = "wrote the Credentials";
const KEY: &str = "re_secret_123";
const ACCEPTED: &str = r#"{"id":"49a3999c-0ce1-4ea6-ab68-afcd6dc2e794"}"#;

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

fn table(result: &TerminalResult) -> toml::Table {
    let text = result.user_config.as_ref().expect("no User config written");
    text.parse().unwrap()
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
fn on_a_terminal_the_answers_are_written_with_the_comments_on_each_key() {
    let scenario = Scenario::new();
    scenario.git_email_is(None);

    let result = setup_on_terminal(
        &scenario,
        &[("RESEND_API_KEY", KEY)],
        &[
            (HARNESS, ""),
            (MODEL, ""),
            (EFFORT, ""),
            (MERGE, "y"),
            (BASE_FIX, "y"),
            (PULL, "yes"),
            (NOTIFY, "y"),
            (TO, "me@example.com"),
            (FROM, "ts@acme.dev"),
            (TEST_EMAIL, ""),
        ],
    );

    let config = table(&result);
    assert_eq!(config["merge"]["always"].as_bool(), Some(true));
    assert_eq!(config["base"]["fix"].as_bool(), Some(true));
    assert_eq!(config["launch"]["pull"].as_bool(), Some(true));
    assert_eq!(config["email"]["always"].as_bool(), Some(true));
    assert_eq!(config["email"]["to"].as_str(), Some("me@example.com"));
    assert_eq!(config["email"]["from"].as_str(), Some("ts@acme.dev"));
    let text = result.user_config.unwrap();
    assert_eq!(
        key_names(&text),
        [
            "merge.always",
            "base.fix",
            "launch.pull",
            "email.always",
            "email.to",
            "email.from",
            "logs.dir",
            "activity.quiet_skips",
            "spec.parallel",
            "pickup.limit",
            "harness.default",
            "harness.claude.model",
            "harness.claude.effort",
            "harness.codex.model",
            "harness.codex.effort"
        ],
        "{text}"
    );
    for (section, line) in key_lines(&text) {
        assert!(
            !line.starts_with('#'),
            "[{section}] {line:?} is commented out"
        );
        let Some((_, comment)) = line.split_once(" # ") else {
            panic!("no trailing comment on [{section}] {line:?}");
        };
        assert!(comment.contains("default"), "[{section}] {line:?}");
    }
    assert!(!text.contains(KEY), "{text}");
    assert!(
        !result.stderr.contains(KEY_QUESTION),
        "terminal: {}",
        result.stderr
    );
}

#[test]
fn on_a_terminal_with_merging_on_setup_asks_about_base_fixes_and_writes_the_answer() {
    for (answer, allowed) in [("y", true), ("", false), ("n", false)] {
        let scenario = Scenario::new();
        scenario.git_email_is(None);

        let result = setup_on_terminal(
            &scenario,
            &[],
            &[
                (HARNESS, ""),
                (MODEL, ""),
                (EFFORT, ""),
                (MERGE, "y"),
                (BASE_FIX, answer),
                (PULL, ""),
                (NOTIFY, ""),
            ],
        );

        let config = table(&result);
        assert_eq!(config["merge"]["always"].as_bool(), Some(true));
        assert_eq!(config["base"]["fix"].as_bool(), Some(allowed), "{answer:?}");
        assert!(
            result.stderr.contains(&format!("{BASE_FIX} [y/N]")),
            "terminal: {}",
            result.stderr
        );
    }
}

#[test]
fn on_a_terminal_with_merging_off_setup_asks_nothing_about_base_fixes_and_writes_the_default() {
    for answer in ["", "n"] {
        let scenario = Scenario::new();
        scenario.git_email_is(None);

        let result = setup_on_terminal(
            &scenario,
            &[],
            &[
                (HARNESS, ""),
                (MODEL, ""),
                (EFFORT, ""),
                (MERGE, answer),
                (PULL, ""),
                (NOTIFY, ""),
            ],
        );

        assert!(
            !result.stderr.contains(BASE_FIX),
            "terminal: {}",
            result.stderr
        );
        let config = table(&result);
        assert_eq!(config["merge"]["always"].as_bool(), Some(false));
        assert_eq!(config["base"]["fix"].as_bool(), Some(false));
    }
}

#[test]
fn on_a_terminal_the_base_fix_question_defaults_to_the_user_configs_base_fix() {
    let scenario = Scenario::new();
    scenario.git_email_is(None);
    scenario.user_config_is("[merge]\nalways = true\n\n[base]\nfix = true # mine\n");

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
            (NOTIFY, ""),
        ],
    );

    assert!(
        result.stderr.contains(&format!("{BASE_FIX} [Y/n]")),
        "terminal: {}",
        result.stderr
    );
    let text = result.user_config.clone().unwrap();
    assert!(text.contains("fix = true # mine\n"), "{text}");
}

#[test]
fn on_a_terminal_turning_merging_off_writes_base_fix_at_its_default() {
    let scenario = Scenario::new();
    scenario.git_email_is(None);
    scenario.user_config_is("[merge]\nalways = true\n\n[base]\nfix = true  # mine\n");

    let result = setup_on_terminal(
        &scenario,
        &[],
        &[
            (HARNESS, ""),
            (MODEL, ""),
            (EFFORT, ""),
            (MERGE, "n"),
            (PULL, ""),
            (NOTIFY, ""),
        ],
    );

    assert!(
        !result.stderr.contains(BASE_FIX),
        "terminal: {}",
        result.stderr
    );
    let config = table(&result);
    assert_eq!(config["merge"]["always"].as_bool(), Some(false));
    assert_eq!(config["base"]["fix"].as_bool(), Some(false));
    let text = result.user_config.unwrap();
    assert!(text.contains("fix = false # mine\n"), "{text}");
}

#[test]
fn on_a_terminal_the_address_defaults_to_the_suggested_github_email() {
    let scenario = Scenario::new();
    scenario.github_email_is(Some("octo@example.com"));

    let result = setup_on_terminal(
        &scenario,
        &[],
        &[
            (HARNESS, ""),
            (MODEL, ""),
            (EFFORT, ""),
            (MERGE, ""),
            (PULL, ""),
            (NOTIFY, "y"),
            (TO, ""),
            (FROM, ""),
            (KEY_PROMPT, ""),
        ],
    );

    assert!(
        result.stderr.contains("octo@example.com"),
        "terminal: {}",
        result.stderr
    );
    let config = table(&result);
    assert_eq!(config["email"]["always"].as_bool(), Some(true));
    assert_eq!(config["email"]["to"].as_str(), Some("octo@example.com"));
    assert_eq!(
        config["email"]["from"].as_str(),
        Some("onboarding@resend.dev")
    );
}

#[test]
fn on_a_terminal_pressing_enter_throughout_keeps_an_existing_user_config() {
    let scenario = Scenario::new();
    scenario.github_email_is(Some("octo@example.com"));
    let mine = "\
[merge]
always = true

[base]
fix = true

[launch]
pull = true

[email]
always = true
to = \"mine@example.net\"
from = \"ts@acme.dev\"

[logs]
dir = \"/var/log/thirdshift\"

[activity]
quiet_skips = true

[spec]
parallel = 5

[pickup]
limit = 5

[harness]
default = \"claude\"

[harness.claude]
model = \"opus\"
effort = \"high\"

[harness.codex]
model = \"gpt-6.1-sol\"
effort = \"max\"
";
    scenario.user_config_is(mine);

    let result = setup_on_terminal(
        &scenario,
        &[("RESEND_API_KEY", KEY)],
        &[
            (HARNESS, ""),
            (MODEL, ""),
            (EFFORT, ""),
            (MERGE, ""),
            (BASE_FIX, ""),
            (PULL, ""),
            (NOTIFY, ""),
            (TO, ""),
            (FROM, ""),
            (TEST_EMAIL, ""),
        ],
    );

    assert_eq!(result.user_config.as_deref(), Some(mine));
    // Setup asks nothing about the Claim limit.
    assert!(
        !result.stderr.contains("limit"),
        "terminal: {}",
        result.stderr
    );
    for current in [
        "[Y/n]",
        "mine@example.net",
        "ts@acme.dev",
        "[opus]",
        "[high]",
    ] {
        assert!(
            result.stderr.contains(current),
            "terminal: {}",
            result.stderr
        );
    }
}

#[test]
fn on_a_terminal_setup_over_a_hand_commented_user_config_keeps_the_comments() {
    let scenario = Scenario::new();
    scenario.git_email_is(None);
    let mine = "\
# My machine.
[merge]
# Merging is for later.
always = false   # not yet

[email]
# to = \"someday@example.com\"
always = false # quiet, please
";
    scenario.user_config_is(mine);

    let result = setup_on_terminal(
        &scenario,
        &[],
        &[
            (HARNESS, ""),
            (MODEL, ""),
            (EFFORT, ""),
            (MERGE, "y"),
            (BASE_FIX, ""),
            (PULL, ""),
            (NOTIFY, "y"),
            (TO, "me@example.com"),
            (FROM, ""),
            (KEY_PROMPT, ""),
        ],
    );

    let text = result.user_config.clone().unwrap();
    for kept in [
        "# My machine.\n[merge]\n# Merging is for later.\nalways = true    # not yet\n",
        "always = true  # quiet, please\n",
    ] {
        assert!(text.contains(kept), "{text}");
    }
    assert!(!text.contains("# to = "), "{text}");
    let config = table(&result);
    assert_eq!(config["merge"]["always"].as_bool(), Some(true));
    assert_eq!(config["email"]["to"].as_str(), Some("me@example.com"));
    let mut names = key_names(&text);
    names.sort();
    assert_eq!(names, EVERY_KEY, "{text}");
}

#[test]
fn on_a_terminal_an_address_without_an_at_is_asked_again() {
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
            (NOTIFY, "y"),
            (TO, ""),
            (TO, "me.example.com"),
            (TO, "me@example.com"),
            (FROM, ""),
            (KEY_PROMPT, ""),
        ],
    );

    assert_eq!(result.stderr.matches(TO).count(), 3, "{}", result.stderr);
    assert_eq!(
        table(&result)["email"]["to"].as_str(),
        Some("me@example.com")
    );
}

#[test]
fn on_a_terminal_with_notifications_off_nothing_about_email_is_asked() {
    let scenario = Scenario::new();
    let resend = ResendStandIn::replying(200, ACCEPTED);
    scenario.git_email_is(None);

    let saved = "[resend]\nkey = \"re_saved_456\"\n";
    scenario.credentials_are(saved);

    let result = setup_on_terminal(
        &scenario,
        &[("THIRDSHIFT_RESEND_URL", resend.url())],
        &[
            (HARNESS, ""),
            (MODEL, ""),
            (EFFORT, ""),
            (MERGE, ""),
            (PULL, ""),
            (NOTIFY, "n"),
        ],
    );

    assert_eq!(scenario.credentials().as_deref(), Some(saved));
    for asked in [TO, FROM, KEY_QUESTION, "RESEND_API_KEY", TEST_EMAIL] {
        assert!(
            !result.stderr.contains(asked),
            "terminal: {}",
            result.stderr
        );
    }
    assert_eq!(table(&result)["email"]["always"].as_bool(), Some(false));
    assert!(resend.requests().is_empty());
}

#[test]
fn on_a_terminal_with_no_key_skipping_it_writes_no_credentials_and_says_how_to_add_one() {
    for key in [None, Some("")] {
        let scenario = Scenario::new();
        let env: Vec<(&str, &str)> = key.map(|key| ("RESEND_API_KEY", key)).into_iter().collect();

        let result = setup_on_terminal(&scenario, &env, &notifications_on(&[(KEY_PROMPT, "")]));

        assert!(
            result.stderr.contains(SKIPPED),
            "{key:?}: {}",
            result.stderr
        );
        for said in ["rerun `thirdshift setup`", "set RESEND_API_KEY"] {
            assert!(result.stderr.contains(said), "{key:?}: {}", result.stderr);
        }
        for unsaid in [OLD_KEY_HINT, TEST_EMAIL, WROTE_CREDENTIALS] {
            assert!(
                !result.stderr.contains(unsaid),
                "{key:?}: {}",
                result.stderr
            );
        }
        assert_eq!(scenario.credentials(), None, "{key:?}");
        assert_eq!(table(&result)["email"]["always"].as_bool(), Some(true));
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
fn on_a_terminal_a_key_not_starting_with_re_is_asked_again() {
    let scenario = Scenario::new();

    let result = setup_on_terminal(
        &scenario,
        &[],
        &notifications_on(&[
            (KEY_PROMPT, "sk_not_resend"),
            (KEY_PROMPT, KEY),
            (TEST_EMAIL, ""),
        ]),
    );

    assert_eq!(
        result.stderr.matches(KEY_PROMPT).count(),
        2,
        "{}",
        result.stderr
    );
    assert!(
        !result.stderr.contains("sk_not_resend"),
        "{}",
        result.stderr
    );
    let credentials: toml::Table = scenario.credentials().unwrap().parse().unwrap();
    assert_eq!(credentials["resend"]["key"].as_str(), Some(KEY));
}

#[test]
fn on_a_terminal_pressing_enter_keeps_the_saved_key() {
    let scenario = Scenario::new();
    let saved = "# Mine.\n[resend]\nkey = \"re_saved_456\"\n";
    scenario.credentials_are(saved);

    let result = setup_on_terminal(
        &scenario,
        &[],
        &notifications_on(&[(KEPT, ""), (TEST_EMAIL, "")]),
    );

    assert_eq!(scenario.credentials().as_deref(), Some(saved));
    assert!(
        !result.stderr.contains(WROTE_CREDENTIALS),
        "{}",
        result.stderr
    );
    assert!(!result.stderr.contains("re_saved"), "{}", result.stderr);
}

#[test]
fn on_a_terminal_a_new_key_replaces_the_saved_one_keeping_the_rest_of_the_file() {
    let scenario = Scenario::new();
    scenario.credentials_are(
        "# My secrets.\n[resend]\n# Rotated monthly.\nkey = \"re_saved_456\"   # from the dashboard\n",
    );

    setup_on_terminal(
        &scenario,
        &[],
        &notifications_on(&[(KEPT, KEY), (TEST_EMAIL, "")]),
    );

    assert_eq!(
        scenario.credentials().as_deref(),
        Some(
            "# My secrets.\n[resend]\n# Rotated monthly.\nkey = \"re_secret_123\"   # from the dashboard\n"
        )
    );
}

#[test]
fn on_a_terminal_a_key_is_added_to_credentials_that_hold_none() {
    let scenario = Scenario::new();
    scenario.credentials_are("# Keys go here.\n");

    setup_on_terminal(
        &scenario,
        &[],
        &notifications_on(&[(KEY_PROMPT, KEY), (TEST_EMAIL, "")]),
    );

    let text = scenario.credentials().unwrap();
    assert!(text.starts_with("# Keys go here.\n"), "{text}");
    assert!(!text.contains("# key"), "{text}");
    let credentials: toml::Table = text.parse().unwrap();
    assert_eq!(credentials["resend"]["key"].as_str(), Some(KEY));
}

#[test]
fn on_a_terminal_with_resend_api_key_set_no_key_is_asked_and_its_source_is_said() {
    let scenario = Scenario::new();
    let saved = "[resend]\nkey = \"re_saved_456\"\n";
    scenario.credentials_are(saved);

    let result = setup_on_terminal(
        &scenario,
        &[("RESEND_API_KEY", KEY)],
        &notifications_on(&[(TEST_EMAIL, "")]),
    );

    assert!(
        result
            .stderr
            .contains("The Resend API key comes from RESEND_API_KEY"),
        "{}",
        result.stderr
    );
    assert!(!result.stderr.contains(KEY_QUESTION), "{}", result.stderr);
    assert!(!result.stderr.contains(KEY), "{}", result.stderr);
    assert_eq!(scenario.credentials().as_deref(), Some(saved));
}

#[test]
fn on_a_terminal_the_test_email_goes_with_the_key_just_entered() {
    let scenario = Scenario::new();
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = setup_on_terminal(
        &scenario,
        &[("THIRDSHIFT_RESEND_URL", resend.url())],
        &notifications_on(&[(KEY_PROMPT, KEY), (TEST_EMAIL, "y")]),
    );

    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{}", result.stderr);
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some(format!("Bearer {KEY}").as_str())
    );
    let wrote = result.stderr.find(WROTE_CREDENTIALS).unwrap();
    let sent = result
        .stderr
        .find("accepted by Resend; check your inbox")
        .expect(&result.stderr);
    assert!(wrote < sent, "{}", result.stderr);
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
fn on_a_terminal_setup_refuses_broken_credentials_before_asking() {
    for broken in [
        "[resend]\nkye = \"re_saved_456\"\n",
        "[resend\n",
        "[resend]\nkey = 456\n",
    ] {
        let scenario = Scenario::new();
        scenario.credentials_are(broken);

        let result = scenario.run_on_terminal(&["setup"], &[("RESEND_API_KEY", KEY)], &[]);

        assert_eq!(result.code, Some(1), "{broken:?}: {}", result.stderr);
        assert!(
            result.stderr.contains("credentials.toml"),
            "{broken:?}: {}",
            result.stderr
        );
        assert!(
            !result.stderr.contains(MERGE),
            "{broken:?}: {}",
            result.stderr
        );
        assert_eq!(result.user_config, None);
        assert_eq!(scenario.credentials().as_deref(), Some(broken));
    }
}

#[test]
fn on_a_terminal_accepting_the_test_email_sends_one_and_declining_sends_none() {
    for (answer, sent) in [("y", 1), ("n", 0), ("", 0)] {
        let scenario = Scenario::new();
        let resend = ResendStandIn::replying(200, ACCEPTED);

        let result = setup_on_terminal(
            &scenario,
            &[
                ("RESEND_API_KEY", KEY),
                ("THIRDSHIFT_RESEND_URL", resend.url()),
            ],
            &[
                (HARNESS, ""),
                (MODEL, ""),
                (EFFORT, ""),
                (MERGE, ""),
                (PULL, ""),
                (NOTIFY, "y"),
                (TO, "me@example.com"),
                (FROM, ""),
                (TEST_EMAIL, answer),
            ],
        );

        let requests = resend.requests();
        assert_eq!(requests.len(), sent, "{answer:?}: {requests:?}");
        if sent == 1 {
            assert_eq!(requests[0].body["to"], "me@example.com");
            assert!(
                result
                    .stderr
                    .contains("accepted by Resend; check your inbox"),
                "terminal: {}",
                result.stderr
            );
        }
        assert!(result.user_config.is_some(), "{answer:?}");
    }
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

#[test]
fn on_a_terminal_setup_still_refuses_a_broken_user_config_before_asking() {
    let scenario = Scenario::new();
    let broken = "[merge]\nalway = true\n";
    scenario.user_config_is(broken);

    let result = scenario.run_on_terminal(&["setup"], &[], &[]);

    assert_eq!(result.code, Some(1), "terminal: {}", result.stderr);
    assert!(result.stderr.contains("unknown key merge.alway"));
    assert!(
        !result.stderr.contains(MERGE),
        "terminal: {}",
        result.stderr
    );
    assert_eq!(result.user_config.as_deref(), Some(broken));
}

// The Harness, Model and Effort.

/// What Claude says, and how it exits, when it refuses the Model it is
/// asked to run on.
const CLAUDE_REFUSES_THE_MODEL: &str = r#"
echo "There's an issue with the selected model (Opus 5.5). It may not exist or you may not have access to it."
exit 1
"#;

/// The answers that leave every setting but the Harness's as it is.
const NOTHING_ELSE: [Keystrokes; 3] = [(MERGE, ""), (PULL, ""), (NOTIFY, "")];

/// `PATH` with the scenario's fakes and the system's tools, and so no other
/// `claude` or `codex`, whatever is installed on the machine the tests run
/// on.
fn fakes_only_path(scenario: &Scenario) -> String {
    format!("{}:/usr/bin:/bin", scenario.path("bin").display())
}

/// Each call to `claude` that was the test call checking a Model, by its
/// arguments.
fn test_calls(scenario: &Scenario) -> Vec<Vec<String>> {
    scenario
        .claude_calls()
        .into_iter()
        .map(|call| {
            call["argv"]
                .as_array()
                .unwrap()
                .iter()
                .map(|arg| arg.as_str().unwrap().to_string())
                .collect()
        })
        .collect()
}

#[test]
fn on_a_terminal_the_harness_question_lists_both_harnesses_and_defaults_to_claude() {
    let scenario = Scenario::new();
    scenario.git_email_is(None);
    let path = fakes_only_path(&scenario);
    let mut keystrokes = vec![(HARNESS, ""), (MODEL, ""), (EFFORT, "")];
    keystrokes.extend(NOTHING_ELSE);

    let result = setup_on_terminal(&scenario, &[("PATH", &path)], &keystrokes);

    assert!(
        result
            .stderr
            .contains("Harness for every Run's sessions, claude or codex [claude]: "),
        "terminal: {}",
        result.stderr
    );
    assert!(
        !result.stderr.contains("for codex"),
        "terminal: {}",
        result.stderr
    );
    let config = table(&result);
    assert_eq!(config["harness"]["default"].as_str(), Some("claude"));
    for harness in ["claude", "codex"] {
        for key in ["model", "effort"] {
            assert_eq!(config["harness"][harness][key].as_str(), Some(""));
        }
    }
    assert!(test_calls(&scenario).is_empty());
}

#[test]
fn on_a_terminal_a_harness_not_installed_is_refused_and_asked_again() {
    let scenario = Scenario::new();
    scenario.git_email_is(None);
    fs::remove_file(scenario.path("bin/claude")).unwrap();
    let path = fakes_only_path(&scenario);
    let mut keystrokes = vec![
        (HARNESS, "claude"),
        (HARNESS, "gemini"),
        (HARNESS, "codex"),
        (CODEX_MODEL, ""),
        (CODEX_EFFORT, ""),
    ];
    keystrokes.extend(NOTHING_ELSE);

    let result = setup_on_terminal(&scenario, &[("PATH", &path)], &keystrokes);

    for refusal in [
        "claude is not installed: it isn't on PATH.",
        "Choose claude or codex.",
    ] {
        assert!(
            result.stderr.contains(refusal),
            "terminal: {}",
            result.stderr
        );
    }
    assert_eq!(table(&result)["harness"]["default"].as_str(), Some("codex"));
}

#[test]
fn on_a_terminal_with_neither_harness_installed_the_harness_is_left_as_it_was() {
    let scenario = Scenario::new();
    scenario.git_email_is(None);
    scenario.user_config_is("[harness.claude]\nmodel = \"opus\"\n");
    fs::remove_file(scenario.path("bin/claude")).unwrap();
    fs::remove_file(scenario.path("bin/codex")).unwrap();
    let path = fakes_only_path(&scenario);

    let result = setup_on_terminal(&scenario, &[("PATH", &path)], &NOTHING_ELSE);

    assert!(
        result
            .stderr
            .contains("claude (not installed) or codex (not installed)"),
        "terminal: {}",
        result.stderr
    );
    assert!(
        !result.stderr.contains(MODEL),
        "terminal: {}",
        result.stderr
    );
    let config = table(&result);
    assert_eq!(config["harness"]["default"].as_str(), Some("claude"));
    assert_eq!(config["harness"]["claude"]["model"].as_str(), Some("opus"));
}

#[test]
fn on_a_terminal_the_model_and_effort_answers_are_written_after_a_test_call_with_them() {
    let scenario = Scenario::new();
    scenario.git_email_is(None);
    let mut keystrokes = vec![(HARNESS, ""), (MODEL, "claude-opus-5-5"), (EFFORT, "high")];
    keystrokes.extend(NOTHING_ELSE);

    let result = setup_on_terminal(&scenario, &[], &keystrokes);

    let config = table(&result);
    assert_eq!(
        config["harness"]["claude"]["model"].as_str(),
        Some("claude-opus-5-5")
    );
    assert_eq!(config["harness"]["claude"]["effort"].as_str(), Some("high"));
    assert_eq!(config["harness"]["codex"]["model"].as_str(), Some(""));
    let text = result.user_config.unwrap();
    assert!(
        text.contains("model = \"claude-opus-5-5\" # the Model Claude Code's sessions run on"),
        "{text}"
    );
    assert_eq!(
        test_calls(&scenario),
        [["-p", "--model", "claude-opus-5-5", "--effort", "high"]]
    );
}

#[test]
fn on_a_terminal_the_model_and_effort_default_to_the_current_ones_and_a_dash_clears_one() {
    let scenario = Scenario::new();
    scenario.git_email_is(None);
    scenario.user_config_is(
        "[harness.claude]\nmodel = \"opus\"   # mine\neffort = \"high\"\n\n\
         [harness.codex]\nmodel = \"gpt-6.1-sol\"\n",
    );
    let mut keystrokes = vec![(HARNESS, ""), (MODEL, ""), (EFFORT, "-")];
    keystrokes.extend(NOTHING_ELSE);

    let result = setup_on_terminal(&scenario, &[], &keystrokes);

    for shown in [
        "Model for claude, - for claude's own default [opus]: ",
        "Effort for claude, - for claude's own default [high]: ",
    ] {
        assert!(result.stderr.contains(shown), "terminal: {}", result.stderr);
    }
    let text = result.user_config.clone().unwrap();
    assert!(text.contains("model = \"opus\"   # mine\n"), "{text}");
    let config = table(&result);
    assert_eq!(config["harness"]["claude"]["effort"].as_str(), Some(""));
    assert_eq!(
        config["harness"]["codex"]["model"].as_str(),
        Some("gpt-6.1-sol")
    );
    assert_eq!(test_calls(&scenario), [["-p", "--model", "opus"]]);
}

#[test]
fn on_a_terminal_a_model_claude_refuses_is_asked_again_with_claudes_error() {
    let scenario = Scenario::new();
    scenario.git_email_is(None);
    scenario.agent_does_in_session(1, CLAUDE_REFUSES_THE_MODEL);
    let mut keystrokes = vec![
        (HARNESS, ""),
        (MODEL, "Opus 5.5"),
        (EFFORT, ""),
        (MODEL, "opus"),
        (EFFORT, ""),
    ];
    keystrokes.extend(NOTHING_ELSE);

    let result = setup_on_terminal(&scenario, &[], &keystrokes);

    assert!(
        result.stderr.contains(
            "claude refused a test call on the Model Opus 5.5: There's an issue with the \
             selected model (Opus 5.5)."
        ),
        "terminal: {}",
        result.stderr
    );
    assert_eq!(
        table(&result)["harness"]["claude"]["model"].as_str(),
        Some("opus")
    );
    assert_eq!(test_calls(&scenario).len(), 2);
}

// Codex.

const CODEX_MODEL: &str = "Model for codex";
const CODEX_EFFORT: &str = "Effort for codex";

/// How Setup lists the fake `codex`'s catalog of Models.
const CODEX_MODELS: &str =
    "Codex's Models: gpt-6.1-sol (GPT-6.1-Sol), gpt-6-luna (GPT-6-Luna), gpt-5.5 (GPT-5.5)";

#[test]
fn on_a_terminal_codex_is_offered_with_its_models_and_the_chosen_models_efforts() {
    let scenario = Scenario::new();
    scenario.git_email_is(None);
    let path = fakes_only_path(&scenario);
    let mut keystrokes = vec![
        (HARNESS, "codex"),
        (CODEX_MODEL, "gpt-5.5"),
        (CODEX_EFFORT, "high"),
    ];
    keystrokes.extend(NOTHING_ELSE);

    let result = setup_on_terminal(&scenario, &[("PATH", &path)], &keystrokes);

    for shown in [
        "Harness for every Run's sessions, claude or codex [claude]: ",
        CODEX_MODELS,
        "Efforts gpt-5.5 supports: low, medium, high, xhigh",
    ] {
        assert!(result.stderr.contains(shown), "terminal: {}", result.stderr);
    }
    let config = table(&result);
    assert_eq!(config["harness"]["default"].as_str(), Some("codex"));
    assert_eq!(
        config["harness"]["codex"]["model"].as_str(),
        Some("gpt-5.5")
    );
    assert_eq!(config["harness"]["codex"]["effort"].as_str(), Some("high"));
    assert_eq!(config["harness"]["claude"]["model"].as_str(), Some(""));
    assert!(test_calls(&scenario).is_empty());
    assert!(scenario.codex_calls().is_empty());
}

#[test]
fn on_a_terminal_with_only_codex_installed_codex_is_the_default_answer() {
    let scenario = Scenario::new();
    scenario.git_email_is(None);
    fs::remove_file(scenario.path("bin/claude")).unwrap();
    let path = fakes_only_path(&scenario);
    let mut keystrokes = vec![(HARNESS, ""), (CODEX_MODEL, ""), (CODEX_EFFORT, "")];
    keystrokes.extend(NOTHING_ELSE);

    let result = setup_on_terminal(&scenario, &[("PATH", &path)], &keystrokes);

    for shown in [
        "Harness for every Run's sessions, claude (not installed) or codex [codex]: ",
        "Efforts Codex's Models support: low, medium, high, xhigh, max, ultra",
    ] {
        assert!(result.stderr.contains(shown), "terminal: {}", result.stderr);
    }
    let config = table(&result);
    assert_eq!(config["harness"]["default"].as_str(), Some("codex"));
    assert_eq!(config["harness"]["codex"]["model"].as_str(), Some(""));
    assert_eq!(config["harness"]["codex"]["effort"].as_str(), Some(""));
}

#[test]
fn on_a_terminal_a_codex_display_name_and_capitalised_effort_are_written_as_codex_names_them() {
    let scenario = Scenario::new();
    scenario.git_email_is(None);
    let path = fakes_only_path(&scenario);
    let mut keystrokes = vec![
        (HARNESS, "codex"),
        (CODEX_MODEL, "GPT-6.1-Sol"),
        (CODEX_EFFORT, "Max"),
    ];
    keystrokes.extend(NOTHING_ELSE);

    let result = setup_on_terminal(&scenario, &[("PATH", &path)], &keystrokes);

    let config = table(&result);
    assert_eq!(
        config["harness"]["codex"]["model"].as_str(),
        Some("gpt-6.1-sol")
    );
    assert_eq!(config["harness"]["codex"]["effort"].as_str(), Some("max"));
}

#[test]
fn on_a_terminal_an_unknown_codex_model_or_unsupported_effort_is_asked_again_with_the_choices() {
    let scenario = Scenario::new();
    scenario.git_email_is(None);
    let path = fakes_only_path(&scenario);
    let mut keystrokes = vec![
        (HARNESS, "codex"),
        (CODEX_MODEL, "gpt-7"),
        (CODEX_MODEL, "GPT-5.5"),
        (CODEX_EFFORT, "max"),
        (CODEX_EFFORT, "XHigh"),
    ];
    keystrokes.extend(NOTHING_ELSE);

    let result = setup_on_terminal(&scenario, &[("PATH", &path)], &keystrokes);

    for refusal in [
        "the Model gpt-7 is not in Codex's catalog: choose one of gpt-6.1-sol, gpt-6-luna, \
         gpt-5.5",
        "the Effort max is not one the Codex Model gpt-5.5 supports: choose one of low, \
         medium, high, xhigh",
    ] {
        assert!(
            result.stderr.contains(refusal),
            "terminal: {}",
            result.stderr
        );
    }
    let config = table(&result);
    assert_eq!(
        config["harness"]["codex"]["model"].as_str(),
        Some("gpt-5.5")
    );
    assert_eq!(config["harness"]["codex"]["effort"].as_str(), Some("xhigh"));
}

#[test]
fn on_a_terminal_the_codex_model_and_effort_default_to_the_current_ones_as_codex_names_them() {
    let scenario = Scenario::new();
    scenario.git_email_is(None);
    scenario.user_config_is("[harness.codex]\nmodel = \"GPT-6-Luna\"\neffort = \"High\"\n");
    let path = fakes_only_path(&scenario);
    let mut keystrokes = vec![(HARNESS, "codex"), (CODEX_MODEL, ""), (CODEX_EFFORT, "")];
    keystrokes.extend(NOTHING_ELSE);

    let result = setup_on_terminal(&scenario, &[("PATH", &path)], &keystrokes);

    for shown in [
        "Model for codex, - for codex's own default [GPT-6-Luna]: ",
        "Efforts gpt-6-luna supports: low, medium, high, xhigh, max",
        "Effort for codex, - for codex's own default [High]: ",
    ] {
        assert!(result.stderr.contains(shown), "terminal: {}", result.stderr);
    }
    let config = table(&result);
    assert_eq!(
        config["harness"]["codex"]["model"].as_str(),
        Some("gpt-6-luna")
    );
    assert_eq!(config["harness"]["codex"]["effort"].as_str(), Some("high"));
}

#[test]
fn on_a_terminal_when_codexs_catalog_cant_be_read_the_harness_is_left_as_it_was() {
    let scenario = Scenario::new();
    scenario.git_email_is(None);
    scenario.user_config_is("[harness.codex]\nmodel = \"gpt-5.5\"\n");
    // The fake is a link to the build every test shares, so it's replaced,
    // not written through.
    let codex = scenario.path("bin/codex");
    fs::remove_file(&codex).unwrap();
    fs::write(&codex, "#!/bin/sh\necho 'not logged in' >&2\nexit 1\n").unwrap();
    fs::set_permissions(&codex, fs::Permissions::from_mode(0o755)).unwrap();
    let path = fakes_only_path(&scenario);
    let mut keystrokes = vec![(HARNESS, "codex")];
    keystrokes.extend(NOTHING_ELSE);

    let result = setup_on_terminal(&scenario, &[("PATH", &path)], &keystrokes);

    assert!(
        result.stderr.contains(
            "codex debug models failed, so Codex's Models can't be read: not logged in\n\
             The harness settings stay as they are; rerun `thirdshift setup` once codex debug \
             models works."
        ),
        "terminal: {}",
        result.stderr
    );
    assert!(
        !result.stderr.contains(CODEX_MODEL),
        "terminal: {}",
        result.stderr
    );
    let config = table(&result);
    assert_eq!(config["harness"]["default"].as_str(), Some("claude"));
    assert_eq!(
        config["harness"]["codex"]["model"].as_str(),
        Some("gpt-5.5")
    );
}
