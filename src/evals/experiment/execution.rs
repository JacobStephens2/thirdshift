//! One scoped Trial execution, through its completed mechanical Score.
//!
//! Trace and active evidence remain private and alive through publication.

mod monitor;
mod toolchains;

use std::cell::RefCell;
use std::fs;
use std::os::unix::fs::symlink;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use serde_json::json;

use super::{Arm, Setup, Task, arms, grading, integrity, store, trial_results};
use crate::process::{self, Control, Interruption};
use crate::session::limits;
use crate::trial::{Outcome, Output, VoidCause};
use crate::{interrupt, logs, progress};

/// Fixed inputs shared by both workers. Caps and PATH are read per execution.
pub(super) struct Execution<'a> {
    directory: &'a Path,
    prepared: &'a arms::Prepared,
    setups: [&'a Setup; 2],
    timeout: Duration,
    signal: &'a Path,
    toolchains: toolchains::Toolchains,
    pricing: Option<&'a crate::trial::accounting::Prices>,
}

pub(super) struct Request<'a> {
    pub task: &'a Task,
    pub identity: trial_results::Identity<'a>,
    pub attempt: usize,
    pub trace: &'a logs::trace::Context,
}

impl<'a> Execution<'a> {
    /// Prepare pinned Harness directories once, before either worker starts.
    pub fn prepare(
        directory: &'a Path,
        prepared: &'a arms::Prepared,
        setups: [&'a Setup; 2],
        timeout: Duration,
        signal: &'a Path,
    ) -> Result<Self> {
        for (name, arm) in [
            ("baseline", &prepared.baseline),
            ("candidate", &prepared.candidate),
        ] {
            let harness = directory.join("harnesses").join(name);
            fs::create_dir_all(&harness)?;
            let harnesses = std::iter::once(&arm.harness).chain(
                arm.process
                    .iter()
                    .flat_map(|process| process.steps.iter().map(|step| &step.harness)),
            );
            for fixed in harnesses {
                let link = harness.join(&fixed.name);
                if !link.exists() {
                    symlink(&fixed.path, link)?;
                }
            }
        }
        Ok(Self {
            directory,
            prepared,
            setups,
            timeout,
            signal,
            toolchains: toolchains::Toolchains::new(directory),
            pricing: None,
        })
    }

    pub fn with_pricing(mut self, pricing: Option<&'a crate::trial::accounting::Prices>) -> Self {
        self.pricing = pricing;
        self
    }

    /// Invoke completion exactly once after classification and grading.
    /// On every exit Trace finalizes before active evidence is removed.
    pub fn run<T>(
        &self,
        request: Request<'_>,
        complete: impl FnOnce(PathBuf, Output, grading::Score) -> Result<T>,
    ) -> Result<T> {
        let task = request.task;
        let (arm, setup) = match request.identity.arm {
            Arm::Baseline => (&self.prepared.baseline, self.setups[0]),
            Arm::Candidate => (&self.prepared.candidate, self.setups[1]),
        };
        for arm in [&self.prepared.baseline, &self.prepared.candidate] {
            if let Some(process) = &arm.process {
                process.verify()?;
            }
        }
        let key = request.identity.key();
        let directory = self.directory.join("trials").join(&key);
        fs::create_dir_all(&directory)?;
        let attempt = tempfile::Builder::new()
            .prefix("attempt-")
            .tempdir_in(&directory)?
            .keep();
        let caches = super::retention::Caches(attempt.clone());
        let prepared_repo = super::super::repository::prepare_trial(
            &task.directory,
            &task.manifest,
            &attempt.join("repository"),
        )?;
        let started = Instant::now();
        let mut trace = super::trace::Attempt::start(&attempt, &key, request.trace)?;
        let _active = super::evidence::Active::start(&directory, &attempt, request.attempt)?;
        let toolchains = match self.toolchains.ensure(&prepared_repo.repository) {
            Ok(toolchains) => toolchains,
            Err(error) => {
                interrupt::check()?;
                let mut output =
                    Output::failed(started.elapsed().as_millis(), &format!("{error:#}"));
                output.outcome = Outcome::Void;
                output.void_cause = Some(VoidCause::SessionUnavailable);
                output.timings.session_ms = 0;
                let score = trace.grade(|| {
                    grading::score(task, &prepared_repo.repository, &output, self.timeout)
                })?;
                trace.outcome(output.outcome);
                return complete(attempt, output, score);
            }
        };
        // Establish the Harness's idempotent instruction fallback before the
        // comparison and the private repository captured for mechanical checks.
        setup
            .choice()
            .harness
            .adapter()
            .link_instruction_fallback(&prepared_repo.repository)?;
        let before = integrity::Snapshot::capture(&prepared_repo.repository, &task.manifest)?;
        // This directory is outside the Trial's sandbox. Graders must see
        // the prepared state even when the review fixes and commits code.
        // The pass-through launcher can write this temp directory. Checks
        // run only when the copy still matches the pre-session digest.
        let grading_snapshot = if task.manifest.checks.is_empty()
            || task.manifest.session == crate::evals::format::Session::Process
        {
            None
        } else {
            let directory = tempfile::Builder::new()
                .prefix("prepared-grader-")
                .tempdir()?;
            let repository = directory.path().join("repository");
            crate::sandbox::code_grader::copy_repository(&prepared_repo.repository, &repository)?;
            let digest = integrity::tree_digest(&repository)?;
            Some(SealedSnapshot {
                repository,
                digest,
                _directory: directory,
            })
        };
        let grading_repository = grading_snapshot.as_ref().map_or_else(
            || prepared_repo.repository.clone(),
            |snapshot| snapshot.repository.clone(),
        );
        let inputs = task
            .manifest
            .trial
            .as_ref()
            .context("missing Trial inputs")?;
        let request_path = attempt.join("request.json");
        let mut document = json!({
            "version": 1, "repository": prepared_repo.repository, "session_kind": task.manifest.session,
            "inputs": inputs, "harness": setup.harness, "model": setup.model, "effort": setup.effort,
            "output_dir": attempt.join("session")
        });
        if task.manifest.session == crate::evals::format::Session::Process {
            document["process"] = match &arm.process {
                Some(process) => {
                    let file = process.install(&attempt.join("home"))?;
                    // Earlier Arm builds already support the Built-in name.
                    // Its frozen bytes were read from that exact executable.
                    if setup.process.as_deref() == Some("ticket-run") {
                        json!("ticket-run")
                    } else {
                        json!(file)
                    }
                }
                None => json!(setup.process.as_deref().unwrap_or("ticket-run")),
            };
        }
        store::write(&request_path, &document)?;
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.directory.join("started"))?;
        limits::check(self.signal)?;
        progress::step(format!("Trial {} starting", key));
        let monitor = RefCell::new(monitor::Monitor::new(
            &attempt.join("session"),
            &prepared_repo.repository,
            &setup.choice(),
        ));
        let started = Instant::now();
        // Machine caps are read live per attempt, not fixed with the Bar: an
        // operator edit between attempts applies to the next one.
        let caps = crate::config::UserConfig::load()?.trial_caps.with_env()?;
        let mut launched =
            self.command(arm, &request, &request_path, &caps, toolchains.as_deref())?;
        // The unit name stays in the attempt for `journalctl --user -u` after.
        fs::write(attempt.join("scope"), launched.unit())?;
        trace.inherit(launched.command())?;
        let output = process::output_until(
            &mut launched,
            None,
            Control {
                name: "Trial",
                interruption: Interruption::Ordinary,
                stop: &|child| process::stop(child, &[libc::SIGTERM, libc::SIGKILL]),
            },
            Instant::now() + self.timeout,
            &|| monitor.borrow_mut().check(self.signal),
        );
        let stopped_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
            .try_into()?;
        drop(caches);
        let interruption = self.accounting_interruption()?;
        // A kernel kill in the Trial's scope is the outer launcher's evidence
        // the cap killed Trial work. The live watch catches slow kills; the
        // journal's `oom-kill` result covers fast ones, whose scope is already
        // gone. Only failed Trials consult it: explicit Void causes stay as
        // the Trial diagnosed them, so limit pauses keep pausing, and an
        // agent's own out-of-memory error leaves no kernel kill behind.
        let cap_killed = || launched.oom_killed();
        let mut protocol = if let Some(error) = interruption {
            void_interrupted(started.elapsed().as_millis(), &error)
        } else {
            match output {
                Ok(output) => {
                    fs::write(attempt.join("stdout"), &output.stdout)?;
                    fs::write(attempt.join("stderr"), &output.stderr)?;
                    // systemd-run reports its scope's signal death as an exit
                    // code, so a scoped SIGKILL arrives as 137, not as a signal.
                    let sigkilled = output.status.signal() == Some(libc::SIGKILL)
                        || output.status.code() == Some(128 + libc::SIGKILL);
                    if sigkilled {
                        Output::failed(
                            started.elapsed().as_millis(),
                            "Trial process killed (SIGKILL); external cause unknown",
                        )
                    } else if !output.status.success() {
                        if cap_killed() {
                            void_memory_cap(started.elapsed().as_millis())
                        } else {
                            bail!(
                                "Arm Trial protocol failed ({}): {}",
                                output.status,
                                String::from_utf8_lossy(&output.stderr)
                            );
                        }
                    } else {
                        match serde_json::from_slice::<Output>(&output.stdout) {
                            Ok(protocol) => {
                                ensure!(protocol.version == 1, "unsupported Trial result");
                                protocol
                            }
                            Err(error) => {
                                if cap_killed() {
                                    void_memory_cap(started.elapsed().as_millis())
                                } else {
                                    return Err(error).context("invalid Trial result");
                                }
                            }
                        }
                    }
                }
                Err(error) if error.is::<process::TimedOut>() => {
                    Output::failed(u128::from(self.timeout.as_secs()) * 1000, "Trial timed out")
                }
                Err(error) if error.is::<limits::Reached>() => monitor
                    .into_inner()
                    .cut_short(started.elapsed().as_millis()),
                Err(error) => return Err(error),
            }
        };
        if protocol.outcome == Outcome::Failed && cap_killed() {
            protocol.outcome = Outcome::Void;
            protocol.void_cause = Some(VoidCause::MemoryCap);
            protocol.cause = Some(crate::scope::OOM_KILL_CAUSE.into());
        }
        if task.manifest.session == crate::evals::format::Session::Process
            && protocol.steps.is_empty()
        {
            let path = attempt.join("session/steps.json");
            if path.is_file() {
                protocol.steps = serde_json::from_slice(&fs::read(path)?)?;
            }
        }
        let unchanged = protocol.void_cause == Some(VoidCause::Interrupted)
            || task.manifest.session == crate::evals::format::Session::Process
            || before
                .unchanged(&prepared_repo.repository)
                .is_ok_and(|unchanged| unchanged);
        let snapshot_intact = grading_snapshot.as_ref().is_none_or(|snapshot| {
            integrity::tree_digest(&snapshot.repository)
                .is_ok_and(|digest| digest == snapshot.digest)
        });
        if let Some(error) = self.accounting_interruption()? {
            protocol.outcome = Outcome::Void;
            protocol.void_cause = Some(VoidCause::Interrupted);
            protocol.cause = Some(format!("{error:#}"));
        }
        // Keep known failures and external Void causes, including limit
        // stops. Repository changes invalidate an otherwise successful Trial,
        // including a process-shaped delivered outcome on a review session.
        if matches!(protocol.outcome, Outcome::Succeeded | Outcome::Delivered)
            && (!unchanged || !snapshot_intact)
        {
            protocol.outcome = Outcome::Void;
            protocol.void_cause = Some(VoidCause::RepositoryChanged);
            protocol.cause = Some("Trial repository changed during its session".into());
        }
        let score = trace.grade(|| {
            if let Some(prices) = self.pricing {
                let role = serde_json::to_value(&task.manifest.session)?;
                let accounting = super::accounting::capture(
                    &attempt,
                    &protocol,
                    role.as_str().unwrap(),
                    prices,
                    stopped_at,
                )?;
                if protocol.outcome != Outcome::Void
                    && let Some(gap) = accounting.gap()
                {
                    protocol.outcome = Outcome::Void;
                    protocol.void_cause = Some(VoidCause::AccountingGap);
                    protocol.cause = Some(gap.into());
                }
                protocol.accounting = Some(accounting);
            }
            match grading::score(task, &grading_repository, &protocol, self.timeout) {
                Err(error) if self.pricing.is_some() && interrupt::check().is_err() => {
                    protocol.outcome = Outcome::Void;
                    protocol.void_cause = Some(VoidCause::Interrupted);
                    protocol.cause = Some(format!("{error:#}"));
                    // Interrupted code checks produced no Score. Retain the
                    // sessions already priced before propagating interruption.
                    grading::score(task, &grading_repository, &protocol, self.timeout)
                }
                Err(error) if error.is::<crate::sandbox::Unavailable>() => {
                    protocol.outcome = Outcome::Void;
                    protocol.void_cause = Some(VoidCause::SandboxUnavailable);
                    protocol.cause = Some(format!("{error:#}"));
                    grading::score(task, &grading_repository, &protocol, self.timeout)
                }
                score => score,
            }
        })?;
        trace.steps(&protocol.steps)?;
        trace.outcome(protocol.outcome);
        if protocol.void_cause == Some(VoidCause::Interrupted)
            && interrupt::check().is_err()
            && let Some(accounting) = &mut protocol.accounting
        {
            accounting.incomplete = true;
        }
        let completed = complete(attempt, protocol, score)?;
        interrupt::check()?;
        Ok(completed)
    }

    // Priced Experiments must retain every started Session before propagating
    // interruption. Legacy Experiments keep their original checkpoint policy.
    fn accounting_interruption(&self) -> Result<Option<anyhow::Error>> {
        match interrupt::check() {
            Err(error) if self.pricing.is_some() => Ok(Some(error)),
            result => result.map(|()| None),
        }
    }

    fn command(
        &self,
        arm: &arms::Arm,
        request: &Request<'_>,
        path: &Path,
        caps: &crate::scope::Caps,
        toolchains: Option<&Path>,
    ) -> Result<crate::scope::Launch> {
        let directory = self
            .directory
            .join("harnesses")
            .join(request.identity.arm.name());
        let path_env = std::env::join_paths(
            std::iter::once(directory)
                .chain(toolchains.map(|root| root.join("session/bin")))
                .chain(std::env::split_paths(
                    &std::env::var_os("PATH").unwrap_or_default(),
                )),
        )?;
        let path_env = path_env
            .into_string()
            .map_err(|_| anyhow::anyhow!("PATH is not valid Unicode"))?;
        let path = path
            .to_str()
            .context("Trial request path is not valid Unicode")?;
        let mut launched = crate::scope::trial_command(
            &arm.binary,
            &["--trial", path],
            &request.identity.key(),
            request.attempt,
            caps,
        )?;
        launched
            .command()
            .env(limits::SIGNAL_ENV, self.signal)
            .env("PATH", path_env)
            .env_remove(crate::skills::TRIAL_HOME);
        if arm.process.is_some()
            && request.task.manifest.session == crate::evals::format::Session::Process
        {
            launched.command().env(
                crate::skills::TRIAL_HOME,
                Path::new(path)
                    .parent()
                    .context("Trial request has no directory")?
                    .join("home"),
            );
        }
        if let Some(toolchains) = toolchains {
            launched.command().env("CARGO_HOME", toolchains);
        }
        Ok(launched)
    }
}

/// Prepared files checks grade, sealed before the session starts.
struct SealedSnapshot {
    repository: PathBuf,
    digest: Vec<u8>,
    _directory: tempfile::TempDir,
}

/// A Trial its scope's cap killed: void with `memory_cap`, from the scope's
/// `memory.events` or journal evidence. Only the outer launcher makes these,
/// never the Trial itself.
fn void_interrupted(total_ms: u128, error: &anyhow::Error) -> Output {
    let mut output = Output::failed(total_ms, &format!("{error:#}"));
    output.outcome = Outcome::Void;
    output.void_cause = Some(VoidCause::Interrupted);
    output
}

fn void_memory_cap(total_ms: u128) -> Output {
    let mut output = Output::failed(total_ms, crate::scope::OOM_KILL_CAUSE);
    output.outcome = Outcome::Void;
    output.void_cause = Some(VoidCause::MemoryCap);
    output
}

#[cfg(test)]
mod tests;
