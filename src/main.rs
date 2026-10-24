mod architect;
mod args;
mod asks;
mod base_fix;
mod branch;
mod child_run;
mod ci;
mod claim;
mod command;
mod config;
mod console;
mod cronjobs;
mod delivery;
mod email;
mod failed_run;
mod git;
mod github;
mod harness;
mod host;
mod interrupt;
mod issue;
mod labels;
mod launch;
mod logs;
mod notification;
mod otlp_jsonl;
mod pass;
mod pickup;
mod poll;
mod process;
mod progress;
mod prompt;
#[cfg(test)]
mod prompts_page;
mod proxy;
mod pull_request;
mod ready;
mod recovery;
mod resend_key;
mod run;
mod run_ending;
mod sandbox;
mod scoring;
mod security;
mod session;
mod setup;
mod skills;
mod spec_run;
mod terminal;
#[cfg(test)]
mod test_support;
mod trust;
mod update;
mod worktree;

use std::process::ExitCode;

use architect::Outcome;
use args::{ArchitectArgs, Command, PassArgs, RunArgs};
use asks::Asks;
use command::{Ending, failure};
use config::UserConfig;
use logs::Begin;
use notification::About;
use pickup::Took;
use run::StartedBy;

const HELP: &str = "\
thirdshift turns a GitHub issue into a ready-for-review pull request, unattended.

usage: thirdshift <Issue URL>                         Run the factory on the issue
       thirdshift <number>                            Resolve the issue from origin and run
       thirdshift merge <number>                      Run it, then merge the pull request
       thirdshift merge <Issue URL>                   Run, then merge the pull request
       thirdshift --no-merge <Issue URL>              Run, leaving the pull request for review
       thirdshift --email <Issue URL>                 Run, then email the outcome
       thirdshift architect [<focus>]                 Review architecture, publish a plan and run it
       thirdshift architect [<focus>] --plan-only     Publish a plan and stop
       thirdshift architect base <branch> [<focus>]   Use <branch> as the Base branch
       thirdshift pickup                              Take the lowest-numbered Ready issue and run it
       thirdshift pickup base <branch>                Use <branch> as the Base branch
       thirdshift secure                              Audit and record findings privately
       thirdshift secure base <branch>                Use <branch> as the Base branch
       thirdshift score                               Sample merged factory pull requests
       thirdshift scores [<pull request>] [--json|--mine]  Show Scores; --mine enters yours
       thirdshift email-test [<address>]              Send a test email through Resend
       thirdshift setup                               Choose defaults and write the User config
       thirdshift status [--all] [--json]             List this machine's Commands
       thirdshift cronjobs [--json]                   List this account's thirdshift cron jobs
       thirdshift console [--no-sign-in]              Serve this machine's Console on localhost
       thirdshift console install [--output <file>]   Print, or save, a service and proxy install script for sudo
       thirdshift console password [--email <address>]  Set a Console password privately
       thirdshift console sign-out [--email <address>] [--json]  End an account's sessions
       thirdshift console log [--json]                Read this machine's Console log
       thirdshift console theme <name|list> [--json]  Choose or list Console Themes
       thirdshift console theme import <file>        Import a Theme JSON document
       thirdshift console theme export <name>        Export a Theme as JSON
       thirdshift console sections [--hide <section>] [--show <section>] [--json]  Choose visible sections
       thirdshift show <command> [--json]             Show one Command in full
       thirdshift stop <command> [--requeue] [--json]  Stop a live Command and wait for cleanup
       thirdshift logs [--command <id>] [--json]      List this machine's logs
       thirdshift logs --file <id> [--follow]         Read a log; --convert / --remove-converted migrate old Sessions
       thirdshift logs <command> [--session <kind>] [--follow]  Read a log; --export <dir> writes its whole Trace
       thirdshift update                              Update to the latest release
       thirdshift version                             Print the version
       thirdshift help                                Print this reference

Start from the issue's repository clone. The Base branch is the branch checked out.
Positive issue numbers use origin's GitHub repository, including from nested directories.
An issue with sub-issues starts a Spec run: its Tickets merge into a Spec branch.
merge on a Spec merges the Spec PR. <focus> is one quoted free-text argument.

Shared options (before or after the Issue URL or number, or after a Pass command):
  merge / no-merge                    Merge the pull request / leave it for review
  email [<address>] / no-email        Send a Run notification / skip it
  base-fix / no-base-fix              Allow / forbid fixing inherited CI failures
  parallel <n>                       Run up to <n> Tickets at once (Spec runs; default 3)
  harness <name>                     Choose claude, codex, agy, grok, muse, opencode
  model <name>                       Choose the Harness's Model
  effort <level>                     Choose the Harness's Effort
  security-review / no-security-review  Enable / disable a Run's Security review
  security-fix / no-security-fix     Allow / forbid fixing reproduced Security findings

Options also accept a -- prefix. Each option may be given once; opposite options
cannot be combined. Command options override the User config's defaults.

Pass commands: architect, pickup, secure (no Issue URL); base <branch> uses an origin branch.
  --plan-only                        architect only; stops before dispatching a Run
With --plan-only, only base, email and no-email apply.

Defaults: ~/.thirdshift/config.toml; thirdshift setup writes every setting.
For workflow details, settings, Harness setup, email, logs and scheduling, see the manual:
  https://thirdshift.app/docs/
";

fn main() -> ExitCode {
    child_run::name_this_process();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(result) = setup::sandbox::child(&args) {
        return result;
    }
    let RunArgs {
        issue,
        flags,
        given,
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
        Ok(Command::Setup) => return outcome(setup::setup()),
        Ok(Command::Score) => return score(),
        Ok(Command::Scores {
            pull_request,
            mine,
            json,
        }) => {
            return printed(scoring::show(pull_request.as_deref(), mine, json));
        }
        Ok(Command::CronJobs { json }) => return printed(cronjobs::show(json)),
        Ok(Command::ConsoleInstall { output: None }) => {
            return match console::install() {
                Ok(script) => {
                    print!("{script}");
                    ExitCode::SUCCESS
                }
                Err(error) => failure(&error),
            };
        }
        Ok(Command::ConsoleInstall { output: Some(path) }) => {
            return outcome(console::install_to(std::path::Path::new(&path)));
        }
        Ok(Command::ConsolePassword(email)) => return outcome(console::password(email.as_deref())),
        Ok(Command::ConsoleTheme { name, json }) => {
            return printed(console::theme(name.as_deref(), json));
        }
        Ok(Command::ConsoleThemeImport(path)) => return printed(console::import_theme(&path)),
        Ok(Command::ConsoleThemeExport(name)) => return printed(console::export_theme(&name)),
        Ok(Command::ConsoleSections { hide, show, json }) => {
            return printed(console::sections(&hide, &show, json));
        }
        Ok(Command::ConsoleSignOut { email, json }) => {
            return match console::sign_out(email.as_deref(), json) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => failure(&error),
            };
        }
        Ok(Command::ConsoleLog { json }) => {
            return match console::print_log(json) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => failure(&error),
            };
        }
        Ok(Command::Console { no_sign_in }) => {
            return match console::serve(no_sign_in) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => failure(&error),
            };
        }
        Ok(Command::Stop {
            command,
            requeue,
            json,
        }) => {
            return match logs::stop::stop(&command, requeue, json) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => failure(&error),
            };
        }
        Ok(Command::Show { command, json }) => {
            return match logs::status::inspect::show(&command, json) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => failure(&error),
            };
        }
        Ok(Command::LogsConvert { remove }) => {
            return match logs::convert::run(remove) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => failure(&error),
            };
        }
        Ok(Command::Logs {
            command,
            session,
            follow,
        }) => {
            return match logs::status::inspect::logs(&command, session.as_deref(), follow) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => failure(&error),
            };
        }
        Ok(Command::LogsExport { command, directory }) => {
            return match logs::export::export(&command, &directory) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => failure(&error),
            };
        }
        Ok(Command::LogsList { json, owner }) => {
            return match logs::catalog::show(json, owner.as_deref()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => failure(&error),
            };
        }
        Ok(Command::LogFile { id, follow }) => {
            return match logs::catalog::print_file(&id, follow) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => failure(&error),
            };
        }
        Ok(Command::Status { json, all }) => {
            return match logs::status::show(json, all) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => failure(&error),
            };
        }
        Ok(Command::ReleaseSummary) => return release_summary(),
        Ok(Command::EmailTest(to)) => {
            return outcome(
                UserConfig::load().and_then(|config| email::send_test(to, &config.email)),
            );
        }
        Ok(Command::Architect(architect_args)) => return architect(architect_args),
        Ok(Command::Pickup(pickup_args)) => return pickup(pickup_args),
        Ok(Command::Secure(args)) => return secure(args),
        Ok(Command::Run(run_args)) => *run_args,
        Err(error) => return argument_error(format_args!("{error:#}")),
    };
    let issue = match launch::resolve_issue(issue) {
        Ok(issue) => issue,
        Err(error) => return failure(&error),
    };
    let begin = match &given {
        Some(given) => Begin::ChildRun(&given.stamp, given.command),
        None => Begin::Run(&issue),
    };
    let asks_of = |config: &UserConfig| match &given {
        Some(given) => Asks::of_child_run(given, config),
        None => Asks::of_run(&issue, &flags, config),
    };
    let ask = |config: &UserConfig| asks_of(config).notification;
    let (config, mut started) = match command::start(begin, ask, About::Issue(&issue)) {
        Ok(started) => started,
        Err(failure) => return failure,
    };
    let mut asks = asks_of(&config);
    let started_by = given
        .as_ref()
        .map_or(StartedBy::Command, |given| StartedBy::Child(&given.kind));
    let ended = run::run_to_end(&issue, &mut asks, started_by);
    started.built_with(&asks.harness);
    started.finish(Ending::Run(ended))
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
/// skipped says why on stderr, and is no failure: as another Pass on its
/// repository is still running, it puts nothing on stdout,
/// and as Architect plans are still open there, or Architect ideas wait for
/// triage there, the URL of each, or as it has a Ready issue, that issue's
/// URL. If asked, by the command or the User config, it sends one Run
/// notification, however it ended, short of being skipped; the run it
/// dispatched sends none of its own. The notification's checks are made
/// before any other work all the same, so a run that would be skipped fails
/// on them too. Its Command log, kept once it is past its skip checks, covers
/// that run too. The repository's Activity log records the skip, if it
/// differs from the last, or the start and end of its work. With
/// `activity.quiet_skips` set, a skipped run prints nothing at all.
fn architect(args: ArchitectArgs) -> ExitCode {
    let ask = |config: &UserConfig| args.flags.notification(config);
    let (config, mut started) = match command::start(Begin::ArchitectRun, ask, About::ArchitectRun)
    {
        Ok(started) => started,
        Err(failure) => return failure,
    };
    let mut harness = args.flags.harness(&config);
    let outcome = architect::run(
        args.focus.as_deref(),
        args.base.as_deref(),
        args.plan_only,
        &args.flags,
        &config,
        &mut harness,
    );
    started.built_with(&harness);
    started.finish(match outcome {
        Outcome::Skipped(skipped) => Ending::Skipped(skipped.into()),
        Outcome::Ran { review, dispatched } => Ending::Architect { review, dispatched },
    })
}

/// A Pickup run: the search for the lowest-numbered Ready issue in the Launch
/// directory's repository, then that issue dispatched as `thirdshift <Issue
/// URL>` with the same flags would be, but on the Pickup run's Base branch,
/// whatever the Launch directory has checked out. The dispatched run's ending
/// is the Pickup run's. One that is skipped, as when the repository is at the
/// User config's Claim limit, says why on stderr, puts nothing on stdout, and
/// is no failure. If asked, by the command or the User config, one that took
/// an issue sends one Run notification, naming the Pickup run and the kind
/// of run it dispatched, and that run sends none of its own; one that is skipped
/// sends none. The notification's checks are made before any other work all
/// the same, so a pass that would be skipped fails on them too. Its Command
/// log is kept once it has taken an issue, and covers the dispatched run.
/// The repository's Activity log records the skip, if it differs from the
/// last, or the start and end of its work. With `activity.quiet_skips` set, a
/// skipped pass prints nothing at all.
fn pickup(args: PassArgs) -> ExitCode {
    let ask = |config: &UserConfig| args.flags.notification(config);
    let (config, mut started) = match command::start(Begin::PickupRun, ask, About::PickupRun) {
        Ok(started) => started,
        Err(failure) => return failure,
    };
    let mut harness = args.flags.harness(&config);
    let took = pickup::run(args.base.as_deref(), &args.flags, &config, &mut harness);
    started.built_with(&harness);
    match took {
        Ok(pickup::Outcome::Took(Took {
            issue,
            is_spec,
            title,
            ended,
        })) => {
            started.took(&issue, title, is_spec);
            started.finish(Ending::Run(ended))
        }
        Ok(pickup::Outcome::Skipped(skipped)) => started.finish(Ending::Skipped(skipped.into())),
        Err(error) => failure(&error),
    }
}

/// A Security run, with one notification when asked, unless skipped.
fn secure(args: PassArgs) -> ExitCode {
    let (config, mut started) = match command::start(
        Begin::SecurityRun,
        |config| args.flags.notification(config),
        About::SecurityRun,
    ) {
        Ok(started) => started,
        Err(failure) => return failure,
    };
    let mut harness = args.flags.security_harness(&config);
    let outcome = security::run(args.base.as_deref(), &args.flags, &config, &mut harness);
    started.built_with(&harness);
    started.finish(match outcome {
        security::Outcome::Skipped(skipped) => Ending::Skipped(command::Skip {
            reason: skipped.to_string(),
            urls: Vec::new(),
        }),
        security::Outcome::Audited(audited) => Ending::Security(audited),
        security::Outcome::Fixed { ended, findings } => Ending::SecurityFix { ended, findings },
    })
}

fn score() -> ExitCode {
    let _lock = match scoring::lock() {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            progress::step("Scoring run skipped: another Scoring run is running on this machine");
            return ExitCode::SUCCESS;
        }
        Err(error) => return failure(&error),
    };
    let started_at = chrono::Utc::now();
    let (config, started) = match command::start(
        Begin::ScoringRun,
        |_| notification::NotificationAsk::Skip,
        About::ScoringRun,
    ) {
        Ok(started) => started,
        Err(failure) => return failure,
    };
    started.finish(Ending::Scoring(
        scoring::sample(&config, started_at).map_err(Into::into),
    ))
}

/// The release script's internal interface: the prompt on stdin, only the
/// summary on stdout, and failures on stderr for its generated-notes fallback.
fn release_summary() -> ExitCode {
    use std::io::Read;
    let summary = (|| -> anyhow::Result<String> {
        interrupt::install()?;
        let config = UserConfig::load()?;
        let mut prompt = String::new();
        std::io::stdin().read_to_string(&mut prompt)?;
        harness::write_summary(&config.harness, &prompt)
    })();
    match summary {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(error) => failure(&error),
    }
}

/// A Console settings view or action, with machine-readable output on stdout.
fn printed(result: anyhow::Result<String>) -> ExitCode {
    match result {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => failure(&error),
    }
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
