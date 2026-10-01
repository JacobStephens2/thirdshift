//! The command line: which command, and for a Run, its Issue URL and flags,
//! or for an Architect run, its focus and flags.

use std::mem::discriminant;
use std::num::NonZeroUsize;

use anyhow::{Context, Result, bail};

use crate::base_fix::BaseFixAsk;
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
    Architect(ArchitectArgs),
    Run(RunArgs),
}

/// The flag that stops an Architect run once its plan is published and
/// marked ready.
const PLAN_ONLY: &str = "--plan-only";

/// An Architect run's arguments.
#[derive(Debug, PartialEq, Eq)]
pub struct ArchitectArgs {
    /// The free text that points the Architecture review at an area, if
    /// given.
    pub focus: Option<String>,
    /// What `email` or `no-email` asked for, if either was given; without
    /// one, the User config decides.
    pub email: Option<NotificationAsk>,
    /// The flags for the Spec run or Run the plan is dispatched as, or none
    /// with [`PLAN_ONLY`], which dispatches nothing.
    pub dispatch: Option<DispatchArgs>,
}

/// What an Architect run's flags ask of the Spec run or Run it dispatches
/// its plan as, each meaning what it does in [`RunArgs`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DispatchArgs {
    pub goal: Option<Goal>,
    pub parallel: Option<NonZeroUsize>,
    pub base_fix: Option<BaseFixAsk>,
}

/// The word that lets a Run start a Base fix, which a Spec run passes on to
/// each Ticket's Run.
pub const BASE_FIX: &str = "base-fix";

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
    /// What `base-fix` or `no-base-fix` asked for, if either was given;
    /// without one, the User config decides.
    pub base_fix: Option<BaseFixAsk>,
    /// What the Run is, if another thirdshift started it, given with
    /// [`SPEC_BRANCH`] or [`BASE_FIX_INTO`].
    pub child: Option<Kind>,
}

/// Parse the arguments after the program name. `help`, `version`, `update`,
/// `setup`, `email-test` and `architect` are commands only as the first
/// argument. Otherwise it is a Run: one Issue URL, with each Run flag at most once,
/// before or after it.
/// `email` may be followed by the address to send the Run notification to,
/// and `parallel` must be followed by a whole number from 1 up.
/// `merge` and `no-merge` contradict each other, as do `email` and `no-email`,
/// and `base-fix` and `no-base-fix`.
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
        Some("architect") => return parse_architect(&args[1..]).map(Command::Architect),
        _ => {}
    }
    let mut issue = None;
    let mut goal = None;
    let mut email = None;
    let mut parallel = None;
    let mut base_fix = None;
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
            "parallel" | "--parallel" => ask_parallel(&mut parallel, arg, args.next())?,
            BASE_FIX | "--base-fix" => {
                ask_once(&mut base_fix, BaseFixAsk::Allow, arg, BASE_FIX_FLAGS)?
            }
            "no-base-fix" | "--no-base-fix" => {
                ask_once(&mut base_fix, BaseFixAsk::Forbid, arg, BASE_FIX_FLAGS)?
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

/// Parse the arguments after `architect`: at most one focus, and its flags,
/// each at most once, in any order. [`PLAN_ONLY`] dispatches nothing, so the
/// flags for the dispatched run, `merge`, `no-merge`, `parallel`, `base-fix`
/// and `no-base-fix`, which a Run takes too, can't go with it. `email` and `no-email` are for the
/// Architect run's own Run notification, so they can, and `email` takes an
/// address as it does for a Run. None of these is ever the focus, and any
/// other argument that starts with a dash is unexpected rather than a focus.
fn parse_architect(args: &[String]) -> Result<ArchitectArgs> {
    let mut focus = None;
    let mut plan_only = false;
    let mut email = None;
    let mut dispatch = DispatchArgs::default();
    let mut args = args.iter().peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            PLAN_ONLY => {
                if plan_only {
                    bail!("repeated argument: {arg}");
                }
                plan_only = true;
            }
            "merge" | "--merge" => ask_once(&mut dispatch.goal, Goal::Merged, arg, MERGE_FLAGS)?,
            "no-merge" | "--no-merge" => {
                ask_once(&mut dispatch.goal, Goal::ReadyForReview, arg, MERGE_FLAGS)?
            }
            "email" | "--email" => {
                let to = args.next_if(|next| is_address(next)).cloned();
                ask_once(&mut email, NotificationAsk::Send(to), arg, EMAIL_FLAGS)?
            }
            "no-email" | "--no-email" => {
                ask_once(&mut email, NotificationAsk::Skip, arg, EMAIL_FLAGS)?
            }
            "parallel" | "--parallel" => ask_parallel(&mut dispatch.parallel, arg, args.next())?,
            BASE_FIX | "--base-fix" => ask_once(
                &mut dispatch.base_fix,
                BaseFixAsk::Allow,
                arg,
                BASE_FIX_FLAGS,
            )?,
            "no-base-fix" | "--no-base-fix" => ask_once(
                &mut dispatch.base_fix,
                BaseFixAsk::Forbid,
                arg,
                BASE_FIX_FLAGS,
            )?,
            _ if arg.starts_with('-') => bail!("unexpected argument after architect: {arg}"),
            _ if focus.is_some() => bail!("unexpected argument after the focus: {arg}"),
            _ if arg.trim().is_empty() => bail!("the focus is empty"),
            _ => focus = Some(arg.clone()),
        }
    }
    if !plan_only {
        return Ok(ArchitectArgs {
            focus,
            email,
            dispatch: Some(dispatch),
        });
    }
    if dispatch != DispatchArgs::default() {
        bail!(
            "merge, no-merge, parallel, base-fix and no-base-fix can't be used with \
             {PLAN_ONLY}: it dispatches no run for them to apply to"
        );
    }
    Ok(ArchitectArgs {
        focus,
        email,
        dispatch: None,
    })
}

const MERGE_FLAGS: &str = "merge and no-merge";
const EMAIL_FLAGS: &str = "email and no-email";
const BASE_FIX_FLAGS: &str = "base-fix and no-base-fix";

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

/// Record in `parallel` the number `n` that follows the flag `arg`: a whole
/// number from 1 up, given once.
fn ask_parallel(parallel: &mut Option<NonZeroUsize>, arg: &str, n: Option<&String>) -> Result<()> {
    if parallel.is_some() {
        bail!("repeated argument: {arg}");
    }
    let Some(n) = n.and_then(|n| n.parse::<NonZeroUsize>().ok()) else {
        let given = n.map(|n| format!(", not {n}")).unwrap_or_default();
        bail!("{arg} must be followed by a whole number from 1 up{given}");
    };
    *parallel = Some(n);
    Ok(())
}

/// Is `arg`, after `email`, the address to send to? Only if it looks like
/// one, so the Issue URL is never taken for it, nor is an Architect run's
/// focus unless it has an `@`.
fn is_address(arg: &str) -> bool {
    arg.contains('@') && !arg.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://github.com/acme/widgets/issues/7";

    fn parse_strs(args: &[&str]) -> Result<Command> {
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        parse(&args)
    }

    /// The Run `args` parse to.
    fn run_args(args: &[&str]) -> RunArgs {
        match parse_strs(args) {
            Ok(Command::Run(run_args)) => run_args,
            Ok(_) => panic!("{args:?}: not a Run"),
            Err(error) => panic!("{args:?}: {error:#}"),
        }
    }

    /// The Architect run `args` parse to.
    fn architect_args(args: &[&str]) -> ArchitectArgs {
        match parse_strs(args) {
            Ok(Command::Architect(architect_args)) => architect_args,
            Ok(_) => panic!("{args:?}: not an Architect run"),
            Err(error) => panic!("{args:?}: {error:#}"),
        }
    }

    /// The error `args` are rejected with.
    fn rejection(args: &[&str]) -> String {
        match parse_strs(args) {
            Ok(_) => panic!("{args:?}: not rejected"),
            Err(error) => format!("{error:#}"),
        }
    }

    #[test]
    fn architect_takes_plan_only_with_or_without_a_focus_on_either_side_of_it() {
        let focus = || Some("the Spec run".to_string());
        for (args, expected) in [
            (vec!["architect", "--plan-only"], None),
            (vec!["architect", "the Spec run", "--plan-only"], focus()),
            (vec!["architect", "--plan-only", "the Spec run"], focus()),
        ] {
            let architect_args = architect_args(&args);
            assert_eq!(architect_args.focus, expected, "{args:?}");
            assert_eq!(architect_args.dispatch, None, "{args:?}");
        }
    }

    #[test]
    fn architect_without_plan_only_dispatches_its_plan_with_or_without_a_focus() {
        for (args, focus) in [
            (vec!["architect"], None),
            (
                vec!["architect", "the Spec run"],
                Some("the Spec run".to_string()),
            ),
        ] {
            let architect_args = architect_args(&args);
            assert_eq!(architect_args.focus, focus, "{args:?}");
            assert_eq!(
                architect_args.dispatch,
                Some(DispatchArgs::default()),
                "{args:?}"
            );
        }
    }

    #[test]
    fn architect_takes_the_merge_and_parallel_flags_for_the_run_it_dispatches() {
        let two = NonZeroUsize::new(2);
        for (args, goal, parallel) in [
            (vec!["architect", "merge"], Some(Goal::Merged), None),
            (vec!["architect", "--merge"], Some(Goal::Merged), None),
            (
                vec!["architect", "no-merge"],
                Some(Goal::ReadyForReview),
                None,
            ),
            (
                vec!["architect", "--no-merge"],
                Some(Goal::ReadyForReview),
                None,
            ),
            (vec!["architect", "parallel", "2"], None, two),
            (
                vec!["architect", "--parallel", "2", "the Spec run", "merge"],
                Some(Goal::Merged),
                two,
            ),
        ] {
            assert_eq!(
                architect_args(&args).dispatch,
                Some(DispatchArgs {
                    goal,
                    parallel,
                    base_fix: None
                }),
                "{args:?}"
            );
        }
        let args = ["architect", "--parallel", "2", "the Spec run", "merge"];
        assert_eq!(architect_args(&args).focus.as_deref(), Some("the Spec run"));
    }

    #[test]
    fn architect_takes_the_base_fix_flags_for_the_run_it_dispatches_and_never_as_the_focus() {
        for (args, base_fix) in [
            (vec!["architect"], None),
            (vec!["architect", "base-fix"], Some(BaseFixAsk::Allow)),
            (vec!["architect", "--base-fix"], Some(BaseFixAsk::Allow)),
            (vec!["architect", "no-base-fix"], Some(BaseFixAsk::Forbid)),
            (
                vec!["architect", "the Spec run", "--no-base-fix", "merge"],
                Some(BaseFixAsk::Forbid),
            ),
        ] {
            let architect_args = architect_args(&args);
            let dispatch = architect_args.dispatch.expect("a dispatch");
            assert_eq!(dispatch.base_fix, base_fix, "{args:?}");
            let focus = args.contains(&"the Spec run").then_some("the Spec run");
            assert_eq!(architect_args.focus.as_deref(), focus, "{args:?}");
        }
    }

    #[test]
    fn architect_takes_the_email_flags_with_or_without_plan_only_and_never_as_the_focus() {
        let bare = || Some(NotificationAsk::Send(None));
        let to_me = || Some(NotificationAsk::Send(Some("me@example.com".to_string())));
        let focus = || Some("the Spec run".to_string());
        for (args, email, focus) in [
            (vec!["architect"], None, None),
            (vec!["architect", "email"], bare(), None),
            (vec!["architect", "--email", "--plan-only"], bare(), None),
            (vec!["architect", "email", "the Spec run"], bare(), focus()),
            (vec!["architect", "email", "merge"], bare(), None),
            (vec!["architect", "email", "me@example.com"], to_me(), None),
            (
                vec!["architect", "the Spec run", "--email", "me@example.com"],
                to_me(),
                focus(),
            ),
            (
                vec!["architect", "--plan-only", "no-email"],
                Some(NotificationAsk::Skip),
                None,
            ),
            (
                vec!["architect", "--no-email", "the Spec run"],
                Some(NotificationAsk::Skip),
                focus(),
            ),
        ] {
            let architect_args = architect_args(&args);
            assert_eq!(architect_args.email, email, "{args:?}");
            assert_eq!(architect_args.focus, focus, "{args:?}");
        }
    }

    #[test]
    fn architect_rejects_contradictory_and_repeated_email_flags() {
        for (args, error) in [
            (
                vec!["architect", "email", "--no-email"],
                "email and no-email can't be used together",
            ),
            (
                vec!["architect", "no-email", "--plan-only", "--email"],
                "email and no-email can't be used together",
            ),
            (
                vec!["architect", "--email", "me@example.com", "email"],
                "repeated argument: email",
            ),
        ] {
            assert_eq!(rejection(&args), error, "{args:?}");
        }
    }

    #[test]
    fn architect_rejects_contradictory_repeated_and_malformed_dispatch_flags() {
        for (args, error) in [
            (
                vec!["architect", "merge", "--no-merge"],
                "merge and no-merge can't be used together",
            ),
            (
                vec!["architect", "no-merge", "the Spec run", "merge"],
                "merge and no-merge can't be used together",
            ),
            (
                vec!["architect", "merge", "--merge"],
                "repeated argument: --merge",
            ),
            (
                vec!["architect", "parallel", "2", "parallel", "3"],
                "repeated argument: parallel",
            ),
            (
                vec!["architect", "base-fix", "--no-base-fix"],
                "base-fix and no-base-fix can't be used together",
            ),
            (
                vec!["architect", "no-base-fix", "the Spec run", "base-fix"],
                "base-fix and no-base-fix can't be used together",
            ),
            (
                vec!["architect", "base-fix", "--base-fix"],
                "repeated argument: --base-fix",
            ),
            (
                vec!["architect", "parallel"],
                "parallel must be followed by a whole number from 1 up",
            ),
            (
                vec!["architect", "parallel", "the Spec run"],
                "parallel must be followed by a whole number from 1 up, not the Spec run",
            ),
        ] {
            assert_eq!(rejection(&args), error, "{args:?}");
        }
    }

    #[test]
    fn architect_rejects_the_dispatch_flags_with_plan_only_which_dispatches_nothing() {
        for args in [
            vec!["architect", "--plan-only", "merge"],
            vec!["architect", "--no-merge", "--plan-only"],
            vec!["architect", "the Spec run", "parallel", "2", "--plan-only"],
            vec!["architect", "--plan-only", "base-fix"],
            vec!["architect", "--no-base-fix", "--plan-only"],
        ] {
            assert_eq!(
                rejection(&args),
                "merge, no-merge, parallel, base-fix and no-base-fix can't be used with \
                 --plan-only: it dispatches no run for them to apply to",
                "{args:?}"
            );
        }
    }

    #[test]
    fn architect_rejects_stray_arguments() {
        for (args, error) in [
            (
                vec!["architect", "the Spec run", "the Run", "--plan-only"],
                "unexpected argument after the focus: the Run",
            ),
            (
                vec!["architect", "--plan-only", "--plan-only"],
                "repeated argument: --plan-only",
            ),
            (
                vec!["architect", "--verbose", "--plan-only"],
                "unexpected argument after architect: --verbose",
            ),
            (vec!["architect", " ", "--plan-only"], "the focus is empty"),
        ] {
            assert_eq!(rejection(&args), error, "{args:?}");
        }
    }

    #[test]
    fn architect_is_a_command_only_as_the_first_argument() {
        for (args, error) in [
            (
                vec![URL, "architect"],
                "unexpected argument after the Issue URL: architect",
            ),
            (
                vec!["merge", "architect", "--plan-only"],
                "not a GitHub issue URL: architect",
            ),
        ] {
            assert_eq!(rejection(&args), error, "{args:?}");
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
