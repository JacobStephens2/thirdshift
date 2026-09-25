mod branch;
mod ci;
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
mod run;
mod session;
mod update;
mod worktree;

use std::process::ExitCode;

use issue::IssueUrl;

const HELP: &str = "\
thirdshift turns a GitHub issue into a ready-for-review pull request, unattended.

usage: thirdshift <Issue URL>   Run the factory on the issue, from the clone on the Base branch
       thirdshift update        Update thirdshift to the latest release
       thirdshift version       Print thirdshift's version
       thirdshift help          Print this help
";

fn main() -> ExitCode {
    let arg = std::env::args().nth(1);
    let issue = match arg.as_deref() {
        Some("help" | "--help" | "-h") => {
            print!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Some("version" | "--version" | "-V") => {
            println!("thirdshift {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some("update") => {
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
        Some(arg) => match IssueUrl::parse(arg) {
            Ok(issue) => issue,
            Err(error) => return argument_error(format_args!("{error:#}")),
        },
        None => return argument_error(format_args!("missing Issue URL")),
    };
    if let Err(error) = interrupt::install() {
        progress::step(format_args!("{error:#}"));
        return ExitCode::FAILURE;
    }
    match run::run(&issue) {
        Ok(pr_url) => {
            // Also on stderr, so the outcome shows even when stdout is captured.
            progress::step(format_args!("PR {pr_url} is ready for review"));
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
