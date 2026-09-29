mod args;
mod branch;
mod ci;
mod config;
mod failed_run;
mod git;
mod github;
mod interrupt;
mod issue;
mod plugin;
mod poll;
mod preflight;
mod progress;
mod prompt;
#[cfg(test)]
mod prompts_page;
mod run;
mod session;
mod update;
mod worktree;

use std::process::ExitCode;

use args::{Command, RunArgs};
use config::UserConfig;

const HELP: &str = "\
thirdshift turns a GitHub issue into a ready-for-review pull request, or a merged one, unattended.

usage: thirdshift <Issue URL>              Run the factory on the issue, from the clone on the Base branch
       thirdshift merge <Issue URL>        Run the factory on the issue, then merge its pull request
       thirdshift --no-merge <Issue URL>   Run the factory on the issue and leave its pull request for review
       thirdshift update                   Update thirdshift to the latest release
       thirdshift version                  Print thirdshift's version
       thirdshift help                     Print this help

merge and --no-merge go before or after the Issue URL.

The User config, ~/.thirdshift/config.toml, sets defaults for every Run on this machine.
With merge.always set, every Run is a Merge run unless given --no-merge:

    [merge]
    always = true

logs.dir sets where session logs go instead of ~/.thirdshift/logs: an absolute path, or one under ~/.

    [logs]
    dir = \"~/elsewhere/logs\"
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let RunArgs { issue, goal } = match args::parse(&args) {
        Ok(Command::Help) => {
            print!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Ok(Command::Version) => {
            println!("thirdshift {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Ok(Command::Update) => {
            return match update::update() {
                Ok(outcome) => {
                    progress::step(outcome);
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    progress::step(format_args!("{error:#}"));
                    ExitCode::FAILURE
                }
            };
        }
        Ok(Command::Run(run_args)) => run_args,
        Err(error) => return argument_error(format_args!("{error:#}")),
    };
    let config = match UserConfig::load() {
        Ok(config) => config,
        Err(error) => {
            progress::step(format_args!("{error:#}"));
            return ExitCode::FAILURE;
        }
    };
    let goal = goal.unwrap_or(config.default_goal());
    if let Err(error) = interrupt::install() {
        progress::step(format_args!("{error:#}"));
        return ExitCode::FAILURE;
    }
    match run::run(&issue, goal, &config.logs_dir) {
        Ok(pr_url) => {
            // Also on stderr, so the outcome shows even when stdout is captured.
            progress::step(format_args!("PR {pr_url} {}", goal.outcome()));
            println!("{pr_url}");
            ExitCode::SUCCESS
        }
        Err(failed) => {
            progress::step(format_args!("{:#}", failed.error));
            if let Some(log) = failed.log {
                progress::step(format_args!("session log: {}", log.display()));
            }
            if let Some(pr_url) = failed.pr_url {
                println!("{pr_url}");
            }
            ExitCode::FAILURE
        }
    }
}

/// An argument thirdshift can't use: the error, then the help, on stderr.
fn argument_error(error: std::fmt::Arguments) -> ExitCode {
    progress::step(error);
    eprint!("\n{HELP}");
    ExitCode::from(2)
}
