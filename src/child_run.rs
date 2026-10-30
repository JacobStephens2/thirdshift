//! A Run in a child `thirdshift`, started from the same Launch directory by a
//! Spec run for one of its Tickets (ADR-0006) or by a Run for its Base fix
//! (ADR-0008). [`Runs`] owns concurrent children and their collective cleanup;
//! standalone [`start`] and [`Handle::wait`] serve Base fix.

mod asks;
mod runs;

pub use asks::ChildAsks;
pub use runs::Runs;

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};

use crate::base_fix::BaseFixAsk;
use crate::harness::{Choice, ChosenBy, Harness};
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::logs;
use crate::progress;
use crate::run_ending;

#[cfg(test)]
use execution_tests::Fault;

/// How often to check whether a child Run has ended, or been interrupted.
const POLL: Duration = Duration::from_millis(100);

/// What a child Run is, with the Base branch it is given: a Merge run into
/// that branch that sends no Run notification and leaves the Launch directory
/// alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// A Ticket's Run in a Spec run, into the Spec branch, with the Spec
    /// PR the Spec run had captured when it started the Ticket, if any.
    Ticket {
        spec_branch: String,
        spec_pr: Option<SpecPr>,
    },
    /// A Base fix, into the Base branch of the Run that started it, with
    /// that Run's Spec PR when it is a Ticket's Run. It sees no Inherited
    /// failures and starts no Base fix of its own.
    BaseFix {
        base: String,
        spec_pr: Option<SpecPr>,
    },
}

impl Kind {
    /// The Base branch the child Run is given.
    pub fn base(&self) -> &str {
        match self {
            Kind::Ticket { spec_branch, .. } => spec_branch,
            Kind::BaseFix { base, .. } => base,
        }
    }

    /// The Spec PR a Self-merge into a Spec branch depends on, if any.
    pub fn spec_pr(&self) -> Option<&SpecPr> {
        match self {
            Kind::Ticket { spec_pr, .. } | Kind::BaseFix { spec_pr, .. } => spec_pr.as_ref(),
        }
    }

    /// The hidden argument that says which kind the child Run is.
    fn hidden_argument(&self) -> &'static str {
        match self {
            Kind::Ticket { .. } => SPEC_BRANCH,
            Kind::BaseFix { .. } => BASE_FIX_INTO,
        }
    }
}

/// The Spec PR a Ticket's Self-merge into its Spec branch depends on: its
/// number, and the Base branch the Spec run chose for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecPr {
    pub number: u64,
    pub base: String,
}

/// What a child Run is given by the Run that starts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Given {
    /// The Ticket Process selected for this child; Base fixes use the Built-in.
    pub process: Option<String>,
    /// What it is, with its Base branch.
    pub kind: Kind,
    /// The start stamp of the command that started it, which its Session
    /// logs take.
    pub stamp: String,
    /// The owning command, whose folder its Session traces inherit.
    pub command: logs::CommandKind,
    /// When set, this child and the Runs it starts export no OTLP spans.
    /// Finding Steps set it so private records stay off a Process command's endpoint.
    pub suppress_otlp: bool,
    /// What it is asked about a Base fix.
    pub base_fix: BaseFixAsk,
    /// What the Run that started it hands on to every child Run.
    pub asks: ChildAsks,
}

/// The hidden argument that makes a child Run a [`Kind::Ticket`], followed by
/// the Spec branch. None of the hidden arguments are in help: a child Run is
/// always the same binary as its parent (see [`own_executable`]), so only
/// this module writes them and reads them back.
const SPEC_BRANCH: &str = "--spec-branch";

/// The hidden argument that gives a child Run its [`SpecPr`], followed
/// by the Spec PR's number and the Spec run's Base branch.
const SPEC_PR: &str = "--spec-pr";

/// The hidden argument that makes a child Run a [`Kind::BaseFix`], followed
/// by the Base branch of the Run that started it.
const BASE_FIX_INTO: &str = "--base-fix-into";

/// The hidden argument followed by the command's start stamp.
const STAMP: &str = "--stamp";

/// The hidden argument followed by the owning command's log folder.
const LOGS_COMMAND: &str = "--logs-command";

/// The hidden argument that keeps a child Run's sessions off the OTLP
/// endpoint. Finding Steps pass it; a Security run's children need no such
/// argument because their command kind already exports nothing.
const SUPPRESS_OTLP: &str = "--suppress-otlp";

/// The hidden argument that asks a child Run [`BaseFixAsk::Allow`].
const ALLOW_BASE_FIX: &str = "--allow-base-fix";

/// The hidden arguments that give a child Run [`ChildAsks::security_review`]
/// and [`ChildAsks::security_fix`].
const SECURITY_REVIEW: &str = "--run-security-review";
const SECURITY_REVIEW_DEFAULT: &str = "--security-review-default";
const SECURITY_REVIEW_DEFAULT_MODEL: &str = "--security-review-default-model";
const SECURITY_REVIEW_DEFAULT_EFFORT: &str = "--security-review-default-effort";
const ALLOW_SECURITY_FIX: &str = "--allow-security-fix";
const ALLOW_SECURITY_CLOSE: &str = "--allow-security-close";
const DELIVERY_LIMITS: &str = "--delivery-limits";
const TICKET_PROCESS: &str = "--ticket-process";

/// The hidden argument followed by the command that starts the Spec run
/// again with a Base fix allowed, for the child Run to offer: it asks
/// [`BaseFixAsk::Undecided`]. With neither this nor [`ALLOW_BASE_FIX`], a
/// child Run is asked [`BaseFixAsk::Forbid`].
const OFFER_BASE_FIX: &str = "--offer-base-fix";

/// The hidden argument followed by the Harness the child Run's sessions run
/// on. Not `--harness`, which is the `harness` flag with its dashes.
const SESSIONS_HARNESS: &str = "--sessions-harness";

/// The hidden argument followed by the Model the child Run's sessions run
/// on. Without it, the Model is left to the Harness.
const SESSIONS_MODEL: &str = "--sessions-model";

/// The hidden argument followed by the Effort the child Run's sessions run
/// on. Without it, the Effort is left to the Harness.
const SESSIONS_EFFORT: &str = "--sessions-effort";

/// The hidden argument followed by the Harness the child Run's Security
/// review runs on, when it is not [`SESSIONS_HARNESS`]'s.
const SECURITY_REVIEW_HARNESS: &str = "--security-review-harness";

/// The hidden argument followed by the Model the child Run's Security
/// review runs on, given only with [`SECURITY_REVIEW_HARNESS`].
const SECURITY_REVIEW_MODEL: &str = "--security-review-model";

/// The hidden argument followed by the Effort the child Run's Security
/// review runs on, given only with [`SECURITY_REVIEW_HARNESS`].
const SECURITY_REVIEW_EFFORT: &str = "--security-review-effort";

impl Given {
    /// The hidden arguments that give a child Run this, as [`Reader`] reads
    /// them back.
    fn to_args(&self) -> Vec<String> {
        let mut args = vec![
            self.kind.hidden_argument(),
            self.kind.base(),
            STAMP,
            &self.stamp,
            LOGS_COMMAND,
            self.command.folder(),
        ];
        match &self.base_fix {
            BaseFixAsk::Allow => args.push(ALLOW_BASE_FIX),
            BaseFixAsk::Forbid => {}
            BaseFixAsk::Undecided { retry } => args.extend([OFFER_BASE_FIX, retry]),
        }
        if self.asks.security_fix {
            args.push(ALLOW_SECURITY_FIX);
        }
        if self.asks.security_close {
            args.push(ALLOW_SECURITY_CLOSE);
        }
        if self.suppress_otlp {
            args.push(SUPPRESS_OTLP);
        }
        if self.asks.security_review {
            args.push(SECURITY_REVIEW);
        }
        if let Some(default) = &self.asks.security_review_default {
            append_choice(&mut args, default, DEFAULT_FLAGS);
        }
        append_choice(&mut args, &self.asks.harness, SESSION_FLAGS);
        if let Some(security) = &self.asks.security_review_harness {
            append_choice(&mut args, security, REVIEW_FLAGS);
        }
        let mut args: Vec<String> = args.into_iter().map(str::to_string).collect();
        if let Some(process) = &self.process {
            args.extend([TICKET_PROCESS.to_string(), process.clone()]);
        }
        if let Some(spec_pr) = self.kind.spec_pr() {
            args.extend([
                SPEC_PR.to_string(),
                spec_pr.number.to_string(),
                spec_pr.base.clone(),
            ]);
        }
        if self.asks.delivery_limits != crate::delivery::Limits::default() {
            args.extend([
                DELIVERY_LIMITS.to_string(),
                serde_json::to_string(&self.asks.delivery_limits)
                    .expect("Delivery limits serialize"),
            ]);
        }
        args
    }
}

const SESSION_FLAGS: [&str; 3] = [SESSIONS_HARNESS, SESSIONS_MODEL, SESSIONS_EFFORT];
const REVIEW_FLAGS: [&str; 3] = [
    SECURITY_REVIEW_HARNESS,
    SECURITY_REVIEW_MODEL,
    SECURITY_REVIEW_EFFORT,
];
const DEFAULT_FLAGS: [&str; 3] = [
    SECURITY_REVIEW_DEFAULT,
    SECURITY_REVIEW_DEFAULT_MODEL,
    SECURITY_REVIEW_DEFAULT_EFFORT,
];

fn append_choice<'a>(args: &mut Vec<&'a str>, choice: &'a Choice, flags: [&'static str; 3]) {
    args.extend([flags[0], choice.harness.name()]);
    for (flag, value) in [(flags[1], &choice.model), (flags[2], &choice.effort)] {
        if let Some(value) = value {
            args.extend([flag, value]);
        }
    }
}

/// One Harness choice while its hidden command arguments are being read.
#[derive(Default)]
struct PendingChoice {
    harness: Option<Harness>,
    model: Option<String>,
    effort: Option<String>,
}

impl PendingChoice {
    fn is_given(&self) -> bool {
        self.harness.is_some() || self.model.is_some() || self.effort.is_some()
    }

    fn take<'a>(
        &mut self,
        arg: &str,
        rest: &mut impl Iterator<Item = &'a String>,
        flags: [&str; 3],
    ) -> Result<bool> {
        let Some(at) = flags.iter().position(|flag| *flag == arg) else {
            return Ok(false);
        };
        let name = ["Harness", "Model", "Effort"][at];
        if at == 0 {
            not_yet_given(&self.harness, arg)?;
            let value = rest.next().with_context(|| format!("missing {name}"))?;
            self.harness =
                Some(Harness::named(value).with_context(|| {
                    format!("{arg} must be followed by a Harness, not {value}")
                })?);
        } else {
            let given = if at == 1 {
                &mut self.model
            } else {
                &mut self.effort
            };
            not_yet_given(given, arg)?;
            let value = rest.next().with_context(|| format!("missing {name}"))?;
            *given = Some(value.clone());
        }
        Ok(true)
    }

    fn finish(self, flags: [&str; 3], chosen_by: ChosenBy) -> Result<Option<Choice>> {
        if self.harness.is_none() && (self.model.is_some() || self.effort.is_some()) {
            bail!("{} and {} need {}", flags[1], flags[2], flags[0]);
        }
        Ok(self.harness.map(|harness| Choice {
            harness,
            model: self.model,
            effort: self.effort,
            chosen_by,
        }))
    }
}

/// Reads what a child Run was given back from its arguments, one hidden
/// argument at a time, among the Issue URL and flags the argument parser
/// takes itself.
#[derive(Default)]
pub struct Reader {
    process: Option<String>,
    delivery_limits: Option<crate::delivery::Limits>,
    kind: Option<Kind>,
    spec_pr: Option<SpecPr>,
    stamp: Option<String>,
    command: Option<logs::CommandKind>,
    base_fix: Option<BaseFixAsk>,
    security_review: bool,
    security_fix: bool,
    security_close: bool,
    suppress_otlp: bool,
    sessions: PendingChoice,
    review: PendingChoice,
    review_default: PendingChoice,
}

impl Reader {
    /// Take `arg`, with the value after it from `rest`, if it is one of the
    /// hidden arguments. False, taking nothing, if it is not. One that is
    /// repeated, or has no value after it, is an error.
    pub fn take<'a>(
        &mut self,
        arg: &str,
        rest: &mut impl Iterator<Item = &'a String>,
    ) -> Result<bool> {
        if self.sessions.take(arg, rest, SESSION_FLAGS)?
            || self.review.take(arg, rest, REVIEW_FLAGS)?
            || self.review_default.take(arg, rest, DEFAULT_FLAGS)?
        {
            return Ok(true);
        }
        let mut value = |missing: &str| match rest.next() {
            Some(value) => Ok(value.clone()),
            None => bail!("missing {missing}"),
        };
        match arg {
            TICKET_PROCESS => {
                not_yet_given(&self.process, arg)?;
                let name = value("Ticket Process")?;
                if !crate::args::one_path_component(&name) {
                    bail!("{TICKET_PROCESS} must name a Process");
                }
                self.process = Some(name);
            }
            DELIVERY_LIMITS => {
                not_yet_given(&self.delivery_limits, arg)?;
                let limits: crate::delivery::Limits =
                    serde_json::from_str(&value("Delivery limits")?)?;
                if limits.repair_cap > crate::delivery::MAX_REPAIRS {
                    bail!("Delivery limits cannot raise the Repair cap");
                }
                self.delivery_limits = Some(limits);
            }
            SECURITY_REVIEW => {
                if self.security_review {
                    bail!("repeated argument: {arg}");
                }
                self.security_review = true;
            }
            ALLOW_SECURITY_FIX => {
                if self.security_fix {
                    bail!("repeated argument: {arg}");
                }
                self.security_fix = true;
            }
            ALLOW_SECURITY_CLOSE => {
                if self.security_close {
                    bail!("repeated argument: {arg}");
                }
                self.security_close = true;
            }
            SUPPRESS_OTLP => {
                if self.suppress_otlp {
                    bail!("repeated argument: {arg}");
                }
                self.suppress_otlp = true;
            }
            SPEC_BRANCH | BASE_FIX_INTO => {
                not_yet_given(&self.kind, arg)?;
                let base = value("Base branch")?;
                self.kind = Some(if arg == SPEC_BRANCH {
                    Kind::Ticket {
                        spec_branch: base,
                        spec_pr: None,
                    }
                } else {
                    Kind::BaseFix {
                        base,
                        spec_pr: None,
                    }
                });
            }
            SPEC_PR => {
                not_yet_given(&self.spec_pr, arg)?;
                let number = value("Spec PR number")?;
                let number = number
                    .parse()
                    .ok()
                    .filter(|number| *number > 0)
                    .with_context(|| {
                        format!("{arg} must be followed by a PR number, not {number}")
                    })?;
                self.spec_pr = Some(SpecPr {
                    number,
                    base: value("Base branch")?,
                });
            }
            STAMP => {
                not_yet_given(&self.stamp, arg)?;
                self.stamp = Some(value("stamp")?);
            }
            LOGS_COMMAND => {
                not_yet_given(&self.command, arg)?;
                let name = value("logs command")?;
                self.command = Some(logs::CommandKind::named(&name).with_context(|| {
                    format!("{arg} must name issue, pickup, architect or secure, not {name}")
                })?);
            }
            ALLOW_BASE_FIX | OFFER_BASE_FIX => {
                if let Some(given) = &self.base_fix
                    && matches!(given, BaseFixAsk::Allow) != (arg == ALLOW_BASE_FIX)
                {
                    bail!("{ALLOW_BASE_FIX} and {OFFER_BASE_FIX} can't be used together");
                }
                not_yet_given(&self.base_fix, arg)?;
                self.base_fix = Some(if arg == ALLOW_BASE_FIX {
                    BaseFixAsk::Allow
                } else {
                    BaseFixAsk::Undecided {
                        retry: value("command to offer")?,
                    }
                });
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// What the child Run was given, once every argument has been taken, if
    /// it is one. A kind needs a stamp and a Harness, and the other hidden
    /// arguments a kind.
    pub fn finish(self) -> Result<Option<Given>> {
        let kind = match (self.kind, self.spec_pr) {
            (Some(Kind::Ticket { spec_branch, .. }), Some(spec_pr)) => Some(Kind::Ticket {
                spec_branch,
                spec_pr: Some(spec_pr),
            }),
            (Some(Kind::BaseFix { base, .. }), Some(spec_pr)) => Some(Kind::BaseFix {
                base,
                spec_pr: Some(spec_pr),
            }),
            (None, Some(_)) => bail!("only a child Run is given {SPEC_PR}"),
            (kind, None) => kind,
        };
        let Some(kind) = kind else {
            if self.process.is_some()
                || self.command.is_some()
                || self.delivery_limits.is_some()
                || self.stamp.is_some()
                || self.base_fix.is_some()
                || self.security_review
                || self.security_fix
                || self.security_close
                || self.suppress_otlp
                || self.sessions.is_given()
                || self.review.is_given()
                || self.review_default.is_given()
            {
                bail!(
                    "only a child Ticket run is given {STAMP}, {ALLOW_BASE_FIX}, {OFFER_BASE_FIX}, \
                     {SESSIONS_HARNESS}, {SESSIONS_MODEL}, {SESSIONS_EFFORT}, \
                     {SECURITY_REVIEW_HARNESS}, {SECURITY_REVIEW_MODEL}, \
                     {SECURITY_REVIEW_EFFORT}, {SECURITY_REVIEW_DEFAULT}, \
                     {SECURITY_REVIEW_DEFAULT_MODEL}, {SECURITY_REVIEW_DEFAULT_EFFORT}, {LOGS_COMMAND} or \
                     {SUPPRESS_OTLP}"
                );
            }
            return Ok(None);
        };
        let security_review_harness = self.review.finish(REVIEW_FLAGS, ChosenBy::SecurityConfig)?;
        let security_review_default = self
            .review_default
            .finish(DEFAULT_FLAGS, ChosenBy::UserConfig)?;
        let stamp = self.stamp.with_context(|| format!("missing {STAMP}"))?;
        let harness = self
            .sessions
            .finish(SESSION_FLAGS, ChosenBy::Command)?
            .with_context(|| format!("missing {SESSIONS_HARNESS}"))?;
        Ok(Some(Given {
            process: self.process,
            kind,
            stamp,
            command: self
                .command
                .with_context(|| format!("missing {LOGS_COMMAND}"))?,
            suppress_otlp: self.suppress_otlp,
            base_fix: self.base_fix.unwrap_or(BaseFixAsk::Forbid),
            asks: ChildAsks {
                delivery_limits: self.delivery_limits.unwrap_or_default(),
                harness,
                security_review_harness,
                security_review_default,
                security_fix: self.security_fix,
                security_close: self.security_close,
                security_review: self.security_review,
            },
        }))
    }
}

/// Fail if what the hidden argument `arg` gives was `given` already.
fn not_yet_given<T>(given: &Option<T>, arg: &str) -> Result<()> {
    if given.is_some() {
        bail!("repeated argument: {arg}");
    }
    Ok(())
}

/// How a child Run ended.
#[derive(Debug)]
pub enum Ended {
    /// Diagnosis completed, but the Ticket remains open needing information.
    NotReproduced { comment_url: String },
    /// A Custom Ticket Process ended successfully without delivering its Ticket.
    ArchitectureReview { reason: String },
    /// It reached its goal, with its PR as it printed it, and what became
    /// of the Base fix it took, if any, as it reported it.
    Reached {
        pr_url: Option<String>,
        base_fix: Option<String>,
    },
    /// Why, and its session trace, as it reported them.
    Failed {
        cause: String,
        log: Option<String>,
        local_work: Vec<String>,
    },
    /// Ended by an interrupt passed on to it.
    Interrupted { local_work: Vec<String> },
}

/// Own a child Run and both readers from spawn through reaping and joining.
/// A terminal result is delivered once; callers remove the handle then.
/// Dropping a live handle gracefully stops it and waits for cleanup.
pub struct Handle {
    /// Its issue's number.
    number: u64,
    child: Child,
    stdout: Option<Worker<String>>,
    stderr: Option<Worker<run_ending::Reader>>,
    status: Option<ExitStatus>,
    reaped: bool,
    stop_sent: bool,
    interrupted: bool,
    error: Option<anyhow::Error>,
    completed: bool,
    #[cfg(test)]
    fault: Option<Fault>,
}

/// Start a Run of `kind` on `issue` in a child `thirdshift`, from the same
/// Launch directory, given this command's start stamp for its Session traces,
/// asked `base_fix` about a Base fix, and handed `asks`.
pub fn start(
    issue: &IssueUrl,
    kind: Kind,
    base_fix: BaseFixAsk,
    asks: &ChildAsks,
) -> Result<Handle> {
    start_process(issue, kind, base_fix, asks, "ticket-run")
}

pub(super) fn start_process(
    issue: &IssueUrl,
    kind: Kind,
    base_fix: BaseFixAsk,
    asks: &ChildAsks,
    process: &str,
) -> Result<Handle> {
    let given = Given {
        process: (process != "ticket-run").then(|| process.to_string()),
        kind,
        stamp: logs::stamp(),
        command: logs::command_kind(),
        suppress_otlp: logs::otlp_suppressed(),
        base_fix,
        asks: asks.clone(),
    };
    start_from(own_executable(), issue, &given)
}

/// The executable this process is running, as a child `thirdshift` is
/// started: the same binary, so the hidden arguments it is given never meet
/// another version.
///
/// The kernel's own link still reaches it once
/// the file at its install path has been replaced or removed, as an update
/// does while a Run is going. The path that link resolves to names no file by
/// then, so a child can't be started by it (#261).
fn own_executable() -> &'static Path {
    Path::new("/proc/self/exe")
}

/// [`start`], with the `thirdshift` at `executable` as the child.
fn start_from(executable: &Path, issue: &IssueUrl, given: &Given) -> Result<Handle> {
    start_using(executable, issue, given, Handle::start_readers)
}

/// Keep the post-spawn setup under ownership, including failures and unwinding.
fn start_using(
    executable: &Path,
    issue: &IssueUrl,
    given: &Given,
    startup: impl FnOnce(&mut Handle) -> Result<()>,
) -> Result<Handle> {
    let mut command = Command::new(executable);
    logs::inherit_record(&mut command);
    crate::session::recording::Recording::capture().inherit_command(&mut command);
    // `executable` is a link, and the child goes by this process's
    // command instead.
    if let Some(own) = own_command() {
        command.arg0(own);
    }
    let child = command
        .args(given.to_args())
        .arg(&issue.url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            // Where `executable` is a link, the file it leads to says more.
            let leads_to = match std::fs::read_link(executable) {
                Ok(target) => format!(" ({})", target.display()),
                Err(_) => String::new(),
            };
            format!(
                "could not start the Ticket run for #{} from {}{leads_to}",
                issue.number,
                executable.display()
            )
        })?;
    let mut owned = Handle {
        number: issue.number,
        child,
        stdout: None,
        stderr: None,
        status: None,
        reaped: false,
        stop_sent: false,
        interrupted: false,
        error: None,
        completed: false,
        #[cfg(test)]
        fault: None,
    };
    if let Err(error) = startup(&mut owned) {
        owned.fail(error);
        return match owned.wait() {
            Err(error) => Err(error),
            Ok(_) => Err(anyhow!("interrupted")),
        };
    }
    Ok(owned)
}

/// The command this process was started as, its `argv[0]`.
fn own_command() -> Option<OsString> {
    std::env::args_os().next()
}

/// Name this process after the command it was started as, for `pgrep`,
/// `pkill` and `top`. The kernel names a process after the last part of the
/// path it was started by, which for a child Run is the link to the
/// running executable: every child Run would be an `exe`.
pub fn name_this_process() {
    if let Some(own) = own_command()
        && let Some(name) = Path::new(&own).file_name()
        && let Ok(name) = std::ffi::CString::new(name.as_encoded_bytes())
    {
        // SAFETY: PR_SET_NAME reads a NUL-terminated string, which `name`
        // is, and keeps only as much of it as a name holds.
        unsafe { libc::prctl(libc::PR_SET_NAME, name.as_ptr()) };
    }
}

impl Handle {
    fn start_readers(&mut self) -> Result<()> {
        let number = self.number;
        #[cfg(test)]
        self.inject_fault(Fault::StdoutPipe)?;
        let mut stdout = self
            .child
            .stdout
            .take()
            .with_context(|| format!("no stdout from the Ticket run for #{number}"))?;
        #[cfg(test)]
        self.inject_fault(Fault::StdoutStart)?;
        #[cfg(test)]
        let fault = self
            .fault
            .take_if(|fault| matches!(fault, Fault::StdoutRead | Fault::StdoutPanic));
        self.stdout = Some(Worker::start(
            format!("stdout reader of the Ticket run for #{number}"),
            move || {
                #[cfg(test)]
                if let Some(fault) = fault {
                    fault
                        .fire()
                        .with_context(|| format!("could not read the Ticket run for #{number}"))?;
                }
                let mut text = String::new();
                stdout
                    .read_to_string(&mut text)
                    .with_context(|| format!("could not read the Ticket run for #{number}"))?;
                Ok(text)
            },
        )?);
        #[cfg(test)]
        self.inject_fault(Fault::StderrPipe)?;
        let stderr = self
            .child
            .stderr
            .take()
            .with_context(|| format!("no stderr from the Ticket run for #{number}"))?;
        #[cfg(test)]
        self.inject_fault(Fault::StderrStart)?;
        #[cfg(test)]
        let fault = self
            .fault
            .take_if(|fault| matches!(fault, Fault::StderrRead | Fault::StderrPanic));
        self.stderr = Some(Worker::start(
            format!("relay of the Ticket run for #{number}"),
            move || {
                #[cfg(test)]
                if let Some(fault) = fault {
                    fault
                        .fire()
                        .with_context(|| format!("could not read the Ticket run for #{number}"))?;
                }
                let mut reader = run_ending::Reader::default();
                let mut stderr = BufReader::new(stderr);
                let mut bytes = Vec::new();
                while stderr
                    .read_until(b'\n', &mut bytes)
                    .with_context(|| format!("could not read the Ticket run for #{number}"))?
                    != 0
                {
                    if bytes.last() == Some(&b'\n') {
                        bytes.pop();
                        if bytes.last() == Some(&b'\r') {
                            bytes.pop();
                        }
                    }
                    let line = String::from_utf8_lossy(&bytes);
                    reader.read(progress::relay(number, &line));
                    bytes.clear();
                }
                Ok(reader)
            },
        )?);
        Ok(())
    }

    /// Poll the process and finished readers without waiting for unfinished
    /// work. Pending and stopping return None. A failure is returned only
    /// after graceful shutdown, reaping, and joining both readers.
    fn try_wait(&mut self) -> Option<Result<Ended>> {
        if self.completed || !self.poll_completion() {
            return None;
        }
        self.completed = true;
        Some(self.ending())
    }

    /// Drive the same supervision synchronously. An interrupt is forwarded
    /// once as SIGTERM; the child has unbounded time to save unfinished work.
    pub fn wait(mut self) -> Result<Ended> {
        loop {
            if let Some(ended) = self.try_wait() {
                return ended;
            }
            thread::sleep(POLL);
        }
    }

    fn poll_completion(&mut self) -> bool {
        if interrupt::requested() && !self.reaped {
            self.interrupted = true;
            self.stop();
        }
        if !self.reaped {
            #[cfg(test)]
            if let Err(error) = self.inject_fault(Fault::Wait) {
                self.fail(error.context(format!(
                    "could not wait for the Ticket run for #{}",
                    self.number
                )));
                return false;
            }
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    self.status = Some(status);
                    self.reaped = true;
                }
                Ok(None) => {}
                Err(error) => {
                    // ECHILD means ownership of the PID has already ended.
                    // Never signal a PID after the OS says it isn't our child.
                    self.reaped = error.raw_os_error() == Some(libc::ECHILD);
                    self.fail(anyhow!(error).context(format!(
                        "could not wait for the Ticket run for #{}",
                        self.number
                    )));
                }
            }
        }
        if let Some(worker) = &mut self.stdout
            && let Err(error) = worker.poll()
        {
            self.fail(error);
        }
        if let Some(worker) = &mut self.stderr
            && let Err(error) = worker.poll()
        {
            self.fail(error);
        }
        self.reaped
            && self.stdout.as_ref().is_none_or(Worker::finished)
            && self.stderr.as_ref().is_none_or(Worker::finished)
    }

    /// Preserve the first transport cause, even if stopping uncovers another.
    fn fail(&mut self, error: anyhow::Error) {
        self.error.get_or_insert(error);
        self.stop();
    }

    #[cfg(test)]
    fn inject_fault(&mut self, at: Fault) -> Result<()> {
        if self.fault == Some(at) {
            self.fault.take().unwrap().fire().with_context(|| {
                format!("could not prepare the Ticket run for #{}", self.number)
            })?;
        }
        Ok(())
    }

    fn stop(&mut self) {
        if !self.reaped && !self.stop_sent {
            // The process group is shared. Signal only this unreaped child,
            // allowing its Failed run path to finish before reaping it.
            let _ = crate::process::signal(self.child.id() as libc::pid_t, libc::SIGTERM);
            self.stop_sent = true;
        }
        // Setup may have failed before either pipe got its reader.
        drop(self.child.stdout.take());
        drop(self.child.stderr.take());
    }

    fn ending(&mut self) -> Result<Ended> {
        let local_work = self
            .stderr
            .as_ref()
            .and_then(|worker| worker.value.as_ref())
            .map(|reader| reader.local_work().to_vec())
            .unwrap_or_default();
        if self.interrupted || (self.error.is_some() && interrupt::requested()) {
            return Ok(Ended::Interrupted { local_work });
        }
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        let status = self
            .status
            .expect("a reaped Run without a wait error has a status");
        let stdout = self.stdout.as_mut().unwrap().value.take().unwrap();
        let reader = self.stderr.as_mut().unwrap().value.take().unwrap();
        let not_reproduced = reader.not_reproduced().map(String::from);
        let architecture_end = reader.architecture_end().map(String::from);
        let ending = reader.finish(&stdout, status.success());
        Ok(match ending.outcome {
            Ok(_) if not_reproduced.is_some() => Ended::NotReproduced {
                comment_url: not_reproduced.unwrap(),
            },
            Ok(_) if architecture_end.is_some() => Ended::ArchitectureReview {
                reason: architecture_end.unwrap(),
            },
            Ok(pr_url) => Ended::Reached {
                pr_url,
                base_fix: ending.base_fix,
            },
            Err(_) if interrupt::requested() => Ended::Interrupted { local_work },
            Err(failure) => Ended::Failed {
                cause: failure
                    .cause
                    .unwrap_or_else(|| format!("the Ticket run {status}")),
                log: failure.log,
                local_work,
            },
        })
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.completed {
            self.stop();
            while !self.poll_completion() {
                thread::sleep(POLL);
            }
            self.completed = true;
        }
    }
}

/// A reader's result stays owned after its finished thread is joined.
struct Worker<T> {
    thread: Option<JoinHandle<Result<T>>>,
    value: Option<T>,
    name: String,
}

impl<T: Send + 'static> Worker<T> {
    fn start(name: String, read: impl FnOnce() -> Result<T> + Send + 'static) -> Result<Self> {
        let thread = thread::Builder::new()
            .spawn(read)
            .with_context(|| format!("could not start the {name}"))?;
        Ok(Self {
            thread: Some(thread),
            value: None,
            name,
        })
    }
}

impl<T> Worker<T> {
    fn poll(&mut self) -> Result<()> {
        if self.thread.as_ref().is_some_and(JoinHandle::is_finished) {
            self.value = Some(self.thread.take().unwrap().join().map_err(|panic| {
                let cause = panic
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| panic.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown panic");
                anyhow!("the {} panicked: {cause}", self.name)
            })??);
        }
        Ok(())
    }

    fn finished(&self) -> bool {
        self.thread.is_none()
    }
}

#[cfg(test)]
mod execution_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// Why Ticket #248's Run could not be started from `executable`.
    fn cause_of_not_starting_from(executable: &Path) -> String {
        let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/248").unwrap();
        let given = Given {
            process: None,
            kind: ticket(),
            stamp: STAMPED.to_string(),
            command: logs::CommandKind::Issue,
            suppress_otlp: false,
            base_fix: BaseFixAsk::Forbid,
            asks: ChildAsks::default(),
        };
        let Err(error) = start_from(executable, &issue, &given) else {
            panic!("started from {}", executable.display());
        };
        format!("{error:#}")
    }

    const URL: &str = "https://github.com/acme/widgets/issues/248";
    const STAMPED: &str = "20261003T120000-0400";

    fn ticket() -> Kind {
        Kind::Ticket {
            spec_branch: "issue-237".to_string(),
            spec_pr: None,
        }
    }

    fn ticket_with_its_spec_pr() -> Kind {
        Kind::Ticket {
            spec_branch: "issue-237".to_string(),
            spec_pr: Some(SpecPr {
                number: 240,
                base: "main".to_string(),
            }),
        }
    }

    fn base_fix() -> Kind {
        Kind::BaseFix {
            base: "main".to_string(),
            spec_pr: None,
        }
    }

    fn base_fix_into_a_spec_branch() -> Kind {
        Kind::BaseFix {
            base: "issue-237".to_string(),
            spec_pr: Some(SpecPr {
                number: 240,
                base: "main".to_string(),
            }),
        }
    }

    /// What a child Run started with `args` reads back that it was given, as
    /// the argument parser takes each argument it doesn't recognise itself.
    fn read(args: &[impl AsRef<str>]) -> Result<Option<Given>> {
        let args: Vec<String> = args.iter().map(|arg| arg.as_ref().to_string()).collect();
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "a test helper that reads back a Run; any other Command fails the test"
        )]
        match crate::args::parse(&args)? {
            crate::args::Command::Run(run) => Ok(run.given),
            _ => panic!("{args:?}: not a Run"),
        }
    }

    #[test]
    fn what_a_child_run_is_given_reads_back_as_written_around_its_issue_url_and_a_flag() {
        let offer = BaseFixAsk::Undecided {
            retry: format!("thirdshift {URL} --no-merge base-fix"),
        };
        let on_claude = Choice {
            chosen_by: ChosenBy::Command,
            ..Choice::default()
        };
        let on_opus = Choice {
            model: Some("opus".to_string()),
            effort: Some("max".to_string()),
            ..on_claude.clone()
        };
        let reviewed_on_codex = Choice {
            harness: Harness::Codex,
            model: Some("gpt-6.1-sol".to_string()),
            effort: Some("max".to_string()),
            chosen_by: ChosenBy::SecurityConfig,
        };
        let reviewed_on_claude = Choice {
            chosen_by: ChosenBy::SecurityConfig,
            ..Choice::default()
        };
        let harnesses = [on_claude, on_opus];
        let reviews = [None, Some(reviewed_on_codex), Some(reviewed_on_claude)];
        let securities = [
            (false, false, false),
            (false, false, true),
            (true, false, false),
            (true, false, true),
            (true, true, false),
            (true, true, true),
        ];
        for ((kind, harness), security_review_harness) in [
            ticket(),
            ticket_with_its_spec_pr(),
            base_fix(),
            base_fix_into_a_spec_branch(),
        ]
        .into_iter()
        .zip(harnesses.iter().cycle())
        .flat_map(|kind| reviews.clone().map(|review| (kind.clone(), review)))
        {
            for (base_fix, (security_fix, security_close, security_review)) in
                [BaseFixAsk::Allow, BaseFixAsk::Forbid, offer.clone()]
                    .into_iter()
                    .flat_map(|base_fix| securities.map(|security| (base_fix.clone(), security)))
            {
                let given = Given {
                    process: None,
                    kind: kind.clone(),
                    stamp: STAMPED.to_string(),
                    command: logs::CommandKind::Issue,
                    suppress_otlp: false,
                    base_fix,
                    asks: ChildAsks {
                        delivery_limits: crate::delivery::Limits::default(),
                        harness: harness.clone(),
                        security_review_harness: security_review_harness.clone(),
                        security_review_default: security_review.then(|| Choice {
                            harness: Harness::Codex,
                            model: Some("gpt-6.1-sol".to_string()),
                            effort: Some("high".to_string()),
                            chosen_by: ChosenBy::UserConfig,
                        }),
                        security_fix,
                        security_close,
                        security_review,
                    },
                };
                let written = given.to_args();
                // Before, between and after the hidden arguments, each of
                // which is a name and, but for `--allow-base-fix`, its value.
                for at in [0, 2, 4, written.len()] {
                    let mut args = written.clone();
                    args.insert(at, URL.to_string());
                    args.insert(at, "no-merge".to_string());

                    assert_eq!(read(&args).unwrap(), Some(given.clone()), "{args:?}");
                }
            }
        }
    }

    #[test]
    fn a_run_given_no_hidden_argument_is_no_child_run() {
        assert_eq!(read(&[URL, "base-fix"]).unwrap(), None);
    }

    /// What a Run given a hidden argument only a child Run is given, without
    /// a kind, is rejected with.
    const ONLY_A_CHILD_RUN: &str = "only a child Ticket run is given --stamp, --allow-base-fix, \
        --offer-base-fix, --sessions-harness, --sessions-model, --sessions-effort, \
        --security-review-harness, --security-review-model, --security-review-effort, \
        --security-review-default, --security-review-default-model, --security-review-default-effort, \
        --logs-command or --suppress-otlp";

    #[test]
    fn hidden_arguments_that_are_repeated_have_no_value_or_lack_a_kind_or_stamp_are_rejected() {
        let stamp = ["--stamp", STAMPED];
        let ticket = ["--spec-branch", "issue-237"];
        for (args, error) in [
            (
                vec![&ticket[..], &ticket, &stamp],
                "repeated argument: --spec-branch",
            ),
            (
                vec![&ticket, &["--base-fix-into", "main"], &stamp],
                "repeated argument: --base-fix-into",
            ),
            (vec![&ticket, &stamp, &stamp], "repeated argument: --stamp"),
            (
                vec![&ticket, &stamp, &["--allow-base-fix", "--allow-base-fix"]],
                "repeated argument: --allow-base-fix",
            ),
            (
                vec![
                    &ticket,
                    &stamp,
                    &["--offer-base-fix", "x", "--offer-base-fix", "y"],
                ],
                "repeated argument: --offer-base-fix",
            ),
            (
                vec![
                    &ticket,
                    &stamp,
                    &["--allow-base-fix", "--offer-base-fix", "x"],
                ],
                "--allow-base-fix and --offer-base-fix can't be used together",
            ),
            (
                vec![
                    &ticket,
                    &stamp,
                    &["--offer-base-fix", "x", "--allow-base-fix"],
                ],
                "--allow-base-fix and --offer-base-fix can't be used together",
            ),
            (vec![&[URL, "--spec-branch"]], "missing Base branch"),
            (vec![&[URL, "--base-fix-into"]], "missing Base branch"),
            (vec![&ticket, &[URL, "--stamp"]], "missing stamp"),
            (
                vec![&ticket, &stamp, &[URL, "--offer-base-fix"]],
                "missing command to offer",
            ),
            (vec![&ticket, &[URL]], "missing --stamp"),
            (vec![&ticket, &stamp], "missing --sessions-harness"),
            (
                vec![&ticket, &stamp, &["--sessions-harness", "gemini"]],
                "--sessions-harness must be followed by a Harness, not gemini",
            ),
            (
                vec![
                    &ticket,
                    &stamp,
                    &["--sessions-model", "opus", "--sessions-model", "opus"],
                ],
                "repeated argument: --sessions-model",
            ),
            (vec![&[URL, "--sessions-effort", "max"]], ONLY_A_CHILD_RUN),
            (
                vec![&[URL, "--security-review-harness", "claude"]],
                ONLY_A_CHILD_RUN,
            ),
            (
                vec![
                    &ticket,
                    &stamp,
                    &[
                        "--sessions-harness",
                        "codex",
                        "--security-review-model",
                        "opus",
                    ],
                ],
                "--security-review-model and --security-review-effort need \
                 --security-review-harness",
            ),
            (
                vec![
                    &ticket,
                    &stamp,
                    &[
                        "--security-review-harness",
                        "claude",
                        "--security-review-harness",
                        "claude",
                    ],
                ],
                "repeated argument: --security-review-harness",
            ),
            (vec![&["--base-fix-into", "main", URL]], "missing --stamp"),
            (vec![&stamp, &[URL]], ONLY_A_CHILD_RUN),
            (vec![&[URL, "--allow-base-fix"]], ONLY_A_CHILD_RUN),
            (vec![&[URL, "--offer-base-fix", "x"]], ONLY_A_CHILD_RUN),
        ] {
            let mut args: Vec<&str> = args.concat();
            if !args.contains(&URL) {
                args.push(URL);
            }
            let rejection = match read(&args) {
                Ok(given) => panic!("{args:?}: not rejected, read {given:?}"),
                Err(error) => format!("{error:#}"),
            };
            assert_eq!(rejection, error, "{args:?}");
        }
    }

    #[test]
    fn a_child_run_that_cannot_be_started_names_the_executable_that_was_tried() {
        let cause = cause_of_not_starting_from(Path::new("/no/such/thirdshift"));

        assert!(
            cause.starts_with("could not start the Ticket run for #248 from /no/such/thirdshift: "),
            "{cause}"
        );
    }

    #[test]
    fn a_child_run_that_cannot_be_started_from_a_link_names_the_file_it_leads_to() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("exe");
        std::os::unix::fs::symlink("/no/such/thirdshift (deleted)", &link).unwrap();

        let cause = cause_of_not_starting_from(&link);

        assert!(
            cause.starts_with(&format!(
                "could not start the Ticket run for #248 from {} (/no/such/thirdshift (deleted)): ",
                link.display()
            )),
            "{cause}"
        );
    }
}
