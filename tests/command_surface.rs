//! The command surface beyond the Run: `help`, `version`, and what an argument
//! thirdshift can't use prints. `update` has its own tests, in `update.rs`.

mod support;

use support::{RunResult, Scenario};

/// Assert that `text` lists every form of the command, each on its own line
/// with a description after it.
fn assert_help_text(text: &str) {
    for form in [
        "thirdshift <Issue URL>",
        "thirdshift merge <Issue URL>",
        "thirdshift --email <Issue URL>",
        "thirdshift email-test [<address>]",
        "thirdshift update",
        "thirdshift version",
        "thirdshift help",
    ] {
        let line = text.lines().find(|line| line.contains(form));
        let description = line
            .and_then(|line| line.split_once(form))
            .map(|(_, rest)| rest.trim());
        assert!(
            description.is_some_and(|description| !description.is_empty()),
            "expected {form:?} with a description in help: {text}"
        );
    }
}

#[test]
fn help_lists_every_form_of_the_command_on_stdout() {
    let scenario = Scenario::new();

    let help = scenario.run(&["help"]);

    assert_eq!(help.code, Some(0), "stderr: {}", help.stderr);
    assert_eq!(help.stderr, "");
    assert_help_text(&help.stdout);
    for alias in ["--help", "-h"] {
        let result = scenario.run(&[alias]);
        assert_eq!(result.code, Some(0), "{alias}: {}", result.stderr);
        assert_eq!(result.stderr, "", "{alias}");
        assert_eq!(result.stdout, help.stdout, "{alias}");
    }
}

#[test]
fn help_does_not_mention_the_flag_aliases() {
    let scenario = Scenario::new();

    let help = scenario.run(&["help"]);

    for alias in ["--help", "-h", "--version", "-V", "--merge"] {
        assert!(
            !help.stdout.contains(alias),
            "help mentions {alias}: {}",
            help.stdout
        );
    }
}

#[test]
fn version_prints_the_package_version_on_stdout() {
    let scenario = Scenario::new();

    for arg in ["version", "--version", "-V"] {
        let result = scenario.run(&[arg]);

        assert_eq!(result.code, Some(0), "{arg}: {}", result.stderr);
        assert_eq!(result.stderr, "", "{arg}");
        assert_eq!(
            result.stdout,
            format!("thirdshift {}\n", env!("CARGO_PKG_VERSION")),
            "{arg}"
        );
    }
}

/// Assert that thirdshift rejected its argument with exit 2: `error` first on
/// stderr, then the help text, and nothing on stdout.
fn assert_argument_error(scenario: &Scenario, result: &RunResult, error: &str) {
    assert_eq!(result.code, Some(2), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert!(
        result.stderr.starts_with(&format!("thirdshift: {error}\n")),
        "expected error {error:?} first in stderr: {}",
        result.stderr
    );
    let help = scenario.run(&["help"]).stdout;
    assert!(
        result.stderr.ends_with(&help),
        "expected the help text in stderr: {}",
        result.stderr
    );
}

#[test]
fn no_argument_prints_an_error_and_the_help_to_stderr() {
    let scenario = Scenario::new();

    let result = scenario.run(&[]);

    assert_argument_error(&scenario, &result, "missing Issue URL");
}

#[test]
fn an_unknown_word_prints_an_error_and_the_help_to_stderr() {
    let scenario = Scenario::new();

    let result = scenario.run(&["frobnicate"]);

    assert_argument_error(&scenario, &result, "not a GitHub issue URL: frobnicate");
}

#[test]
fn a_url_that_is_not_a_github_issue_prints_an_error_and_the_help_to_stderr() {
    let scenario = Scenario::new();
    let url = "https://github.com/acme/widgets/pull/7";

    let result = scenario.run(&[url]);

    assert_argument_error(
        &scenario,
        &result,
        &format!("not a GitHub issue URL: {url}"),
    );
}

#[test]
fn merge_without_an_issue_url_prints_an_error_and_the_help_to_stderr() {
    let scenario = Scenario::new();

    for merge in ["merge", "--merge"] {
        let result = scenario.run(&[merge]);

        assert_argument_error(&scenario, &result, "missing Issue URL");
    }
}

#[test]
fn an_extra_argument_prints_an_error_and_the_help_to_stderr() {
    let scenario = Scenario::new();
    let url = scenario.issue_url(7);

    for (args, extra) in [
        (vec![url.as_str(), "extra"], "extra"),
        (vec!["merge", url.as_str(), "extra"], "extra"),
        (vec![url.as_str(), url.as_str()], url.as_str()),
        (vec![url.as_str(), "--no-merge", "frobnicate"], "frobnicate"),
    ] {
        let result = scenario.run(&args);

        assert_argument_error(
            &scenario,
            &result,
            &format!("unexpected argument after the Issue URL: {extra}"),
        );
        assert!(scenario.claude_calls().is_empty(), "{args:?} started a Run");
    }
}

#[test]
fn a_repeated_flag_prints_an_error_and_the_help_to_stderr() {
    let scenario = Scenario::new();
    let url = scenario.issue_url(7);

    for (args, repeated) in [
        (vec!["merge", url.as_str(), "merge"], "merge"),
        (vec!["--merge", url.as_str(), "--merge"], "--merge"),
        (vec!["merge", "--merge", url.as_str()], "--merge"),
        (vec![url.as_str(), "no-merge", "--no-merge"], "--no-merge"),
        (vec!["no-email", url.as_str(), "--no-email"], "--no-email"),
        (vec!["--email", url.as_str(), "--email"], "--email"),
        (
            vec!["email", "a@example.com", url.as_str(), "email"],
            "email",
        ),
        (
            vec![url.as_str(), "email", "--email", "b@example.com"],
            "--email",
        ),
    ] {
        let result = scenario.run(&args);

        assert_argument_error(
            &scenario,
            &result,
            &format!("repeated argument: {repeated}"),
        );
        assert!(scenario.claude_calls().is_empty(), "{args:?} started a Run");
    }
}

#[test]
fn merge_with_no_merge_prints_an_error_and_the_help_to_stderr() {
    let scenario = Scenario::new();
    let url = scenario.issue_url(7);

    for args in [
        vec!["merge", url.as_str(), "no-merge"],
        vec!["--no-merge", "--merge", url.as_str()],
        vec![url.as_str(), "--merge", "no-merge"],
    ] {
        let result = scenario.run(&args);

        assert_argument_error(
            &scenario,
            &result,
            "merge and no-merge can't be used together",
        );
        assert!(scenario.claude_calls().is_empty(), "{args:?} started a Run");
    }
}

#[test]
fn email_with_no_email_prints_an_error_and_the_help_to_stderr() {
    let scenario = Scenario::new();
    let url = scenario.issue_url(7);

    for args in [
        vec!["email", url.as_str(), "no-email"],
        vec!["--no-email", "--email", "a@example.com", url.as_str()],
        vec![url.as_str(), "--email", "--no-email"],
    ] {
        let result = scenario.run(&args);

        assert_argument_error(
            &scenario,
            &result,
            "email and no-email can't be used together",
        );
        assert!(scenario.claude_calls().is_empty(), "{args:?} started a Run");
    }
}

#[test]
fn no_merge_without_an_issue_url_prints_an_error_and_the_help_to_stderr() {
    let scenario = Scenario::new();

    for no_merge in ["no-merge", "--no-merge"] {
        let result = scenario.run(&[no_merge]);

        assert_argument_error(&scenario, &result, "missing Issue URL");
    }
}

#[test]
fn help_lists_no_merge_and_the_user_config() {
    let scenario = Scenario::new();

    let help = scenario.run(&["help"]).stdout;

    for mention in ["--no-merge", "~/.thirdshift/config.toml", "merge.always"] {
        assert!(help.contains(mention), "help lacks {mention:?}: {help}");
    }
}

#[test]
fn help_lists_the_email_settings() {
    let scenario = Scenario::new();

    let help = scenario.run(&["help"]).stdout;

    for mention in [
        "email.to",
        "email.from",
        "Run notification",
        "--email <address>",
        "email.always",
        "--no-email",
        "onboarding@resend.dev",
        "RESEND_API_KEY",
    ] {
        assert!(help.contains(mention), "help lacks {mention:?}: {help}");
    }
}
