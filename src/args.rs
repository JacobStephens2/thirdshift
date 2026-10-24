//! The command line: which command, and for a Run, its Issue URL and flags,
//! or for a Pass, its flags and any arguments its own kind takes.
//! Also owns public retry guidance: explicit asks rendered as literal POSIX words.

use std::iter::Peekable;
use std::mem::discriminant;
use std::num::NonZeroUsize;

use anyhow::{Context, Result, bail};

use crate::asks::Flags;
use crate::base_fix::BaseFixAsk;
use crate::child_run::{self, Given};
use crate::harness::{self, Harness};
use crate::issue::{IssueInput, IssueUrl};
use crate::notification::NotificationAsk;
use crate::ready::PickupOrder;
use crate::run::Goal;

/// What thirdshift was asked to do.
pub enum Command {
    Help,
    Version,
    Update,
    Setup,
    Score,
    Scores {
        pull_request: Option<String>,
        mine: bool,
        json: bool,
    },
    BugOrigins(crate::scoring::origins::Options),
    Evals(crate::evals::Action),
    Experiment(crate::evals::experiment::Options),
    Experiments(Vec<String>),
    Recheck(Vec<String>),
    CronJobs {
        json: bool,
    },
    Resources {
        json: bool,
    },
    Console {
        no_sign_in: bool,
    },
    ConsoleInstall {
        output: Option<String>,
    },
    ConsolePassword(Option<String>),
    ConsoleSignOut {
        email: Option<String>,
        json: bool,
    },
    ConsoleLog {
        json: bool,
    },
    ConsoleTheme {
        name: Option<String>,
        json: bool,
    },
    ConsoleThemeImport(String),
    ConsoleThemeExport(String),
    ConsoleSections {
        hide: Vec<String>,
        show: Vec<String>,
        json: bool,
    },
    Status {
        json: bool,
        all: bool,
    },
    Show {
        command: String,
        json: bool,
    },
    Spans {
        command: String,
        span: Option<String>,
        json: bool,
    },
    Stop {
        command: String,
        requeue: bool,
        json: bool,
    },
    Dismiss {
        command: String,
        undo: bool,
        json: bool,
    },
    Logs {
        command: String,
        session: Option<String>,
        follow: bool,
    },
    LogsList {
        json: bool,
        owner: Option<String>,
    },
    LogsExport {
        command: String,
        directory: std::path::PathBuf,
    },
    LogFile {
        id: String,
        follow: bool,
    },
    /// Internal interface for scripts/release.sh: prompt on stdin.
    ReleaseSummary,
    /// Internal versioned session protocol for Evals.
    Trial(std::path::PathBuf),
    /// Internal scope wrapper for Evals: raise this scope's own `oom_score_adj`,
    /// then become `program`, keeping the raised score, the scope and every
    /// stdio descriptor.
    OomExec {
        program: String,
        args: Vec<String>,
    },
    /// `email-test`, with the address it was given, if any.
    EmailTest(Option<String>),
    Architect(ArchitectArgs),
    Pickup(PassArgs),
    Secure(PassArgs),
    Run(Box<RunArgs>),
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
    /// The flags it shares with a Run: the email flags for its own
    /// Run notification, the rest for the Spec run or Run the plan is
    /// dispatched as, which are never given with [`PLAN_ONLY`].
    pub flags: Flags,
    /// Whether [`PLAN_ONLY`] was given: the plan is dispatched as nothing.
    pub plan_only: bool,
}

/// A Pickup run or Security run's shared Pass arguments.
#[derive(Debug, PartialEq, Eq)]
pub struct PassArgs {
    /// The Base branch `base <branch>` named, if given; without it, the
    /// branch checked out in the Launch directory is the Base branch.
    pub base: Option<String>,
    /// The flags it shares with a Run: the email flags for its own
    /// Run notification, the rest for a run the Pass dispatches. The
    /// Security run also uses `security-fix` for its own choice of work.
    pub flags: Flags,
    /// What `lowest-first` or `highest-first` asked for, on a Pickup run:
    /// which end of the issue numbers it takes its Ready issue from. None
    /// leaves it to the User config. A Security run takes neither.
    pub order: Option<PickupOrder>,
}

/// The word that lets a Run start a Base fix, which the command a Base fix
/// is offered with ends in.
const BASE_FIX: &str = "base-fix";

/// A Run's arguments.
pub struct RunArgs {
    pub issue: IssueInput,
    /// Its flags, as given: for each one the command left out, the User
    /// config decides.
    pub flags: Flags,
    /// What the Run was given, if another thirdshift started it as a child
    /// Run, as the child Run module read it back from its hidden arguments.
    pub given: Option<Given>,
}

/// Retry the original Issue URL with Base fixing allowed, preserving only
/// explicit public choices so omitted choices still consult the User config.
pub fn retry_with_base_fix(issue: &IssueUrl, flags: &Flags) -> String {
    let flags = Flags {
        base_fix: Some(BaseFixAsk::Allow),
        ..flags.clone()
    };
    let mut words = vec!["thirdshift".to_string(), issue.url.clone()];
    public_flag_words(&flags, &mut words);
    shell_command(words)
}

/// Re-run the original issue with the effective public choices. Spec parallelism
/// is valid only on a Spec; Launch directory/Base branch are stated separately.
pub fn continuation(issue: &IssueUrl, asks: &crate::asks::Asks, is_spec: bool) -> String {
    let flags = Flags {
        goal: Some(asks.goal),
        parallel: is_spec.then_some(asks.tickets_at_once),
        base_fix: Some(asks.base_fix.clone()),
        harness: harness::Asked {
            harness: Some(asks.harness.harness),
            model_and_effort: crate::harness::ModelAndEffort {
                model: asks.harness.model.clone(),
                effort: asks.harness.effort.clone(),
            },
        },
        security_fix: Some(if asks.security_fix {
            crate::security::FixAsk::Allow
        } else {
            crate::security::FixAsk::Forbid
        }),
        security_review: Some(if asks.security_review {
            crate::security::review::Ask::Allow
        } else {
            crate::security::review::Ask::Forbid
        }),
        ..Flags::default()
    };
    let mut words = vec!["thirdshift".into(), issue.url.clone()];
    public_flag_words(&flags, &mut words);
    shell_command(words)
}

/// Retry a Security run with fixing allowed, retaining the original optional
/// Base branch rather than the one launch preparation resolved.
pub fn retry_with_security_fix(base: Option<&str>, flags: &Flags) -> String {
    let flags = Flags {
        security_fix: Some(crate::security::FixAsk::Allow),
        ..flags.clone()
    };
    let mut words = vec!["thirdshift".to_string(), "secure".to_string()];
    if let Some(base) = base {
        words.extend(["base".into(), base.into()]);
    }
    public_flag_words(&flags, &mut words);
    shell_command(words)
}

/// Serialize public asks, never computed retry text or hidden child Run facts.
fn public_flag_words(flags: &Flags, words: &mut Vec<String>) {
    match flags.goal {
        Some(Goal::Merged) => words.push("merge".into()),
        Some(Goal::ReadyForReview) => words.push("--no-merge".into()),
        None => {}
    }
    match &flags.email {
        Some(NotificationAsk::Send(to)) => {
            words.push("--email".into());
            if let Some(to) = to {
                words.push(to.clone());
            }
        }
        Some(NotificationAsk::SendUnlessMerged(to)) => {
            words.push("--email-unless-merged".into());
            if let Some(to) = to {
                words.push(to.clone());
            }
        }
        Some(NotificationAsk::Skip) => words.push("--no-email".into()),
        None => {}
    }
    if let Some(parallel) = flags.parallel {
        words.extend(["parallel".into(), parallel.to_string()]);
    }
    match flags.security_fix {
        Some(crate::security::FixAsk::Allow) => words.push("security-fix".into()),
        Some(crate::security::FixAsk::Forbid) => words.push("no-security-fix".into()),
        None => {}
    }
    match flags.security_close {
        Some(crate::security::CloseAsk::Allow) => words.push("security-close".into()),
        Some(crate::security::CloseAsk::Forbid) => words.push("no-security-close".into()),
        None => {}
    }
    match flags.security_review {
        Some(crate::security::review::Ask::Allow) => words.push("security-review".into()),
        Some(crate::security::review::Ask::Forbid) => words.push("no-security-review".into()),
        None => {}
    }
    let asked = &flags.harness;
    if let Some(harness) = asked.harness {
        words.extend(["harness".into(), harness.name().into()]);
    }
    if let Some(model) = &asked.model_and_effort.model {
        words.extend(["model".into(), model.clone()]);
    }
    if let Some(effort) = &asked.model_and_effort.effort {
        words.extend(["effort".into(), effort.clone()]);
    }
    match &flags.base_fix {
        Some(BaseFixAsk::Allow) => words.push(BASE_FIX.into()),
        Some(BaseFixAsk::Forbid) => words.push("no-base-fix".into()),
        Some(BaseFixAsk::Undecided { .. }) | None => {}
    }
}

/// Render literal POSIX shell words; readable safe words need no quoting.
pub(crate) fn shell_command(words: Vec<String>) -> String {
    words
        .into_iter()
        .map(|word| {
            if !word.is_empty()
                && word
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_/.:@=".contains(c))
            {
                word
            } else {
                format!("'{}'", word.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse the arguments after the program name. `help`, `version`, `update`,
/// `setup`, `email-test`, `architect`, `pickup` and `secure` are commands only as the
/// first argument. Otherwise it is a Run: one Issue URL or bare issue number,
/// with each Run flag at most once, before or after it.
/// `email` and `email-unless-merged` may be followed by the address to send
/// the Run notification to, and `parallel` must be followed by a whole number
/// from 1 up. `merge` and `no-merge` contradict each other, as do any two of
/// `email`, `email-unless-merged` and `no-email`, and `base-fix` and
/// `no-base-fix`. Every other argument is offered to the
/// child Run module first, which takes the hidden arguments a child Run is
/// given.
pub fn parse(args: &[String]) -> Result<Command> {
    match args.first().map(String::as_str) {
        Some("--trial") => {
            let [_, request] = args else {
                bail!("usage: thirdshift --trial <request.json>");
            };
            return Ok(Command::Trial(request.into()));
        }
        Some("--oom-exec") => {
            let [_, program, rest @ ..] = args else {
                bail!("usage: thirdshift --oom-exec <program> [args...]");
            };
            return Ok(Command::OomExec {
                program: program.clone(),
                args: rest.to_vec(),
            });
        }
        Some("help" | "--help" | "-h") => return Ok(Command::Help),
        Some("version" | "--version" | "-V") => return Ok(Command::Version),
        Some("update") => return Ok(Command::Update),
        Some("evals") => return crate::evals::parse(&args[1..]).map(Command::Evals),
        Some("bug-origins") => {
            return crate::scoring::origins::parse(&args[1..]).map(Command::BugOrigins);
        }
        Some("experiment") => {
            return crate::evals::experiment::parse(&args[1..]).map(Command::Experiment);
        }
        Some("experiments") => return Ok(Command::Experiments(args[1..].to_vec())),
        Some("recheck") => return Ok(Command::Recheck(args[1..].to_vec())),
        Some("console") => {
            if args.get(1).is_some_and(|action| action == "theme") {
                let json = args.last().is_some_and(|arg| arg == "--json");
                let end = args.len() - usize::from(json);
                if let [action, value] = &args[2..end] {
                    match action.as_str() {
                        "import" => return Ok(Command::ConsoleThemeImport(value.clone())),
                        "export" => return Ok(Command::ConsoleThemeExport(value.clone())),
                        _ => {}
                    }
                }
                let [name] = &args[2..end] else {
                    bail!(
                        "usage: thirdshift console theme <name|list> [--json] | import <file> | export <name>"
                    );
                };
                return Ok(Command::ConsoleTheme {
                    name: (name != "list").then(|| name.clone()),
                    json,
                });
            }
            if args.get(1).is_some_and(|action| action == "sections") {
                let (mut hide, mut show, mut json) = (Vec::new(), Vec::new(), false);
                let mut arguments = args[2..].iter();
                while let Some(flag) = arguments.next() {
                    match flag.as_str() {
                        "--json" if !json => json = true,
                        "--hide" | "--show" => {
                            let section = arguments
                                .next()
                                .filter(|s| !s.starts_with('-'))
                                .context("--hide and --show need a section name")?;
                            let target = if flag == "--hide" {
                                &mut hide
                            } else {
                                &mut show
                            };
                            target.extend(section.split(',').map(str::to_owned));
                        }
                        _ => bail!(
                            "usage: thirdshift console sections [--hide <section>] [--show <section>] [--json]"
                        ),
                    }
                }
                return Ok(Command::ConsoleSections { hide, show, json });
            }
            if args.get(1).is_some_and(|action| action == "sign-out") {
                let mut email = None;
                let mut json = false;
                let mut arguments = args[2..].iter();
                while let Some(flag) = arguments.next() {
                    match flag.as_str() {
                        "--email" if email.is_none() => {
                            email = Some(
                                arguments
                                    .next()
                                    .filter(|email| !email.starts_with('-'))
                                    .ok_or_else(|| anyhow::anyhow!("--email needs an address"))?
                                    .clone(),
                            );
                        }
                        "--json" if !json => json = true,
                        _ => {
                            bail!("usage: thirdshift console sign-out [--email <address>] [--json]")
                        }
                    }
                }
                return Ok(Command::ConsoleSignOut { email, json });
            }
            return match &args[1..] {
                [] => Ok(Command::Console { no_sign_in: false }),
                [flag] if flag == "--no-sign-in" => Ok(Command::Console { no_sign_in: true }),
                [action] if action == "install" => Ok(Command::ConsoleInstall { output: None }),
                [action, flag, file]
                    if action == "install" && flag == "--output" && !file.starts_with('-') =>
                {
                    Ok(Command::ConsoleInstall {
                        output: Some(file.clone()),
                    })
                }
                [action] if action == "password" => Ok(Command::ConsolePassword(None)),
                [action, flag, email]
                    if action == "password" && flag == "--email" && !email.starts_with('-') =>
                {
                    Ok(Command::ConsolePassword(Some(email.clone())))
                }
                [action] if action == "log" => Ok(Command::ConsoleLog { json: false }),
                [action, flag] if action == "log" && flag == "--json" => {
                    Ok(Command::ConsoleLog { json: true })
                }
                _ => bail!(
                    "usage: thirdshift console [--no-sign-in] | install [--output <file>] | password [--email <address>] | sign-out [--email <address>] [--json] | log [--json]"
                ),
            };
        }
        Some("stop") => {
            let usage = "usage: thirdshift stop <command> [--requeue] [--json]";
            let Some((command, options)) = args[1..].split_first() else {
                bail!(usage)
            };
            if command.starts_with('-') {
                bail!(usage);
            }
            let (mut requeue, mut json) = (false, false);
            for option in options {
                match option.as_str() {
                    "--requeue" if !requeue => requeue = true,
                    "--json" if !json => json = true,
                    _ => bail!(usage),
                }
            }
            return Ok(Command::Stop {
                command: command.clone(),
                requeue,
                json,
            });
        }
        Some("dismiss") => {
            let usage = "usage: thirdshift dismiss <command> [--undo] [--json]";
            let Some((command, options)) = args[1..].split_first() else {
                bail!(usage)
            };
            if command.starts_with('-') {
                bail!(usage);
            }
            let (mut undo, mut json) = (false, false);
            for option in options {
                match option.as_str() {
                    "--undo" if !undo => undo = true,
                    "--json" if !json => json = true,
                    _ => bail!(usage),
                }
            }
            return Ok(Command::Dismiss {
                command: command.clone(),
                undo,
                json,
            });
        }
        Some("show") => {
            let usage = "usage: thirdshift show <command> [--spans | --span <id>] [--json]";
            if args.iter().any(|arg| arg == "--spans" || arg == "--span") {
                let command = args
                    .get(1)
                    .filter(|arg| !arg.starts_with('-'))
                    .ok_or_else(|| anyhow::anyhow!(usage))?;
                let (mut tree, mut span, mut json) = (false, None, false);
                let mut flags = args[2..].iter();
                while let Some(flag) = flags.next() {
                    match flag.as_str() {
                        "--json" if !json => json = true,
                        "--spans" if !tree && span.is_none() => tree = true,
                        "--span" if !tree && span.is_none() => {
                            span = Some(
                                flags
                                    .next()
                                    .filter(|id| !id.starts_with('-'))
                                    .ok_or_else(|| anyhow::anyhow!(usage))?
                                    .clone(),
                            );
                        }
                        _ => bail!(usage),
                    }
                }
                return Ok(Command::Spans {
                    command: command.clone(),
                    span,
                    json,
                });
            }
            return match &args[1..] {
                [command] if !command.starts_with('-') => Ok(Command::Show {
                    command: command.clone(),
                    json: false,
                }),
                [command, flag] if flag == "--json" && !command.starts_with('-') => {
                    Ok(Command::Show {
                        command: command.clone(),
                        json: true,
                    })
                }
                _ => bail!("usage: thirdshift show <command> [--json]"),
            };
        }
        Some("logs") => {
            match &args[1..] {
                [] => {
                    return Ok(Command::LogsList {
                        json: false,
                        owner: None,
                    });
                }
                [flag] if flag == "--json" => {
                    return Ok(Command::LogsList {
                        json: true,
                        owner: None,
                    });
                }
                [flag, id] if flag == "--command" && !id.starts_with('-') => {
                    return Ok(Command::LogsList {
                        json: false,
                        owner: Some(id.clone()),
                    });
                }
                [flag, id, json]
                    if flag == "--command" && !id.starts_with('-') && json == "--json" =>
                {
                    return Ok(Command::LogsList {
                        json: true,
                        owner: Some(id.clone()),
                    });
                }
                [flag, id] if flag == "--file" && !id.starts_with('-') => {
                    return Ok(Command::LogFile {
                        id: id.clone(),
                        follow: false,
                    });
                }
                [flag, id, follow]
                    if flag == "--file" && !id.starts_with('-') && follow == "--follow" =>
                {
                    return Ok(Command::LogFile {
                        id: id.clone(),
                        follow: true,
                    });
                }
                _ => {}
            }
            let usage = "usage: thirdshift logs <command> [--session <kind>] [--follow] | thirdshift logs <command> --export <dir>";
            let command = args
                .get(1)
                .filter(|name| !name.starts_with('-'))
                .ok_or_else(|| anyhow::anyhow!(usage))?;
            if let [flag, directory] = &args[2..]
                && flag == "--export"
                && !directory.starts_with('-')
            {
                return Ok(Command::LogsExport {
                    command: command.clone(),
                    directory: directory.into(),
                });
            }
            let mut session = None;
            let mut follow = false;
            let mut flags = args[2..].iter();
            while let Some(flag) = flags.next() {
                match flag.as_str() {
                    "--follow" if !follow => follow = true,
                    "--session" if session.is_none() => {
                        session = Some(
                            flags
                                .next()
                                .filter(|kind| !kind.starts_with('-'))
                                .ok_or_else(|| anyhow::anyhow!(usage))?
                                .clone(),
                        );
                    }
                    _ => bail!(usage),
                }
            }
            return Ok(Command::Logs {
                command: command.clone(),
                session,
                follow,
            });
        }
        Some("cronjobs") => {
            return match &args[1..] {
                [] => Ok(Command::CronJobs { json: false }),
                [flag] if flag == "--json" => Ok(Command::CronJobs { json: true }),
                _ => bail!("usage: thirdshift cronjobs [--json]"),
            };
        }
        Some("score") => {
            return match &args[1..] {
                [] => Ok(Command::Score),
                _ => bail!("usage: thirdshift score"),
            };
        }
        Some("scores") => {
            let mut pull_request = None;
            let mut mine = false;
            let mut json = false;
            for argument in &args[1..] {
                match argument.as_str() {
                    "--mine" if !mine => mine = true,
                    "--json" if !json => json = true,
                    value if !value.starts_with('-') && pull_request.is_none() => {
                        pull_request = Some(value.to_owned());
                    }
                    _ => bail!("usage: thirdshift scores [<pull request>] [--mine | --json]"),
                }
            }
            if mine && (pull_request.is_none() || json) {
                bail!("usage: thirdshift scores <pull request> --mine");
            }
            return Ok(Command::Scores {
                pull_request,
                mine,
                json,
            });
        }
        Some("resources") => {
            return match &args[1..] {
                [] => Ok(Command::Resources { json: false }),
                [flag] if flag == "--json" => Ok(Command::Resources { json: true }),
                _ => bail!("usage: thirdshift resources [--json]"),
            };
        }
        Some("status") => {
            return match &args[1..] {
                [] => Ok(Command::Status {
                    json: false,
                    all: false,
                }),
                [flag] if flag == "--json" => Ok(Command::Status {
                    json: true,
                    all: false,
                }),
                [flag] if flag == "--all" => Ok(Command::Status {
                    json: false,
                    all: true,
                }),
                [a, b] if (a == "--all" && b == "--json") || (a == "--json" && b == "--all") => {
                    Ok(Command::Status {
                        json: true,
                        all: true,
                    })
                }
                _ => bail!("usage: thirdshift status [--all] [--json]"),
            };
        }
        Some("--release-summary") => {
            return match &args[1..] {
                [] => Ok(Command::ReleaseSummary),
                [extra, ..] => bail!("unexpected argument after --release-summary: {extra}"),
            };
        }
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
        Some("pickup") => return parse_pass(&args[1..], "pickup").map(Command::Pickup),
        Some("secure") => return parse_pass(&args[1..], "secure").map(Command::Secure),
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
        issue = Some(IssueInput::parse(arg)?);
    }
    let given = hidden.finish()?;
    let Some(issue) = issue else {
        bail!("missing Issue URL");
    };
    Ok(Command::Run(Box::new(RunArgs {
        issue,
        flags,
        given,
    })))
}

/// Parse the arguments after `architect`: at most one focus, and its flags,
/// each at most once, in any order. `base` must be followed by the Base
/// branch. [`PLAN_ONLY`] dispatches nothing, so the flags for the dispatched
/// run, `merge`, `no-merge`, `parallel`, `base-fix`, `no-base-fix`,
/// `harness`, `model` and `effort`, can't go with it. `email`,
/// `email-unless-merged` and `no-email` are for the Architect run's own Run
/// notification, and `base` is for the
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
        if take_pass_flag(&mut base, &mut flags, arg, &mut args)? {
            continue;
        }
        match arg.as_str() {
            PLAN_ONLY => {
                if plan_only {
                    bail!("repeated argument: {arg}");
                }
                plan_only = true;
            }
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

/// Parse the arguments after `pickup` or `secure`: shared flags, at most once, in any
/// order, and nothing else. `base` must be followed by the Base branch.
/// `lowest-first` and `highest-first` are a Pickup run's only: they say
/// which end of the issue numbers it takes its Ready issue from, over the
/// User config.
fn parse_pass(args: &[String], command: &str) -> Result<PassArgs> {
    let mut base = None;
    let mut flags = Flags::default();
    let mut order = None;
    let mut args = args.iter().peekable();
    while let Some(arg) = args.next() {
        if command == "pickup" && take_pickup_order(&mut order, arg)? {
            continue;
        }
        if !take_pass_flag(&mut base, &mut flags, arg, &mut args)? {
            bail!("unexpected argument after {command}: {arg}");
        }
    }
    Ok(PassArgs { base, flags, order })
}

/// Record in `order` what the Pickup flag `arg` asks for, if it is one of
/// the order words, with or without its dashes. False, taking nothing, if
/// `arg` is none of them.
fn take_pickup_order(order: &mut Option<PickupOrder>, arg: &str) -> Result<bool> {
    let asked = match arg {
        "lowest-first" | "--lowest-first" => PickupOrder::LowestFirst,
        "highest-first" | "--highest-first" => PickupOrder::HighestFirst,
        _ => return Ok(false),
    };
    ask_once(order, asked, arg, ORDER_FLAGS)?;
    Ok(true)
}

/// Take a Pass's shared words: its Base branch and the flags for its
/// dispatched run and its own Run notification. Each kind's parser handles
/// only the arguments specific to that kind.
fn take_pass_flag<'a>(
    base: &mut Option<String>,
    flags: &mut Flags,
    arg: &str,
    rest: &mut Peekable<impl Iterator<Item = &'a String>>,
) -> Result<bool> {
    match arg {
        "base" | "--base" => {
            ask_word(base, arg, rest.next(), "a branch")?;
            Ok(true)
        }
        _ => take_flag(flags, arg, rest),
    }
}

/// Record in `flags` what `arg` asks for, if it is one of the flags a Run
/// and a Pass both take, with or without its dashes,
/// taking from `rest` the address after `email` or `email-unless-merged`,
/// if one is there, the
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
        "email-unless-merged" | "--email-unless-merged" => {
            let to = rest.next_if(|next| is_address(next)).cloned();
            ask_once(
                &mut flags.email,
                NotificationAsk::SendUnlessMerged(to),
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
        "security-fix" | "--security-fix" => ask_once(
            &mut flags.security_fix,
            crate::security::FixAsk::Allow,
            arg,
            SECURITY_FIX_FLAGS,
        )?,
        "no-security-fix" | "--no-security-fix" => ask_once(
            &mut flags.security_fix,
            crate::security::FixAsk::Forbid,
            arg,
            SECURITY_FIX_FLAGS,
        )?,
        "security-close" | "--security-close" => ask_once(
            &mut flags.security_close,
            crate::security::CloseAsk::Allow,
            arg,
            SECURITY_CLOSE_FLAGS,
        )?,
        "no-security-close" | "--no-security-close" => ask_once(
            &mut flags.security_close,
            crate::security::CloseAsk::Forbid,
            arg,
            SECURITY_CLOSE_FLAGS,
        )?,
        "security-review" | "--security-review" => ask_once(
            &mut flags.security_review,
            crate::security::review::Ask::Allow,
            arg,
            SECURITY_REVIEW_FLAGS,
        )?,
        "no-security-review" | "--no-security-review" => ask_once(
            &mut flags.security_review,
            crate::security::review::Ask::Forbid,
            arg,
            SECURITY_REVIEW_FLAGS,
        )?,
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

const ORDER_FLAGS: &str = "lowest-first and highest-first";
const MERGE_FLAGS: &str = "merge and no-merge";
const EMAIL_FLAGS: &str = "email, email-unless-merged and no-email";
const BASE_FIX_FLAGS: &str = "base-fix and no-base-fix";
const SECURITY_REVIEW_FLAGS: &str = "security-review and no-security-review";
const SECURITY_FIX_FLAGS: &str = "security-fix and no-security-fix";
const SECURITY_CLOSE_FLAGS: &str = "security-close and no-security-close";

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

/// Is `arg`, after `email` or `email-unless-merged`, the address to send to? Only if it looks like
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

    use crate::test_support::guidance::words as shell_words;

    #[test]
    fn continuation_roundtrips_effective_choices_and_literal_shell_values() {
        use crate::asks::Asks;
        use crate::harness::{Choice, ChosenBy};
        for is_spec in [false, true] {
            for goal in [Goal::Merged, Goal::ReadyForReview] {
                for base_fix in [
                    BaseFixAsk::Allow,
                    BaseFixAsk::Forbid,
                    BaseFixAsk::Undecided {
                        retry: "do not serialize this".into(),
                    },
                ] {
                    let asks = Asks {
                        security_review_harness: None,
                        security_review_default: None,
                        goal,
                        base_fix: base_fix.clone(),
                        tickets_at_once: NonZeroUsize::new(5).unwrap(),
                        parallel_asked: false,
                        security_fix: true,
                        security_review: true,
                        launch_pull: false,
                        notification: NotificationAsk::Skip,
                        harness: Choice {
                            harness: Harness::Codex,
                            model: Some("vendor/model's $(touch never); `literal`".into()),
                            effort: Some("max".into()),
                            chosen_by: ChosenBy::UserConfig,
                        },
                    };
                    let command = continuation(&IssueUrl::parse(URL).unwrap(), &asks, is_spec);
                    let Command::Run(run) = parse(&shell_words(&command)).unwrap() else {
                        panic!("not a Run: {command}")
                    };
                    assert_eq!(run.issue, IssueInput::Url(IssueUrl::parse(URL).unwrap()));
                    assert_eq!(run.flags.goal, Some(goal));
                    assert_eq!(
                        run.flags.parallel,
                        is_spec.then_some(NonZeroUsize::new(5).unwrap())
                    );
                    assert_eq!(run.flags.harness.harness, Some(Harness::Codex));
                    assert_eq!(run.flags.harness.model_and_effort.model, asks.harness.model);
                    assert_eq!(
                        run.flags.harness.model_and_effort.effort,
                        asks.harness.effort
                    );
                    assert_eq!(
                        run.flags.base_fix,
                        if matches!(base_fix, BaseFixAsk::Undecided { .. }) {
                            None
                        } else {
                            Some(base_fix)
                        }
                    );
                    assert_eq!(run.flags.security_fix, Some(crate::security::FixAsk::Allow));
                    assert_eq!(
                        run.flags.security_review,
                        Some(crate::security::review::Ask::Allow)
                    );
                    assert!(run.given.is_none());
                    assert!(!command.contains("do not serialize"));
                }
            }
        }
    }

    #[test]
    fn base_fix_guidance_preserves_a_model_label_with_spaces() {
        let flags = Flags {
            harness: harness::Asked {
                model_and_effort: ModelAndEffort {
                    model: Some("Model Label With Spaces".into()),
                    effort: None,
                },
                ..harness::Asked::default()
            },
            ..Flags::default()
        };
        let command = retry_with_base_fix(&IssueUrl::parse(URL).unwrap(), &flags);
        let Command::Run(run) = parse(&shell_words(&command)).unwrap() else {
            panic!("not a Run: {command}");
        };
        assert_eq!(
            run.flags,
            Flags {
                base_fix: Some(BaseFixAsk::Allow),
                ..flags
            }
        );
        assert_eq!(run.issue, IssueInput::Url(IssueUrl::parse(URL).unwrap()));
        assert!(run.given.is_none());
    }

    /// Supplied public asks cover positive, negative, undecided and omitted choices.
    fn guidance_flags() -> Vec<Flags> {
        vec![
            Flags::default(),
            Flags {
                goal: Some(Goal::Merged),
                email: Some(NotificationAsk::Send(Some("day@example.com".into()))),
                parallel: NonZeroUsize::new(2),
                base_fix: Some(BaseFixAsk::Forbid),
                security_fix: Some(crate::security::FixAsk::Forbid),
                security_close: None,
                security_review: Some(crate::security::review::Ask::Allow),
                harness: harness::Asked {
                    harness: Some(Harness::Codex),
                    model_and_effort: ModelAndEffort {
                        model: Some("gpt-6.1-sol".into()),
                        effort: Some("max".into()),
                    },
                },
            },
            Flags {
                goal: Some(Goal::ReadyForReview),
                email: Some(NotificationAsk::Skip),
                parallel: NonZeroUsize::new(1),
                base_fix: Some(BaseFixAsk::Allow),
                security_fix: Some(crate::security::FixAsk::Allow),
                security_review: Some(crate::security::review::Ask::Forbid),
                ..Flags::default()
            },
            Flags {
                email: Some(NotificationAsk::Send(None)),
                base_fix: Some(BaseFixAsk::Undecided {
                    retry: "computed child retry must stay private".into(),
                }),
                ..Flags::default()
            },
            Flags {
                email: Some(NotificationAsk::SendUnlessMerged(Some(
                    "day@example.com".into(),
                ))),
                ..Flags::default()
            },
            Flags {
                email: Some(NotificationAsk::SendUnlessMerged(None)),
                ..Flags::default()
            },
        ]
    }

    #[test]
    fn both_guidance_operations_round_trip_public_choices_and_enable_permission_once() {
        let issue = IssueUrl::parse(URL).unwrap();
        for flags in guidance_flags() {
            let command = retry_with_base_fix(&issue, &flags);
            let words = shell_words(&command);
            assert_eq!(words.iter().filter(|word| *word == "base-fix").count(), 1);
            let Command::Run(run) = parse(&words).unwrap() else {
                panic!("{command}")
            };
            assert_eq!(run.issue, IssueInput::Url(issue.clone()));
            assert!(run.given.is_none());
            assert_eq!(
                run.flags,
                Flags {
                    base_fix: Some(BaseFixAsk::Allow),
                    ..flags.clone()
                }
            );

            for base in [None, Some("main")] {
                let command = retry_with_security_fix(base, &flags);
                let words = shell_words(&command);
                assert_eq!(
                    words.iter().filter(|word| *word == "security-fix").count(),
                    1
                );
                let Command::Secure(pass) = parse(&words).unwrap() else {
                    panic!("{command}")
                };
                assert_eq!(pass.base.as_deref(), base);
                let public_base_fix = match &flags.base_fix {
                    Some(BaseFixAsk::Undecided { .. }) => None,
                    other => other.clone(),
                };
                assert_eq!(
                    pass.flags,
                    Flags {
                        security_fix: Some(crate::security::FixAsk::Allow),
                        base_fix: public_base_fix,
                        ..flags.clone()
                    }
                );
            }
        }
    }

    #[test]
    fn guidance_preserves_literal_shell_punctuation_in_every_public_value() {
        let issue = IssueUrl::parse(URL).unwrap();
        for value in [
            "Model Label With Spaces",
            "O'Brien",
            "\"double quotes\"",
            "Unicode 模型 π",
            "literal $fixture ${fixture} $(printf fixture) `printf fixture` ; & | < > ( ) * ? ! [ ] \\ \t\nnext",
        ] {
            let flags = Flags {
                email: Some(NotificationAsk::Send(Some(format!("literal@{value}")))),
                harness: harness::Asked {
                    model_and_effort: ModelAndEffort {
                        model: Some(value.into()),
                        effort: Some(value.into()),
                    },
                    ..harness::Asked::default()
                },
                ..Flags::default()
            };
            let command = retry_with_base_fix(&issue, &flags);
            let Command::Run(run) = parse(&shell_words(&command)).unwrap() else {
                panic!("{command}")
            };
            assert_eq!(
                run.flags,
                Flags {
                    base_fix: Some(BaseFixAsk::Allow),
                    ..flags.clone()
                }
            );
            assert!(run.given.is_none());
            let command = retry_with_security_fix(Some(value), &flags);
            let Command::Secure(pass) = parse(&shell_words(&command)).unwrap() else {
                panic!("{command}")
            };
            assert_eq!(pass.base.as_deref(), Some(value));
            assert_eq!(
                pass.flags,
                Flags {
                    security_fix: Some(crate::security::FixAsk::Allow),
                    ..flags
                }
            );
        }
    }

    #[test]
    fn both_guidance_operations_preserve_every_explicit_harness() {
        for harness in [
            Harness::Claude,
            Harness::Codex,
            Harness::Agy,
            Harness::Grok,
            Harness::Muse,
            Harness::OpenCode,
        ] {
            let flags = Flags {
                harness: harness::Asked {
                    harness: Some(harness),
                    ..harness::Asked::default()
                },
                ..Flags::default()
            };
            let command = retry_with_base_fix(&IssueUrl::parse(URL).unwrap(), &flags);
            let Command::Run(run) = parse(&shell_words(&command)).unwrap() else {
                panic!("{command}")
            };
            assert_eq!(
                run.flags,
                Flags {
                    base_fix: Some(BaseFixAsk::Allow),
                    ..flags.clone()
                }
            );
            let command = retry_with_security_fix(None, &flags);
            let Command::Secure(pass) = parse(&shell_words(&command)).unwrap() else {
                panic!("{command}")
            };
            assert_eq!(
                pass.flags,
                Flags {
                    security_fix: Some(crate::security::FixAsk::Allow),
                    ..flags
                }
            );
        }
    }

    #[test]
    fn security_review_words_are_accepted_on_commands_that_start_runs() {
        for command in [URL, "pickup", "architect", "secure"] {
            for word in [
                "security-review",
                "--security-review",
                "no-security-review",
                "--no-security-review",
            ] {
                assert!(parse_strs(&[command, word]).is_ok(), "{command} {word}");
            }
        }
        assert!(parse_strs(&[URL, "security-review", "no-security-review"]).is_err());
        assert!(parse_strs(&[URL, "security-review", "--security-review"]).is_err());
    }

    #[test]
    fn security_close_words_are_accepted_on_commands_that_start_runs() {
        use crate::security::CloseAsk;
        for command in [URL, "pickup", "architect", "secure"] {
            for (word, expected) in [
                ("security-close", CloseAsk::Allow),
                ("--security-close", CloseAsk::Allow),
                ("no-security-close", CloseAsk::Forbid),
                ("--no-security-close", CloseAsk::Forbid),
            ] {
                let flags = match parse_strs(&[command, word]) {
                    Ok(Command::Run(run)) => run.flags,
                    Ok(Command::Pickup(pass) | Command::Secure(pass)) => pass.flags,
                    Ok(Command::Architect(architect)) => architect.flags,
                    _ => panic!("{command} {word}: unexpected command"),
                };
                assert_eq!(flags.security_close, Some(expected), "{command} {word}");
            }
        }
        assert!(parse_strs(&[URL, "security-close", "no-security-close"]).is_err());
        assert!(parse_strs(&[URL, "security-close", "--security-close"]).is_err());
        assert!(parse_strs(&["secure", "security-close", "no-security-fix"]).is_ok());
    }

    fn parse_strs(args: &[&str]) -> Result<Command> {
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        parse(&args)
    }

    /// The Run `args` parse to.
    fn run_args(args: &[&str]) -> RunArgs {
        match parse_strs(args) {
            Ok(Command::Run(run_args)) => *run_args,
            Ok(_) => panic!("{args:?}: not a Run"),
            Err(error) => panic!("{args:?}: {error:#}"),
        }
    }

    #[test]
    fn issue_numbers_leave_numeric_option_values_with_their_flags() {
        for input in ["106", "000106"] {
            for args in [
                vec![
                    "parallel", "2", "model", "123", "effort", "456", "email", "merge", input,
                ],
                vec![
                    input,
                    "--parallel",
                    "2",
                    "--model",
                    "123",
                    "--effort",
                    "456",
                    "--email",
                    "--merge",
                ],
            ] {
                let run = run_args(&args);
                assert_eq!(run.issue, IssueInput::Number(106.try_into().unwrap()));
                assert_eq!(run.flags.parallel, NonZeroUsize::new(2));
                assert_eq!(
                    run.flags.harness.model_and_effort.model.as_deref(),
                    Some("123")
                );
                assert_eq!(
                    run.flags.harness.model_and_effort.effort.as_deref(),
                    Some("456")
                );
                assert_eq!(run.flags.email, Some(NotificationAsk::Send(None)));
                assert_eq!(run.flags.goal, Some(Goal::Merged));
            }
        }
        for args in [
            vec!["model", "106"],
            vec!["effort", "106"],
            vec!["parallel", "106"],
        ] {
            assert_eq!(rejection(&args), "missing Issue URL");
        }
        let run = run_args(&["18446744073709551615"]);
        assert_eq!(run.issue, IssueInput::Number(u64::MAX.try_into().unwrap()));
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
                vec!["architect", "email-unless-merged", "the Spec run"],
                Some(NotificationAsk::SendUnlessMerged(None)),
                focus(),
            ),
            (
                vec![
                    "architect",
                    "--plan-only",
                    "--email-unless-merged",
                    "me@example.com",
                ],
                Some(NotificationAsk::SendUnlessMerged(Some(
                    "me@example.com".to_string(),
                ))),
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
                "email, email-unless-merged and no-email can't be used together",
            ),
            (
                vec!["architect", "no-email", "--plan-only", "--email"],
                "email, email-unless-merged and no-email can't be used together",
            ),
            (
                vec!["architect", "--email", "me@example.com", "email"],
                "repeated argument: email",
            ),
            (
                vec!["architect", "email", "email-unless-merged"],
                "email, email-unless-merged and no-email can't be used together",
            ),
            (
                vec!["architect", "--email-unless-merged", "no-email"],
                "email, email-unless-merged and no-email can't be used together",
            ),
            (
                vec!["architect", "email-unless-merged", "--email-unless-merged"],
                "repeated argument: --email-unless-merged",
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
    fn pickup_args(args: &[&str]) -> PassArgs {
        match parse_strs(args) {
            Ok(Command::Pickup(pickup_args)) => pickup_args,
            Ok(_) => panic!("{args:?}: not a Pickup run"),
            Err(error) => panic!("{args:?}: {error:#}"),
        }
    }

    #[test]
    fn pickup_takes_each_of_its_flags_with_or_without_dashes_in_any_order() {
        let none = PassArgs {
            base: None,
            flags: Flags::default(),
            order: None,
        };
        assert_eq!(pickup_args(&["pickup"]), none);
        let all = PassArgs {
            base: Some("develop".to_string()),
            flags: Flags {
                security_fix: None,
                security_close: None,
                security_review: None,
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
            order: None,
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
        let cautious = PassArgs {
            base: None,
            flags: Flags {
                security_fix: None,
                security_close: None,
                security_review: None,
                goal: Some(Goal::ReadyForReview),
                email: Some(NotificationAsk::Skip),
                parallel: None,
                base_fix: Some(BaseFixAsk::Forbid),
                harness: harness::Asked::default(),
            },
            order: None,
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
    fn pickup_takes_an_order_word_with_or_without_dashes() {
        for (word, expected) in [
            ("lowest-first", PickupOrder::LowestFirst),
            ("--lowest-first", PickupOrder::LowestFirst),
            ("highest-first", PickupOrder::HighestFirst),
            ("--highest-first", PickupOrder::HighestFirst),
        ] {
            assert_eq!(
                pickup_args(&["pickup", word]).order,
                Some(expected),
                "{word}"
            );
        }
        assert_eq!(pickup_args(&["pickup"]).order, None);
        assert_eq!(
            pickup_args(&["pickup", "highest-first", "merge"]).order,
            Some(PickupOrder::HighestFirst)
        );
    }

    #[test]
    fn pickup_rejects_repeated_and_contradictory_order_words() {
        for (args, error) in [
            (
                vec!["pickup", "highest-first", "lowest-first"],
                "lowest-first and highest-first can't be used together",
            ),
            (
                vec!["pickup", "lowest-first", "--lowest-first"],
                "repeated argument: --lowest-first",
            ),
            (
                vec!["pickup", "highest-first", "highest-first"],
                "repeated argument: highest-first",
            ),
        ] {
            assert_eq!(rejection(&args), error, "{args:?}");
        }
    }

    #[test]
    fn order_words_are_rejected_after_secure_and_an_issue_url() {
        for (args, error) in [
            (
                vec!["secure", "highest-first"],
                "unexpected argument after secure: highest-first",
            ),
            (
                vec!["secure", "--lowest-first"],
                "unexpected argument after secure: --lowest-first",
            ),
            (
                vec![URL, "highest-first"],
                "unexpected argument after the Issue URL: highest-first",
            ),
        ] {
            assert_eq!(rejection(&args), error, "{args:?}");
        }
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
                "email, email-unless-merged and no-email can't be used together",
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
            assert_eq!(
                run_args.issue,
                IssueInput::Url(IssueUrl::parse(URL).unwrap()),
                "{args:?}"
            );
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
            assert_eq!(
                run_args.issue,
                IssueInput::Url(IssueUrl::parse(URL).unwrap()),
                "{args:?}"
            );
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
