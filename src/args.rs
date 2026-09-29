//! The command line: which command, and for a Run, its Issue URL and flags.

use anyhow::{Result, bail};

use crate::issue::IssueUrl;
use crate::run::Goal;

/// What thirdshift was asked to do.
pub enum Command {
    Help,
    Version,
    Update,
    /// `email-test`, with the address it was given, if any.
    EmailTest(Option<String>),
    Run(RunArgs),
}

/// A Run's arguments.
pub struct RunArgs {
    pub issue: IssueUrl,
    /// The goal `merge` or `no-merge` asked for, if either was given; without
    /// one, the User config decides.
    pub goal: Option<Goal>,
}

/// Parse the arguments after the program name. `help`, `version`, `update`
/// and `email-test` are commands only as the first argument. Otherwise it is
/// a Run: one Issue URL, with each Run flag at most once, before or after it.
pub fn parse(args: &[String]) -> Result<Command> {
    match args.first().map(String::as_str) {
        Some("help" | "--help" | "-h") => return Ok(Command::Help),
        Some("version" | "--version" | "-V") => return Ok(Command::Version),
        Some("update") => return Ok(Command::Update),
        Some("email-test") => {
            return match &args[1..] {
                [] => Ok(Command::EmailTest(None)),
                [address] => Ok(Command::EmailTest(Some(address.clone()))),
                [_, extra, ..] => bail!("unexpected argument after the address: {extra}"),
            };
        }
        _ => {}
    }
    let mut issue = None;
    let mut goal = None;
    for arg in args {
        let asked = match arg.as_str() {
            "merge" | "--merge" => Goal::Merged,
            "no-merge" | "--no-merge" => Goal::ReadyForReview,
            _ => {
                if issue.is_some() {
                    bail!("unexpected argument after the Issue URL: {arg}");
                }
                issue = Some(IssueUrl::parse(arg)?);
                continue;
            }
        };
        match goal {
            None => goal = Some(asked),
            Some(given) if given == asked => bail!("repeated argument: {arg}"),
            Some(_) => bail!("merge and no-merge can't be used together"),
        }
    }
    let Some(issue) = issue else {
        bail!("missing Issue URL");
    };
    Ok(Command::Run(RunArgs { issue, goal }))
}
