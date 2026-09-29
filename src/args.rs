//! The command line: which command, and for a Run, its Issue URL and flags.

use std::mem::discriminant;

use anyhow::{Result, bail};

use crate::issue::IssueUrl;
use crate::notification::NotificationAsk;
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
    /// What `email` or `no-email` asked for, if either was given; without
    /// one, the User config decides.
    pub email: Option<NotificationAsk>,
}

/// Parse the arguments after the program name. `help`, `version`, `update`
/// and `email-test` are commands only as the first argument. Otherwise it is
/// a Run: one Issue URL, with each Run flag at most once, before or after it.
/// `email` may be followed by the address to send the Run notification to.
/// `merge` and `no-merge` contradict each other, as do `email` and `no-email`.
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
    let mut email = None;
    let mut args = args.iter().peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "merge" | "--merge" => ask_once(&mut goal, Goal::Merged, arg, MERGE_FLAGS)?,
            "no-merge" | "--no-merge" => {
                ask_once(&mut goal, Goal::ReadyForReview, arg, MERGE_FLAGS)?
            }
            "email" | "--email" => {
                let to = args.next_if(|next| is_address(next)).cloned();
                ask_once(&mut email, NotificationAsk::Send(to), arg, EMAIL_FLAGS)?
            }
            "no-email" | "--no-email" => {
                ask_once(&mut email, NotificationAsk::Skip, arg, EMAIL_FLAGS)?
            }
            _ => {
                if issue.is_some() {
                    bail!("unexpected argument after the Issue URL: {arg}");
                }
                issue = Some(IssueUrl::parse(arg)?);
            }
        }
    }
    let Some(issue) = issue else {
        bail!("missing Issue URL");
    };
    Ok(Command::Run(RunArgs { issue, goal, email }))
}

const MERGE_FLAGS: &str = "merge and no-merge";
const EMAIL_FLAGS: &str = "email and no-email";

/// Record in `given` what the flag `arg` asked for: the same kind of ask
/// twice is a repeated argument, and a different one contradicts the first,
/// as one of the pair of `flags` can't go with the other.
fn ask_once<T>(given: &mut Option<T>, asked: T, arg: &str, flags: &str) -> Result<()> {
    match given {
        None => *given = Some(asked),
        Some(given) if discriminant(given) == discriminant(&asked) => {
            bail!("repeated argument: {arg}")
        }
        Some(_) => bail!("{flags} can't be used together"),
    }
    Ok(())
}

/// Is `arg`, after `email`, the address to send to? Only if it looks like
/// one, so the Issue URL is never taken for it.
fn is_address(arg: &str) -> bool {
    arg.contains('@') && !arg.starts_with("https://")
}
