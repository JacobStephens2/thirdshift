//! Headless agent sessions on the chosen Harness, and their logs.

use std::cell::{OnceCell, RefCell};
#[cfg(test)]
use std::fs::{self, File};
#[cfg(test)]
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(test)]
use anyhow::bail;
use anyhow::{Context, Result, anyhow};

#[cfg(test)]
pub(crate) use crate::harness::claude::claude_args;
#[cfg(test)]
pub(crate) use crate::harness::codex::{codex_args, codex_prompt};
use crate::harness::interpretation::{Ended, SafeguardRefusal, SessionFailure};
use crate::harness::{Adapter, Choice};
use crate::issue::{IssueUrl, Repo};
use crate::logs;
use crate::progress;
use crate::prompt;
use crate::skills;

pub mod kind;
pub(crate) mod limits;
pub(crate) mod private;
pub mod recording;
mod review_reports;
pub(crate) mod trace;
pub(crate) mod usage;
use review_reports::ReviewReports;

pub use kind::{Kind, Logged};

#[cfg(test)]
mod execution_tests;

/// The session's launch policy, retained by its Resume independently of
/// the label used for logs and progress.
#[derive(Clone, Copy)]
pub enum Purpose {
    Ordinary,
    CodeReview,
    Security,
    /// Public fix publishing uses Security's protocol without inherited
    /// GitHub credentials. Its Resume retains the same launch policy.
    SecurityPublishing,
    /// A guidance review retains security checks without delegating auditors.
    SecurityReview,
}

/// Where Session traces go: beside the owning Command log in
/// `commands/<command>/` under the repository's logs, under the User config's `logs.dir` or
/// `~/.thirdshift/logs`, each named for what is run and stamped with the
/// command's start stamp, which its Command log shares. Only once the logs
/// are [`logs::configured`].
pub struct Logs {
    /// What every log's name starts with: the issue's number for a Run,
    /// `architect` for an Architect run.
    name: String,
    dir: PathBuf,
    recording: OnceCell<recording::Recording>,
}

impl Logs {
    /// A Custom process Step, under its owning Command's log directory.
    pub fn of_process(repo: &Repo) -> Self {
        Logs {
            name: "process".to_string(),
            dir: logs::session_dir(repo),
            recording: OnceCell::new(),
        }
    }

    /// An isolated Trial's artifacts, with no Command owner or factory logs.
    pub fn of_trial(dir: &Path) -> Self {
        Logs {
            name: "trial".to_string(),
            dir: dir.to_path_buf(),
            recording: recording::Recording::for_trial().into(),
        }
    }

    /// The logs of the Run on `issue`.
    pub fn of_run(issue: &IssueUrl) -> Self {
        Logs {
            name: issue.number.to_string(),
            dir: logs::session_dir(&issue.repo()),
            recording: OnceCell::new(),
        }
    }

    /// The logs of the Architect run on `repo`.
    pub fn of_architect_run(repo: &Repo) -> Self {
        Logs {
            name: "architect".to_string(),
            dir: logs::session_dir(repo),
            recording: OnceCell::new(),
        }
    }

    /// The logs of the Security run on `repo`.
    pub fn of_security_run(repo: &Repo) -> Self {
        Logs {
            name: "secure".to_string(),
            dir: logs::session_dir(repo),
            recording: OnceCell::new(),
        }
    }

    /// Where a session's stream is logged:
    /// `<root>/commands/<command>/<name>-<stamp>~<token>-<kind>.logs.jsonl`.
    pub fn path(&self, session: Logged) -> Result<PathBuf> {
        self.recording()
            .session_path(&self.dir, &self.name, &session.token())
    }

    /// Runs construct Logs before work starts. Capture once the first Session
    /// needs it, after the Command has allocated its immutable owner.
    fn recording(&self) -> &recording::Recording {
        self.recording.get_or_init(recording::Recording::capture)
    }
}

/// Where a Run's or an Architect run's sessions run: in `worktree`, with the
/// Factory skills linked into it, on the Command's `harness`, each logged in
/// its `logs`. It lasts for the steps given to [`Sessions::within`].
pub struct Sessions<'a> {
    logs: &'a Logs,
    worktree: &'a Path,
    /// How each session is run, and where progress lines go. Held, not
    /// passed to each call as the other seams' are, since the steps reach
    /// it through the `&Sessions` they are given.
    outside: RefCell<Box<dyn Outside>>,
    /// What must still hold before each session starts, a Resume included:
    /// a session it fails is not started, and the step fails with it.
    before_each: Box<dyn Fn() -> Result<()> + 'a>,
    /// The session started last, a Resume included, and its log.
    last_log: RefCell<Option<(Logged, PathBuf)>>,
    /// How each session ended whose last ending, its Resume's if it got one,
    /// left background work running, as in "implement session ended with a
    /// background task still running (cargo test), which was killed", oldest
    /// first.
    endings_with_killed_work: RefCell<Vec<String>>,
}

impl<'a> Sessions<'a> {
    /// Link the Factory skills into `worktree`, then take `steps`, which run
    /// their sessions on `harness` through the `Sessions` they are given.
    /// Returns what `steps` came to, with the most recent session's log, if a
    /// session created it. If a session's last ending, its Resume's if it got
    /// one, left background work to be killed and `steps` then fail, the
    /// failure names that work ahead of its own cause: the session may have
    /// stopped short of its job.
    pub fn within<T>(
        logs: &'a Logs,
        worktree: &'a Path,
        harness: &'a Choice,
        steps: impl FnOnce(&Self) -> Result<T>,
    ) -> (Result<T>, Option<PathBuf>) {
        Self::checked_within(logs, worktree, harness, None, None, &|| Ok(()), steps)
    }

    /// [`Sessions::within`], with every session run in `sandbox`: the
    /// Trial's bwrap sandbox, prepared before its output directory's home.
    /// The Factory skills are linked as usual; the sandbox binds them in.
    pub fn within_sandboxed<T>(
        logs: &'a Logs,
        worktree: &'a Path,
        harness: &'a Choice,
        sandbox: &crate::sandbox::Prepared,
        steps: impl FnOnce(&Self) -> Result<T>,
    ) -> (Result<T>, Option<PathBuf>) {
        let adapter = harness.harness.adapter();
        if let Err(error) = skills::link_into(worktree, adapter)
            .and_then(|()| adapter.link_instruction_fallback(worktree))
        {
            return (Err(error), None);
        }
        let on_machine = OnMachine {
            worktree: worktree.to_path_buf(),
            harness: harness.clone(),
            adapter,
            security_review: None,
            security_review_default: None,
            recording: logs.recording().clone(),
            sandbox: Some(sandbox.launch()),
        };
        let (taken, log) = Sessions::taking(
            logs,
            worktree,
            Box::new(on_machine),
            crate::resources::wait,
            steps,
        );
        (taken, log.filter(|log| log.exists()))
    }

    /// [`Sessions::within`], checking `before_each` before each session
    /// starts, a Resume included: a session it fails is not started. A
    /// Security review, and its Resume, run on `security_review` instead of
    /// `harness`, when given, with its Factory skills linked too.
    pub fn checked_within<T>(
        logs: &'a Logs,
        worktree: &'a Path,
        harness: &'a Choice,
        security_review: Option<&'a Choice>,
        security_review_default: Option<&'a Choice>,
        before_each: &'a dyn Fn() -> Result<()>,
        steps: impl FnOnce(&Self) -> Result<T>,
    ) -> (Result<T>, Option<PathBuf>) {
        let adapter = harness.harness.adapter();
        let security_review =
            security_review.map(|choice| (choice.clone(), choice.harness.adapter()));
        let linked = std::iter::once(adapter)
            .chain(security_review.as_ref().map(|(_, adapter)| *adapter))
            .try_for_each(|adapter| {
                skills::link_into(worktree, adapter)?;
                adapter.link_instruction_fallback(worktree)
            });
        if let Err(error) = linked {
            return (Err(error), None);
        }
        let on_machine = OnMachine {
            worktree: worktree.to_path_buf(),
            harness: harness.clone(),
            adapter,
            security_review,
            security_review_default: security_review_default.cloned(),
            recording: logs.recording().clone(),
            sandbox: None,
        };
        let (taken, log) = Sessions::taking(
            logs,
            worktree,
            Box::new(on_machine),
            move || {
                crate::resources::wait()?;
                before_each()
            },
            steps,
        );
        (taken, log.filter(|log| log.exists()))
    }

    /// [`Sessions::within`], its sessions run through `outside`, once the
    /// Factory skills are linked. The log it returns is the most recent
    /// session's, whether or not the session created it.
    fn taking<T>(
        logs: &'a Logs,
        worktree: &'a Path,
        outside: Box<dyn Outside>,
        before_each: impl Fn() -> Result<()> + 'a,
        steps: impl FnOnce(&Self) -> Result<T>,
    ) -> (Result<T>, Option<PathBuf>) {
        let sessions = Sessions {
            logs,
            worktree,
            outside: RefCell::new(outside),
            before_each: Box::new(before_each),
            last_log: RefCell::default(),
            endings_with_killed_work: RefCell::default(),
        };
        let taken = steps(&sessions);
        let log = sessions.last_log.into_inner().map(|(_, log)| log);
        let endings = sessions.endings_with_killed_work.into_inner();
        if endings.is_empty() {
            return (taken, log);
        }
        let taken = taken.map_err(|error| {
            error.context(format!("{}, and a later step failed", endings.join("; ")))
        });
        (taken, log)
    }

    /// Run a session of `kind` given `prompt`, logged and labelled by its
    /// token. A session that ends with background work still running, which
    /// is killed with it, gets one Resume, as `<token>-resume`. If the Resume ends the
    /// same way, or the session has no id to resume, the work may have been
    /// abandoned rather than awaited: a progress line names it and this
    /// succeeds, leaving the steps that follow to decide the outcome.
    /// Returns the final message while retaining the prompt's launch policy.
    pub fn run(&self, kind: Kind, prompt: &str) -> Result<Option<String>> {
        // The shared review-report contract identifies code review sessions
        // before expansion. Retain that policy even if report setup fails.
        let purpose = if prompt.contains(prompt::REVIEW_REPORTS_DIRECTORY) {
            Purpose::CodeReview
        } else {
            Purpose::Ordinary
        };
        self.run_to_final_message(purpose, kind, prompt)
    }

    /// Run a session as [`Sessions::run`] does, and return its final message:
    /// what the agent said as it ended its last turn, its Resume's if it got
    /// one, if anything. `purpose` selects the launch policy for both the
    /// session and its Resume; `kind` names their logs, labels their
    /// progress and gives their Command record stage.
    pub fn run_to_final_message(
        &self,
        purpose: Purpose,
        kind: Kind,
        prompt: &str,
    ) -> Result<Option<String>> {
        self.run_to_end(purpose, kind, prompt)
            .map(|ended| ended.final_message)
    }

    /// The same session and Resume policy, retaining the Harness session ID
    /// for a Trial's result as well as its final message.
    pub fn run_to_end(&self, purpose: Purpose, kind: Kind, prompt: &str) -> Result<Ended> {
        // Progress lines name the session by its kind's token.
        let label = kind.token();
        let reports = if prompt.contains(prompt::REVIEW_REPORTS_DIRECTORY) {
            match ReviewReports::new(self.worktree) {
                Ok(reports) => Some(reports),
                Err(error) => {
                    self.step(format!(
                        "{label}: could not prepare review reports: {error:#}; carrying on"
                    ));
                    None
                }
            }
        } else {
            None
        };
        let first = self.start(purpose, Logged::of(kind), None, prompt, reports.as_ref());
        let kind = if let Err(error) = &first
            && kind == Kind::SecurityReview
            && matches!(purpose, Purpose::SecurityReview)
            && self.outside.borrow_mut().fallback_review(error)?
        {
            Kind::SecurityReviewFallback
        } else {
            kind
        };
        let mut ended = if kind == Kind::SecurityReviewFallback {
            self.start(purpose, Logged::of(kind), None, prompt, reports.as_ref())?
        } else {
            first?
        };
        // What ended last, as the progress line on its killed work calls it.
        let mut ended_last = "session";
        if let (false, Some(session_id)) = (ended.killed.is_empty(), &ended.session_id) {
            let mut resume_prompt = prompt::resume(&ended.killed_work());
            if reports.is_some() {
                resume_prompt.push_str(prompt::REVIEW_REPORTS);
            }
            self.step(format!(
                "{label}: background work was killed as the session ended; resuming it once"
            ));
            ended = self
                .start(
                    purpose,
                    Logged::resume_of(kind),
                    Some(session_id),
                    &resume_prompt,
                    reports.as_ref(),
                )
                .map_err(|mut error| {
                    // Work was observed before this Resume, even if its first
                    // turn fails.
                    if let Some(failure) = error.downcast_mut::<SessionFailure>() {
                        failure.before_first_turn = false;
                    }
                    error
                })?;
            ended_last = "Resume";
        }
        if !ended.killed.is_empty() {
            let ending = ending_with(&ended.killed_work());
            if matches!(
                purpose,
                Purpose::Security | Purpose::SecurityPublishing | Purpose::SecurityReview
            ) {
                return Err(anyhow!(
                    "{label}: the {ended_last} {ending}; Security session incomplete"
                ));
            }
            self.step(format!(
                "{label}: the {ended_last} {ending}; carrying on, as it may have been abandoned"
            ));
            self.endings_with_killed_work
                .borrow_mut()
                .push(format!("{label} session {ending}"));
        }
        Ok(ended)
    }

    /// Run `session`, logged under its own path, which becomes the last log,
    /// once `before_each` holds. Below here its token is only a label.
    fn start(
        &self,
        purpose: Purpose,
        session: Logged,
        resume: Option<&str>,
        prompt: &str,
        reports: Option<&ReviewReports>,
    ) -> Result<Ended> {
        (self.before_each)()?;
        let log = self.logs.path(session)?;
        logs::session(session, &log);
        let label = &session.token();
        self.step(format!("logging the session to {}", log.display()));
        *self.last_log.borrow_mut() = Some((session, log.clone()));
        let prompt = reports.map_or_else(
            || prompt.replace(prompt::REVIEW_REPORTS, ""),
            |reports| reports.prompt(prompt),
        );
        let ended = self
            .outside
            .borrow_mut()
            .run_session(purpose, label, resume, &prompt, &log);
        if let Some(reports) = reports {
            for line in reports.keep(&log) {
                self.step(format!("{label}: {line}"));
            }
        }
        ended
    }

    /// The log of the session started last, its Resume's if it got one, if
    /// that session is of `kind` and the log exists: the log a cause from
    /// that session is reported with.
    pub fn last_log_of(&self, kind: Kind) -> Option<PathBuf> {
        self.last_log
            .borrow()
            .as_ref()
            .filter(|(session, log)| {
                (session.kind == kind
                    || (kind == Kind::SecurityReview
                        && session.kind == Kind::SecurityReviewFallback))
                    && log.exists()
            })
            .map(|(_, log)| log.clone())
    }

    /// Hand on the progress line `line`.
    fn step(&self, line: String) {
        self.outside.borrow_mut().step(line);
    }
}

/// How the Resume rules run a session, and where their progress lines go.
trait Outside {
    /// Switch only a review that failed before any work to the default Harness.
    fn fallback_review(&mut self, _error: &anyhow::Error) -> Result<bool> {
        Ok(false)
    }
    /// Run a session given `prompt`, labelled by its kind's token `label`,
    /// continuing the session with id `resume`, if any, its stream logged to
    /// `log`. Returns what it ended with once it has exited cleanly, its turn
    /// not failed.
    fn run_session(
        &mut self,
        purpose: Purpose,
        label: &str,
        resume: Option<&str>,
        prompt: &str,
        log: &Path,
    ) -> Result<Ended>;
    /// Hand on the progress line `line`.
    fn step(&mut self, line: String);
}

/// Sessions run by the Command's `harness`'s CLI in `worktree`, where it
/// finds the Factory skills, and progress lines printed on stderr. A
/// Security review runs on `security_review`'s instead, if it is given.
struct OnMachine {
    worktree: PathBuf,
    harness: Choice,
    adapter: &'static dyn Adapter,
    security_review: Option<(Choice, &'static dyn Adapter)>,
    security_review_default: Option<Choice>,
    recording: recording::Recording,
    sandbox: Option<crate::sandbox::Launch>,
}

impl Outside for OnMachine {
    fn fallback_review(&mut self, error: &anyhow::Error) -> Result<bool> {
        crate::interrupt::check()?;
        let Some(failure) = error.downcast_ref::<SessionFailure>() else {
            return Ok(false);
        };
        let Some(reason) = failure
            .diagnostic
            .as_deref()
            .filter(|text| !text.trim().is_empty())
        else {
            return Ok(false);
        };
        let Some(default) = &mut self.security_review_default else {
            return Ok(false);
        };
        let review = self
            .security_review
            .as_ref()
            .map_or(&self.harness, |(choice, _)| choice);
        if !failure.before_first_turn
            || error.is::<SafeguardRefusal>()
            || review.harness == default.harness
        {
            return Ok(false);
        }
        crate::harness::warn_review_fallback(review.harness, default.harness, reason);
        default.check()?;
        let adapter = default.harness.adapter();
        skills::link_into(&self.worktree, adapter)?;
        adapter.link_instruction_fallback(&self.worktree)?;
        self.security_review = Some((default.clone(), adapter));
        Ok(true)
    }
    fn run_session(
        &mut self,
        purpose: Purpose,
        label: &str,
        resume: Option<&str>,
        prompt: &str,
        log: &Path,
    ) -> Result<Ended> {
        let (harness, adapter) = match (purpose, &self.security_review) {
            (Purpose::SecurityReview, Some((choice, adapter))) => {
                progress::step(format!(
                    "{label}: on {choice}, chosen by {}",
                    choice.chosen_by
                ));
                (choice, *adapter)
            }
            _ => (&self.harness, self.adapter),
        };
        let (mut invocation, policy) = match purpose {
            Purpose::Security | Purpose::SecurityPublishing | Purpose::SecurityReview => (
                adapter.security_session(
                    harness,
                    resume,
                    prompt,
                    matches!(purpose, Purpose::Security | Purpose::SecurityPublishing),
                ),
                recording::Policy::Security,
            ),
            Purpose::Ordinary => (
                adapter.session(harness, resume, prompt),
                recording::Policy::Ordinary,
            ),
            Purpose::CodeReview => (
                adapter.code_review_session(harness, resume, prompt),
                recording::Policy::Ordinary,
            ),
        };
        // A sandboxed session is always a Trial's: it runs with the Harness's
        // web search switched off, behind the sandbox's allowlisting proxy.
        if self.sandbox.is_some() && matches!(purpose, Purpose::CodeReview) {
            invocation = adapter.trial_session(harness, resume, prompt);
        }
        let mut command = match &self.sandbox {
            Some(launch) => {
                let mut command = launch.command(&invocation.args)?;
                command.current_dir(&self.worktree);
                command
            }
            None => {
                let mut command = Command::new(adapter.name());
                command.args(&invocation.args).current_dir(&self.worktree);
                command
            }
        };
        // Keep the empty config alive until the process and descendants
        // finish. A fresh one on each launch also fences the Resume.
        let github_config = if matches!(purpose, Purpose::SecurityPublishing) {
            let config = tempfile::Builder::new()
                .prefix("thirdshift-gh-")
                .tempdir()
                .context("could not prepare an unauthenticated GitHub config")?;
            for name in [
                "GH_TOKEN",
                "GITHUB_TOKEN",
                "GH_ENTERPRISE_TOKEN",
                "GITHUB_ENTERPRISE_TOKEN",
            ] {
                command.env_remove(name);
            }
            command
                .env("GH_CONFIG_DIR", config.path())
                .env("GH_PROMPT_DISABLED", "1");
            Some(config)
        } else {
            None
        };
        let ended = self.recording.run(
            label,
            harness,
            &mut command,
            recording::Input {
                prompt,
                policy,
                stdin: invocation.stdin.as_deref(),
                ..Default::default()
            },
            log,
        );
        drop(github_config);
        ended
    }

    fn step(&mut self, line: String) {
        progress::step(line);
    }
}

/// How a session ended whose `killed` background work, by description, was
/// still running: what follows "implement session" or "the Resume".
pub(crate) fn ending_with(killed: &[&str]) -> String {
    match killed {
        [task] => format!("ended with a background task still running ({task}), which was killed"),
        _ => format!(
            "ended with background tasks still running ({}), which were killed",
            killed.join("; ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::rc::Rc;

    use anyhow::anyhow;

    use super::*;

    #[test]
    fn unsafe_project_skills_fail_session_start_before_the_harness_can_run() {
        for harness in crate::harness::Harness::ALL {
            let worktree = crate::test_support::scratch_worktree();
            let outside = tempfile::tempdir().unwrap();
            let dir = worktree.path().join(harness.adapter().project_skills());
            fs::create_dir_all(dir.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(outside.path(), &dir).unwrap();
            let logs = Logs {
                name: "670".into(),
                dir: worktree.path().join("session-traces"),
                recording: OnceCell::new(),
            };
            let choice = Choice {
                harness,
                ..Choice::default()
            };
            let (result, log) =
                Sessions::within(&logs, worktree.path(), &choice, |_| -> Result<()> {
                    panic!("unsafe project skills must prevent session steps");
                });
            let error = result.unwrap_err();
            assert!(format!("{error:#}").contains(&dir.display().to_string()));
            assert!(log.is_none());
            assert!(!logs.dir.exists());
            assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        }
    }

    /// What the rules asked of the scripted [`Outside`], in order.
    #[derive(Debug, PartialEq)]
    enum Call {
        /// A session run, as `kind`, continuing `resume`, logged to `log`.
        Session {
            kind: String,
            resume: Option<String>,
            prompt: String,
            log: PathBuf,
        },
        /// A progress line handed on.
        Step(String),
    }

    /// Sessions that end as scripted, in turn, recording every call in
    /// `calls`.
    struct Scripted {
        endings: VecDeque<Result<Ended>>,
        calls: Rc<RefCell<Vec<Call>>>,
    }

    impl Outside for Scripted {
        fn run_session(
            &mut self,
            _purpose: Purpose,
            kind: &str,
            resume: Option<&str>,
            prompt: &str,
            log: &Path,
        ) -> Result<Ended> {
            self.calls.borrow_mut().push(Call::Session {
                kind: kind.to_string(),
                resume: resume.map(String::from),
                prompt: prompt.to_string(),
                log: log.to_path_buf(),
            });
            self.endings
                .pop_front()
                .expect("a session was run that was not scripted")
        }

        fn step(&mut self, line: String) {
            self.calls.borrow_mut().push(Call::Step(line));
        }
    }

    /// A session ending with id `session_id`, if any, with `killed` work
    /// and the final message `said`.
    fn ended(session_id: Option<&str>, killed: &[&str], said: &str) -> Result<Ended> {
        Ok(Ended {
            session_id: session_id.map(String::from),
            killed: killed.iter().map(|work| work.to_string()).collect(),
            final_message: Some(said.to_string()),
            completed: true,
        })
    }

    /// The logs of the Run on issue 7, under `/logs`.
    fn logs() -> Logs {
        Logs {
            name: "7".to_string(),
            dir: PathBuf::from("/logs/sessions"),
            recording: recording::Recording::new(
                Some(PathBuf::from("/logs/7-20261003T120000-0400~fixture.log")),
                None,
                "20261003T120000-0400".into(),
            )
            .into(),
        }
    }

    #[test]
    fn same_second_commands_keep_session_model_and_review_contents_separate() {
        let root = crate::test_support::scratch_worktree();
        let stamp = "20261003T120000-0400";
        let mut captured = Vec::new();
        for marker in ["first", "second"] {
            let recording = recording::Recording::new(
                Some(root.path().join(format!("7-{stamp}~{marker}.log"))),
                None,
                stamp.into(),
            );
            let logs = Logs {
                name: "7".into(),
                dir: root.path().into(),
                recording: recording.clone().into(),
            };
            let log = logs.path(Logged::of(Kind::Implementation)).unwrap();
            recording.run(
                "implement", &Choice::default(),
                Command::new("/bin/sh").args(["-c", &format!(
                    "echo '{{\"type\":\"assistant\",\"message\":{{\"model\":\"{marker}-model\",\"content\":[]}}}}'"
                )]).current_dir(root.path()), recording::Input::default(), &log,
            ).unwrap();
            let reports = ReviewReports::new(root.path()).unwrap();
            for axis in ["standards", "spec"] {
                fs::write(
                    Path::new(&reports.prompt(prompt::REVIEW_REPORTS_DIRECTORY))
                        .join(format!("{axis}.md")),
                    format!("{marker} {axis}"),
                )
                .unwrap();
            }
            assert!(reports.keep(&log).is_empty());
            captured.push((marker, log, recording));
        }
        for (marker, log, recording) in captured {
            assert!(
                fs::read_to_string(&log)
                    .unwrap()
                    .contains(&format!("{marker}-model")),
                "lost {marker} Session"
            );
            for axis in ["standards", "spec"] {
                assert_eq!(
                    fs::read_to_string(logs::session::original(&log).with_extension(axis)).unwrap(),
                    format!("{marker} {axis}")
                );
            }
            assert_eq!(
                recording.notification_lines(),
                [format!(
                    "- implement: claude · {marker}-model (reported) · session effort: default effort"
                )]
            );
        }
    }

    /// Take `steps` through sessions that end as `endings` script them.
    /// Returns what the steps came to, the last log, and every call made.
    fn take<T>(
        endings: Vec<Result<Ended>>,
        steps: impl FnOnce(&Sessions) -> Result<T>,
    ) -> (Result<T>, Option<PathBuf>, Vec<Call>) {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let scripted = Scripted {
            endings: endings.into(),
            calls: Rc::clone(&calls),
        };
        let logs = logs();
        let (taken, log) = Sessions::taking(
            &logs,
            Path::new("/unused"),
            Box::new(scripted),
            || Ok(()),
            steps,
        );
        let calls = calls.take();
        (taken, log, calls)
    }

    /// Where the session whose token is `token` is logged.
    fn log_of(token: &str) -> PathBuf {
        logs().path(Logged::read(token).unwrap()).unwrap()
    }

    /// The call that runs a session as `kind`, logged under its own path.
    fn session(kind: &str, resume: Option<&str>, prompt: &str) -> Call {
        Call::Session {
            kind: kind.to_string(),
            resume: resume.map(String::from),
            prompt: prompt.to_string(),
            log: log_of(kind),
        }
    }

    /// The progress line saying where a session as `kind` is logged.
    fn logging(kind: &str) -> Call {
        Call::Step(format!("logging the session to {}", log_of(kind).display()))
    }

    /// The sessions among `calls`.
    fn sessions_run(calls: &[Call]) -> Vec<&Call> {
        calls
            .iter()
            .filter(|call| matches!(call, Call::Session { .. }))
            .collect()
    }

    /// How a session that left `cargo test` running is said to have ended.
    const TESTS_KILLED: &str =
        "ended with a background task still running (cargo test), which was killed";

    #[test]
    fn a_clean_session_runs_once_and_returns_its_final_message() {
        let (taken, log, calls) = take(vec![ended(Some("s1"), &[], "done")], |sessions| {
            sessions.run_to_final_message(Purpose::Ordinary, Kind::Implementation, "do it")
        });

        assert_eq!(taken.unwrap(), Some("done".to_string()));
        assert_eq!(
            calls,
            [logging("implement"), session("implement", None, "do it")]
        );
        assert_eq!(log, Some(log_of("implement")));
    }

    #[test]
    fn a_session_whose_background_work_was_killed_is_resumed_once_on_its_id() {
        let (taken, log, calls) = take(
            vec![
                ended(Some("s1"), &["cargo test"], "waiting"),
                ended(Some("s1"), &[], "done"),
            ],
            |sessions| {
                sessions.run_to_final_message(Purpose::Ordinary, Kind::Implementation, "do it")
            },
        );

        assert_eq!(taken.unwrap(), Some("done".to_string()));
        assert_eq!(
            calls,
            [
                logging("implement"),
                session("implement", None, "do it"),
                Call::Step(
                    "implement: background work was killed as the session ended; resuming it once"
                        .to_string()
                ),
                logging("implement-resume"),
                session(
                    "implement-resume",
                    Some("s1"),
                    &prompt::resume(&["cargo test"])
                ),
            ]
        );
        assert_eq!(log, Some(log_of("implement-resume")));
    }

    #[test]
    fn a_security_session_cannot_claim_completion_with_killed_work() {
        for id in [None, Some("s1")] {
            let mut endings = vec![ended(
                id,
                &["cargo test"],
                "Security review: {\"unaddressed_count\":0,\"findings\":[]}",
            )];
            if id.is_some() {
                endings.push(ended(
                    id,
                    &["cargo test"],
                    "Security review: {\"unaddressed_count\":0,\"findings\":[]}",
                ));
            }
            let (taken, _, _) = take(endings, |sessions| {
                sessions.run_to_final_message(Purpose::Security, Kind::SecurityReview, "review")
            });
            assert!(
                taken.is_err(),
                "unfinished security work must hold Self-merge"
            );
        }
    }

    #[test]
    fn a_resume_that_ends_the_same_way_gets_no_second_and_a_later_failure_names_the_work() {
        let (taken, log, calls) = take(
            vec![
                ended(Some("s1"), &["cargo test"], "waiting"),
                ended(Some("s1"), &["cargo test"], "still waiting"),
            ],
            |sessions| -> Result<()> {
                assert_eq!(
                    sessions
                        .run_to_final_message(Purpose::Ordinary, Kind::Implementation, "do it")
                        .unwrap(),
                    Some("still waiting".to_string())
                );
                bail!("no PR found")
            },
        );

        assert_eq!(sessions_run(&calls).len(), 2, "one Resume, never two");
        assert_eq!(
            calls.last(),
            Some(&Call::Step(format!(
                "implement: the Resume {TESTS_KILLED}; carrying on, as it may have been abandoned"
            )))
        );
        assert_eq!(
            format!("{:#}", taken.unwrap_err()),
            format!("implement session {TESTS_KILLED}, and a later step failed: no PR found")
        );
        assert_eq!(log, Some(log_of("implement-resume")));
    }

    #[test]
    fn a_session_with_no_id_to_resume_is_not_resumed_and_carries_on() {
        let (taken, log, calls) = take(
            vec![ended(None, &["cargo test", "npm test"], "waiting")],
            |sessions| sessions.run(Kind::Implementation, "do it"),
        );

        taken.unwrap();
        assert_eq!(
            calls,
            [
                logging("implement"),
                session("implement", None, "do it"),
                Call::Step(
                    "implement: the session ended with background tasks still running \
                     (cargo test; npm test), which were killed; carrying on, as it may have \
                     been abandoned"
                        .to_string()
                ),
            ]
        );
        assert_eq!(log, Some(log_of("implement")));
    }

    #[test]
    fn a_failing_session_fails_unchanged_without_a_resume_or_an_ending() {
        let (taken, log, calls) = take(vec![Err(anyhow!("claude exited 3"))], |sessions| {
            let failed = sessions.run(Kind::Implementation, "do it").unwrap_err();
            assert_eq!(format!("{failed:#}"), "claude exited 3");
            Err::<(), _>(failed)
        });

        // No ending was recorded, so the failure names no killed work.
        assert_eq!(format!("{:#}", taken.unwrap_err()), "claude exited 3");
        assert_eq!(
            calls,
            [logging("implement"), session("implement", None, "do it")]
        );
        // The failing session's log is still the last log.
        assert_eq!(log, Some(log_of("implement")));
    }

    #[test]
    fn steps_that_succeed_after_killed_work_come_to_what_they_came_to() {
        let (taken, _, _) = take(vec![ended(None, &["cargo test"], "waiting")], |sessions| {
            sessions.run(Kind::Implementation, "do it")?;
            Ok(42)
        });

        assert_eq!(taken.unwrap(), 42);
    }

    #[test]
    fn every_session_that_left_killed_work_is_named_oldest_first() {
        let (taken, _, calls) = take(
            vec![
                ended(None, &["cargo test"], "waiting"),
                ended(None, &["npm test"], "waiting"),
            ],
            |sessions| -> Result<()> {
                sessions.run(Kind::Implementation, "do it")?;
                sessions.run(Kind::Repair(1), "fix it")?;
                bail!("CI failed")
            },
        );

        assert_eq!(sessions_run(&calls).len(), 2);
        assert_eq!(
            format!("{:#}", taken.unwrap_err()),
            format!(
                "implement session {TESTS_KILLED}; repair-1 session ended with a background \
                 task still running (npm test), which was killed, and a later step failed: \
                 CI failed"
            )
        );
    }

    #[test]
    fn a_prompt_that_loads_a_skill_loads_it_with_codexs_sigil_on_codex() {
        let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
        for (prompt, first_line) in [
            (
                prompt::fresh(&issue, "main", "issue-7"),
                "$thirdshift-implement https://github.com/acme/widgets/issues/7",
            ),
            (
                prompt::conflict_repair(&issue, "main", "issue-7", "<pr>"),
                "$thirdshift-resolving-merge-conflicts",
            ),
            (
                prompt::review_repair(&issue, "issue-7", "<pr>", "abc123"),
                "$thirdshift-code-review abc123",
            ),
            (
                prompt::resume(&["cargo test"]),
                "Your background work (cargo test) was killed when your turn ended, because \
                 ending the turn ends the session.",
            ),
        ] {
            let codex_prompt = codex_prompt(&prompt);
            assert_eq!(codex_prompt.lines().next(), Some(first_line));
            assert_eq!(codex_prompt[1..], prompt[1..]);
        }
    }
}
