//! A Run in a child `thirdshift`, started from the same Launch directory by a
//! Spec run for one of its Tickets (ADR-0006) or by a Run for its Base fix
//! (ADR-0008).

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};

use crate::args;
use crate::failed_run;
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::progress;

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
    /// It reached its goal, with its PR as it printed it.
    Reached(Option<String>),
    /// Why, and its session log, as it reported them.
    Failed { cause: String, log: Option<String> },
    /// Ended by an interrupt passed on to it.
    Interrupted,
}

/// Start a Run of `kind` on `issue` in a child `thirdshift`, from the same
/// Launch directory. With `base_fix`, it is given `base-fix`, so it may start
/// a Base fix.
pub fn start(issue: &IssueUrl, kind: &Kind, base_fix: bool) -> Result<Child> {
    Command::new(std::env::current_exe().context("no thirdshift executable")?)
        .args([kind.hidden_argument(), kind.base()])
        .args(base_fix.then_some("base-fix"))
        .arg(&issue.url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not start the Run for #{}", issue.number))
}

/// Relay the stderr of `child`, the Run for issue `number`, with a
/// `#<number>: ` prefix until it exits. An interrupt is passed on to the
/// child, which is waited for as it goes down its Failed run path, and it
/// ended `Interrupted`. Otherwise it reached its goal if it exits 0, and
/// failed otherwise, with the cause and session log it ended on.
pub fn wait(number: u64, mut child: Child) -> Result<Ended> {
    // Relay on its own thread, so this one can watch for an interrupt.
    let stderr = child.stderr.take().context("no stderr from the Run")?;
    // A failed Run ends on its error, then its session log if it has one.
    let relay = thread::spawn(move || -> std::io::Result<[Option<String>; 2]> {
        let mut last_lines: [Option<String>; 2] = [None, None];
        for line in BufReader::new(stderr).lines() {
            let line = line?;
            let message = progress::relay(format_args!("#{number}"), &line).to_string();
            last_lines = [last_lines[1].take(), Some(message)];
        }
        Ok(last_lines)
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
    let last_lines = relay
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
    if status.success() {
        return Ok(Ended::Reached(stdout.lines().last().map(String::from)));
    }
    if interrupt::requested() {
        return Ok(Ended::Interrupted);
    }
    let [before, last] = last_lines;
    let (cause, log) = match last {
        Some(last) => match last.strip_prefix(failed_run::SESSION_LOG) {
            Some(log) => (before, Some(log.to_string())),
            None => (Some(last), None),
        },
        None => (None, None),
    };
    Ok(Ended::Failed {
        cause: cause.unwrap_or_else(|| format!("the Run {status}")),
        log,
    })
}
