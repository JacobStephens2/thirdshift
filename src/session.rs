//! Headless agent sessions on the chosen Harness, and their logs.

use std::cell::{OnceCell, RefCell};
#[cfg(test)]
use std::fs::{self, File};
#[cfg(test)]
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::process::Command;

#[cfg(test)]
use anyhow::bail;
use anyhow::{Context, Result, anyhow};

#[cfg(test)]
pub(crate) use crate::harness::claude::claude_args;
#[cfg(test)]
pub(crate) use crate::harness::codex::{codex_args, codex_prompt};
use crate::harness::interpretation::{Ended, SafeguardRefusal, SessionFailure};
use crate::harness::{Adapter, Choice, ChosenBy};
use crate::issue::{IssueUrl, Repo};
use crate::logs;
use crate::plan_usage::Router;
use crate::plan_usage::router::Picked;
use crate::progress;
use crate::prompt;
use crate::skills;
use limits::Limit;

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

impl Purpose {
    /// A Security run's, Security fix publishing's or Security review's
    /// session: it walks the Security routes, and ends incomplete with
    /// killed work.
    fn is_security(self) -> bool {
        match self {
            Self::Security | Self::SecurityPublishing | Self::SecurityReview => true,
            Self::Ordinary | Self::CodeReview => false,
        }
    }

    /// The launch policy of a session given `prompt`: the shared
    /// review-report contract identifies code review sessions before
    /// expansion.
    fn of_prompt(prompt: &str) -> Self {
        if prompt.contains(prompt::REVIEW_REPORTS_DIRECTORY) {
            Self::CodeReview
        } else {
            Self::Ordinary
        }
    }
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
            recording: recording::Recording::for_process().into(),
        }
    }

    /// A child Ticket's Steps must not overwrite its parent's Session traces.
    pub fn for_child(mut self, issue: &IssueUrl, process: crate::processes::Identity) -> Self {
        self.name = format!("{}-{}", issue.number, self.name);
        self.recording = recording::Recording::capture().with_process(process).into();
        self
    }

    /// An isolated Trial's artifacts, with no Command owner or factory logs.
    pub fn of_trial(dir: &Path) -> Self {
        Logs {
            name: "trial".to_string(),
            dir: dir.to_path_buf(),
            recording: recording::Recording::for_trial().into(),
        }
    }

    /// Whether these are a Trial's logs: a Trial keeps the Eval stop
    /// policy, not a factory Command's pause.
    fn is_trial(&self) -> bool {
        self.recording().is_trial()
    }

    pub(crate) fn of_trial_step(dir: &Path, role: &str) -> Self {
        Self {
            recording: recording::Recording::for_trial_step(role).into(),
            ..Self::of_trial(dir)
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

    pub(crate) fn of_fix_run(repo: &Repo) -> Self {
        Logs {
            name: "fix".into(),
            dir: logs::session_dir(repo),
            recording: OnceCell::new(),
        }
    }

    /// A Custom process's review keeps the Security review session kind for
    /// fallback and Resume, with a distinct trace for each one-based Step.
    pub fn of_process_review(repo: &Repo, step_index: usize) -> Self {
        let mut logs = Self::of_process(repo);
        logs.name = format!("process-step-{step_index}");
        logs
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

    pub fn of_triage_run(repo: &Repo) -> Self {
        Logs {
            name: "triage".to_string(),
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
    /// caller's prepared bwrap sandbox.
    /// The Factory skills are linked as usual; the sandbox binds them in.
    pub fn within_sandboxed<T>(
        logs: &'a Logs,
        worktree: &'a Path,
        harness: &'a Choice,
        sandbox: &crate::sandbox::Launch,
        steps: impl FnOnce(&Self) -> Result<T>,
    ) -> (Result<T>, Option<PathBuf>) {
        Self::checked_within_sandboxed(logs, worktree, harness, sandbox, &|| Ok(()), steps)
    }

    /// A local Process restores its Ticket before every sandboxed session,
    /// including a Resume, using the same admission lifecycle as the factory.
    pub(crate) fn checked_within_sandboxed<T>(
        logs: &'a Logs,
        worktree: &'a Path,
        harness: &'a Choice,
        sandbox: &crate::sandbox::Launch,
        before_each: &'a dyn Fn() -> Result<()>,
        steps: impl FnOnce(&Self) -> Result<T>,
    ) -> (Result<T>, Option<PathBuf>) {
        let adapter = harness.harness.adapter();
        if let Err(error) = skills::link_into(worktree, adapter)
            .and_then(|()| adapter.link_instruction_fallback(worktree))
        {
            return (Err(error), None);
        }
        // A prepared sandbox binds its own Harness's files: its sessions
        // stay on that Harness.
        let on_machine = OnMachine {
            worktree: worktree.to_path_buf(),
            harness: harness.clone(),
            adapter,
            security_review: None,
            security_review_default: None,
            recording: logs.recording().clone(),
            sandbox: Some(sandbox.clone()),
            router: Router::pinned(),
            pauses: !logs.is_trial(),
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
        let router = router(harness, security_review, security_review_default);
        let security_review =
            security_review.map(|choice| (choice.clone(), choice.harness.adapter()));
        // Every route's Factory skills, so a Step that switches Harnesses
        // finds its own.
        let mut adapters: Vec<&'static dyn Adapter> = vec![adapter];
        adapters.extend(security_review.as_ref().map(|(_, adapter)| *adapter));
        for routed in router.harnesses() {
            let routed = routed.adapter();
            if !adapters.iter().any(|linked| linked.name() == routed.name()) {
                adapters.push(routed);
            }
        }
        let linked = adapters.into_iter().try_for_each(|adapter| {
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
            router,
            pauses: !logs.is_trial(),
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
        (with_killed_work(taken, &endings), log)
    }

    /// Transfer killed-work context to a caller whose Steps outlive this
    /// session scope. That caller must apply [`with_killed_work`] to its
    /// final outcome; taking it prevents annotating the same failure twice.
    pub fn take_killed_work(&self) -> Vec<String> {
        std::mem::take(&mut *self.endings_with_killed_work.borrow_mut())
    }

    /// Run a session of `kind` given `prompt`, logged and labelled by its
    /// token. A session that ends with background work still running, which
    /// is killed with it, gets one Resume, as `<token>-resume`. If the Resume ends the
    /// same way, or the session has no id to resume, the work may have been
    /// abandoned rather than awaited: a progress line names it and this
    /// succeeds, leaving the steps that follow to decide the outcome.
    /// Returns the final message while retaining the prompt's launch policy.
    pub fn run(&self, kind: Kind, prompt: &str) -> Result<Option<String>> {
        self.run_implementation(kind, prompt, None)
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
        self.run_step(purpose, kind, prompt, None)
    }

    /// Run an implement session as [`Sessions::run`] does. When Plan usage
    /// moves it to another route after work began, `continuation` is asked,
    /// given whether the session made commits, for the prompt it starts
    /// again with: the existing Continuation prompt, so no prompt says why.
    /// The prompt's launch policy holds even if report setup fails.
    pub fn run_implementation(
        &self,
        kind: Kind,
        prompt: &str,
        continuation: Option<ContinueWith<'_>>,
    ) -> Result<Option<String>> {
        self.run_step(Purpose::of_prompt(prompt), kind, prompt, continuation)
            .map(|ended| ended.final_message)
    }

    /// One session Step: its session, and its Resume if it gets one. On a
    /// Command routed by the User config's routes, the Step starts on its
    /// first route that could take it now, and when a session stops on
    /// structured Plan-usage evidence, starts again, as a fresh session, on
    /// the next route it has not tried. A session that stopped after work
    /// began has its uncommitted changes discarded first, its commits kept;
    /// an implement session starts again on what `continuation` gives. A
    /// cybersecurity safeguard refusal never routes.
    fn run_step(
        &self,
        purpose: Purpose,
        kind: Kind,
        prompt: &str,
        mut continuation: Option<ContinueWith<'_>>,
    ) -> Result<Ended> {
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
        let security = purpose.is_security();
        let routes = self.outside.borrow().routes(security);
        if routes.len() < 2 {
            self.pause_if_exhausted(purpose, &label)?;
            let ended = self.unrouted(purpose, kind, prompt, reports.as_ref());
            return self.paused_at_a_limit(&label, ended);
        }
        let at_start = self.outside.borrow_mut().worktree_at_start()?;
        let mut tried = Vec::new();
        let mut current = self
            .take_route(purpose, &label, &routes, &mut tried, false)?
            .unwrap_or(0);
        let mut prompt = prompt.to_owned();
        let mut logged = Logged::of(kind);
        loop {
            let error = match self.attempt(purpose, &label, logged, &prompt, reports.as_ref()) {
                Ok(ended) => return Ok(ended),
                Err(error) => error,
            };
            let Some(failure) = error.downcast_ref::<SessionFailure>() else {
                return Err(error);
            };
            // A Security session that could not start on `security.harness`
            // falls back as a Security review always has.
            let started = !failure.before_first_turn;
            let unstarted_security = security
                && routes[current].chosen_by == ChosenBy::SecurityConfig
                && current == 0
                && !started
                && failure
                    .diagnostic
                    .as_deref()
                    .is_some_and(|text| !text.trim().is_empty());
            if error.is::<SafeguardRefusal>()
                || !(failure.plan_usage_exhausted || unstarted_security)
            {
                return Err(error);
            }
            let from = &routes[current];
            let stopped = format!(
                "{label}: {} {}",
                from.harness.name(),
                if failure.plan_usage_exhausted {
                    "stopped at its Plan usage limit"
                } else {
                    "could not start"
                }
            );
            let reason = failure
                .diagnostic
                .as_deref()
                .unwrap_or_default()
                .to_string();
            let plan_usage = failure.plan_usage_exhausted;
            tried.push(current);
            let Some(next) = self.take_route(purpose, &label, &routes, &mut tried, true)? else {
                self.step(format!("{stopped}; no other route is available"));
                return self.paused_at_a_limit(&label, Err(error));
            };
            let to = &routes[next];
            if plan_usage || kind != Kind::SecurityReview {
                self.step(format!(
                    "{stopped}; starting the Step again as a fresh session on {to}"
                ));
            } else if matches!(to.chosen_by, ChosenBy::UserConfig | ChosenBy::Default) {
                crate::harness::warn_review_fallback(from.harness, to.harness, &reason);
            } else {
                crate::harness::warn_review_reroute(from.harness, to.harness, &reason);
            }
            if started && let Some(at_start) = &at_start {
                let committed = self.outside.borrow_mut().reset(at_start)?;
                self.step(format!(
                    "{label}: kept the session's commits and discarded its uncommitted changes"
                ));
                if let Some(continue_with) = continuation.as_deref_mut()
                    && let Some(next) = continue_with(committed)?
                {
                    prompt = next;
                }
            }
            current = next;
            // Each later attempt has its own Session trace. A Security
            // review's first move keeps its long-standing
            // `security-review-fallback` name; later ones number from there.
            logged = match (kind, logged.kind) {
                (Kind::SecurityReview, Kind::SecurityReview) => {
                    Logged::of(Kind::SecurityReviewFallback)
                }
                _ => logged.on_route(logged.route + 1),
            };
        }
    }

    /// Switch this purpose's sessions to the first of `routes` not in
    /// `tried` that could take a session now, by machine-local Plan usage,
    /// and return its index. A route whose CLI can't run is tried and
    /// passed. At the start of a Step with no route available, the Step
    /// starts on its first route; `after_a_stop`, none left means the stop
    /// stands.
    fn take_route(
        &self,
        purpose: Purpose,
        label: &str,
        routes: &[Choice],
        tried: &mut Vec<usize>,
        after_a_stop: bool,
    ) -> Result<Option<usize>> {
        let security = purpose.is_security();
        loop {
            let picked = self.outside.borrow_mut().pick(security, tried);
            let Some(Picked { index, passed }) = picked else {
                if after_a_stop {
                    return Ok(None);
                }
                self.pause_if_exhausted(purpose, label)?;
                self.outside.borrow_mut().switch(purpose, &routes[0])?;
                return Ok(Some(0));
            };
            let choice = &routes[index];
            if let Err(error) = self.outside.borrow_mut().switch(purpose, choice) {
                crate::interrupt::check()?;
                let reason = format!("{error:#}")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                self.step(format!(
                    "{label}: route {choice} can't be used ({reason}); passing it"
                ));
                tried.push(index);
                continue;
            }
            if !after_a_stop && (index > 0 || !passed.is_empty()) {
                self.step(format!(
                    "{label}: on {choice}, routed by Plan usage: {}",
                    passed.join("; ")
                ));
            }
            return Ok(Some(index));
        }
    }

    /// End a factory Command's Step before its session starts, as a Paused
    /// Command, when every route it may take is unavailable by this
    /// machine's saved Plan usage: no session would get work done, and
    /// none sleeps waiting for a Plan window to reset.
    fn pause_if_exhausted(&self, purpose: Purpose, label: &str) -> Result<()> {
        let exhaustion = {
            let mut outside = self.outside.borrow_mut();
            if !outside.pauses() {
                return Ok(());
            }
            outside.exhaustion(purpose)
        };
        let Some(exhaustion) = exhaustion else {
            return Ok(());
        };
        self.step(format!("{label}: not started: {exhaustion}"));
        let paused = limits::PlanUsagePaused::new(exhaustion.limit());
        logs::paused(&paused.limit);
        Err(paused.into())
    }

    /// A factory Command's Step whose session stopped at a Plan usage limit
    /// a structured wire frame reported, with no other route to take, ends
    /// the Command as a Paused Command. A cybersecurity safeguard refusal,
    /// and every other failure, stands as it is.
    fn paused_at_a_limit(&self, label: &str, ended: Result<Ended>) -> Result<Ended> {
        let error = match ended {
            Ok(ended) => return Ok(ended),
            Err(error) => error,
        };
        if !self.outside.borrow().pauses() || error.is::<SafeguardRefusal>() {
            return Err(error);
        }
        let Some(failure) = error
            .downcast_ref::<SessionFailure>()
            .filter(|failure| failure.plan_usage_exhausted)
        else {
            return Err(error);
        };
        let limit = failure.plan_usage_limit.clone().unwrap_or_else(|| Limit {
            harness: Some(failure.harness.to_owned()),
            ..Limit::default()
        });
        let paused = limits::PlanUsagePaused::new(limit);
        self.step(format!("{label}: {paused}"));
        logs::paused(&paused.limit);
        Err(error.context(paused))
    }

    /// A Step with no routes to take: its session, retried once on the
    /// default Harness if it is a Security review that could not start.
    fn unrouted(
        &self,
        purpose: Purpose,
        kind: Kind,
        prompt: &str,
        reports: Option<&ReviewReports>,
    ) -> Result<Ended> {
        let label = kind.token();
        let first = self.attempt(purpose, &label, Logged::of(kind), prompt, reports);
        if let Err(error) = &first
            && kind == Kind::SecurityReview
            && matches!(purpose, Purpose::SecurityReview)
            && self.outside.borrow_mut().fallback_review(error)?
        {
            return self.attempt(
                purpose,
                &label,
                Logged::of(Kind::SecurityReviewFallback),
                prompt,
                reports,
            );
        }
        first
    }

    /// One attempt at a Step on one route: the session logged as `logged`,
    /// and its Resume once if it ended with background work killed. Its
    /// progress lines name the Step by `label`.
    fn attempt(
        &self,
        purpose: Purpose,
        label: &str,
        logged: Logged,
        prompt: &str,
        reports: Option<&ReviewReports>,
    ) -> Result<Ended> {
        let mut ended = self.start(purpose, logged, None, prompt, reports)?;
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
                    Logged {
                        resume: true,
                        ..logged
                    },
                    Some(session_id),
                    &resume_prompt,
                    reports,
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
            if purpose.is_security() {
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
    /// What each route of Security sessions, when `security`, else of
    /// ordinary ones, runs on, in order: fewer than two when pinned.
    fn routes(&self, _security: bool) -> Vec<Choice> {
        Vec::new()
    }
    /// The first route not in `tried` that could take a session now.
    fn pick(&mut self, _security: bool, _tried: &[usize]) -> Option<Picked> {
        None
    }
    /// Whether a Plan usage limit with no route left ends the Command as a
    /// Paused Command: a factory Command's sessions, not a Trial's.
    fn pauses(&self) -> bool {
        false
    }
    /// Whether every route this purpose's sessions may take is unavailable
    /// by this machine's saved Plan usage.
    fn exhaustion(&mut self, _purpose: Purpose) -> Option<crate::plan_usage::Exhaustion> {
        None
    }
    /// Run this purpose's sessions on `choice` from now on, once its CLI
    /// is checked.
    fn switch(&mut self, _purpose: Purpose, _choice: &Choice) -> Result<()> {
        Ok(())
    }
    /// The worktree as a routed Step finds it, for [`Outside::reset`]; none
    /// where its uncommitted changes must never be discarded.
    fn worktree_at_start(&mut self) -> Result<Option<AtStart>> {
        Ok(None)
    }
    /// Discard the worktree's uncommitted changes, keeping its commits, and
    /// merge a conflicted merge the Step found in progress again. True when
    /// commits were made since `at_start`.
    fn reset(&mut self, _at_start: &AtStart) -> Result<bool> {
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

/// Asked, once a routed implement session's uncommitted changes are
/// discarded, whether it made commits; gives the prompt it starts again
/// with, if not its own.
pub type ContinueWith<'a> = &'a mut dyn FnMut(bool) -> Result<Option<String>>;

/// The worktree as a routed Step found it: its commit, and the pinned
/// commit and message of a conflicted merge in progress, as a conflict
/// Repair starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AtStart {
    head: Option<String>,
    merge: Option<(String, String)>,
}

/// The routes the Command's sessions may take, as the User config sets
/// them: none in unit tests, which never read the machine's User config.
fn router(
    harness: &Choice,
    security_review: Option<&Choice>,
    security_review_default: Option<&Choice>,
) -> Router {
    if cfg!(test) {
        return Router::pinned();
    }
    match crate::config::UserConfig::load() {
        Ok(config) => Router::of(
            &config.harness,
            &config.routing,
            harness,
            security_review,
            security_review_default,
        ),
        Err(_) => Router::pinned(),
    }
}

/// Sessions run by the Command's `harness`'s CLI in `worktree`, where it
/// finds the Factory skills, and progress lines printed on stderr. A
/// Security review runs on `security_review`'s instead, if it is given.
/// Plan usage may move a Step to another of `router`'s routes.
struct OnMachine {
    worktree: PathBuf,
    harness: Choice,
    adapter: &'static dyn Adapter,
    security_review: Option<(Choice, &'static dyn Adapter)>,
    security_review_default: Option<Choice>,
    recording: recording::Recording,
    sandbox: Option<crate::sandbox::Launch>,
    router: Router,
    /// Whether a Plan usage limit with no route left pauses the Command:
    /// every factory Command's sessions, not a Trial's.
    pauses: bool,
}

impl OnMachine {
    /// What this purpose's sessions run on now: a Security review's on
    /// `security_review` when it is given, every other on `harness`.
    fn current(&self, purpose: Purpose) -> (&Choice, &'static dyn Adapter) {
        match (purpose, &self.security_review) {
            (Purpose::SecurityReview, Some((review, adapter))) => (review, *adapter),
            _ => (&self.harness, self.adapter),
        }
    }
}

impl Outside for OnMachine {
    fn routes(&self, security: bool) -> Vec<Choice> {
        self.router
            .candidates(security)
            .iter()
            .map(|candidate| candidate.choice.clone())
            .collect()
    }

    fn pick(&mut self, security: bool, tried: &[usize]) -> Option<Picked> {
        self.router.pick(security, tried)
    }

    fn pauses(&self) -> bool {
        self.pauses
    }

    fn exhaustion(&mut self, purpose: Purpose) -> Option<crate::plan_usage::Exhaustion> {
        let current = self.current(purpose).0.harness;
        self.router.exhaustion(purpose.is_security(), current)
    }

    fn switch(&mut self, purpose: Purpose, choice: &Choice) -> Result<()> {
        if self.current(purpose).0.runs_as(choice) {
            return Ok(());
        }
        let mut checked = choice.clone();
        checked.check_without_probe()?;
        let adapter = checked.harness.adapter();
        match purpose {
            Purpose::SecurityReview => self.security_review = Some((checked, adapter)),
            Purpose::Ordinary
            | Purpose::CodeReview
            | Purpose::Security
            | Purpose::SecurityPublishing => {
                self.harness = checked;
                self.adapter = adapter;
            }
        }
        Ok(())
    }

    fn worktree_at_start(&mut self) -> Result<Option<AtStart>> {
        let git = crate::git::Git::new(&self.worktree);
        // Only a linked worktree thirdshift owns, never a person's checkout.
        let git_dir = PathBuf::from(git.run(&["rev-parse", "--absolute-git-dir"])?);
        let linked = git_dir.canonicalize()? != git.common_dir()?.canonicalize()?;
        if !linked {
            return Ok(None);
        }
        let head = git.run_optional(&["rev-parse", "--verify", "-q", "HEAD"])?;
        let merge = match git.run_optional(&["rev-parse", "-q", "--verify", "MERGE_HEAD"])? {
            Some(commit) => {
                let path = git.run(&["rev-parse", "--git-path", "MERGE_MSG"])?;
                let message = std::fs::read_to_string(self.worktree.join(path))
                    .unwrap_or_default()
                    .lines()
                    .filter(|line| !line.starts_with('#'))
                    .collect::<Vec<_>>()
                    .join("\n")
                    .trim()
                    .to_string();
                let message = if message.is_empty() {
                    format!("Merge {commit}")
                } else {
                    message
                };
                Some((commit, message))
            }
            None => None,
        };
        Ok(Some(AtStart { head, merge }))
    }

    fn reset(&mut self, at_start: &AtStart) -> Result<bool> {
        let git = crate::git::Git::new(&self.worktree);
        if git.merge_in_progress()? {
            git.run(&["merge", "--abort"])?;
        }
        // Ignored files stay: `.git/info/exclude` keeps the factory's
        // handover files and linked skills out of `clean`'s reach.
        git.run(&["reset", "--hard", "HEAD"])?;
        git.run(&["clean", "-fd"])?;
        if let Some((commit, message)) = &at_start.merge
            && !git.succeeds(&["merge-base", "--is-ancestor", commit, "HEAD"])?
        {
            // The conflict Repair's next route starts from the same
            // conflicted merge of the same pinned commit.
            let merged =
                git.run_optional(&["merge", "--no-edit", "--ff", "-m", message, commit])?;
            if merged.is_none() && !git.merge_in_progress()? {
                anyhow::bail!("could not merge {commit} again for the next route");
            }
        }
        Ok(git.run_optional(&["rev-parse", "--verify", "-q", "HEAD"])? != at_start.head)
    }

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
        let (harness, adapter) = self.current(purpose);
        if matches!(purpose, Purpose::SecurityReview) && self.security_review.is_some() {
            progress::step(format!(
                "{label}: on {harness}, chosen by {}",
                harness.chosen_by
            ));
        }
        crate::logs::session_choice(harness);
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
        // Trial reviews run behind the allowlisting proxy, with web search off.
        if self.recording.is_trial() && matches!(purpose, Purpose::CodeReview) {
            invocation = adapter.trial_session(harness, resume, prompt);
        }
        let mut launched = match &self.sandbox {
            Some(launch) => launch.session_command(&invocation.args, self.recording.is_trial())?,
            None => {
                let mut launched = if self.recording.is_trial() {
                    crate::scope::trial_session_command(adapter.name())
                } else {
                    crate::scope::session_command(adapter.name())
                };
                launched.command().args(&invocation.args);
                launched
            }
        };
        let command = launched.command();
        command.current_dir(&self.worktree);
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
            launched.as_mut(),
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

/// Name earlier killed work ahead of a later failure, leaving success alone.
pub fn with_killed_work<T>(taken: Result<T>, endings: &[String]) -> Result<T> {
    if endings.is_empty() {
        return taken;
    }
    taken.map_err(|error| error.context(format!("{}, and a later step failed", endings.join("; "))))
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

    /// Sessions routed across `routes`, of which those at `unavailable`
    /// can't take a session at a Step's start, ending as scripted. Each
    /// switch names the Harness switched to, and each reset says so.
    struct Routed {
        scripted: Scripted,
        routes: Vec<Choice>,
        unavailable: Vec<usize>,
        committed: bool,
        moves: Rc<RefCell<Vec<String>>>,
    }

    impl Outside for Routed {
        fn routes(&self, _security: bool) -> Vec<Choice> {
            self.routes.clone()
        }
        fn pick(&mut self, _security: bool, tried: &[usize]) -> Option<Picked> {
            let index = (0..self.routes.len())
                .find(|index| !tried.contains(index) && !self.unavailable.contains(index))?;
            Some(Picked {
                index,
                passed: self
                    .unavailable
                    .iter()
                    .filter(|unavailable| **unavailable < index)
                    .map(|unavailable| {
                        format!(
                            "{} is in cooldown",
                            self.routes[*unavailable].harness.name()
                        )
                    })
                    .collect(),
            })
        }
        fn switch(&mut self, _purpose: Purpose, choice: &Choice) -> Result<()> {
            self.moves
                .borrow_mut()
                .push(format!("on {}", choice.harness.name()));
            Ok(())
        }
        fn worktree_at_start(&mut self) -> Result<Option<AtStart>> {
            Ok(Some(AtStart {
                head: Some("start".into()),
                merge: None,
            }))
        }
        fn reset(&mut self, _at_start: &AtStart) -> Result<bool> {
            self.moves.borrow_mut().push("reset".into());
            Ok(self.committed)
        }
        fn run_session(
            &mut self,
            purpose: Purpose,
            kind: &str,
            resume: Option<&str>,
            prompt: &str,
            log: &Path,
        ) -> Result<Ended> {
            self.scripted
                .run_session(purpose, kind, resume, prompt, log)
        }
        fn step(&mut self, line: String) {
            self.scripted.step(line);
        }
    }

    /// Claude Code, then Codex, as `harness.routes` lists them.
    fn claude_then_codex() -> Vec<Choice> {
        [
            crate::harness::Harness::Claude,
            crate::harness::Harness::Codex,
        ]
        .map(|harness| Choice {
            harness,
            model: None,
            effort: None,
            chosen_by: crate::harness::ChosenBy::Routes,
        })
        .to_vec()
    }

    /// Take an implement Step, given `continuation`, through `routes`, of
    /// which `unavailable` can't start it, whose sessions end as `endings`.
    /// Returns what it came to, every call, and every switch and reset.
    fn route(
        routes: Vec<Choice>,
        unavailable: Vec<usize>,
        committed: bool,
        endings: Vec<Result<Ended>>,
    ) -> (Result<Option<String>>, Vec<Call>, Vec<String>) {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let moves = Rc::new(RefCell::new(Vec::new()));
        let routed = Routed {
            scripted: Scripted {
                endings: endings.into(),
                calls: Rc::clone(&calls),
            },
            routes,
            unavailable,
            committed,
            moves: Rc::clone(&moves),
        };
        let logs = logs();
        let (taken, _) = Sessions::taking(
            &logs,
            Path::new("/unused"),
            Box::new(routed),
            || Ok(()),
            |sessions| {
                // The Continuation prompt, once the session made commits.
                let mut continue_with = |committed: bool| -> Result<Option<String>> {
                    Ok(committed.then(|| "go on".to_string()))
                };
                sessions.run_implementation(Kind::Implementation, "do it", Some(&mut continue_with))
            },
        );
        (taken, calls.take(), moves.take())
    }

    fn limit(before_first_turn: bool) -> Result<Ended> {
        Err(SessionFailure::plan_usage_limit("claude", before_first_turn).into())
    }

    #[test]
    fn a_step_starts_on_its_first_route_that_can_take_it() {
        let (taken, calls, moves) = route(
            claude_then_codex(),
            vec![0],
            false,
            vec![ended(None, &[], "done")],
        );
        assert_eq!(taken.unwrap().as_deref(), Some("done"));
        assert_eq!(moves, ["on codex"]);
        assert_eq!(sessions_run(&calls), [&session("implement", None, "do it")]);
        assert!(calls.iter().any(|call| matches!(call,
            Call::Step(line) if line.contains("routed by Plan usage: claude is in cooldown"))));
    }

    #[test]
    fn a_limit_before_the_first_turn_starts_the_step_again_on_the_next_route_untouched() {
        let (taken, calls, moves) = route(
            claude_then_codex(),
            vec![],
            false,
            vec![limit(true), ended(None, &[], "done")],
        );
        assert_eq!(taken.unwrap().as_deref(), Some("done"));
        assert_eq!(moves, ["on claude", "on codex"]);
        assert_eq!(
            sessions_run(&calls),
            [
                &session("implement", None, "do it"),
                &session("implement-route-2", None, "do it"),
            ]
        );
    }

    #[test]
    fn a_limit_after_work_keeps_commits_discards_the_rest_and_implements_on_as_a_continuation() {
        for (committed, prompt) in [(true, "go on"), (false, "do it")] {
            let (taken, calls, moves) = route(
                claude_then_codex(),
                vec![],
                committed,
                vec![limit(false), ended(None, &[], "done")],
            );
            assert_eq!(taken.unwrap().as_deref(), Some("done"));
            // The next route is taken before the reset: with none left,
            // the worktree stays as it was for Failed run preservation.
            assert_eq!(moves, ["on claude", "on codex", "reset"]);
            assert_eq!(
                sessions_run(&calls),
                [
                    &session("implement", None, "do it"),
                    &session("implement-route-2", None, prompt),
                ],
                "{committed}"
            );
            // No prompt says why: the Continuation prompt is unchanged.
            for call in &calls {
                if let Call::Session { prompt, .. } = call {
                    assert!(!prompt.to_lowercase().contains("limit"), "{prompt}");
                }
            }
        }
    }

    #[test]
    fn each_route_is_tried_once_and_other_failures_never_route() {
        let (taken, calls, _) = route(
            claude_then_codex(),
            vec![],
            false,
            vec![limit(true), limit(true)],
        );
        let failure = taken.unwrap_err();
        assert!(
            failure
                .downcast_ref::<SessionFailure>()
                .unwrap()
                .plan_usage_exhausted
        );
        assert_eq!(sessions_run(&calls).len(), 2);
        assert!(calls.iter().any(|call| matches!(call,
            Call::Step(line) if line.ends_with("no other route is available"))));

        let refused = anyhow::Error::new(SessionFailure::plan_usage_limit("claude", true))
            .context(SafeguardRefusal::ClaudeCyber);
        for failure in [
            anyhow::Error::new(SessionFailure::exited("claude", 1)),
            anyhow!("claude exited 429: rate limited"),
            refused,
        ] {
            let (taken, calls, moves) =
                route(claude_then_codex(), vec![], false, vec![Err(failure)]);
            assert!(taken.is_err());
            assert_eq!(sessions_run(&calls).len(), 1);
            assert_eq!(moves, ["on claude"]);
        }
    }

    #[test]
    fn a_single_route_is_pinned_and_never_switches() {
        let (taken, calls, moves) = route(
            claude_then_codex()[..1].to_vec(),
            vec![0],
            false,
            vec![limit(true)],
        );
        assert!(taken.is_err());
        assert!(moves.is_empty());
        assert_eq!(sessions_run(&calls).len(), 1);
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
