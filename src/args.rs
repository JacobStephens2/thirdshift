//! The command line: which command, and for a Run, its Issue URL and flags,
//! for an Architect run, its focus and flags, or for a Pickup run, its flags.

use std::iter::Peekable;
use std::mem::discriminant;
use std::num::NonZeroUsize;

use anyhow::{Result, bail};

use crate::asks::Flags;
use crate::base_fix::BaseFixAsk;
use crate::child_run::{self, Given};
use crate::harness::{self, Harness};
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
    /// The flags it shares with a Run: `email` and `no-email` for its own
    /// Run notification, the rest for the Spec run or Run the plan is
    /// dispatched as, which are never given with [`PLAN_ONLY`].
    pub flags: Flags,
    /// Whether [`PLAN_ONLY`] was given: the plan is dispatched as nothing.
    pub plan_only: bool,
}

/// A Pickup run's arguments.
#[derive(Debug, PartialEq, Eq)]
pub struct PickupArgs {
    /// The Base branch `base <branch>` named, if given; without it, the
    /// branch checked out in the Launch directory is the Base branch.
    pub base: Option<String>,
    /// The flags it shares with a Run: `email` and `no-email` for its own
    /// Run notification, the rest for the Spec run or Run the Ready issue is
    /// dispatched as.
    pub flags: Flags,
}

/// The word that lets a Run start a Base fix, which the command a Base fix
/// is offered with ends in.
pub const BASE_FIX: &str = "base-fix";

/// A Run's arguments.
pub struct RunArgs {
    pub issue: IssueUrl,
    /// Its flags, as given: for each one the command left out, the User
    /// config decides.
    pub flags: Flags,
    /// What the Run was given, if another thirdshift started it as a child
    /// Run, as the child Run module read it back from its hidden arguments.
    pub given: Option<Given>,
}

/// Parse the arguments after the program name. `help`, `version`, `update`,
/// `setup`, `email-test`, `architect` and `pickup` are commands only as the
/// first argument. Otherwise it is a Run: one Issue URL, with each Run flag at
/// most once, before or after it.
/// `email` may be followed by the address to send the Run notification to,
/// and `parallel` must be followed by a whole number from 1 up.
/// `merge` and `no-merge` contradict each other, as do `email` and `no-email`,
/// and `base-fix` and `no-base-fix`. Every other argument is offered to the
/// child Run module first, which takes the hidden arguments a child Run is
/// given.
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
    let mut flags = Flags::default();
    let mut hidden = child_run::Reader::default();
    let mut args = args.iter().peekable();
    while let Some(arg) = args.next() {
        if take_flag(&mut flags, arg, &mut args)? || hidden.take(arg, &mut args)? {
            continue;
        }
        if issue.is_some() {
            bail!("unexpected argument after the Issue URL: {arg}");
        }
        issue = Some(IssueUrl::parse(arg)?);
    }
    let given = hidden.finish()?;
    let Some(issue) = issue else {
        bail!("missing Issue URL");
    };
    Ok(Command::Run(RunArgs {
        issue,
        flags,
        given,
    }))
}

/// Parse the arguments after `architect`: at most one focus, and its flags,
/// each at most once, in any order. `base` must be followed by the Base
/// branch. [`PLAN_ONLY`] dispatches nothing, so the flags for the dispatched
/// run, `merge`, `no-merge`, `parallel`, `base-fix`, `no-base-fix`,
/// `harness`, `model` and `effort`, can't go with it. `email` and `no-email`
/// are for the Architect run's own Run notification, and `base` is for the
/// Architecture review too, so they can. None of a Run's flags, nor `base`,
/// is ever the focus, and any other argument that starts with a dash is
/// unexpected rather than a focus.
fn parse_architect(args: &[String]) -> Result<ArchitectArgs> {
    let mut focus = None;
    let mut base = None;
    let mut plan_only = false;
    let mut flags = Flags::default();
    let mut args = args.iter().peekable();
    while let Some(arg) = args.next() {
        if take_flag(&mut flags, arg, &mut args)? {
            continue;
        }
        match arg.as_str() {
            PLAN_ONLY => {
                if plan_only {
                    bail!("repeated argument: {arg}");
                }
                plan_only = true;
            }
            "base" | "--base" => ask_word(&mut base, arg, args.next(), "a branch")?,
            _ if arg.starts_with('-') => bail!("unexpected argument after architect: {arg}"),
            _ if focus.is_some() => bail!("unexpected argument after the focus: {arg}"),
            _ if arg.trim().is_empty() => bail!("the focus is empty"),
            _ => focus = Some(arg.clone()),
        }
    }
    if plan_only && flags.any_for_dispatched_run() {
        bail!(
            "merge, no-merge, parallel, base-fix, no-base-fix, harness, model and effort \
             can't be used with {PLAN_ONLY}: it dispatches no run for them to apply to"
        );
    }
    Ok(ArchitectArgs {
        focus,
        base,
        flags,
        plan_only,
    })
}

/// Parse the arguments after `pickup`: its flags, each at most once, in any
/// order, and nothing else. `base` must be followed by the Base branch.
fn parse_pickup(args: &[String]) -> Result<PickupArgs> {
    let mut base = None;
    let mut flags = Flags::default();
    let mut args = args.iter().peekable();
    while let Some(arg) = args.next() {
        if take_flag(&mut flags, arg, &mut args)? {
            continue;
        }
        match arg.as_str() {
            "base" | "--base" => ask_word(&mut base, arg, args.next(), "a branch")?,
            _ => bail!("unexpected argument after pickup: {arg}"),
        }
    }
    Ok(PickupArgs { base, flags })
}

/// Record in `flags` what `arg` asks for, if it is one of the flags a Run,
/// an Architect run and a Pickup run all take, with or without its dashes,
/// taking from `rest` the address after `email`, if one is there, the
/// number after `parallel`, and the name or level after `harness`, `model`
/// and `effort`. False, taking nothing, if `arg` is none of them.
fn take_flag<'a>(
    flags: &mut Flags,
    arg: &str,
    rest: &mut Peekable<impl Iterator<Item = &'a String>>,
) -> Result<bool> {
    match arg {
        "merge" | "--merge" => ask_once(&mut flags.goal, Goal::Merged, arg, MERGE_FLAGS)?,
        "no-merge" | "--no-merge" => {
            ask_once(&mut flags.goal, Goal::ReadyForReview, arg, MERGE_FLAGS)?
        }
        "email" | "--email" => {
            let to = rest.next_if(|next| is_address(next)).cloned();
            ask_once(
                &mut flags.email,
                NotificationAsk::Send(to),
                arg,
                EMAIL_FLAGS,
            )?
        }
        "no-email" | "--no-email" => {
            ask_once(&mut flags.email, NotificationAsk::Skip, arg, EMAIL_FLAGS)?
        }
        "parallel" | "--parallel" => ask_parallel(&mut flags.parallel, arg, rest.next())?,
        BASE_FIX | "--base-fix" => {
            ask_once(&mut flags.base_fix, BaseFixAsk::Allow, arg, BASE_FIX_FLAGS)?
        }
        "no-base-fix" | "--no-base-fix" => {
            ask_once(&mut flags.base_fix, BaseFixAsk::Forbid, arg, BASE_FIX_FLAGS)?
        }
        "harness" | "--harness" => ask_harness(&mut flags.harness.harness, arg, rest.next())?,
        "model" | "--model" => {
            let model = &mut flags.harness.model_and_effort.model;
            ask_word(model, arg, rest.next(), "a model")?
        }
        "effort" | "--effort" => {
            let effort = &mut flags.harness.model_and_effort.effort;
            ask_word(effort, arg, rest.next(), "an effort level")?
        }
        _ => return Ok(false),
    }
    Ok(true)
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

/// Record in `given` the `word` that follows the flag `arg`, given once: a
/// branch, a model or an effort level, as `what` says. None of those starts
/// with a dash, so a flag there is not taken for one.
fn ask_word(
    given: &mut Option<String>,
    arg: &str,
    word: Option<&String>,
    what: &str,
) -> Result<()> {
    if given.is_some() {
        bail!("repeated argument: {arg}");
    }
    match word {
        Some(word) if word.starts_with('-') => {
            bail!("{arg} must be followed by {what}, not {word}")
        }
        Some(word) if !word.trim().is_empty() => *given = Some(word.clone()),
        _ => bail!("{arg} must be followed by {what}"),
    }
    Ok(())
}

/// Record in `harness` the Harness `name` that follows the flag `arg`, given
/// once.
fn ask_harness(harness: &mut Option<Harness>, arg: &str, name: Option<&String>) -> Result<()> {
    if harness.is_some() {
        bail!("repeated argument: {arg}");
    }
    let Some(named) = name.and_then(|name| Harness::named(name)) else {
        let given = name.map(|name| format!(", not {name}")).unwrap_or_default();
        bail!("{arg} must be followed by {}{given}", harness::names());
    };
    *harness = Some(named);
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
    use crate::harness::ModelAndEffort;

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
            assert!(architect_args.plan_only, "{args:?}");
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
            assert!(!architect_args.plan_only, "{args:?}");
            assert_eq!(architect_args.flags, Flags::default(), "{args:?}");
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
                architect_args(&args).flags,
                Flags {
                    goal,
                    parallel,
                    ..Flags::default()
                },
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
            assert_eq!(architect_args.flags.base_fix, base_fix, "{args:?}");
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
            assert_eq!(architect_args.flags.email, email, "{args:?}");
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
                "merge, no-merge, parallel, base-fix, no-base-fix, harness, model and effort \
                 can't be used with --plan-only: it dispatches no run for them to apply to",
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
            assert!(architect_args.plan_only, "{args:?}");
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
            flags: Flags::default(),
        };
        assert_eq!(pickup_args(&["pickup"]), none);
        let all = PickupArgs {
            base: Some("develop".to_string()),
            flags: Flags {
                goal: Some(Goal::Merged),
                email: Some(NotificationAsk::Send(Some("me@example.com".to_string()))),
                parallel: NonZeroUsize::new(2),
                base_fix: Some(BaseFixAsk::Allow),
                harness: harness::Asked {
                    harness: Some(Harness::Codex),
                    model_and_effort: ModelAndEffort {
                        model: Some("gpt-6.1-sol".to_string()),
                        effort: Some("max".to_string()),
                    },
                },
            },
        };
        for args in [
            vec![
                "pickup",
                "merge",
                "parallel",
                "2",
                "harness",
                "codex",
                "base-fix",
                "email",
                "me@example.com",
                "model",
                "gpt-6.1-sol",
                "base",
                "develop",
                "effort",
                "max",
            ],
            vec![
                "pickup",
                "--effort",
                "max",
                "--base",
                "develop",
                "--model",
                "gpt-6.1-sol",
                "--email",
                "me@example.com",
                "--base-fix",
                "--harness",
                "codex",
                "--parallel",
                "2",
                "--merge",
            ],
        ] {
            assert_eq!(pickup_args(&args), all, "{args:?}");
        }
        let cautious = PickupArgs {
            base: None,
            flags: Flags {
                goal: Some(Goal::ReadyForReview),
                email: Some(NotificationAsk::Skip),
                parallel: None,
                base_fix: Some(BaseFixAsk::Forbid),
                harness: harness::Asked::default(),
            },
        };
        for args in [
            ["pickup", "no-merge", "no-base-fix", "no-email"],
            ["pickup", "--no-email", "--no-merge", "--no-base-fix"],
        ] {
            assert_eq!(pickup_args(&args), cautious, "{args:?}");
        }
        let bare_email = pickup_args(&["pickup", "email", "merge"]);
        assert_eq!(bare_email.flags.email, Some(NotificationAsk::Send(None)));
        assert_eq!(bare_email.flags.goal, Some(Goal::Merged));
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
            assert_eq!(run_args.flags.email, Some(asked), "{args:?}");
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
            assert_eq!(run_args.flags.email, Some(asked), "{args:?}");
            assert_eq!(run_args.flags.goal, goal, "{args:?}");
        }
    }

    #[test]
    fn no_email_asks_for_no_notification_with_or_without_dashes() {
        for flag in ["no-email", "--no-email"] {
            assert_eq!(
                run_args(&[flag, URL]).flags.email,
                Some(NotificationAsk::Skip),
                "{flag}"
            );
        }
    }

    #[test]
    fn parallel_takes_the_number_after_it_with_or_without_dashes() {
        for flag in ["parallel", "--parallel"] {
            for args in [[flag, "1", URL], [URL, flag, "1"]] {
                assert_eq!(
                    run_args(&args).flags.parallel,
                    NonZeroUsize::new(1),
                    "{args:?}"
                );
            }
        }
    }

    #[test]
    fn harness_model_and_effort_take_the_word_after_them_on_a_run_an_architect_run_and_a_pickup_run()
     {
        let asked = |harness, model: Option<&str>, effort: Option<&str>| harness::Asked {
            harness,
            model_and_effort: ModelAndEffort {
                model: model.map(str::to_string),
                effort: effort.map(str::to_string),
            },
        };
        for (args, expected) in [
            (
                vec!["harness", "claude", URL],
                asked(Some(Harness::Claude), None, None),
            ),
            (
                vec![URL, "--model", "claude-opus-5-5", "merge", "effort", "high"],
                asked(None, Some("claude-opus-5-5"), Some("high")),
            ),
            (
                vec!["effort", "max", URL, "--harness", "codex"],
                asked(Some(Harness::Codex), None, Some("max")),
            ),
        ] {
            assert_eq!(run_args(&args).flags.harness, expected, "{args:?}");
        }
        let architect = architect_args(&[
            "architect",
            "the Spec run",
            "model",
            "opus",
            "harness",
            "claude",
        ]);
        assert_eq!(
            architect.flags.harness,
            asked(Some(Harness::Claude), Some("opus"), None)
        );
        assert_eq!(architect.focus.as_deref(), Some("the Spec run"));
        assert_eq!(
            pickup_args(&["pickup", "--effort", "low"]).flags.harness,
            asked(None, None, Some("low"))
        );
    }

    #[test]
    fn harness_model_and_effort_are_rejected_twice_without_a_value_or_with_plan_only() {
        let plan_only = "merge, no-merge, parallel, base-fix, no-base-fix, harness, model and \
                         effort can't be used with --plan-only: it dispatches no run for them \
                         to apply to";
        for (args, error) in [
            (
                vec![URL, "harness"],
                "harness must be followed by claude or codex or agy or grok or muse or opencode",
            ),
            (
                vec![URL, "harness", "gemini"],
                "harness must be followed by claude or codex or agy or grok or muse or opencode, not gemini",
            ),
            (
                vec!["--harness", "claude", URL, "harness", "codex"],
                "repeated argument: harness",
            ),
            (vec![URL, "model"], "model must be followed by a model"),
            (
                vec![URL, "--model", "--merge"],
                "--model must be followed by a model, not --merge",
            ),
            (
                vec!["model", "opus", URL, "model", "sonnet"],
                "repeated argument: model",
            ),
            (
                vec![URL, "effort"],
                "effort must be followed by an effort level",
            ),
            (
                vec!["pickup", "effort", "max", "--effort", "low"],
                "repeated argument: --effort",
            ),
            (
                vec!["architect", "--plan-only", "harness", "claude"],
                plan_only,
            ),
            (vec!["architect", "model", "opus", "--plan-only"], plan_only),
            (
                vec!["architect", "--plan-only", "--effort", "max"],
                plan_only,
            ),
        ] {
            assert_eq!(rejection(&args), error, "{args:?}");
        }
    }
}
