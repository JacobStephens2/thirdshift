//! The command line: which command, and for a Run, its Issue URL and flags,
//! for an Architect run, its focus and flags, or for a Pickup run, its flags.

use std::iter::Peekable;
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
    Pickup(PickupArgs),
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
    /// The Base branch `base <branch>` named, if given; without it, the
    /// branch checked out in the Launch directory is the Base branch.
    pub base: Option<String>,
    /// What `email` or `no-email` asked for, if either was given; without
    /// one, the User config decides.
    pub email: Option<NotificationAsk>,
    /// The flags for the Spec run or Run the plan is dispatched as, or none
    /// with [`PLAN_ONLY`], which dispatches nothing.
    pub dispatch: Option<DispatchArgs>,
}

/// A Pickup run's arguments.
#[derive(Debug, PartialEq, Eq)]
pub struct PickupArgs {
    /// The Base branch `base <branch>` named, if given; without it, the
    /// branch checked out in the Launch directory is the Base branch.
    pub base: Option<String>,
    /// What `email` or `no-email` asked for, if either was given; without
    /// one, the User config decides.
    pub email: Option<NotificationAsk>,
    /// The flags for the Spec run or Run the Ready issue is dispatched as.
    pub dispatch: DispatchArgs,
}

/// What the flags of an Architect run or a Pickup run ask of the Spec run or
/// Run it dispatches, each meaning what it does in [`RunArgs`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DispatchArgs {
    pub goal: Option<Goal>,
    pub parallel: Option<NonZeroUsize>,
    pub base_fix: Option<BaseFixAsk>,
}

/// The word that lets a Run start a Base fix, which a Spec run passes on to
/// each Ticket's Run.
pub const BASE_FIX: &str = "base-fix";

/// The hidden argument a Spec run that nobody decided about a Base fix for
/// starts each Ticket's Run with, followed by the command that starts the
/// Spec run again with one allowed, for the Ticket's Run to offer: it asks
/// [`BaseFixAsk::Undecided`]. Not in help.
pub const OFFER_BASE_FIX: &str = "--offer-base-fix";

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
/// `setup`, `email-test`, `architect` and `pickup` are commands only as the
/// first argument. Otherwise it is a Run: one Issue URL, with each Run flag at
/// most once, before or after it.
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
        Some("pickup") => return parse_pickup(&args[1..]).map(Command::Pickup),
        _ => {}
    }
    let mut issue = None;
    let mut flags = RunFlags::default();
    let mut child = None;
    let mut args = args.iter().peekable();
    while let Some(arg) = args.next() {
        if flags.take(arg, &mut args)? {
            continue;
        }
        match arg.as_str() {
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
            OFFER_BASE_FIX => {
                let retry = args.next().context("missing command to offer")?.clone();
                let undecided = BaseFixAsk::Undecided { retry };
                ask_once(&mut flags.base_fix, undecided, arg, BASE_FIX_FLAGS)?;
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
        goal: flags.goal,
        email: flags.email,
        parallel: flags.parallel,
        base_fix: flags.base_fix,
        child,
    }))
}

/// The command that starts the Run on `issue` again as its command asked for
/// it, with `goal`, `email` and `parallel` as [`RunArgs`] has them, and with
/// `base-fix` added.
pub fn retry_with_base_fix(
    issue: &IssueUrl,
    goal: Option<Goal>,
    email: Option<&NotificationAsk>,
    parallel: Option<NonZeroUsize>,
) -> String {
    let mut command = format!("thirdshift {}", issue.url);
    match goal {
        Some(Goal::Merged) => command += " merge",
        Some(Goal::ReadyForReview) => command += " --no-merge",
        None => {}
    }
    match email {
        Some(NotificationAsk::Send(Some(to))) => command += &format!(" --email {to}"),
        Some(NotificationAsk::Send(None)) => command += " --email",
        Some(NotificationAsk::Skip) => command += " --no-email",
        None => {}
    }
    if let Some(parallel) = parallel {
        command += &format!(" parallel {parallel}");
    }
    command + " " + BASE_FIX
}

/// Parse the arguments after `architect`: at most one focus, and its flags,
/// each at most once, in any order. `base` must be followed by the Base
/// branch. [`PLAN_ONLY`] dispatches nothing, so the
/// flags for the dispatched run, `merge`, `no-merge`, `parallel`, `base-fix`
/// and `no-base-fix`, can't go with it. `email` and `no-email` are for the
/// Architect run's own Run notification, and `base` is for the Architecture
/// review too, so they can. None of a Run's flags, nor `base`,
/// is ever the focus, and any other argument that starts with a dash is
/// unexpected rather than a focus.
fn parse_architect(args: &[String]) -> Result<ArchitectArgs> {
    let mut focus = None;
    let mut base = None;
    let mut plan_only = false;
    let mut flags = RunFlags::default();
    let mut args = args.iter().peekable();
    while let Some(arg) = args.next() {
        if flags.take(arg, &mut args)? {
            continue;
        }
        match arg.as_str() {
            PLAN_ONLY => {
                if plan_only {
                    bail!("repeated argument: {arg}");
                }
                plan_only = true;
            }
            "base" | "--base" => ask_base(&mut base, arg, args.next())?,
            _ if arg.starts_with('-') => bail!("unexpected argument after architect: {arg}"),
            _ if focus.is_some() => bail!("unexpected argument after the focus: {arg}"),
            _ if arg.trim().is_empty() => bail!("the focus is empty"),
            _ => focus = Some(arg.clone()),
        }
    }
    let (email, dispatch) = flags.for_dispatch();
    if plan_only && dispatch != DispatchArgs::default() {
        bail!(
            "merge, no-merge, parallel, base-fix and no-base-fix can't be used with \
             {PLAN_ONLY}: it dispatches no run for them to apply to"
        );
    }
    Ok(ArchitectArgs {
        focus,
        base,
        email,
        dispatch: (!plan_only).then_some(dispatch),
    })
}

/// Parse the arguments after `pickup`: its flags, each at most once, in any
/// order, and nothing else. `base` must be followed by the Base branch.
fn parse_pickup(args: &[String]) -> Result<PickupArgs> {
    let mut base = None;
    let mut flags = RunFlags::default();
    let mut args = args.iter().peekable();
    while let Some(arg) = args.next() {
        if flags.take(arg, &mut args)? {
            continue;
        }
        match arg.as_str() {
            "base" | "--base" => ask_base(&mut base, arg, args.next())?,
            _ => bail!("unexpected argument after pickup: {arg}"),
        }
    }
    let (email, dispatch) = flags.for_dispatch();
    Ok(PickupArgs {
        base,
        email,
        dispatch,
    })
}

/// The flags a Run, an Architect run and a Pickup run all take, as given so
/// far: each field is what [`RunArgs`] says of the one it becomes.
#[derive(Default)]
struct RunFlags {
    goal: Option<Goal>,
    email: Option<NotificationAsk>,
    parallel: Option<NonZeroUsize>,
    base_fix: Option<BaseFixAsk>,
}

impl RunFlags {
    /// The flags as an Architect run or a Pickup run takes them: what was
    /// asked about its Run notification, and the flags for the run it
    /// dispatches.
    fn for_dispatch(self) -> (Option<NotificationAsk>, DispatchArgs) {
        let dispatch = DispatchArgs {
            goal: self.goal,
            parallel: self.parallel,
            base_fix: self.base_fix,
        };
        (self.email, dispatch)
    }

    /// Record what `arg` asks for, if it is one of these flags, with or
    /// without its dashes, taking from `rest` the address after `email`, if
    /// one is there, and the number after `parallel`. False, taking nothing,
    /// if `arg` is none of them.
    fn take<'a>(
        &mut self,
        arg: &str,
        rest: &mut Peekable<impl Iterator<Item = &'a String>>,
    ) -> Result<bool> {
        match arg {
            "merge" | "--merge" => ask_once(&mut self.goal, Goal::Merged, arg, MERGE_FLAGS)?,
            "no-merge" | "--no-merge" => {
                ask_once(&mut self.goal, Goal::ReadyForReview, arg, MERGE_FLAGS)?
            }
            "email" | "--email" => {
                let to = rest.next_if(|next| is_address(next)).cloned();
                ask_once(&mut self.email, NotificationAsk::Send(to), arg, EMAIL_FLAGS)?
            }
            "no-email" | "--no-email" => {
                ask_once(&mut self.email, NotificationAsk::Skip, arg, EMAIL_FLAGS)?
            }
            "parallel" | "--parallel" => ask_parallel(&mut self.parallel, arg, rest.next())?,
            BASE_FIX | "--base-fix" => {
                ask_once(&mut self.base_fix, BaseFixAsk::Allow, arg, BASE_FIX_FLAGS)?
            }
            "no-base-fix" | "--no-base-fix" => {
                ask_once(&mut self.base_fix, BaseFixAsk::Forbid, arg, BASE_FIX_FLAGS)?
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
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

/// Record in `base` the `branch` that follows the flag `arg`, given once. No
/// branch's name starts with a dash, so a flag there is not taken for one.
fn ask_base(base: &mut Option<String>, arg: &str, branch: Option<&String>) -> Result<()> {
    if base.is_some() {
        bail!("repeated argument: {arg}");
    }
    match branch {
        Some(branch) if branch.starts_with('-') => {
            bail!("{arg} must be followed by a branch, not {branch}")
        }
        Some(branch) if !branch.trim().is_empty() => *base = Some(branch.clone()),
        _ => bail!("{arg} must be followed by a branch"),
    }
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
    fn architect_takes_base_and_the_branch_after_it_before_or_after_the_focus_and_other_flags() {
        let focus = || Some("the Spec run".to_string());
        for (args, focus) in [
            (vec!["architect", "base", "develop"], None),
            (vec!["architect", "--base", "develop"], None),
            (
                vec!["architect", "base", "develop", "the Spec run"],
                focus(),
            ),
            (
                vec!["architect", "the Spec run", "merge", "--base", "develop"],
                focus(),
            ),
            (
                vec!["architect", "email", "base", "develop", "parallel", "2"],
                None,
            ),
        ] {
            let architect_args = architect_args(&args);
            assert_eq!(architect_args.base.as_deref(), Some("develop"), "{args:?}");
            assert_eq!(architect_args.focus, focus, "{args:?}");
        }
        assert_eq!(architect_args(&["architect", "the Spec run"]).base, None);
    }

    #[test]
    fn architect_takes_base_with_plan_only() {
        for args in [
            ["architect", "base", "develop", "--plan-only"],
            ["architect", "--plan-only", "--base", "develop"],
        ] {
            let architect_args = architect_args(&args);
            assert_eq!(architect_args.base.as_deref(), Some("develop"), "{args:?}");
            assert_eq!(architect_args.dispatch, None, "{args:?}");
        }
    }

    #[test]
    fn architect_rejects_base_without_a_branch_and_base_given_twice() {
        for (args, error) in [
            (
                vec!["architect", "base"],
                "base must be followed by a branch",
            ),
            (
                vec!["architect", "the Spec run", "--base"],
                "--base must be followed by a branch",
            ),
            (
                vec!["architect", "base", "--plan-only"],
                "base must be followed by a branch, not --plan-only",
            ),
            (
                vec!["architect", "base", " "],
                "base must be followed by a branch",
            ),
            (
                vec!["architect", "base", "develop", "base", "main"],
                "repeated argument: base",
            ),
            (
                vec![
                    "architect",
                    "base",
                    "develop",
                    "--plan-only",
                    "--base",
                    "develop",
                ],
                "repeated argument: --base",
            ),
        ] {
            assert_eq!(rejection(&args), error, "{args:?}");
        }
    }

    #[test]
    fn base_is_not_a_flag_of_a_run() {
        for (args, error) in [
            (
                vec![URL, "base", "develop"],
                "unexpected argument after the Issue URL: base",
            ),
            (
                vec!["--base", "develop", URL],
                "not a GitHub issue URL: --base",
            ),
        ] {
            assert_eq!(rejection(&args), error, "{args:?}");
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

    /// The Pickup run `args` parse to.
    fn pickup_args(args: &[&str]) -> PickupArgs {
        match parse_strs(args) {
            Ok(Command::Pickup(pickup_args)) => pickup_args,
            Ok(_) => panic!("{args:?}: not a Pickup run"),
            Err(error) => panic!("{args:?}: {error:#}"),
        }
    }

    #[test]
    fn pickup_takes_each_of_its_flags_with_or_without_dashes_in_any_order() {
        let none = PickupArgs {
            base: None,
            email: None,
            dispatch: DispatchArgs::default(),
        };
        assert_eq!(pickup_args(&["pickup"]), none);
        let all = PickupArgs {
            base: Some("develop".to_string()),
            email: Some(NotificationAsk::Send(Some("me@example.com".to_string()))),
            dispatch: DispatchArgs {
                goal: Some(Goal::Merged),
                parallel: NonZeroUsize::new(2),
                base_fix: Some(BaseFixAsk::Allow),
            },
        };
        for args in [
            vec![
                "pickup",
                "merge",
                "parallel",
                "2",
                "base-fix",
                "email",
                "me@example.com",
                "base",
                "develop",
            ],
            vec![
                "pickup",
                "--base",
                "develop",
                "--email",
                "me@example.com",
                "--base-fix",
                "--parallel",
                "2",
                "--merge",
            ],
        ] {
            assert_eq!(pickup_args(&args), all, "{args:?}");
        }
        let cautious = PickupArgs {
            base: None,
            email: Some(NotificationAsk::Skip),
            dispatch: DispatchArgs {
                goal: Some(Goal::ReadyForReview),
                parallel: None,
                base_fix: Some(BaseFixAsk::Forbid),
            },
        };
        for args in [
            ["pickup", "no-merge", "no-base-fix", "no-email"],
            ["pickup", "--no-email", "--no-merge", "--no-base-fix"],
        ] {
            assert_eq!(pickup_args(&args), cautious, "{args:?}");
        }
        let bare_email = pickup_args(&["pickup", "email", "merge"]);
        assert_eq!(bare_email.email, Some(NotificationAsk::Send(None)));
        assert_eq!(bare_email.dispatch.goal, Some(Goal::Merged));
    }

    #[test]
    fn pickup_rejects_a_focus_plan_only_an_issue_url_and_any_other_argument() {
        for (args, error) in [
            (
                vec!["pickup", "the Spec run"],
                "unexpected argument after pickup: the Spec run",
            ),
            (
                vec!["pickup", "merge", "--plan-only"],
                "unexpected argument after pickup: --plan-only",
            ),
            (
                vec!["pickup", URL],
                "unexpected argument after pickup: https://github.com/acme/widgets/issues/7",
            ),
            (
                vec!["pickup", "--verbose"],
                "unexpected argument after pickup: --verbose",
            ),
            (
                vec!["pickup", "--spec-branch", "issue-7"],
                "unexpected argument after pickup: --spec-branch",
            ),
            (
                vec!["pickup", "pickup"],
                "unexpected argument after pickup: pickup",
            ),
        ] {
            assert_eq!(rejection(&args), error, "{args:?}");
        }
    }

    #[test]
    fn pickup_rejects_repeated_contradictory_and_malformed_flags() {
        for (args, error) in [
            (
                vec!["pickup", "merge", "--no-merge"],
                "merge and no-merge can't be used together",
            ),
            (
                vec!["pickup", "no-merge", "no-merge"],
                "repeated argument: no-merge",
            ),
            (
                vec!["pickup", "base-fix", "no-base-fix"],
                "base-fix and no-base-fix can't be used together",
            ),
            (
                vec!["pickup", "--base-fix", "base-fix"],
                "repeated argument: base-fix",
            ),
            (
                vec!["pickup", "no-email", "email", "me@example.com"],
                "email and no-email can't be used together",
            ),
            (
                vec!["pickup", "email", "--email"],
                "repeated argument: --email",
            ),
            (
                vec!["pickup", "parallel", "2", "parallel", "3"],
                "repeated argument: parallel",
            ),
            (
                vec!["pickup", "parallel"],
                "parallel must be followed by a whole number from 1 up",
            ),
            (
                vec!["pickup", "parallel", "merge"],
                "parallel must be followed by a whole number from 1 up, not merge",
            ),
            (vec!["pickup", "base"], "base must be followed by a branch"),
            (
                vec!["pickup", "--base", "--merge"],
                "--base must be followed by a branch, not --merge",
            ),
            (
                vec!["pickup", "base", "develop", "base", "main"],
                "repeated argument: base",
            ),
        ] {
            assert_eq!(rejection(&args), error, "{args:?}");
        }
    }

    #[test]
    fn pickup_is_a_command_only_as_the_first_argument() {
        for (args, error) in [
            (
                vec![URL, "pickup"],
                "unexpected argument after the Issue URL: pickup",
            ),
            (vec!["merge", "pickup"], "not a GitHub issue URL: pickup"),
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
    fn the_retry_command_is_the_runs_own_flags_and_issue_url_with_base_fix_added() {
        for (args, retry) in [
            (vec![URL], format!("thirdshift {URL} base-fix")),
            (
                vec!["--merge", URL, "email"],
                format!("thirdshift {URL} merge --email base-fix"),
            ),
            (
                vec!["no-merge", "--no-email", URL, "--parallel", "2"],
                format!("thirdshift {URL} --no-merge --no-email parallel 2 base-fix"),
            ),
            (
                vec![URL, "email", "me@example.com"],
                format!("thirdshift {URL} --email me@example.com base-fix"),
            ),
        ] {
            let run = run_args(&args);
            assert_eq!(
                retry_with_base_fix(&run.issue, run.goal, run.email.as_ref(), run.parallel),
                retry,
                "{args:?}"
            );
            // The command it gives asks for what the Run was asked for.
            let words: Vec<&str> = retry.split(' ').skip(1).collect();
            let again = run_args(&words);
            assert_eq!(again.base_fix, Some(BaseFixAsk::Allow), "{retry}");
            assert_eq!(
                (again.issue.url, again.goal, again.email, again.parallel),
                (run.issue.url, run.goal, run.email, run.parallel),
                "{retry}"
            );
        }
    }

    #[test]
    fn a_tickets_run_is_given_the_command_to_offer_a_base_fix_with() {
        let retry = format!("thirdshift {URL} base-fix");
        let run = run_args(&["--spec-branch", "issue-7", "--offer-base-fix", &retry, URL]);

        assert_eq!(run.base_fix, Some(BaseFixAsk::Undecided { retry }));
        assert_eq!(
            rejection(&[URL, "--offer-base-fix"]),
            "missing command to offer"
        );
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
