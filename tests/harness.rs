//! The harness's waits: a test that waits for a Run to reach a point, the
//! fake agent starting or a prompt showing on the terminal, fails as soon as
//! the Run exits without reaching it, however long a slow Run may take.

mod support;

use std::panic::{AssertUnwindSafe, catch_unwind};
use support::Scenario;

/// What Pre-flight says of the closed issue #7, which ends a Run before any
/// agent session.
const CLOSED_ISSUE: &str = "issue #7 is closed";

/// What a Run on a terminal asks first when there is no User config.
const SETUP_OFFER: &str = "Set your defaults now? [Y/n]";

/// The message `wait` panics with.
fn failure_of<T>(wait: impl FnOnce() -> T) -> String {
    let Err(panic) = catch_unwind(AssertUnwindSafe(wait)) else {
        panic!("the wait did not fail");
    };
    match panic.downcast::<String>() {
        Ok(message) => *message,
        Err(panic) => panic.downcast::<&str>().unwrap().to_string(),
    }
}

#[test]
fn waiting_for_an_agent_that_never_starts_fails_with_the_runs_stderr_once_it_exits() {
    let scenario = Scenario::new();
    scenario.issue_is(7, "CLOSED");

    let message =
        failure_of(|| scenario.run_and_signal(&[&scenario.issue_url(7)], "started", "INT"));

    assert!(message.contains("the agent never started"), "{message}");
    assert!(message.contains("the Run exited first"), "{message}");
    assert!(message.contains(CLOSED_ISSUE), "{message}");
}

#[test]
fn waiting_for_a_prompt_that_never_shows_fails_with_the_terminal_once_the_run_exits() {
    let scenario = Scenario::new();
    scenario.issue_is(7, "CLOSED");

    let message = failure_of(|| {
        scenario.run_on_terminal(
            &[&scenario.issue_url(7)],
            &[],
            &[(SETUP_OFFER, "n"), ("Never asked?", "y")],
        )
    });

    assert!(
        message.contains("the terminal never showed \"Never asked?\""),
        "{message}"
    );
    assert!(message.contains("the Run exited first"), "{message}");
    assert!(message.contains(CLOSED_ISSUE), "{message}");
}
