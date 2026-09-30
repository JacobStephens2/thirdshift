//! `thirdshift setup`: Setup writes a complete User config. With no terminal
//! and no User config, it writes every setting at its default without asking,
//! and `email.to` as the GitHub email it suggests, if it finds one; over an
//! existing one, it keeps its values and comments and adds the keys it lacks.
//! From a terminal, it first asks the Setup questions on stderr, each with the
//! current value as its default answer.

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
        ["[merge]", "[launch]", "[email]", "[logs]", "[spec]"],
        "{text}"
    );
    assert_eq!(config["merge"]["always"].as_bool(), Some(false));
    assert_eq!(config["launch"]["pull"].as_bool(), Some(false));
    assert_eq!(config["email"]["always"].as_bool(), Some(false));
    assert_eq!(
        config["email"]["from"].as_str(),
        Some("onboarding@resend.dev")
    );
    assert!(config["email"].get("to").is_none(), "{text}");
    assert_eq!(config["logs"]["dir"].as_str(), Some("~/.thirdshift/logs"));
    assert_eq!(config["spec"]["parallel"].as_integer(), Some(3));
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
            "launch.pull",
            "email.always",
            "email.to",
            "email.from",
            "logs.dir",
            "spec.parallel"
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
    assert_eq!(scenario.entries("home/.thirdshift/logs").len(), 1);
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
const EVERY_KEY: [&str; 7] = [
    "email.always",
    "email.from",
    "email.to",
    "launch.pull",
    "logs.dir",
    "merge.always",
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
        assert_eq!(result.stderr, run.stderr, "{broken:?}");
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
    assert_eq!(scenario.entries("home/elsewhere/logs").len(), 1);
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

const MERGE: &str = "Merge run?";
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

    let result = setup_on_terminal(&scenario, &[], &[(MERGE, ""), (PULL, ""), (NOTIFY, "")]);

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
            (MERGE, "y"),
            (PULL, "yes"),
            (NOTIFY, "y"),
            (TO, "me@example.com"),
            (FROM, "ts@acme.dev"),
            (TEST_EMAIL, ""),
        ],
    );

    let config = table(&result);
    assert_eq!(config["merge"]["always"].as_bool(), Some(true));
    assert_eq!(config["launch"]["pull"].as_bool(), Some(true));
    assert_eq!(config["email"]["always"].as_bool(), Some(true));
    assert_eq!(config["email"]["to"].as_str(), Some("me@example.com"));
    assert_eq!(config["email"]["from"].as_str(), Some("ts@acme.dev"));
    let text = result.user_config.unwrap();
    assert_eq!(
        key_names(&text),
        [
            "merge.always",
            "launch.pull",
            "email.always",
            "email.to",
            "email.from",
            "logs.dir",
            "spec.parallel"
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
fn on_a_terminal_the_address_defaults_to_the_suggested_github_email() {
    let scenario = Scenario::new();
    scenario.github_email_is(Some("octo@example.com"));

    let result = setup_on_terminal(
        &scenario,
        &[],
        &[
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

[launch]
pull = true

[email]
always = true
to = \"mine@example.net\"
from = \"ts@acme.dev\"

[logs]
dir = \"/var/log/thirdshift\"

[spec]
parallel = 5
";
    scenario.user_config_is(mine);

    let result = setup_on_terminal(
        &scenario,
        &[("RESEND_API_KEY", KEY)],
        &[
            (MERGE, ""),
            (PULL, ""),
            (NOTIFY, ""),
            (TO, ""),
            (FROM, ""),
            (TEST_EMAIL, ""),
        ],
    );

    assert_eq!(result.user_config.as_deref(), Some(mine));
    for current in ["[Y/n]", "mine@example.net", "ts@acme.dev"] {
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
            (MERGE, "y"),
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
        &[(MERGE, ""), (PULL, ""), (NOTIFY, "n")],
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

    let result = scenario.run_on_terminal(&["setup"], &[], &[(MERGE, "y"), (PULL, CTRL_C)]);

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
        &[(MERGE, "n"), (PULL, "y"), (NOTIFY, "y"), (TO, CTRL_C)],
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
