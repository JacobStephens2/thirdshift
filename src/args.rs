//! The command line: which command, and for a Run, its Issue URL and flags.

use std::mem::discriminant;
use std::num::NonZeroUsize;

use anyhow::{Context, Result, bail};

use crate::child_run::Kind;
use crate::issue::IssueUrl;
use crate::notification::NotificationAsk;
use crate::run::Goal;

/// What thirdshift was asked to do.
pub enum Command {
    Help,
    Version,
    Update,
    Setup,
    /// `email-test`, with the address it was given, if any.
    EmailTest(Option<String>),
    Run(RunArgs),
}

/// The hidden argument a Spec run starts each Ticket's Run with, followed by
/// the Spec branch: it makes the Run a [`Kind::Ticket`]. Not in help.
pub const SPEC_BRANCH: &str = "--spec-branch";

/// The hidden argument a Run starts its Base fix with, followed by the Run's
/// Base branch: it makes the Run a [`Kind::BaseFix`]. Not in help.
pub const BASE_FIX_INTO: &str = "--base-fix-into";

/// A Run's arguments.
pub struct RunArgs {
    pub issue: IssueUrl,
    /// The goal `merge` or `no-merge` asked for, if either was given; without
    /// one, the User config decides.
    pub goal: Option<Goal>,
    /// What `email` or `no-email` asked for, if either was given; without
    /// one, the User config decides.
    pub email: Option<NotificationAsk>,
    /// How many Tickets a Spec run runs at once, if `parallel <n>` was
    /// given; without it, the User config decides.
    pub parallel: Option<NonZeroUsize>,
    /// Whether `base-fix` was given: the Run may start a Base fix.
    pub base_fix: bool,
    /// What the Run is, if another thirdshift started it, given with
    /// [`SPEC_BRANCH`] or [`BASE_FIX_INTO`].
    pub child: Option<Kind>,
}

/// Parse the arguments after the program name. `help`, `version`, `update`,
/// `setup` and `email-test` are commands only as the first argument.
/// Otherwise it is a Run: one Issue URL, with each Run flag at most once,
/// before or after it.
/// `email` may be followed by the address to send the Run notification to,
/// and `parallel` must be followed by a whole number from 1 up.
/// `merge` and `no-merge` contradict each other, as do `email` and `no-email`.
pub fn parse(args: &[String]) -> Result<Command> {
    match args.first().map(String::as_str) {
        Some("help" | "--help" | "-h") => return Ok(Command::Help),
        Some("version" | "--version" | "-V") => return Ok(Command::Version),
        Some("update") => return Ok(Command::Update),
        Some("setup") => {
            return match &args[1..] {
                [] => Ok(Command::Setup),
                [extra, ..] => bail!("unexpected argument after setup: {extra}"),
            };
        }
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
    let mut parallel = None;
    let mut base_fix = false;
    let mut child = None;
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
            "parallel" | "--parallel" => {
                if parallel.is_some() {
                    bail!("repeated argument: {arg}");
                }
                let n = args.next();
                let Some(n) = n.and_then(|n| n.parse::<NonZeroUsize>().ok()) else {
                    let given = n.map(|n| format!(", not {n}")).unwrap_or_default();
                    bail!("{arg} must be followed by a whole number from 1 up{given}");
                };
                parallel = Some(n);
            }
            "base-fix" | "--base-fix" => {
                if base_fix {
                    bail!("repeated argument: {arg}");
                }
                base_fix = true;
            }
            SPEC_BRANCH | BASE_FIX_INTO => {
                if child.is_some() {
                    bail!("repeated argument: {arg}");
                }
                let base = args.next().context("missing Base branch")?.clone();
                child = Some(if arg == SPEC_BRANCH {
                    Kind::Ticket { spec_branch: base }
                } else {
                    Kind::BaseFix { base }
                });
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
    Ok(Command::Run(RunArgs {
        issue,
        goal,
        email,
        parallel,
        base_fix,
        child,
    }))
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

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://github.com/acme/widgets/issues/7";

    /// The Run `args` parse to.
    fn run_args(args: &[&str]) -> RunArgs {
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        match parse(&args) {
            Ok(Command::Run(run_args)) => run_args,
            Ok(_) => panic!("{args:?}: not a Run"),
            Err(error) => panic!("{args:?}: {error:#}"),
        }
    }

    #[test]
    fn a_bare_flag_asks_for_email_to_and_an_address_after_the_flag_is_the_one_asked_for() {
        let to_flag_address = || NotificationAsk::Send(Some("flag@example.com".to_string()));
        for (args, asked) in [
            (vec![URL, "--email"], NotificationAsk::Send(None)),
            (vec![URL, "email"], NotificationAsk::Send(None)),
            (vec![URL, "--email", "flag@example.com"], to_flag_address()),
            (vec![URL, "email", "flag@example.com"], to_flag_address()),
            (vec!["--email", "flag@example.com", URL], to_flag_address()),
        ] {
            let run_args = run_args(&args);
            assert_eq!(run_args.email, Some(asked), "{args:?}");
            assert_eq!(run_args.issue.url, URL, "{args:?}");
        }
    }

    #[test]
    fn the_flag_goes_anywhere_around_a_merge_goal_and_never_takes_the_issue_url() {
        let bare = || NotificationAsk::Send(None);
        let to_me = || NotificationAsk::Send(Some("me@example.com".to_string()));
        for (args, asked, goal) in [
            (vec!["--email", URL], bare(), None),
            (vec!["email", URL, "merge"], bare(), Some(Goal::Merged)),
            (vec!["merge", "--email", URL], bare(), Some(Goal::Merged)),
            (
                vec!["--email", "me@example.com", URL, "merge"],
                to_me(),
                Some(Goal::Merged),
            ),
            (vec![URL, "merge", "email"], bare(), Some(Goal::Merged)),
            (vec![URL, "--email", "merge"], bare(), Some(Goal::Merged)),
        ] {
            let run_args = run_args(&args);
            assert_eq!(run_args.issue.url, URL, "{args:?}");
            assert_eq!(run_args.email, Some(asked), "{args:?}");
            assert_eq!(run_args.goal, goal, "{args:?}");
        }
    }

    #[test]
    fn no_email_asks_for_no_notification_with_or_without_dashes() {
        for flag in ["no-email", "--no-email"] {
            assert_eq!(
                run_args(&[flag, URL]).email,
                Some(NotificationAsk::Skip),
                "{flag}"
            );
        }
    }

    #[test]
    fn parallel_takes_the_number_after_it_with_or_without_dashes() {
        for flag in ["parallel", "--parallel"] {
            for args in [[flag, "1", URL], [URL, flag, "1"]] {
                assert_eq!(run_args(&args).parallel, NonZeroUsize::new(1), "{args:?}");
            }
        }
    }
}
