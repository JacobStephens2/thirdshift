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
#[cfg(test)]
mod prompts_page;
mod run;
mod session;
mod update;
mod worktree;

use std::process::ExitCode;

use issue::IssueUrl;
use run::Mode;

const HELP: &str = "\
thirdshift turns a GitHub issue into a ready-for-review pull request, unattended.

usage: thirdshift <Issue URL>         Run the factory on the issue, from the clone on the Base branch
       thirdshift merge <Issue URL>   Run the factory on the issue, then merge its pull request
       thirdshift update              Update thirdshift to the latest release
       thirdshift version             Print thirdshift's version
       thirdshift help                Print this help
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mode, rest) = match args.first().map(String::as_str) {
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
        Some("merge" | "--merge") => (Mode::Merge, &args[1..]),
        _ => (Mode::Normal, &args[..]),
    };
    let Some((url, after)) = rest.split_first() else {
        return argument_error(format_args!("missing Issue URL"));
    };
    let issue = match IssueUrl::parse(url) {
        Ok(issue) => issue,
        Err(error) => return argument_error(format_args!("{error:#}")),
    };
    if let Some(extra) = after.first() {
        return argument_error(format_args!(
            "unexpected argument after the Issue URL: {extra}"
        ));
    }
    if let Err(error) = interrupt::install() {
        progress::step(format_args!("{error:#}"));
        return ExitCode::FAILURE;
    }
    match run::run(&issue, mode) {
        Ok(pr_url) => {
            // Also on stderr, so the outcome shows even when stdout is captured.
            let outcome = match mode {
                Mode::Normal => "is ready for review",
                Mode::Merge => "is merged",
            };
            progress::step(format_args!("PR {pr_url} {outcome}"));
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
