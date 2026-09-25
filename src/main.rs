mod git;
mod github;
mod issue;
mod plugin;
mod progress;
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
    match run::run(&issue_url) {
        Ok(pr_url) => {
            println!("{pr_url}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("thirdshift: {error:#}");
            ExitCode::FAILURE
        }
    }
}
