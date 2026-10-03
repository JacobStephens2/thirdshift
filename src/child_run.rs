//! A Run in a child `thirdshift`, started from the same Launch directory by a
//! Spec run for one of its Tickets (ADR-0006) or by a Run for its Base fix
//! (ADR-0008).

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};

use crate::args;
use crate::base_fix::BaseFixAsk;
use crate::command_log;
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::progress;
use crate::run_ending;

/// How often to check whether a child Run has ended, or been interrupted.
const POLL: Duration = Duration::from_millis(100);

/// What a child Run is, with the Base branch it is given: a Merge run into
/// that branch that sends no Run notification and leaves the Launch directory
/// alone.
#[derive(Debug, PartialEq, Eq)]
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
            Kind::Ticket { .. } => args::SPEC_BRANCH,
            Kind::BaseFix { .. } => args::BASE_FIX_INTO,
        }
    }
}

/// How a child Run ended.
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

/// Start a Run of `kind` on `issue` in a child `thirdshift`, from the same
/// Launch directory, given this command's start stamp for its Session logs. If `base_fix` allows one, it is given `base-fix`, so it
/// may start a Base fix; if nobody decided, it is given the command to offer
/// one with.
pub fn start(issue: &IssueUrl, kind: &Kind, base_fix: &BaseFixAsk) -> Result<Child> {
    start_from(&own_executable()?, issue, kind, base_fix)
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
fn start_from(
    executable: &Path,
    issue: &IssueUrl,
    kind: &Kind,
    base_fix: &BaseFixAsk,
) -> Result<Child> {
    let base_fix = match base_fix {
        BaseFixAsk::Allow => vec![args::BASE_FIX],
        BaseFixAsk::Forbid => Vec::new(),
        BaseFixAsk::Undecided { retry } => vec![args::OFFER_BASE_FIX, retry],
    };
    let mut command = Command::new(executable);
    // On Linux `executable` is a link, and the child goes by this process's
    // command instead.
    if cfg!(target_os = "linux")
        && let Some(own) = own_command()
    {
        command.arg0(own);
    }
    command
        .args([kind.hidden_argument(), kind.base()])
        .args([args::STAMP, command_log::stamp()])
        .args(base_fix)
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
        })
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

/// Relay the stderr of `child`, the Run for issue `number`, with a
/// `#<number>: ` prefix until it exits. An interrupt is passed on to the
/// child, which is waited for as it goes down its Failed run path, and it
/// ended `Interrupted`. Otherwise it reached its goal if it exits 0, with
/// the Base fix it reported, and failed otherwise, with the cause and
/// session log it showed, or with its exit status as the cause if it showed
/// none.
pub fn wait(number: u64, mut child: Child) -> Result<Ended> {
    // Relay on its own thread, so this one can watch for an interrupt.
    let stderr = child.stderr.take().context("no stderr from the Run")?;
    let relay = thread::spawn(move || -> std::io::Result<_> {
        let mut reader = run_ending::Reader::default();
        for line in BufReader::new(stderr).lines() {
            reader.read(progress::relay(number, &line?));
        }
        Ok(reader)
    });
    let mut passed_on = false;
    let status = loop {
        // The child shares the process group, so a Ctrl-C or a closed
        // terminal reaches it too, but a signal sent to this process alone
        // doesn't. A second one is harmless: it only records the interrupt.
        if !passed_on && interrupt::requested() {
            // SAFETY: kill has no memory-safety preconditions, and the child
            // is not yet reaped, so its pid is still its own.
            unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
            passed_on = true;
        }
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("could not wait for the Run for #{number}"))?
        {
            break status;
        }
        thread::sleep(POLL);
    };
    let reader = relay
        .join()
        .map_err(|_| anyhow!("the relay of the Run for #{number} panicked"))?
        .with_context(|| format!("could not read the Run for #{number}"))?;
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .context("no stdout from the Run")?
        .read_to_string(&mut stdout)
        .with_context(|| format!("could not read the Run for #{number}"))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Why Ticket #248's Run could not be started from `executable`.
    fn cause_of_not_starting_from(executable: &Path) -> String {
        let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/248").unwrap();
        let kind = Kind::Ticket {
            spec_branch: "issue-237".to_string(),
        };
        let error = start_from(executable, &issue, &kind, &BaseFixAsk::Forbid).unwrap_err();
        format!("{error:#}")
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
