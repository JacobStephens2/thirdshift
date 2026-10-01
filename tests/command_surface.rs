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
        "thirdshift architect [<focus>]",
        "thirdshift architect [<focus>] --plan-only",
        "thirdshift architect base <branch> [<focus>]",
        "thirdshift email-test [<address>]",
        "thirdshift setup",
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

    // Whole words, as `-h` is also in `ready-for-human`.
    let words: Vec<&str> = help
        .stdout
        .split(|c: char| c.is_whitespace() || ",.:;()[]<>".contains(c))
        .collect();
    for alias in ["--help", "-h", "--version", "-V", "--merge"] {
        assert!(
            !words.contains(&alias),
            "help mentions {alias}: {}",
            help.stdout
        );
    }
}

#[test]
fn help_documents_architect_its_focus_the_dispatch_plan_only_and_when_an_architect_run_is_skipped()
{
    let scenario = Scenario::new();

    let help = scenario.run(&["help"]).stdout;

    // However the lines are wrapped.
    let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
    for mention in [
        "<focus> is free text",
        "dispatches it as thirdshift <Issue URL> would: a Spec run on a Spec, a Run on a single Ticket",
        "merge, --no-merge, base-fix, --no-base-fix and parallel <n> apply to that run",
        "as do the User config's defaults",
        "With --plan-only, the Architect run prints the plan's URL and stops instead",
        "thirdshift architect \"the Spec run\"",
        "A review that finds no Strong candidate publishes no plan",
        "thirdshift prints that issue's URL instead, changing no label",
        "Only one Architect run per repository runs at a time on a machine",
        "is skipped: it prints an Architect run is already running on <owner>/<repo>, does nothing else and exits 0",
        "A skipped run still sends its Run notification, with the outcome skipped",
        "swaps its needs-triage label for ready-for-agent, labels it architect-plan",
        "creating the label if the repository lacks it",
        "An Architect run that finds an open issue labelled architect-plan is skipped too, before any review, with or without --plan-only",
        "prints its URL on stdout, gives the command that picks it up, thirdshift <plan URL>, and exits 0",
        "No flag overrides this: finish or close the Architect plan, or remove its label",
        "An Architect run never retries or dispatches an existing Architect plan",
        "--email, --email <address> and --no-email ask an Architect run for its Run notification",
        "It sends one for the whole Architect run, however it ends",
        "The run the plan is dispatched as sends none of its own",
    ] {
        assert!(help.contains(mention), "help lacks {mention:?}: {help}");
    }
}

const PLAN_ONLY_DISPATCHES_NOTHING: &str = "merge, no-merge, parallel, base-fix and no-base-fix can't be used with \
     --plan-only: it dispatches no run for them to apply to";

#[test]
fn architect_with_arguments_it_cant_use_prints_an_error_and_the_help_to_stderr() {
    let scenario = Scenario::new();

    for (args, error) in [
        (
            vec!["architect", "the Spec run", "the Run", "--plan-only"],
            "unexpected argument after the focus: the Run",
        ),
        (
            vec!["architect", "--plan-only", "--verbose"],
            "unexpected argument after architect: --verbose",
        ),
        (
            vec!["architect", "merge", "no-merge"],
            "merge and no-merge can't be used together",
        ),
        (
            vec!["architect", "parallel", "0"],
            "parallel must be followed by a whole number from 1 up, not 0",
        ),
        (
            vec!["architect", "--plan-only", "merge"],
            PLAN_ONLY_DISPATCHES_NOTHING,
        ),
        (
            vec!["architect", "no-merge", "--plan-only"],
            PLAN_ONLY_DISPATCHES_NOTHING,
        ),
        (
            vec!["architect", "--plan-only", "parallel", "2"],
            PLAN_ONLY_DISPATCHES_NOTHING,
        ),
        (
            vec!["architect", "base-fix", "--no-base-fix"],
            "base-fix and no-base-fix can't be used together",
        ),
        (
            vec!["architect", "--plan-only", "base-fix"],
            PLAN_ONLY_DISPATCHES_NOTHING,
        ),
        (
            vec!["architect", "no-base-fix", "--plan-only"],
            PLAN_ONLY_DISPATCHES_NOTHING,
        ),
        (
            vec!["architect", "the Spec run", "base"],
            "base must be followed by a branch",
        ),
        (
            vec!["architect", "--base", "--plan-only"],
            "--base must be followed by a branch, not --plan-only",
        ),
        (
            vec!["architect", "base", "main", "--plan-only", "--base", "main"],
            "repeated argument: --base",
        ),
    ] {
        let result = scenario.run(&args);

        assert_argument_error(&scenario, &result, error);
        assert!(
            scenario.claude_calls().is_empty(),
            "{args:?} started a review"
        );
    }
}

#[test]
fn help_documents_base_for_an_architect_run() {
    let scenario = Scenario::new();

    let help = scenario.run(&["help"]).stdout;

    // However the lines are wrapped.
    let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
    for mention in [
        "base <branch> (or --base <branch>) names the Architect run's Base branch",
        "whatever branch the clone has checked out",
        "on a detached HEAD or with uncommitted changes",
        "The review starts at <branch>'s head on origin",
        "the run the plan is dispatched as branches off <branch> and targets it with its pull request",
        "<branch> must exist on origin, with no local copy of it ahead",
        "launch.pull updates the clone only when <branch> is the branch checked out",
        "base goes with --plan-only too",
        "base is for architect only",
        "Without base, the Base branch is the branch checked out",
        "thirdshift architect base main",
    ] {
        assert!(help.contains(mention), "help lacks {mention:?}: {help}");
    }
}

#[test]
fn base_on_a_run_is_an_argument_error() {
    let scenario = Scenario::new();
    let url = scenario.issue_url(7);

    let result = scenario.run(&[&url, "base", "main"]);

    assert_argument_error(
        &scenario,
        &result,
        "unexpected argument after the Issue URL: base",
    );
    assert!(scenario.claude_calls().is_empty(), "a Run started");
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
        (vec!["base-fix", url.as_str(), "base-fix"], "base-fix"),
        (vec![url.as_str(), "base-fix", "--base-fix"], "--base-fix"),
        (
            vec!["no-base-fix", url.as_str(), "no-base-fix"],
            "no-base-fix",
        ),
        (
            vec![url.as_str(), "no-base-fix", "--no-base-fix"],
            "--no-base-fix",
        ),
        (
            vec!["parallel", "2", url.as_str(), "parallel", "2"],
            "parallel",
        ),
        (
            vec!["parallel", "2", "--parallel", "3", url.as_str()],
            "--parallel",
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
fn base_fix_with_no_base_fix_prints_an_error_and_the_help_to_stderr() {
    let scenario = Scenario::new();
    let url = scenario.issue_url(7);

    for args in [
        vec!["base-fix", url.as_str(), "no-base-fix"],
        vec!["--no-base-fix", "--base-fix", url.as_str()],
        vec![url.as_str(), "--base-fix", "no-base-fix"],
    ] {
        let result = scenario.run(&args);

        assert_argument_error(
            &scenario,
            &result,
            "base-fix and no-base-fix can't be used together",
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

#[test]
fn help_lists_no_base_fix_and_base_fix_in_the_user_config() {
    let scenario = Scenario::new();

    let help = scenario.run(&["help"]).stdout;

    for mention in ["--no-base-fix", "base.fix", "[base]\n    fix = true\n"] {
        assert!(help.contains(mention), "help lacks {mention:?}: {help}");
    }
}

#[test]
fn parallel_without_a_positive_whole_number_prints_an_error_and_the_help_to_stderr() {
    let scenario = Scenario::new();
    let url = scenario.issue_url(7);

    for (args, error) in [
        (
            vec!["parallel", "0", url.as_str()],
            "parallel must be followed by a whole number from 1 up, not 0",
        ),
        (
            vec!["--parallel", "-1", url.as_str()],
            "--parallel must be followed by a whole number from 1 up, not -1",
        ),
        (
            vec![url.as_str(), "parallel", "x"],
            "parallel must be followed by a whole number from 1 up, not x",
        ),
        (
            vec!["parallel", url.as_str()],
            "parallel must be followed by a whole number from 1 up, not https://github.com/acme/widgets/issues/7",
        ),
        (
            vec![url.as_str(), "parallel"],
            "parallel must be followed by a whole number from 1 up",
        ),
    ] {
        let result = scenario.run(&args);

        assert_argument_error(&scenario, &result, error);
        assert!(scenario.claude_calls().is_empty(), "{args:?} started a Run");
    }
}

#[test]
fn help_lists_parallel_and_spec_parallel() {
    let scenario = Scenario::new();

    let help = scenario.run(&["help"]).stdout;

    for mention in ["parallel <n>", "spec.parallel", "Spec"] {
        assert!(help.contains(mention), "help lacks {mention:?}: {help}");
    }
}
