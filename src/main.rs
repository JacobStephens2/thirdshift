mod failure;
mod git;
mod github;
mod interrupt;
mod issue;
mod plugin;
mod prompt;
mod run;
mod session;
mod worktree;

use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(issue_url) = std::env::args().nth(1) else {
        eprintln!("usage: thirdshift <Issue URL>");
        return ExitCode::from(2);
    };
    if let Err(error) = interrupt::install() {
        eprintln!("thirdshift: {error:#}");
        return ExitCode::FAILURE;
    }
    match run::run(&issue_url) {
        Ok(pr_url) => {
            println!("{pr_url}");
            ExitCode::SUCCESS
        }
        Err(failure) => {
            eprintln!("thirdshift: {:#}", failure.error);
            if let Some(log) = failure.log {
                eprintln!("thirdshift: session log: {}", log.display());
            }
            if let Some(pr_url) = failure.pr_url {
                println!("{pr_url}");
            }
            ExitCode::FAILURE
        }
    }
}
