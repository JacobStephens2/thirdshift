//! A command's lifecycle, from its start to its exit code, for every command
//! that does factory work: a Run or Spec run started on an Issue URL, an
//! Architect run or a Pickup run. A child Run goes through it too, as part
//! of the command that started it.
//!
//! A command [`start`]s: it begins its record in the logs, loads the User
//! config, offering Setup, installs the interrupt handler and checks its Run
//! notification, in that order. It does its work, then [`Started::finish`]es
//! with its [`Ending`]: the Activity log's end line, the terminal display and
//! the Run notification, in that order, none of the first and last for a
//! skipped pass.

use std::process::ExitCode;

use crate::architect::Reviewed;
use crate::config::{self, UserConfig};
use crate::failed_run::FailedRun;
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::logs::{self, Begin};
use crate::notification::{About, NotificationAsk, RunNotification};
use crate::progress;
use crate::run::Ended;
use crate::run_ending;

/// How a command that did factory work, or was skipped before any, ended.
pub enum Ending {
    /// A Run or a Spec run ended: started by hand, or dispatched by a Pickup
    /// run.
    Run(Ended),
    /// An Architect run got past its skip checks: how its Architecture
    /// review ended, and how the run it dispatched its plan as ended, if it
    /// dispatched one.
    Architect {
        review: Result<Reviewed, FailedRun>,
        dispatched: Option<Ended>,
    },
    /// A pass, an Architect run or a Pickup run, was skipped before any
    /// work.
    Skipped(Skip),
}

/// Why a pass was skipped, and what it prints.
pub struct Skip {
    /// The line that says why, on stderr.
    pub reason: String,
    /// The URLs it puts on stdout, a line each: those of the issues it was
    /// skipped for, if any.
    pub urls: Vec<String>,
}

/// A command started, with its Run notification checked if it asked for
/// one, to finish with its [`Ending`].
pub struct Started {
    notification: Option<RunNotification>,
}

/// Start a command that begins in the logs as `begin` says: load the User
/// config, after offering Setup where there is none; install the interrupt
/// handler; then check the Run notification that `ask` of that config asks
/// for, if any, about `about`. Returns the User config, for the command's
/// own asks, and the command to finish, or, if any of that fails, the
/// failure to exit with, its error reported.
pub fn start(
    begin: Begin,
    ask: impl FnOnce(&UserConfig) -> NotificationAsk,
    about: About,
) -> Result<(UserConfig, Started), ExitCode> {
    logs::begin(begin);
    let config = config::offer_setup()
        .and_then(|()| UserConfig::load())
        .map_err(|error| failure(&error))?;
    logs::configured(&config);
    // First, so no interrupt can end the command once its notification is
    // checked.
    interrupt::install().map_err(|error| failure(&error))?;
    let notification = match ask(&config) {
        NotificationAsk::Send(to) => {
            Some(RunNotification::new(to, &config.email, about).map_err(|error| failure(&error))?)
        }
        NotificationAsk::Skip => None,
    };
    Ok((config, Started { notification }))
}

impl Started {
    /// Take note that the Pickup run took the Ready issue `issue`, titled
    /// `title`, which its Run notification is then about.
    pub fn took(&mut self, issue: &IssueUrl, title: String) {
        if let Some(notification) = &mut self.notification {
            notification.took(issue, title);
        }
    }

    /// Finish the command that ended as `ending`: write the Activity log's
    /// end line, show how it ended, then send its Run notification, if it
    /// asked for one. A skipped pass only shows how it ended: its skip is
    /// recorded where it was decided, and it sends no notification. Returns
    /// its exit code.
    pub fn finish(self, ending: Ending) -> ExitCode {
        if matches!(ending, Ending::Skipped(_)) {
            return run_ending::show(&ending);
        }
        logs::ended(run_ending::summary(&ending));
        let code = run_ending::show(&ending);
        if let Some(notification) = self.notification {
            notification.send(&ending);
        }
        code
    }
}

/// `error` on stderr, after any lines a pass held from the terminal, and the
/// exit code of a failure.
pub fn failure(error: &anyhow::Error) -> ExitCode {
    logs::show_held();
    progress::step(format_args!("{error:#}"));
    ExitCode::FAILURE
}
