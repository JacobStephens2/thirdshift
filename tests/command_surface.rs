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
        "thirdshift pickup",
        "thirdshift pickup base <branch>",
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

/// The help text with each run of whitespace as one space, so that a mention
/// is found however the lines are wrapped.
fn unwrapped_help(scenario: &Scenario) -> String {
    let help = scenario.run(&["help"]).stdout;
    help.split_whitespace().collect::<Vec<_>>().join(" ")
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

    let help = unwrapped_help(&scenario);

    for mention in [
        "<focus> is free text",
        "dispatches it as thirdshift <Issue URL> would: a Spec run on a Spec, a Run on a single Ticket",
        "merge, --no-merge, base-fix, --no-base-fix and parallel <n> apply to that run",
        "as do the User config's defaults",
        "With --plan-only, the Architect run prints the plan's URL and stops instead",
        "thirdshift architect \"the Spec run\"",
        "A review that finds no Strong candidate publishes no plan",
        "thirdshift prints that issue's URL instead, changing no label",
        "Only one Architect run or Pickup run per repository runs at a time on a machine",
        "is skipped: it prints an Architect run or a Pickup run is already running on <owner>/<repo>, does nothing else and exits 0",
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

    let help = unwrapped_help(&scenario);

    for mention in [
        "base <branch> (or --base <branch>) names the Architect run's Base branch",
        "whatever branch the clone has checked out",
        "on a detached HEAD or with uncommitted changes",
        "The review starts at <branch>'s head on origin",
        "the run the plan is dispatched as branches off <branch> and targets it with its pull request",
        "<branch> must exist on origin, with no local copy of it ahead",
        "launch.pull updates the clone only when <branch> is the branch checked out",
        "base goes with --plan-only too",
        "base is for architect and pickup only",
        "Without base, the Base branch is the branch checked out",
        "thirdshift architect base main",
    ] {
        assert!(help.contains(mention), "help lacks {mention:?}: {help}");
    }
}

#[test]
fn help_documents_pickup_the_ready_issue_the_dispatch_the_skips_and_the_run_notification() {
    let scenario = Scenario::new();

    let help = unwrapped_help(&scenario);

    for mention in [
        "pickup starts a Pickup run from the clone, with no Issue URL",
        "takes the lowest-numbered Ready issue in the repository and dispatches it as thirdshift <Issue URL> would: a Spec run on a Spec, a Run otherwise",
        "A Ready issue is an open issue labelled ready-for-agent",
        "none of ready-for-human, needs-info, wontfix and needs-triage",
        "is not in-progress",
        "is not a sub-issue, is not labelled base-fix, has no open blocker",
        "was never started: no Issue branch for it is on origin, and no pull request from one exists, open, merged or closed",
        "A Spec whose Tickets are all closed is not one either: a Spec run would find nothing to do",
        "It must also be settled: ten minutes have passed since ready-for-agent was applied to it, and since a sub-issue or a \"blocked by\" link of its was last added or removed",
        "A sub-issue is reached through its Spec, when the Spec is itself a Ready issue",
        "Each ready-for-agent issue a pass looks at and does not take gets one line on stderr with the first reason that applies, such as #21 is a Ticket of #20, which is not ready or #30 blocked by #29, before the line that says what the pass did",
        "The Pickup run ends as that run does, with its exit code and its PR's URL",
        "merge, --no-merge, base-fix, --no-base-fix and parallel <n> apply to that run, as do the User config's defaults",
        "parallel <n> is ignored when the issue is not a Spec",
        "base <branch> names the Pickup run's Base branch as it does an Architect run's",
        "pickup takes nothing else: no focus and no --plan-only",
        "A Pickup run is skipped, exiting 0 with nothing on stdout and one line on stderr saying why, after any lines on issues it passed over",
        "when the repository has no Ready issue",
        "while an Architect run or another Pickup run on the same repository is still running on this machine",
        "--email, --email <address> and --no-email ask a Pickup run for its Run notification as they do a Run, and email.always sets the default",
        "A pass that took an issue sends one: the notification the run it dispatched would send by hand, with that run's subject, outcome and body",
        "The dispatched run sends none of its own",
        "A skipped pass sends none, even when asked",
        "The notification's checks, an address and a Resend API key, are made before any other work on every pass, so one that would be skipped fails on them too, with exit 1",
    ] {
        assert!(help.contains(mention), "help lacks {mention:?}: {help}");
    }
}

#[test]
fn help_documents_the_claim_limit_and_taking_in_progress_off_closed_issues() {
    let scenario = Scenario::new();

    let help = unwrapped_help(&scenario);

    for mention in [
        "when the repository is at its Claim limit",
        "A Pickup run takes nothing while as many open issues are labelled in-progress, whoever started them, as the Claim limit, 3 unless set",
        "so a broken Base branch can't fail every Ready issue in turn, and pull requests can't pile up unreviewed",
        "pickup.limit in the User config sets the Claim limit, a whole number from 1 up",
        "[pickup] limit = 5",
        "There is no flag for it",
        "Each Pickup run that gets the lock first makes the Sweep: it takes in-progress off every closed issue that still has it",
        "so an issue merged by hand doesn't look taken",
        "A label it can't take off is a warning: line, and the pass carries on",
    ] {
        assert!(help.contains(mention), "help lacks {mention:?}: {help}");
    }
}

#[test]
fn pickup_with_arguments_it_cant_use_prints_an_error_and_the_help_to_stderr() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["ready-for-agent"]);

    for (args, error) in [
        (
            vec!["pickup", "the Spec run"],
            "unexpected argument after pickup: the Spec run",
        ),
        (
            vec!["pickup", "--plan-only"],
            "unexpected argument after pickup: --plan-only",
        ),
        (
            vec![
                "pickup",
                "merge",
                "https://github.com/acme/widgets/issues/7",
            ],
            "unexpected argument after pickup: https://github.com/acme/widgets/issues/7",
        ),
        (
            vec!["pickup", "merge", "--merge"],
            "repeated argument: --merge",
        ),
        (
            vec!["pickup", "merge", "no-merge"],
            "merge and no-merge can't be used together",
        ),
        (
            vec!["pickup", "base-fix", "--no-base-fix"],
            "base-fix and no-base-fix can't be used together",
        ),
        (
            vec!["pickup", "email", "--no-email"],
            "email and no-email can't be used together",
        ),
        (
            vec!["pickup", "parallel", "0"],
            "parallel must be followed by a whole number from 1 up, not 0",
        ),
        (
            vec!["pickup", "parallel", "2", "--parallel", "2"],
            "repeated argument: --parallel",
        ),
        (vec!["pickup", "base"], "base must be followed by a branch"),
        (
            vec!["pickup", "base", "main", "--base", "main"],
            "repeated argument: --base",
        ),
    ] {
        let result = scenario.run(&args);

        assert_argument_error(&scenario, &result, error);
        assert!(scenario.claude_calls().is_empty(), "{args:?} started a Run");
        assert_eq!(scenario.issue_labels(7), ["ready-for-agent"], "{args:?}");
    }
}

#[test]
fn pickup_is_a_command_only_as_the_first_argument() {
    let scenario = Scenario::new();

    let result = scenario.run(&["merge", "pickup"]);

    assert_argument_error(&scenario, &result, "not a GitHub issue URL: pickup");
}

#[test]
fn help_documents_the_claim() {
    let scenario = Scenario::new();

    let help = unwrapped_help(&scenario);

    for mention in [
        "A Run or a Spec run makes the Claim on its issue once its checks pass, before any work: it labels the issue in-progress, in place of ready-for-agent if it has that",
        "creating the label if the repository lacks it",
        "A Ticket's Run in a Spec run and a Base fix make none",
        "A Run whose Claim can't be made stops there",
        "The Claim is released, the issue's labels put back as they were, when the Run or the Spec run fails with nothing on origin to take over: no Issue branch or Spec branch and no pull request",
        "It is removed once a Self-merge has left the issue closed, and otherwise stays",
        "That run makes the Claim on the plan, which keeps architect-plan",
    ] {
        assert!(help.contains(mention), "help lacks {mention:?}: {help}");
    }
}

#[test]
fn help_points_to_running_an_architect_run_on_a_schedule_without_the_recipe() {
    let scenario = Scenario::new();

    let help = unwrapped_help(&scenario);

    for mention in [
        "To run an Architect run on a schedule, have the operating system's scheduler, such as cron, run thirdshift architect base main from the clone",
        "the README's \"On a schedule\" has a crontab entry",
    ] {
        assert!(help.contains(mention), "help lacks {mention:?}: {help}");
    }
    for recipe in ["PATH=", "* * *", "architect-cron.log"] {
        assert!(!help.contains(recipe), "help repeats {recipe:?}: {help}");
    }
}

#[test]
fn help_points_to_running_a_pickup_run_on_a_schedule_without_the_recipe() {
    let scenario = Scenario::new();

    let help = unwrapped_help(&scenario);

    for mention in [
        "To run a Pickup run on a schedule, have a scheduler, such as cron, run thirdshift pickup base main from the clone",
        "the README's \"A Pickup run on a schedule\" has a crontab entry",
    ] {
        assert!(help.contains(mention), "help lacks {mention:?}: {help}");
    }
    for recipe in ["PATH=", "* * *", "pickup-cron.log"] {
        assert!(!help.contains(recipe), "help repeats {recipe:?}: {help}");
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
