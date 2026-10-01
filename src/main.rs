mod architect;
mod args;
mod branch;
mod ci;
mod config;
mod email;
mod failed_run;
mod git;
mod github;
mod host;
mod interrupt;
mod issue;
mod notification;
mod plugin;
mod poll;
mod preflight;
mod progress;
mod prompt;
#[cfg(test)]
mod prompts_page;
mod questions;
mod resend_key;
mod run;
mod session;
mod spec_run;
mod update;
mod worktree;

use std::io::Write;
use std::process::ExitCode;

use args::{ArchitectArgs, Command, RunArgs};
use config::UserConfig;
use failed_run::FailedRun;
use notification::{NotificationAsk, RunNotification};
use run::{Goal, Reached};
use spec_run::Parallel;

const HELP: &str = "\
thirdshift turns a GitHub issue into a ready-for-review pull request, or a merged one, unattended.

usage: thirdshift <Issue URL>                         Run the factory on the issue, from the clone on the Base branch
       thirdshift merge <Issue URL>                   Run the factory on the issue, then merge its pull request
       thirdshift --no-merge <Issue URL>              Run the factory on the issue and leave its pull request for review
       thirdshift --email <Issue URL>                 Run the factory on the issue, then email how the Run ended
       thirdshift architect [<focus>]                 Review the Base branch's architecture, publish a plan for a refactor, and run it
       thirdshift architect [<focus>] --plan-only     Publish and mark ready the plan for a refactor, and stop there
       thirdshift email-test [<address>]              Send a test email through Resend, to check the email setup
       thirdshift setup                               Choose your defaults, then write the User config with every setting
       thirdshift update                              Update thirdshift to the latest release
       thirdshift version                             Print thirdshift's version
       thirdshift help                                Print this help

merge, --no-merge, --email, --no-email and parallel <n> (or --parallel <n>) go before or
after the Issue URL, in any order.

--email sends one Run notification when the Run ends, whatever the outcome: ready for
review, merged, failed or interrupted. --email <address> sends it to <address>; a word
after --email is the address only if it has an @ and isn't a URL.

On a Spec, an issue with sub-issues, the Run is a Spec run: it takes every Ticket (sub-issue) it
can reach, in the order their \"blocked by\" links allow, each merged into the Spec branch. Its Spec
PR opens as a draft, with a Tickets checklist, once the first Ticket lands, and is marked ready
once every Ticket is done. An Unready Ticket, one labelled ready-for-human, needs-info, wontfix or
needs-triage, is never run, nor is a Ticket with sub-issues, a Ticket in a cycle of blockers, or
any Ticket that one of these, an open issue outside the Spec or a failed Ticket blocks. If any
Ticket is not done, the Spec run fails, with a line on each saying why, leaving the Spec PR a draft.
Running the Spec again continues its Spec branch, its Spec PR and each failed Ticket's Issue branch;
with every Ticket closed and no Spec branch, there is nothing to do.
It runs up to 3 Tickets at once; parallel <n> runs up to <n> for one Spec run, and spec.parallel
sets the default:

    [spec]
    parallel = 2

A Spec run sends one Run notification for the whole Spec, with a line on each Ticket, and its
Ticket Runs send none.

Tickets always merge into the Spec branch, whatever the command or the User config says.
merge on a Spec merges the Spec PR into the Base branch once it is ready, mergeable and green,
as does merge.always; without either, or with --no-merge, the Spec PR is left ready for review.

architect starts an Architect run from the clone on the Base branch, with no Issue URL. Its
Architecture review, an agent session in its own worktree at the Base branch's head on origin,
looks for deepening opportunities and publishes the top one as a plan: a Spec with Tickets, or
a single Ticket. thirdshift then checks that the plan is open, new and not labelled
ready-for-human, needs-info or wontfix, swaps its needs-triage label for ready-for-agent,
and dispatches it as thirdshift <Issue URL> would: a Spec run on a Spec, a Run on a single
Ticket. The Architect run ends as that run does, with its exit code and its PR's URL.
merge, --no-merge and parallel <n> apply to that run, as do the User config's defaults,
except that it sends no Run notification, whatever email.always says;
parallel <n> fails it if the plan is a single Ticket. With --plan-only, the Architect run
prints the plan's URL and stops instead, for you to read, edit and run with
thirdshift <Issue URL>, and takes none of those flags. <focus> is free text, one argument,
that points the review at an area:

    thirdshift architect \"the Spec run\"

A review that fails, is interrupted, or ends without naming a plan fails the Architect run and
leaves any plan it published labelled needs-triage. Start one Architect run per repository at a
time: two at once may publish the same plan.

The User config, ~/.thirdshift/config.toml, sets defaults for every Run on this machine;
thirdshift setup asks for your defaults and writes one listing every setting, to edit.
With merge.always set, every Run is a Merge run unless given --no-merge:

    [merge]
    always = true

With launch.pull set, every Run first fast-forwards the checked-out Base branch to origin:

    [launch]
    pull = true

logs.dir sets where session logs go instead of ~/.thirdshift/logs: an absolute path, or one under ~/.

    [logs]
    dir = \"~/elsewhere/logs\"

--email and email-test send to their <address>, else to email.to, from email.from, else
from onboarding@resend.dev, which only delivers to your own Resend account's address.
With email.always set, every Run sends a Run notification unless given --no-email:

    [email]
    always = true
    to = \"you@example.com\"
    from = \"thirdshift@your-verified-domain.com\"

The Resend API key comes from the RESEND_API_KEY environment variable, else from the
Credentials, ~/.thirdshift/credentials.toml (mode 600), never from the User config.
With Run notifications on, thirdshift setup asks for it, hidden, and saves it there:

    [resend]
    key = \"re_...\"
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let RunArgs {
        issue,
        goal,
        email,
        parallel,
        spec_branch,
    } = match args::parse(&args) {
        Ok(Command::Help) => {
            print!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Ok(Command::Version) => {
            println!("thirdshift {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Ok(Command::Update) => return outcome(update::update()),
        Ok(Command::Setup) => return outcome(config::setup()),
        Ok(Command::EmailTest(to)) => {
            return outcome(
                UserConfig::load().and_then(|config| email::send_test(to, &config.email)),
            );
        }
        Ok(Command::Architect(architect_args)) => return architect(&architect_args),
        Ok(Command::Run(run_args)) => run_args,
        Err(error) => return argument_error(format_args!("{error:#}")),
    };
    let config = match user_config() {
        Ok(config) => config,
        Err(failure) => return failure,
    };
    // A Ticket's Run in a Spec run is always a Merge run, and leaves the Run
    // notification and the Launch directory to the Spec run.
    let (goal, email, launch_pull) = match spec_branch {
        Some(_) => (Goal::Merged, NotificationAsk::Skip, false),
        None => (
            goal.unwrap_or(config.default_goal()),
            email.unwrap_or(config.email.default_ask()),
            config.launch_pull,
        ),
    };
    // First, so no interrupt can end the Run once its notification is checked.
    if let Err(error) = interrupt::install() {
        return failure(&error);
    }
    let notification = match email {
        NotificationAsk::Send(to) => RunNotification::new(to, &config.email, &issue).map(Some),
        NotificationAsk::Skip => Ok(None),
    };
    let notification = match notification {
        Ok(notification) => notification,
        Err(error) => return failure(&error),
    };
    let ended = run::run(
        &issue,
        goal,
        &config.logs_dir,
        launch_pull,
        Parallel::new(parallel, config.spec_parallel),
        spec_branch.as_deref(),
    );
    let code = run_outcome(&ended);
    if let Some(notification) = notification {
        notification.send(&ended);
    }
    code
}

/// An Architect run: the Architecture review and its plan marked ready, then,
/// unless the command asked to stop at the plan, the plan dispatched as
/// `thirdshift <plan URL>` with the same flags would be, whose ending is the
/// Architect run's, though it sends no Run notification. One that stops at
/// the plan puts the plan's URL on stdout; one whose review or plan fails,
/// the cause and the session log on stderr.
fn architect(args: &ArchitectArgs) -> ExitCode {
    let config = match user_config() {
        Ok(config) => config,
        Err(failure) => return failure,
    };
    if let Err(error) = interrupt::install() {
        return failure(&error);
    }
    let plan = match architect::run(args.focus.as_deref(), &config.logs_dir, config.launch_pull) {
        Ok(plan) => plan,
        Err(failed) => return report(&failed),
    };
    let Some(dispatch) = &args.dispatch else {
        // Also on stderr, so the outcome shows even when stdout is captured.
        progress::step(format_args!("plan {} is ready for an agent", plan.url));
        print_url(&plan.url);
        return ExitCode::SUCCESS;
    };
    progress::step(format_args!(
        "dispatching the plan {url}, as thirdshift {url} would",
        url = plan.url
    ));
    run_outcome(&run::run(
        &plan,
        dispatch.goal.unwrap_or(config.default_goal()),
        &config.logs_dir,
        config.launch_pull,
        Parallel::new(dispatch.parallel, config.spec_parallel),
        None,
    ))
}

/// How a Run or a Spec run that `ended` shows: its pull request's URL on
/// stdout once it reached its goal, or as a Failed run does.
fn run_outcome(ended: &Result<Reached, FailedRun>) -> ExitCode {
    match ended {
        Ok(reached) => {
            // Also on stderr, so the outcome shows even when stdout is captured.
            progress::step(format_args!(
                "PR {} is {}",
                reached.pr_url,
                reached.goal.outcome()
            ));
            print_url(&reached.pr_url);
            ExitCode::SUCCESS
        }
        Err(failed) => report(failed),
    }
}

/// The User config, after offering Setup where there is none, or the failure
/// to exit with, its error reported.
fn user_config() -> Result<UserConfig, ExitCode> {
    config::offer_setup()
        .and_then(|()| UserConfig::load())
        .map_err(|error| failure(&error))
}

/// `error` on stderr, and the exit code of a failure.
fn failure(error: &anyhow::Error) -> ExitCode {
    progress::step(format_args!("{error:#}"));
    ExitCode::FAILURE
}

/// How a Failed run, or a failed Architect run, shows: its cause and its
/// session log on stderr, and its pull request's URL, if it left one, on
/// stdout.
fn report(failed: &FailedRun) -> ExitCode {
    progress::step(format_args!("{:#}", failed.error));
    if let Some(log) = &failed.log {
        progress::step(format_args!("{}{}", failed_run::SESSION_LOG, log.display()));
    }
    if let Some(pr_url) = &failed.pr_url {
        print_url(pr_url);
    }
    ExitCode::FAILURE
}

/// A pull request's or a plan's URL on stdout. A failed write, as once the
/// terminal has closed, is ignored, so the Run notification still goes.
fn print_url(url: &str) {
    let _ = writeln!(std::io::stdout(), "{url}");
}

/// The end of a command other than a Run: the line that says how it went,
/// or its error, on stderr.
fn outcome(result: anyhow::Result<impl std::fmt::Display>) -> ExitCode {
    match result {
        Ok(outcome) => {
            progress::step(outcome);
            ExitCode::SUCCESS
        }
        Err(error) => failure(&error),
    }
}

/// An argument thirdshift can't use: the error, then the help, on stderr.
fn argument_error(error: std::fmt::Arguments) -> ExitCode {
    progress::step(error);
    eprint!("\n{HELP}");
    ExitCode::from(2)
}
