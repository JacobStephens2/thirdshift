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
mod worktree;

use std::process::ExitCode;

use issue::IssueUrl;

const USAGE: &str = "usage: thirdshift <Issue URL>";

fn main() -> ExitCode {
    let Some(arg) = std::env::args().nth(1) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let issue = match IssueUrl::parse(&arg) {
        Ok(issue) => issue,
        Err(error) => {
            eprintln!("thirdshift: {error:#}\n{USAGE}");
            return ExitCode::from(2);
        }
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
