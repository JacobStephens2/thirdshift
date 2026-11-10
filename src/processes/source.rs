//! The Ticket source and publication seam. Factory policy lives here; local
//! execution supplies a prepared checkout and never publishes it.

use super::*;
use crate::claim::{self, Claim};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;

/// Preserve unfinished Issue-branch work consistently for direct and
/// dispatched Custom processes.
pub(super) fn execute_issue_steps(
    process: &str,
    prepared: &Prepared,
    issue_run: &mut IssueRun<'_>,
) -> Result<String> {
    if prepared.spec_branch
        && let Some(checkout) = issue_run.checkout.as_mut()
    {
        checkout.keep_unpublished_on_cleanup();
    }
    let result = execute_steps(process, prepared, Some(&mut *issue_run));
    if let Err(error) = &result {
        let context = continuation(issue_run);
        let on_origin = issue_run.on_origin;
        if issue_run.failure.is_none()
            && let Some(checkout) = issue_run.checkout.as_mut()
        {
            issue_run.failure = Some(preserve_step_failure(
                checkout,
                &mut issue_run.pull_request,
                context,
                on_origin,
                error,
                recovery::Stage::Implementation,
            ));
        }
        if let Some(tickets) = &issue_run.spec_tickets {
            let _ = issue_run.pull_request.show_checklist(&tickets.checklist);
            if let Some(failed) = &mut issue_run.failure {
                tickets.record_failure(failed);
            }
        }
        if !issue_run.delivered {
            let context = continuation(issue_run);
            if let Some(failed) = &mut issue_run.failure
                && let Some(recovery) = &mut failed.recovery
            {
                recovery.add_command_context(context, on_origin);
                for line in recovery.lines() {
                    progress::step(line);
                }
            }
        }
    }
    result
}

pub(super) fn preserve_step_failure(
    checkout: &mut Worktree,
    pull_request: &mut PullRequest,
    context: recovery::Continuation,
    on_origin: bool,
    error: &anyhow::Error,
    stage: recovery::Stage,
) -> crate::failed_run::FailedRun {
    let reason = Cause::of(error).first_line().to_string();
    let preserved = checkout.preserve_failed_run(&context.base, &reason, &mut || {
        pull_request.ensure_preservable()
    });
    let preservation = recovery::preservation_of(preserved, progress::step);
    let mut recovery = Recovery::new(stage, preservation);
    recovery.record_worktree(checkout.path().to_path_buf());
    recovery.add_command_context(context, on_origin);
    let mut failed: crate::failed_run::FailedRun = anyhow!("{error:#}").into();
    failed.recovery = Some(Box::new(recovery));
    match pull_request.finish_failed_run(false) {
        Ok(url) => failed.pr_url = url,
        Err(error) => progress::step(format_args!(
            "warning: could not finish the failed PR: {error:#}"
        )),
    }
    failed
}

pub(super) fn resolve_process_issue(issue: Option<IssueInput>) -> Result<Option<IssueUrl>> {
    let Some(issue) = issue else {
        return Ok(None);
    };
    let issue = launch::resolve_issue(issue)?;
    let origin = Git::new(std::env::current_dir()?).run(&["config", "remote.origin.url"])?;
    if !issue.matches_origin(&origin) {
        bail!(
            "origin mismatch: {} is not in the repository at origin {origin}",
            issue.url
        );
    }
    Ok(Some(issue))
}

pub(super) fn require_issue(
    path: &Path,
    file: &ProcessFile,
    issue: Option<&IssueUrl>,
) -> Result<()> {
    ensure!(
        !review::has_pull_request(file),
        "{} needs a pull-request <PR URL> input",
        path.display()
    );
    let Some(worktree) = file
        .worktrees
        .iter()
        .find(|worktree| worktree.branch_line().is_some())
    else {
        return Ok(());
    };
    let spec = worktree.kind == WorktreeKind::SpecBranch;
    let kind = if spec { "Spec" } else { "Issue" };
    if let Some(issue) = issue {
        let tickets = GitHub::new().tickets(issue)?;
        if spec == tickets.is_empty() {
            return Err(rule(
                path,
                worktree.branch_line().expect("branch line"),
                "issue",
                if tickets.is_empty() {
                    "a Spec-branch worktree needs an issue with sub-issues"
                } else {
                    "an Issue-branch worktree refuses an issue with sub-issues"
                },
            ));
        }
        return Ok(());
    }
    Err(rule(
        path,
        worktree.branch_line().expect("branch line"),
        "issue",
        format!(
            "a{article} {kind}-branch worktree needs an issue",
            article = if kind == "Issue" { "n" } else { "" }
        ),
    ))
}

/// All Ticket source and publication policy used by the Step engine.
/// The local adapter borrows a prepared repository; it never owns its cleanup.
pub(super) enum TicketSource {
    Factory,
    Local(Box<Local>),
}

pub(super) struct Local {
    choice: Choice,
    repository: PathBuf,
    issue_text: String,
    issue: IssueUrl,
    base: String,
    /// Base commit sampled before any session. Later moves of the base ref
    /// must not shrink the Security review diff.
    base_commit: String,
    branch: String,
    output: PathBuf,
    sandbox: Option<crate::sandbox::Launch>,
    last_log: Mutex<Option<PathBuf>>,
    steps: Mutex<Vec<crate::trial::Step>>,
}

impl TicketSource {
    pub(super) fn prepare_choices(
        &self,
        command: &mut CommandChoice,
        steps: &mut [ResolvedStep],
        security_review: bool,
        config: &UserConfig,
    ) {
        match self {
            Self::Factory => {
                command.check_steps(steps);
                for step in steps {
                    if matches!(step.kind, StepKind::SecurityReview) {
                        step.check_security_review(security_review, config);
                    }
                }
            }
            Self::Local(local) => {
                command.choice = local.choice.clone();
                for step in steps {
                    if !matches!(step.sources.harness, Source::Step) {
                        step.choice.harness = local.choice.harness;
                    }
                    if !matches!(step.sources.model, Source::Step) {
                        step.choice.model = local.choice.model.clone();
                    }
                    if !matches!(step.sources.effort, Source::Step) {
                        step.choice.effort = local.choice.effort.clone();
                    }
                }
            }
        }
    }

    pub(super) fn session<T>(
        &self,
        run: &StepRun<'_>,
        worktree: &Path,
        index: usize,
        action: impl FnOnce(&session::Sessions<'_>, usize) -> Result<T>,
    ) -> Result<T> {
        match self {
            Self::Local(local) => {
                local
                    .session(
                        index,
                        &run.steps[index - 1].name,
                        worktree,
                        run.choice,
                        action,
                    )
                    .0
            }
            Self::Factory => {
                let mut logs = session::Logs::of_process(&launch::repo()?);
                if let Some(identity) = run.dispatched {
                    logs = logs.for_child(
                        run.issue.expect("a dispatched Ticket has an issue"),
                        identity.clone(),
                    );
                }
                let before_each = || match run.issue {
                    Some(issue) => prepare_for_session(issue, run.started, worktree),
                    None => Ok(()),
                };
                session::Sessions::checked_within(
                    &logs,
                    worktree,
                    run.choice,
                    None,
                    None,
                    &before_each,
                    |sessions| action(sessions, index),
                )
                .0
            }
        }
    }

    pub(super) fn local_checkout(&self) -> Option<&Path> {
        match self {
            Self::Factory => None,
            Self::Local(local) => Some(&local.repository),
        }
    }

    pub(super) fn allow_step(&self, kind: &StepKind) -> Result<()> {
        if matches!(self, Self::Local(_)) {
            Local::allow_step(kind)?;
        }
        Ok(())
    }

    pub(super) fn results_dir(&self, dispatched: Option<&IssueUrl>) -> Result<PathBuf> {
        match self {
            Self::Factory => step_results_dir(dispatched),
            Self::Local(local) => {
                let path = local.output.join("steps");
                fs::create_dir_all(&path)?;
                Ok(path)
            }
        }
    }

    pub(super) fn implement(
        &self,
        run: &StepRun<'_>,
        worktree: &Path,
        step: &ResolvedStep,
    ) -> Result<String> {
        match self {
            Self::Factory => run_implement(run, worktree, step),
            Self::Local(local) => {
                let text = prompt::local_implement(&local.issue, &local.base, &local.branch);
                run_session_step(run, worktree, step, |sessions, index| {
                    let message = sessions.run(session::Kind::ProcessStep(index), &text)?;
                    crate::trial::check_question(message.as_deref())?;
                    local.ensure_actionable()?;
                    Ok(message.unwrap_or_default())
                })
            }
        }
    }

    pub(super) fn security_review(
        &self,
        prepared: &Prepared,
        step: &ResolvedStep,
        index: usize,
        issue_run: Option<&mut IssueRun<'_>>,
    ) -> Result<String> {
        match self {
            Self::Factory => run_security_review(
                prepared,
                step,
                index,
                issue_run.context("security-review has no Issue branch")?,
            ),
            Self::Local(local) => {
                let text = prompt::local_security_review(&local.issue, &local.base, &local.branch);
                local
                    .session(
                        index + 1,
                        &step.name,
                        &local.repository,
                        &step.choice,
                        |sessions, _| {
                            local.ensure_actionable()?;
                            let review = crate::security::review::run_local(
                                sessions,
                                &local.repository,
                                &local.issue,
                                &local.base_commit,
                                &local.output,
                                &text,
                            )?;
                            local.ensure_actionable()?;
                            retain_killed_work(&prepared.killed_work, index + 1, sessions);
                            let outcome = crate::security::review::Outcome::of(
                                Ok(review),
                                sessions.last_log_of(session::Kind::SecurityReview),
                            );
                            if let Some(hold) = outcome.hold() {
                                return Err(hold.into());
                            }
                            Ok(outcome.entry())
                        },
                    )
                    .0
            }
        }
    }
}

impl Local {
    pub(super) fn allow_step(kind: &StepKind) -> Result<()> {
        match kind {
            StepKind::Implement
            | StepKind::SecurityReview
            | StepKind::Skill { .. }
            | StepKind::Command(_) => Ok(()),
            StepKind::Delivery => bail!("local Ticket source refuses publishing Step delivery"),
            StepKind::SelfMerge => bail!("local Ticket source refuses publishing Step self-merge"),
            StepKind::ReviewAndAddress
            | StepKind::ArchitectureReview
            | StepKind::SpecReview
            | StepKind::SecurityAudit
            | StepKind::ReproduceFindings
            | StepKind::RunPublishedIssue { .. }
            | StepKind::LandTickets { .. }
            | StepKind::FixFindings { .. }
            | StepKind::CloseFindings => bail!("this Step requires the factory Ticket source"),
        }
    }
}

impl Local {
    fn ensure_actionable(&self) -> Result<()> {
        let git = Git::new(&self.repository);
        ensure!(
            git.run(&["symbolic-ref", "--short", "HEAD"])? == self.branch,
            "local Ticket source requires Issue branch {}",
            self.branch
        );
        git.run(&["merge-base", &self.base, "HEAD"])?;
        Ok(())
    }

    pub(super) fn session<T>(
        &self,
        index: usize,
        role: &str,
        worktree: &Path,
        choice: &Choice,
        action: impl FnOnce(&session::Sessions<'_>, usize) -> Result<T>,
    ) -> (Result<T>, Option<PathBuf>) {
        let logs = session::Logs::of_trial_step(&self.output, role);
        let before_each = || {
            self.ensure_actionable()?;
            restore_issue_file(&self.repository, &self.issue_text)?;
            if worktree != self.repository {
                skills::exclude(&Git::new(worktree), &format!("/{}", prompt::ISSUE_FILE))?;
                restore_issue_file(worktree, &self.issue_text)?;
            }
            Ok(())
        };
        let action = |sessions: &session::Sessions<'_>| action(sessions, index);
        // Different Step Harnesses need their own login, provider allowlist
        // and launcher. Keep its guard alive through completion and Resume.
        let alternate = if self.sandbox.is_some()
            && (choice.harness != self.choice.harness
                || (choice.harness == Harness::OpenCode && choice.model != self.choice.model))
        {
            match crate::sandbox::for_trial(choice, &self.repository, &self.output) {
                Ok(sandbox) => sandbox,
                Err(error) => return (Err(error), None),
            }
        } else {
            None
        };
        let launch = alternate
            .as_ref()
            .map(|prepared| prepared.launch())
            .or_else(|| self.sandbox.clone())
            .map(|launch| {
                launch
                    .with_writable_directory(worktree)
                    .with_writable_directory(&self.output.join("security-reviews"))
            });
        let result = match launch.as_ref() {
            Some(sandbox) => session::Sessions::checked_within_sandboxed(
                &logs,
                worktree,
                choice,
                sandbox,
                &before_each,
                action,
            ),
            None => session::Sessions::checked_within(
                &logs,
                worktree,
                choice,
                None,
                None,
                &before_each,
                action,
            ),
        };
        if result.1.is_some() {
            *self.last_log.lock().expect("local session log") = result.1.clone();
        }
        result
    }
}

/// Execute a Process's non-publishing prefix in a repository
/// the Trial caller already prepared, with its local origin and issue file.
pub(crate) fn run_local(
    process: &str,
    repository: &Path,
    inputs: &crate::trial::Inputs,
    choice: &Choice,
    output: &Path,
    sandbox: Option<crate::sandbox::Launch>,
) -> (Result<String>, Option<PathBuf>, Vec<crate::trial::Step>) {
    let mut last_log = None;
    let mut steps = Vec::new();
    let result = (|| {
        validate_local(process)?;
        let git = Git::new(repository);
        let origin = git.run(&["remote", "get-url", "origin"])?;
        ensure!(
            Path::new(&origin).is_absolute() && Path::new(&origin).is_dir(),
            "local Ticket source requires a local origin directory"
        );
        let reports = output.join("security-reviews");
        fs::create_dir_all(&reports)?;
        let sandbox = sandbox.map(|sandbox| sandbox.with_writable_directory(&reports));
        // Sample before any session. The Security review diffs against this
        // commit, not against whatever the base ref names afterwards.
        let commit = git.run(&["rev-parse", &inputs.base])?;
        let source = Local {
            choice: choice.clone(),
            repository: repository.to_path_buf(),
            issue_text: read_regular_issue(repository)?,
            issue: IssueUrl::parse(&inputs.issue_url)?,
            base: inputs.base.clone(),
            base_commit: commit.clone(),
            branch: inputs.branch.clone(),
            output: output.to_path_buf(),
            sandbox,
            last_log: Mutex::default(),
            steps: Mutex::default(),
        };
        source.ensure_actionable()?;
        skills::exclude(&git, &format!("/{}", prompt::ISSUE_FILE))?;
        let (path, _, mut file) = local_file(process)?;
        file.security_review = true;
        // Stopping is the caller's policy; the source still refuses a publishing
        // Step if another caller tries to execute one.
        let stop = file
            .steps
            .iter()
            .position(|step| {
                matches!(
                    step.kind,
                    DraftKind::Delivery { .. } | DraftKind::SelfMerge { .. }
                )
            })
            .context("Ticket Process has no publishing Step")?;
        let publishing = file.steps[stop].name.clone();
        file.steps.truncate(stop);
        let prepared = prepare_from(
            &path,
            &UserConfig::load()?,
            &Asked {
                harness: Some(choice.harness),
                model_and_effort: harness::ModelAndEffort {
                    model: choice.model.clone(),
                    effort: choice.effort.clone(),
                },
            },
            file,
            Some(source.issue.clone()),
            Utc::now(),
            WorktreeSource {
                ticket: TicketSource::Local(Box::new(source)),
                git: &git,
                commit: &commit,
                issue_checkout: Some(repository),
                base: &inputs.base,
            },
        )?;
        let result = execute_steps(process, &prepared, None);
        if let TicketSource::Local(local) = &prepared.source {
            last_log = local.last_log.lock().expect("local session log").clone();
            steps = std::mem::take(&mut *local.steps.lock().expect("local Steps"));
        }
        if result.is_ok() {
            progress::step(format!("stopped before publishing Step {publishing}"));
        }
        result
    })();
    (result, last_log, steps)
}

fn local_file(process: &str) -> Result<(PathBuf, String, ProcessFile)> {
    let (path, text) = if process.ends_with(".toml") {
        let path = PathBuf::from(process);
        let text = fs::read_to_string(&path)
            .with_context(|| format!("could not read Process {}", path.display()))?;
        (path, text)
    } else {
        Catalog::installed()?.source(process)?
    };
    let file = validated(&path, &text)?;
    Ok((path, text, file))
}

/// Refuse factory acquisition before building Arms or starting sessions.
pub(crate) fn validate_local(process: &str) -> Result<()> {
    let (_, _, file) = local_file(process)?;
    validate_local_file(process, &file)
}

fn validate_local_file(process: &str, file: &ProcessFile) -> Result<()> {
    ensure!(
        file.worktrees
            .iter()
            .any(|w| w.kind == WorktreeKind::IssueBranch),
        "Process {process} requires GitHub before its first publishing Step: no Issue-branch worktree"
    );
    let mut publishing = false;
    for step in &file.steps {
        match step.kind {
            DraftKind::Delivery { .. } | DraftKind::SelfMerge { .. } => {
                publishing = true;
                break;
            }
            DraftKind::Implement { .. }
            | DraftKind::SecurityReview { .. }
            | DraftKind::Skill { .. }
            | DraftKind::Command { .. } => {}
            DraftKind::ReviewAndAddress { .. }
            | DraftKind::ArchitectureReview { .. }
            | DraftKind::SpecReview { .. }
            | DraftKind::SecurityAudit { .. }
            | DraftKind::ReproduceFindings { .. }
            | DraftKind::RunPublishedIssue { .. }
            | DraftKind::LandTickets { .. }
            | DraftKind::FixFindings { .. }
            | DraftKind::CloseFindings { .. } => bail!(
                "Process {process} Step {} requires GitHub before its first publishing Step",
                step.name
            ),
        }
    }
    ensure!(publishing, "Process {process} has no publishing Step");
    Ok(())
}

pub(crate) struct LocalInputs {
    pub name: String,
    pub text: String,
    pub steps: Vec<(String, Choice)>,
    pub skills: Vec<String>,
}

/// Capture the same Step choices used by the local engine, without probes.
pub(crate) fn local_inputs(
    process: &str,
    choice: &Choice,
    compiled: Option<&str>,
) -> Result<LocalInputs> {
    let (path, text, file) = match compiled {
        Some(text) => {
            let path = PathBuf::from("processes").join(format!("{process}.toml"));
            let file = validated(&path, text)?;
            (path, text.to_owned(), file)
        }
        None => local_file(process)?,
    };
    validate_local_file(process, &file)?;
    let command = CommandChoice {
        choice: choice.clone(),
        sources: Sources {
            harness: Source::Command,
            model: Source::Command,
            effort: Source::Command,
        },
    };
    let steps = file
        .steps
        .iter()
        .map(|step| (step.name.clone(), command.step(step).0))
        .collect();
    let skills = file
        .steps
        .iter()
        .filter_map(|step| match &step.kind {
            DraftKind::Skill { skill, .. } => Some(skill.clone()),
            DraftKind::ReviewAndAddress { .. }
            | DraftKind::Command { .. }
            | DraftKind::Implement { .. }
            | DraftKind::ArchitectureReview { .. }
            | DraftKind::SpecReview { .. }
            | DraftKind::SecurityReview { .. }
            | DraftKind::SecurityAudit { .. }
            | DraftKind::ReproduceFindings { .. }
            | DraftKind::Delivery { .. }
            | DraftKind::SelfMerge { .. }
            | DraftKind::RunPublishedIssue { .. }
            | DraftKind::LandTickets { .. }
            | DraftKind::FixFindings { .. }
            | DraftKind::CloseFindings { .. } => None,
        })
        .collect();
    Ok(LocalInputs {
        name: path
            .file_stem()
            .and_then(|name| name.to_str())
            .context("Process name must be UTF-8")?
            .into(),
        text,
        steps,
        skills,
    })
}

impl TicketSource {
    pub(super) fn command(
        &self,
        worktree: &Path,
        command: &str,
        results: &Path,
        result_file: &Path,
        input: Option<CommandInput<'_>>,
    ) -> Result<String> {
        let sandbox = match self {
            Self::Local(local) => local
                .sandbox
                .clone()
                .map(|launch| launch.with_writable_directory(worktree)),
            Self::Factory => None,
        };
        run_command(
            worktree,
            command,
            results,
            result_file,
            input,
            sandbox.as_ref(),
        )
    }

    pub(super) fn record_step(
        &self,
        step: &ResolvedStep,
        began_at: DateTime<Utc>,
        elapsed: Duration,
        result: &Result<String>,
    ) -> Result<()> {
        if let Self::Local(local) = self {
            let mut steps = local.steps.lock().expect("local Steps");
            steps.push(crate::trial::Step {
                name: step.name.clone(),
                outcome: if result.is_ok() {
                    crate::trial::Outcome::Succeeded
                } else if result
                    .as_ref()
                    .is_err_and(|e| e.is::<crate::trial::StoppedToAsk>())
                {
                    crate::trial::Outcome::StoppedToAsk
                } else {
                    crate::trial::Outcome::Failed
                },
                started_at: began_at.to_rfc3339(),
                ended_at: Utc::now().to_rfc3339(),
                elapsed_ms: elapsed.as_millis(),
            });
            let mut saved = tempfile::NamedTempFile::new_in(&local.output)?;
            serde_json::to_writer(&mut saved, &*steps)?;
            saved.persist(local.output.join("steps.json"))?;
        }
        Ok(())
    }
}

/// The supplied issue file, read without following a symlink. A repository
/// can point that path at a file outside the checkout; the session must not
/// receive that file's contents.
fn read_regular_issue(repository: &Path) -> Result<String> {
    let path = repository.join(prompt::ISSUE_FILE);
    if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        bail!("local Ticket source refuses a symlinked issue file");
    }
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
        .with_context(|| format!("could not read {}", path.display()))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file(),
        "local Ticket source requires a regular issue file"
    );
    let mut text = String::new();
    file.read_to_string(&mut text)
        .with_context(|| format!("could not read {}", path.display()))?;
    Ok(text)
}

/// Put the sampled issue text back without following a symlink planted by an
/// earlier sandboxed session. Following one would write outside the repository.
fn restore_issue_file(repository: &Path, text: &str) -> Result<()> {
    let path = repository.join(prompt::ISSUE_FILE);
    if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        fs::remove_file(&path)
            .with_context(|| format!("could not remove symlinked {}", path.display()))?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
        .with_context(|| format!("could not write {}", path.display()))?;
    file.write_all(text.as_bytes())
        .with_context(|| format!("could not write {}", path.display()))?;
    Ok(())
}

/// Ends the Claim on drop unless the run removed it or ended it already.
struct HeldClaim<'a> {
    claim: Option<Claim<'a>>,
}

impl HeldClaim<'_> {
    fn remove(&mut self) {
        if let Some(claim) = self.claim.take() {
            claim.remove();
        }
    }

    fn end(&mut self, ending: claim::Ending) {
        if let Some(claim) = self.claim.take() {
            claim.end(ending);
        }
    }
}

impl Drop for HeldClaim<'_> {
    fn drop(&mut self) {
        self.end(claim::Ending::Failed);
    }
}

pub(super) fn run_issue_branch(
    command: &ProcessRun,
    config: &UserConfig,
    path: &Path,
    file: ProcessFile,
    issue: IssueUrl,
    started: DateTime<Utc>,
    mut notification: RunNotification,
) -> Result<String> {
    let name = &command.name;
    let base_branch = match launch::checked_out_base() {
        Ok(base) => base,
        Err(error) => return Err(failed_before_steps(notification, error)),
    };
    let launch_git = match std::env::current_dir() {
        Ok(dir) => Git::new(dir),
        Err(error) => return Err(failed_before_steps(notification, error.into())),
    };
    let selection = match branch::select(&launch_git, &GitHub::new(), &issue) {
        Ok(selection) => selection,
        Err(error) => return Err(failed_before_steps(notification, error)),
    };
    let base = selection
        .pr_base(None, Some(base_branch.name()), |line| progress::step(&line))
        .unwrap_or(base_branch.name())
        .to_string();
    let on_origin = matches!(selection, Selection::Continuation { .. });
    let branch = selection.branch().to_string();
    let made = match claim::make(&issue, &launch_git, &branch) {
        Ok(claim) => claim,
        Err(error) => return Err(failed_before_steps(notification, error)),
    };
    let mut claim = HeldClaim { claim: Some(made) };
    let checkout = match &selection {
        Selection::Fresh { .. } => Worktree::create_fresh(&launch_git, &issue.repo, &branch, &base),
        Selection::Continuation { .. } => {
            Worktree::continue_existing(&launch_git, &issue.repo, &branch, &base)
        }
    };
    let checkout = match checkout {
        Ok(checkout) => checkout,
        Err(error) => return Err(failed_before_steps(notification, error)),
    };
    let commit = if base == base_branch.name() {
        base_branch.origin_commit().to_string()
    } else {
        match launch_git.run(&["rev-parse", &format!("origin/{base}")]) {
            Ok(commit) => commit,
            Err(error) => return Err(failed_before_steps(notification, error)),
        }
    };
    let prepared = match prepare_from(
        path,
        config,
        &command.asked,
        file,
        Some(issue.clone()),
        started,
        WorktreeSource {
            ticket: source::TicketSource::Factory,
            git: &launch_git,
            commit: &commit,
            issue_checkout: Some(checkout.path()),
            base: &base,
        },
    ) {
        Ok(prepared) => prepared,
        Err(error) => return Err(failed_before_steps(notification, error)),
    };
    if let Err(error) = refuse_unstartable(&prepared) {
        return Err(failed_before_steps(notification, error));
    }
    let repo = match launch::repo() {
        Ok(repo) => repo,
        Err(error) => return Err(failed_before_steps(notification, error)),
    };
    let mut held = Some(checkout);
    // Session traces ask for the log root, which exists once this command is
    // configured. The Command record stays the Process; the traces are named
    // for the issue.
    logs::begin(Begin::Process { name, pass: false });
    logs::configured(config);
    let logs = session::Logs::of_run(&issue);
    let mut base_fix = BaseFix::new(None, BaseFixAsk::Forbid);
    logs::started(Work::Process { repo: &repo, name }, &prepared.choice);
    notification.built_with(&prepared.choice);
    let delivered;
    let result = {
        let mut issue_run = IssueRun {
            checkout: &mut held,
            issue: &issue,
            branch: &branch,
            base: &base,
            on_origin,
            selection: &selection,
            logs: &logs,
            base_fix: &mut base_fix,
            process: name,
            delivered: false,
            pull_request: PullRequest::new(&issue, &branch, &base, GitHub::new()),
            reviews: Vec::new(),
            spec_tickets: None,
            child_asks: ChildAsks::default(),
            failure: None,
        };
        let result = execute_issue_steps(name, &prepared, &mut issue_run);
        delivered = issue_run.delivered;
        prepared
            .completed
            .lock()
            .expect("completed Runs")
            .failed_run = issue_run.failure.take();
        result
    };
    let completed = prepared.completed.lock().expect("completed Runs");
    let not_reproduced = completed.unreproduced_bug.is_some();
    // An Architecture review that ends before Delivery did not publish the
    // Issue branch. Release the Claim, as a run that published nothing does,
    // instead of removing it the way a finished Delivery does.
    let ended_before_delivery = completed.architecture_end.is_some() && !delivered;
    drop(completed);
    if result.is_ok() && not_reproduced {
        claim.end(claim::Ending::NotReproduced);
    } else if (delivered || (!prepared.spec_branch && result.is_ok())) && !ended_before_delivery {
        claim.remove();
    } else if result.is_ok() {
        claim.end(claim::Ending::BeforeDelivery);
    } else {
        claim.end(claim::Ending::Failed);
    }
    let (outcome, cause) = classify(&result);
    let completed = std::mem::take(&mut *prepared.completed.lock().expect("completed Runs"));
    drop(prepared);
    drop(held);
    logs::ended(outcome);
    if let Ok(comment_url) = &result
        && not_reproduced
    {
        logs::complete(
            "bug not reproduced",
            Some(&format!("bug not reproduced: {comment_url}")),
        );
    } else {
        logs::complete(
            outcome,
            cause.as_deref().or(completed.architecture_end.as_deref()),
        );
    }
    notify(notification, outcome, cause.as_deref(), &completed);
    result
}

pub(super) fn run_security_review(
    prepared: &Prepared,
    step: &ResolvedStep,
    index: usize,
    issue_run: &mut IssueRun<'_>,
) -> Result<String> {
    if !prepared.security_review {
        let skipped = "Security review skipped: not requested";
        progress::step(skipped);
        return Ok(skipped.to_string());
    }
    let checkout = issue_run
        .checkout
        .as_ref()
        .context("the Issue-branch worktree is gone")?;
    let before_each = || prepare_for_session(issue_run.issue, prepared.started, checkout.path());
    let text = prompt::security_review(issue_run.issue, issue_run.base, issue_run.branch);
    let mut logs = session::Logs::of_process_review(&issue_run.issue.repo(), index + 1);
    if let Some(identity) = &prepared.dispatched {
        logs = logs.for_child(issue_run.issue, identity.clone());
    }
    let (review, log) = session::Sessions::checked_within(
        &logs,
        checkout.path(),
        &step.choice,
        None,
        step.review_default.as_ref(),
        &before_each,
        |sessions| {
            let review = crate::security::review::step::Step {
                sessions,
                worktree: checkout,
                issue: issue_run.issue,
                base: issue_run.base,
            }
            .run(&mut issue_run.pull_request, &text);
            retain_killed_work(&prepared.killed_work, index + 1, sessions);
            review
        },
    );
    let review = match review {
        Ok(review) => review,
        Err(error) if error.is::<delivery::UntrustedEdit>() => return Err(error),
        Err(error) => crate::security::review::Outcome::of(Err(error), log),
    };
    let entry = review.entry();
    issue_run.reviews.push(review);
    issue_run.pull_request.security_reviewed(
        issue_run
            .reviews
            .iter()
            .map(crate::security::review::Outcome::unaddressed_count)
            .sum(),
    );
    Ok(entry)
}

/// Publish the Issue branch, and when `merge_follows` run the Self-merge in
/// that same Delivery. A protected path, `self_merge = false`, or a policy
/// refusal leaves the PR ready and fails the self-merge Step.
pub(super) fn run_delivery(
    prepared: &Prepared,
    step: &ResolvedStep,
    issue_run: &mut IssueRun<'_>,
    merge_follows: bool,
) -> (Result<String>, Option<Result<String>>) {
    let checkout = match issue_run.checkout.take() {
        Some(checkout) => checkout,
        None => return (Err(anyhow!("the Issue-branch worktree is gone")), None),
    };
    let asked = prepared.goal == Goal::Merged;
    let forbid = prepared.limits.forbid_self_merge;
    let merge = merge_follows && asked && !forbid;
    progress::step(format_args!(
        "running step {} with {choice}",
        step.name,
        choice = step.choice
    ));
    if merge {
        let merge_step = prepared
            .steps
            .iter()
            .find(|step| matches!(step.kind, StepKind::SelfMerge));
        if let Some(merge_step) = merge_step {
            progress::step(format_args!("running step {}", merge_step.name));
        }
    }
    let child_asks = ChildAsks {
        harness: step.choice.clone(),
        security_fix: prepared.security_fix,
        delivery_limits: delivery::Limits {
            repair_cap: prepared.limits.repair_cap,
            protected_paths: prepared.limits.protected_paths.clone(),
        },
        ..issue_run.child_asks.clone()
    };
    let started = prepared.started;
    let protected = if merge {
        prepared.limits.protected_paths.as_slice()
    } else {
        &[]
    };
    let published = (delivery::Delivery {
        security_review: false,
        child_asks: &child_asks,
        issue: issue_run.issue,
        base: issue_run.base,
        goal: if merge {
            Goal::Merged
        } else {
            Goal::ReadyForReview
        },
        base_fix: issue_run.base_fix,
        logs: issue_run.logs,
        started,
        repair_cap: prepared.limits.repair_cap,
        protected_paths: protected,
    })
    .publish(
        checkout,
        &mut issue_run.pull_request,
        &issue_run.reviews,
        issue_run
            .spec_tickets
            .as_ref()
            .map(|tickets| tickets.checklist.as_str()),
    );
    match published {
        Ok(url) => {
            issue_run.delivered = true;
            prepared
                .completed
                .lock()
                .expect("completed Runs")
                .resolutions
                .push(crate::run::Resolution::PullRequest {
                    url: url.clone(),
                    goal: if merge {
                        Goal::Merged
                    } else {
                        Goal::ReadyForReview
                    },
                });
            let merge_result = if !merge_follows {
                None
            } else if asked && forbid {
                Some(Err(anyhow!(
                    "self_merge = false holds the Self-merge; the pull request stays ready for review"
                )))
            } else {
                Some(Ok(url.clone()))
            };
            (Ok(url), merge_result)
        }
        Err(mut failed) => {
            let held = failed.error.is::<crate::failed_run::MergeHold>()
                || failed.error.is::<crate::failed_run::PolicyRefusal>()
                || failed.error.is::<crate::security::review::Hold>();
            if held {
                issue_run.delivered = true;
            } else if let Some(recovery) = &mut failed.recovery {
                recovery.add_command_context(continuation(issue_run), issue_run.on_origin);
            }
            let url = failed.pr_url.clone().unwrap_or_default();
            // Keep the typed cause in the Step result, including any context
            // execute_steps adds. run_custom rejoins it with these facts.
            let error = std::mem::replace(
                &mut failed.error,
                anyhow!("the Step result carries the failure cause"),
            );
            issue_run.failure = Some(failed);
            if held {
                (Ok(url), Some(Err(error)))
            } else {
                (Err(error), None)
            }
        }
    }
}

/// Check the issue, then write it into the worktree for the session about
/// to start. An untrusted edit fails the Step instead of starting it.
pub(super) fn prepare_for_session(
    issue: &IssueUrl,
    started: DateTime<Utc>,
    worktree: &Path,
) -> Result<()> {
    delivery::no_untrusted_edit_since(issue, started, "Process")?;
    delivery::hand_over_issue(issue, worktree)
}

pub(super) fn run_implement(
    run: &StepRun<'_>,
    worktree: &Path,
    step: &ResolvedStep,
) -> Result<String> {
    let issue = run.issue.context("implement needs an issue")?;
    let checkout = run
        .checkout
        .filter(|_| run.issue_branch)
        .context("implement needs an Issue-branch worktree")?;
    let selection = run.selection.context("implement needs an Issue branch")?;
    let labels = GitHub::new().issue_labels(issue)?;
    let opening = crate::implement::prepare(
        issue,
        run.base,
        checkout,
        delivery::Opening {
            session: delivery::OpeningSession::Implementation,
            prompt: crate::implement::prompt(issue, run.base, selection),
        },
        Some(&labels),
    )?;
    let mut pr = PullRequest::new(issue, checkout.branch(), run.base, GitHub::new());
    run_session_step(run, worktree, step, |sessions, index| {
        let completed = crate::implement::run(issue, &opening, |prompt| {
            let message = sessions.run(session::Kind::ProcessStep(index), prompt)?;
            pr.ensure_actionable(false)?;
            Ok(message)
        })?;
        match completed {
            crate::implement::Completed::Implemented(message) => Ok(message.unwrap_or_default()),
            crate::implement::Completed::NotReproduced(comment_url) => {
                crate::implement::confirm_needs_info(&GitHub::new().issue_labels(issue)?)?;
                let mut completed = run.completed.lock().expect("completed Runs");
                completed.unreproduced_bug = Some(comment_url.clone());
                completed
                    .resolutions
                    .push(crate::run::Resolution::NotReproduced {
                        comment_url: comment_url.clone(),
                    });
                Ok(comment_url)
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_source_refuses_publishing_steps() {
        for (kind, name) in [
            (StepKind::Delivery, "delivery"),
            (StepKind::SelfMerge, "self-merge"),
        ] {
            let error = Local::allow_step(&kind).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("local Ticket source refuses publishing Step {name}")
            );
        }
    }
}
