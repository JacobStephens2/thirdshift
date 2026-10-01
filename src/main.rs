mod architect;
mod args;
mod base_fix;
mod branch;
mod child_run;
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

use architect::{Outcome, Reviewed};
use args::{ArchitectArgs, Command, RunArgs};
use base_fix::BaseFixAsk;
use config::UserConfig;
use failed_run::FailedRun;
use notification::{ArchitectNotification, NotificationAsk, RunNotification};
use run::{Ended, Goal, StartedBy};
use spec_run::Parallel;

const HELP: &str = "\
thirdshift turns a GitHub issue into a ready-for-review pull request, or a merged one, unattended.

usage: thirdshift <Issue URL>                         Run the factory on the issue, from the clone on the Base branch
       thirdshift merge <Issue URL>                   Run the factory on the issue, then merge its pull request
       thirdshift --no-merge <Issue URL>              Run the factory on the issue and leave its pull request for review
       thirdshift --email <Issue URL>                 Run the factory on the issue, then email how the Run ended
       thirdshift architect [<focus>]                 Review the Base branch's architecture, publish a plan for a refactor, and run it
       thirdshift architect [<focus>] --plan-only     Publish and mark ready the plan for a refactor, and stop there
       thirdshift architect base <branch> [<focus>]   Do either with <branch> as the Base branch, from a clone on any branch
       thirdshift email-test [<address>]              Send a test email through Resend, to check the email setup
       thirdshift setup                               Choose your defaults, then write the User config with every setting
       thirdshift update                              Update thirdshift to the latest release
       thirdshift version                             Print thirdshift's version
       thirdshift help                                Print this help

merge, --no-merge, --email, --no-email, base-fix, --no-base-fix and parallel <n> (or
--parallel <n>) go before or after the Issue URL, in any order.

--email sends one Run notification when the Run ends, whatever the outcome: ready for
review, merged, failed or interrupted. --email <address> sends it to <address>; a word
after --email is the address only if it has an @ and isn't a URL.

A check that fails on the pull request and also on the Base branch commit it last merged in
is an Inherited failure, not the branch's to fix: a Run whose only red checks are Inherited
failures fails, saying to fix the Base branch first. With base-fix, it starts a Base fix
instead, once: it opens an issue for those checks, labelled base-fix and ready-for-agent,
runs a Merge run on it into the Base branch, waits for it to merge, then merges the Base
branch in and watches CI again. If the Base fix fails, or the checks still fail on the Base
branch once it has merged, the Run fails, naming the Base fix issue. A Run that finds an
open base-fix issue for the same Base branch and checks waits for that one to close
instead of starting another, and a Spec run's Tickets that meet the same Inherited failure
share one Base fix.

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
labels it architect-plan, creating the label if the repository lacks it, and dispatches it
as thirdshift <Issue URL> would: a Spec run on a Spec, a Run on a single Ticket. A Spec's
Tickets are never labelled architect-plan. The Architect run ends as that run does, with
its exit code and its PR's URL.
merge, --no-merge, base-fix, --no-base-fix and parallel <n> apply to that run, as do the
User config's defaults; parallel <n> fails it if the plan is a single Ticket. The review
itself watches no CI, so only that run can start a Base fix. With --plan-only, the
Architect run prints the plan's URL and stops instead, for you to read, edit and run with
thirdshift <Issue URL>, and takes none of those flags. <focus> is free text, one argument,
that points the review at an area:

    thirdshift architect \"the Spec run\"

base <branch> (or --base <branch>) names the Architect run's Base branch, whatever branch the
clone has checked out, so it can start from a clone on another branch, on a detached HEAD or
with uncommitted changes:

    thirdshift architect base main

The review starts at <branch>'s head on origin, and the run the plan is dispatched as branches
off <branch> and targets it with its pull request. <branch> must exist on origin, with no local
copy of it ahead, and launch.pull updates the clone only when <branch> is the branch checked
out. base goes with --plan-only too, before or after the focus and the other flags. base is
for architect only: thirdshift <Issue URL> doesn't take it. Without base, the Base branch is
the branch checked out.

A review that finds no Strong candidate publishes no plan. It files its top recommendation as
one idea issue labelled needs-triage, or names the open issue that already covers it, and
thirdshift prints that issue's URL instead, changing no label. Its last line says which: the
review filed the idea, or it filed nothing.

A review that fails, is interrupted, or ends without naming one of these issues fails the
Architect run and leaves any plan it published labelled needs-triage. One that finds no
deepening opportunity at all has no issue to name, so it fails the Architect run too.

Only one Architect run per repository runs at a time on a machine. One started while another
on the same repository is still running, the Spec run or Run it dispatched included, is
skipped: it prints an Architect run is already running on <owner>/<repo>, does nothing else
and exits 0. Nothing is left to clear once that other run ends, however it ends. Runs
started on an Issue URL are never skipped this way.

An Architect run that finds an open issue labelled architect-plan is skipped too, before
any review, with or without --plan-only: the last Architect plan is not finished. It names
each open Architect plan, prints its URL on stdout, gives the command that picks it up,
thirdshift <plan URL>, and exits 0. No flag overrides this: finish or close the Architect
plan, or remove its label. An Architect run never retries or dispatches an existing
Architect plan, so one whose run failed stays open until you pick it up.

To run an Architect run on a schedule, have the operating system's scheduler, such as cron, run
thirdshift architect base main from the clone: the README's \"On a schedule\" has a crontab entry.

--email, --email <address> and --no-email ask an Architect run for its Run notification as
they do a Run, with or without --plan-only, and email.always sets the default. It sends one
for the whole Architect run, however it ends: how the review ended, with the plan or idea
issue it named, and how the run the plan was dispatched as ended, with a line on each Ticket
of a Spec run. The run the plan is dispatched as sends none of its own. A skipped run still
sends its Run notification, with the outcome skipped and the reason.

The User config, ~/.thirdshift/config.toml, sets defaults for every Run on this machine;
thirdshift setup asks for your defaults and writes one listing every setting, to edit.
With merge.always set, every Run is a Merge run unless given --no-merge:

    [merge]
    always = true

With base.fix set, every Run may start a Base fix, as if given base-fix, unless given
--no-base-fix:

    [base]
    fix = true

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
        base_fix,
        child,
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
        Ok(Command::Architect(architect_args)) => return architect(architect_args),
        Ok(Command::Run(run_args)) => run_args,
        Err(error) => return argument_error(format_args!("{error:#}")),
    };
    let config = match user_config() {
        Ok(config) => config,
        Err(failure) => return failure,
    };
    // A child Run, a Ticket's Run in a Spec run or a Base fix, is always a
    // Merge run, and leaves the Run notification, the Launch directory and
    // whether it may start a Base fix to what started it.
    let (goal, email, launch_pull, base_fix) = match child {
        Some(_) => (
            Goal::Merged,
            NotificationAsk::Skip,
            false,
            base_fix.unwrap_or(BaseFixAsk::Forbid),
        ),
        None => (
            goal.unwrap_or(config.default_goal()),
            email.unwrap_or(config.email.default_ask()),
            config.launch_pull,
            base_fix.unwrap_or(config.default_base_fix()),
        ),
    };
    // First, so no interrupt can end the Run once its notification is checked.
    if let Err(error) = interrupt::install() {
        return failure(&error);
    }
    let notification = match asked(email, |to| RunNotification::new(to, &config.email, &issue)) {
        Ok(notification) => notification,
        Err(error) => return failure(&error),
    };
    let ended = run::run_to_end(
        &issue,
        goal,
        &config.logs_dir,
        launch_pull,
        Parallel::new(parallel, config.spec_parallel),
        child.as_ref().map_or(StartedBy::Command, StartedBy::Child),
        base_fix,
    );
    let code = run_outcome(&ended);
    if let Some(notification) = notification {
        notification.send(&ended);
    }
    code
}

/// An Architect run: the Architecture review and its plan marked ready, then,
/// unless the command asked to stop at the plan, the plan dispatched as
/// `thirdshift <plan URL>` with the same flags would be, but on the Architect
/// run's Base branch, whatever the Launch directory has checked out. The
/// dispatched run's ending is the Architect run's, with the Base fix it took,
/// if any. One that stops at the plan, or whose review found no Strong
/// candidate and so published no plan to dispatch, puts the URL of the issue
/// it ended on on stdout: the plan, the idea issue the review filed, or the
/// issue that already covers its top recommendation. One whose review or
/// plan fails puts the cause and the session log on stderr. One that is
/// skipped says why on stderr, and is no failure: as another on its
/// repository is still running, it puts nothing on stdout, and as Architect
/// plans are still open there, the URL of each. If asked, by the command or
/// the User config, it sends one Run notification, however it ended, skipped
/// included; the run it dispatched sends none of its own.
fn architect(args: ArchitectArgs) -> ExitCode {
    let config = match user_config() {
        Ok(config) => config,
        Err(failure) => return failure,
    };
    // First, so no interrupt can end the Architect run once its notification
    // is checked.
    if let Err(error) = interrupt::install() {
        return failure(&error);
    }
    let email = args.email.unwrap_or(config.email.default_ask());
    let notification = match asked(email, |to| ArchitectNotification::new(to, &config.email)) {
        Ok(notification) => notification,
        Err(error) => return failure(&error),
    };
    let ended = architect::run(
        args.focus.as_deref(),
        args.base.as_deref(),
        &config.logs_dir,
        config.launch_pull,
    );
    let dispatched = match (&ended, &args.dispatch) {
        (Ok(Outcome::Reviewed(Reviewed::PlanReady { plan, base })), Some(dispatch)) => {
            progress::step(format_args!(
                "dispatching the plan {url}, as thirdshift {url} would",
                url = plan.url
            ));
            Some(run::run_to_end(
                plan,
                dispatch.goal.unwrap_or(config.default_goal()),
                &config.logs_dir,
                config.launch_pull,
                Parallel::new(dispatch.parallel, config.spec_parallel),
                StartedBy::ArchitectRun { base },
                dispatch.base_fix.unwrap_or(config.default_base_fix()),
            ))
        }
        _ => None,
    };
    let code = match (&ended, &dispatched) {
        (_, Some(dispatched)) => run_outcome(dispatched),
        (Ok(outcome), None) => {
            // Also on stderr, so the outcome shows even when stdout is captured.
            progress::step(format_args!("{outcome}"));
            outcome.urls().into_iter().for_each(print_url);
            ExitCode::SUCCESS
        }
        (Err(failed), None) => report(failed),
    };
    if let Some(notification) = notification {
        notification.send(&ended, dispatched.as_ref());
    }
    code
}

/// The Run notification `email` asks for, if it asks for one: what `checked`
/// makes of the address it gave, or its error if a check fails.
fn asked<N>(
    email: NotificationAsk,
    checked: impl FnOnce(Option<String>) -> anyhow::Result<N>,
) -> anyhow::Result<Option<N>> {
    match email {
        NotificationAsk::Send(to) => checked(to).map(Some),
        NotificationAsk::Skip => Ok(None),
    }
}

/// How a Run or a Spec run that `ended` shows: what became of its Base fix,
/// if it took one, then its pull request's URL on stdout once it reached its
/// goal, or as a Failed run does.
fn run_outcome(ended: &Ended) -> ExitCode {
    // Before the outcome, which a failed child Run's last lines are read as.
    if let Some(report) = &ended.base_fix {
        progress::step(format_args!("{}{report}", base_fix::REPORT));
    }
    match &ended.outcome {
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

/// A pull request's URL, or the URL of the issue an Architect run ended on,
/// on stdout. A failed write, as once the terminal has closed, is ignored, so
/// the Run notification still goes.
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
