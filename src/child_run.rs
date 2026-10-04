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

use anyhow::{Context, Result, anyhow, bail};

use crate::base_fix::BaseFixAsk;
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::logs;
use crate::progress;
use crate::run_ending;

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

impl Given {
    /// The hidden arguments that give a child Run this, as [`Reader`] reads
    /// them back.
    pub fn to_args(&self) -> Vec<&str> {
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
                if self.kind.is_some() {
                    bail!("repeated argument: {arg}");
                }
                let base = value("Base branch")?;
                self.kind = Some(if arg == SPEC_BRANCH {
                    Kind::Ticket { spec_branch: base }
                } else {
                    Kind::BaseFix { base }
                });
            }
            STAMP => {
                if self.stamp.is_some() {
                    bail!("repeated argument: {arg}");
                }
                self.stamp = Some(value("stamp")?);
            }
            ALLOW_BASE_FIX | OFFER_BASE_FIX => {
                if self.base_fix.is_some() {
                    bail!("repeated argument: {arg}");
                }
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
    /// it is one. A kind needs a stamp, and the other hidden arguments a
    /// kind.
    pub fn finish(self) -> Result<Option<Given>> {
        let Some(kind) = self.kind else {
            if self.stamp.is_some() || self.base_fix.is_some() {
                bail!("only a child Run is given {STAMP}, {ALLOW_BASE_FIX} or {OFFER_BASE_FIX}");
            }
            return Ok(None);
        };
        let stamp = self.stamp.with_context(|| format!("missing {STAMP}"))?;
        Ok(Some(Given {
            kind,
            stamp,
            base_fix: self.base_fix.unwrap_or(BaseFixAsk::Forbid),
        }))
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

/// A child Run that has started, on its issue.
pub struct Handle {
    /// Its issue's number.
    number: u64,
    child: Child,
}

/// Start a Run of `kind` on `issue` in a child `thirdshift`, from the same
/// Launch directory, given this command's start stamp for its Session logs,
/// and asked `base_fix` about a Base fix.
pub fn start(issue: &IssueUrl, kind: Kind, base_fix: BaseFixAsk) -> Result<Handle> {
    let given = Given {
        kind,
        stamp: logs::stamp().to_string(),
        base_fix,
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
    Ok(Handle {
        number: issue.number,
        child,
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

impl Handle {
    /// Relay the stderr of the child Run with a `#<number>: ` prefix, its
    /// issue's number, until it exits. An interrupt is passed on to the
    /// child, which is waited for as it goes down its Failed run path, and it
    /// ended `Interrupted`. Otherwise it reached its goal if it exits 0, with
    /// the Base fix it reported, and failed otherwise, with the cause and
    /// session log it showed, or with its exit status as the cause if it showed
    /// none.
    pub fn wait(self) -> Result<Ended> {
        let Handle { number, mut child } = self;
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
}

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
        for kind in [ticket(), base_fix()] {
            for base_fix in [BaseFixAsk::Allow, BaseFixAsk::Forbid, offer.clone()] {
                let given = Given {
                    kind: kind.clone(),
                    stamp: STAMPED.to_string(),
                    base_fix,
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
                    &["--allow-base-fix", "--offer-base-fix", "x"],
                ],
                "repeated argument: --offer-base-fix",
            ),
            (vec![&[URL, "--spec-branch"]], "missing Base branch"),
            (vec![&[URL, "--base-fix-into"]], "missing Base branch"),
            (vec![&ticket, &[URL, "--stamp"]], "missing stamp"),
            (
                vec![&ticket, &stamp, &[URL, "--offer-base-fix"]],
                "missing command to offer",
            ),
            (vec![&ticket, &[URL]], "missing --stamp"),
            (vec![&["--base-fix-into", "main", URL]], "missing --stamp"),
            (
                vec![&stamp, &[URL]],
                "only a child Run is given --stamp, --allow-base-fix or --offer-base-fix",
            ),
            (
                vec![&[URL, "--allow-base-fix"]],
                "only a child Run is given --stamp, --allow-base-fix or --offer-base-fix",
            ),
            (
                vec![&[URL, "--offer-base-fix", "x"]],
                "only a child Run is given --stamp, --allow-base-fix or --offer-base-fix",
            ),
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
