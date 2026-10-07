//! A Run in a child `thirdshift`, started from the same Launch directory by a
//! Spec run for one of its Tickets (ADR-0006) or by a Run for its Base fix
//! (ADR-0008). [`Runs`] owns concurrent children and their collective cleanup;
//! standalone [`start`] and [`Handle::wait`] serve Base fix.

mod runs;

pub use runs::Runs;

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
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
    /// A Ticket's Run in a Spec run, into the Spec branch.
    Ticket { spec_branch: String },
    /// A Base fix, into the Base branch of the Run that started it. It sees
    /// no Inherited failures and starts no Base fix of its own.
    BaseFix { base: String },
}

impl Kind {
    /// The Base branch the child Run is given.
    pub fn base(&self) -> &str {
        match self {
            Kind::Ticket { spec_branch } => spec_branch,
            Kind::BaseFix { base } => base,
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

/// What a child Run is given by the Run that starts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Given {
    /// What it is, with its Base branch.
    pub kind: Kind,
    /// The start stamp of the command that started it, which its Session
    /// logs take.
    pub stamp: String,
    /// What it is asked about a Base fix.
    pub base_fix: BaseFixAsk,
    /// The Harness, Model and Effort its sessions run on: the command's
    /// that started it.
    pub harness: Choice,
}

/// The hidden argument that makes a child Run a [`Kind::Ticket`], followed by
/// the Spec branch. None of the hidden arguments are in help: a child Run is
/// always the same binary as its parent (see [`own_executable`]), so only
/// this module writes them and reads them back.
const SPEC_BRANCH: &str = "--spec-branch";

/// The hidden argument that makes a child Run a [`Kind::BaseFix`], followed
/// by the Base branch of the Run that started it.
const BASE_FIX_INTO: &str = "--base-fix-into";

/// The hidden argument followed by the command's start stamp.
const STAMP: &str = "--stamp";

/// The hidden argument that asks a child Run [`BaseFixAsk::Allow`].
const ALLOW_BASE_FIX: &str = "--allow-base-fix";

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

impl Given {
    /// The hidden arguments that give a child Run this, as [`Reader`] reads
    /// them back.
    fn to_args(&self) -> Vec<&str> {
        let mut args = vec![
            self.kind.hidden_argument(),
            self.kind.base(),
            STAMP,
            &self.stamp,
        ];
        match &self.base_fix {
            BaseFixAsk::Allow => args.push(ALLOW_BASE_FIX),
            BaseFixAsk::Forbid => {}
            BaseFixAsk::Undecided { retry } => args.extend([OFFER_BASE_FIX, retry]),
        }
        args.extend([SESSIONS_HARNESS, self.harness.harness.name()]);
        if let Some(model) = &self.harness.model {
            args.extend([SESSIONS_MODEL, model]);
        }
        if let Some(effort) = &self.harness.effort {
            args.extend([SESSIONS_EFFORT, effort]);
        }
        args
    }
}

/// Reads what a child Run was given back from its arguments, one hidden
/// argument at a time, among the Issue URL and flags the argument parser
/// takes itself.
#[derive(Default)]
pub struct Reader {
    kind: Option<Kind>,
    stamp: Option<String>,
    base_fix: Option<BaseFixAsk>,
    harness: Option<Harness>,
    model: Option<String>,
    effort: Option<String>,
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
        let mut value = |missing: &str| match rest.next() {
            Some(value) => Ok(value.clone()),
            None => bail!("missing {missing}"),
        };
        match arg {
            SPEC_BRANCH | BASE_FIX_INTO => {
                not_yet_given(&self.kind, arg)?;
                let base = value("Base branch")?;
                self.kind = Some(if arg == SPEC_BRANCH {
                    Kind::Ticket { spec_branch: base }
                } else {
                    Kind::BaseFix { base }
                });
            }
            STAMP => {
                not_yet_given(&self.stamp, arg)?;
                self.stamp = Some(value("stamp")?);
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
            SESSIONS_HARNESS => {
                not_yet_given(&self.harness, arg)?;
                let name = value("Harness")?;
                let harness = Harness::named(&name)
                    .with_context(|| format!("{arg} must be followed by a Harness, not {name}"))?;
                self.harness = Some(harness);
            }
            SESSIONS_MODEL => {
                not_yet_given(&self.model, arg)?;
                self.model = Some(value("Model")?);
            }
            SESSIONS_EFFORT => {
                not_yet_given(&self.effort, arg)?;
                self.effort = Some(value("Effort")?);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// What the child Run was given, once every argument has been taken, if
    /// it is one. A kind needs a stamp and a Harness, and the other hidden
    /// arguments a kind.
    pub fn finish(self) -> Result<Option<Given>> {
        let Some(kind) = self.kind else {
            if self.stamp.is_some()
                || self.base_fix.is_some()
                || self.harness.is_some()
                || self.model.is_some()
                || self.effort.is_some()
            {
                bail!(
                    "only a child Run is given {STAMP}, {ALLOW_BASE_FIX}, {OFFER_BASE_FIX}, \
                     {SESSIONS_HARNESS}, {SESSIONS_MODEL} or {SESSIONS_EFFORT}"
                );
            }
            return Ok(None);
        };
        let stamp = self.stamp.with_context(|| format!("missing {STAMP}"))?;
        let harness = self
            .harness
            .with_context(|| format!("missing {SESSIONS_HARNESS}"))?;
        Ok(Some(Given {
            kind,
            stamp,
            base_fix: self.base_fix.unwrap_or(BaseFixAsk::Forbid),
            harness: Choice {
                harness,
                model: self.model,
                effort: self.effort,
                // Chosen by the command that started it, whose checks it
                // passed.
                chosen_by: ChosenBy::Command,
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
    /// It reached its goal, with its PR as it printed it, and what became
    /// of the Base fix it took, if any, as it reported it.
    Reached {
        pr_url: Option<String>,
        base_fix: Option<String>,
    },
    /// Why, and its session log, as it reported them.
    Failed { cause: String, log: Option<String> },
    /// Ended by an interrupt passed on to it.
    Interrupted,
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
/// Launch directory, given this command's start stamp for its Session logs,
/// asked `base_fix` about a Base fix, and running its sessions on `harness`.
pub fn start(
    issue: &IssueUrl,
    kind: Kind,
    base_fix: BaseFixAsk,
    harness: &Choice,
) -> Result<Handle> {
    let given = Given {
        kind,
        stamp: logs::stamp().to_string(),
        base_fix,
        harness: harness.clone(),
    };
    start_from(&own_executable()?, issue, &given)
}

/// The executable this process is running, as a child `thirdshift` is
/// started: the same binary, so the hidden arguments it is given never meet
/// another version.
///
/// On Linux that is the kernel's own link to it, which still reaches it once
/// the file at its install path has been replaced or removed, as an update
/// does while a Run is going. The path that link resolves to names no file by
/// then, so a child can't be started by it (#261). Elsewhere there is no such
/// link, and it is the executable's path.
fn own_executable() -> Result<PathBuf> {
    if cfg!(target_os = "linux") {
        Ok(PathBuf::from("/proc/self/exe"))
    } else {
        std::env::current_exe().context("no thirdshift executable")
    }
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
    // On Linux `executable` is a link, and the child goes by this process's
    // command instead.
    if cfg!(target_os = "linux")
        && let Some(own) = own_command()
    {
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
                "could not start the Run for #{} from {}{leads_to}",
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
/// path it was started by, which for a child Run on Linux is the link to the
/// running executable: every child Run would be an `exe`. Elsewhere a child
/// Run is started by the executable's path, and has its name already.
pub fn name_this_process() {
    #[cfg(target_os = "linux")]
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
            .with_context(|| format!("no stdout from the Run for #{number}"))?;
        #[cfg(test)]
        self.inject_fault(Fault::StdoutStart)?;
        #[cfg(test)]
        let fault = self
            .fault
            .take_if(|fault| matches!(fault, Fault::StdoutRead | Fault::StdoutPanic));
        self.stdout = Some(Worker::start(
            format!("stdout reader of the Run for #{number}"),
            move || {
                #[cfg(test)]
                if let Some(fault) = fault {
                    fault
                        .fire()
                        .with_context(|| format!("could not read the Run for #{number}"))?;
                }
                let mut text = String::new();
                stdout
                    .read_to_string(&mut text)
                    .with_context(|| format!("could not read the Run for #{number}"))?;
                Ok(text)
            },
        )?);
        #[cfg(test)]
        self.inject_fault(Fault::StderrPipe)?;
        let stderr = self
            .child
            .stderr
            .take()
            .with_context(|| format!("no stderr from the Run for #{number}"))?;
        #[cfg(test)]
        self.inject_fault(Fault::StderrStart)?;
        #[cfg(test)]
        let fault = self
            .fault
            .take_if(|fault| matches!(fault, Fault::StderrRead | Fault::StderrPanic));
        self.stderr = Some(Worker::start(
            format!("relay of the Run for #{number}"),
            move || {
                #[cfg(test)]
                if let Some(fault) = fault {
                    fault
                        .fire()
                        .with_context(|| format!("could not read the Run for #{number}"))?;
                }
                let mut reader = run_ending::Reader::default();
                let mut stderr = BufReader::new(stderr);
                let mut bytes = Vec::new();
                while stderr
                    .read_until(b'\n', &mut bytes)
                    .with_context(|| format!("could not read the Run for #{number}"))?
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
                self.fail(
                    error.context(format!("could not wait for the Run for #{}", self.number)),
                );
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
                    self.fail(
                        anyhow!(error)
                            .context(format!("could not wait for the Run for #{}", self.number)),
                    );
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
            self.fault
                .take()
                .unwrap()
                .fire()
                .with_context(|| format!("could not prepare the Run for #{}", self.number))?;
        }
        Ok(())
    }

    fn stop(&mut self) {
        if !self.reaped && !self.stop_sent {
            // The process group is shared. Signal only this unreaped child,
            // allowing its Failed run path to finish before reaping it.
            // SAFETY: kill has no memory-safety preconditions; this PID is
            // still owned and has not been reaped.
            unsafe { libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM) };
            self.stop_sent = true;
        }
        // Setup may have failed before either pipe got its reader.
        drop(self.child.stdout.take());
        drop(self.child.stderr.take());
    }

    fn ending(&mut self) -> Result<Ended> {
        if self.interrupted || (self.error.is_some() && interrupt::requested()) {
            return Ok(Ended::Interrupted);
        }
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        let status = self
            .status
            .expect("a reaped Run without a wait error has a status");
        let stdout = self.stdout.as_mut().unwrap().value.take().unwrap();
        let reader = self.stderr.as_mut().unwrap().value.take().unwrap();
        let ending = reader.finish(&stdout, status.success());
        Ok(match ending.outcome {
            Ok(pr_url) => Ended::Reached {
                pr_url,
                base_fix: ending.base_fix,
            },
            Err(_) if interrupt::requested() => Ended::Interrupted,
            Err(failure) => Ended::Failed {
                cause: failure.cause.unwrap_or_else(|| format!("the Run {status}")),
                log: failure.log,
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
            kind: ticket(),
            stamp: STAMPED.to_string(),
            base_fix: BaseFixAsk::Forbid,
            harness: Choice::default(),
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
        }
    }

    fn base_fix() -> Kind {
        Kind::BaseFix {
            base: "main".to_string(),
        }
    }

    /// What a child Run started with `args` reads back that it was given, as
    /// the argument parser takes each argument it doesn't recognise itself.
    fn read(args: &[&str]) -> Result<Option<Given>> {
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
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
        let harnesses = [on_claude, on_opus];
        for (kind, harness) in [ticket(), base_fix()]
            .into_iter()
            .zip(harnesses.iter().cycle())
        {
            for base_fix in [BaseFixAsk::Allow, BaseFixAsk::Forbid, offer.clone()] {
                let given = Given {
                    kind: kind.clone(),
                    stamp: STAMPED.to_string(),
                    base_fix,
                    harness: harness.clone(),
                };
                let written = given.to_args();
                // Before, between and after the hidden arguments, each of
                // which is a name and, but for `--allow-base-fix`, its value.
                for at in [0, 2, 4, written.len()] {
                    let mut args = written.clone();
                    args.insert(at, URL);
                    args.insert(at, "no-merge");

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
    const ONLY_A_CHILD_RUN: &str = "only a child Run is given --stamp, --allow-base-fix, \
        --offer-base-fix, --sessions-harness, --sessions-model or --sessions-effort";

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
            cause.starts_with("could not start the Run for #248 from /no/such/thirdshift: "),
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
                "could not start the Run for #248 from {} (/no/such/thirdshift (deleted)): ",
                link.display()
            )),
            "{cause}"
        );
    }
}
