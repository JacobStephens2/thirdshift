mod branch;
mod git;
mod github;
mod issue;
mod plugin;
mod preflight;
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
    match run::run(&issue) {
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
