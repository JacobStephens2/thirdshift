//! `thirdshift setup`: Setup writes a complete User config. With no terminal
//! and no User config, it writes every setting at its default without asking.

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
    assert_eq!(scenario.run(&["setup"]).code, Some(0));

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

#[test]
fn setup_leaves_an_existing_user_config_as_it_is() {
    let scenario = Scenario::new();
    let mine = "# mine\n[merge]\nalways = true\n";
    scenario.user_config_is(mine);

    let result = scenario.run(&["setup"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert_eq!(user_config(&scenario).as_deref(), Some(mine));
}

#[test]
fn setup_refuses_a_user_config_a_run_would_refuse_and_leaves_it_alone() {
    let scenario = Scenario::new();
    let broken = "[merge]\nalway = true\n";
    scenario.user_config_is(broken);

    let result = scenario.run(&["setup"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert!(
        result.stderr.contains("unknown key merge.alway"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(user_config(&scenario).as_deref(), Some(broken));
}
