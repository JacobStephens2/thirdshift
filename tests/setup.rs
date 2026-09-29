//! `thirdshift setup`: Setup writes a complete User config. With no terminal
//! and no User config, it writes every setting at its default without asking,
//! and `email.to` as the GitHub email it suggests, if it finds one; over an
//! existing one, it keeps its values and comments and adds the keys it lacks.

mod support;

use std::fs;

use support::Scenario;

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
        ["[merge]", "[launch]", "[email]", "[logs]"],
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
            "logs.dir"
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
const EVERY_KEY: [&str; 6] = [
    "email.always",
    "email.from",
    "email.to",
    "launch.pull",
    "logs.dir",
    "merge.always",
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
