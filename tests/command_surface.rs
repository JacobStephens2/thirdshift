//! The command surface beyond the Run: `help`, `version`, and what an argument
//! thirdshift can't use prints.

mod support;

use support::{RunResult, Scenario};

/// Assert that `text` lists all four forms of the command, each on its own
/// line with a description after it.
fn assert_help_text(text: &str) {
    for form in [
        "thirdshift <Issue URL>",
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

    for alias in ["--help", "-h", "--version", "-V"] {
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
fn update_is_not_yet_available() {
    let scenario = Scenario::new();

    let result = scenario.run(&["update"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert!(
        result.stderr.contains("not yet available"),
        "stderr: {}",
        result.stderr
    );
}
