//! Pull-request Processes share eligibility, captured-head checkout, session
//! execution and comment delivery. Recommend-only never publishes branch work.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;

use super::*;
use crate::args::ReviewArgs;
use crate::github::review::ReviewPullRequest;
use crate::review::{Mode, PullRequestUrl};

mod delivery;

use delivery::deliver;

pub(crate) fn run(command: ReviewArgs) -> Result<String> {
    ReviewProcess::built_in()?.execute(&command.pull_request, &command.flags, command.mode, false)
}

pub(super) fn custom(command: &ProcessRun, checking: bool) -> Result<String> {
    ensure!(
        command.issue.is_none(),
        "a Process takes an issue or a pull request, not both"
    );
    ensure!(
        command.goal != Some(Goal::Merged)
            && command.security_review.is_none()
            && command.security_fix.is_none()
            && command.security_close.is_none()
            && !command.plan_only,
        "merge, security and plan-only flags do not apply to pull-request Processes"
    );
    let process = if checking && command.name == "review-run" {
        ReviewProcess::built_in()?
    } else {
        let (path, file, identity) = load(&command.name)?;
        ensure!(
            has_pull_request(&file),
            "{} needs a pull-request worktree to take a pull request",
            path.display()
        );
        ReviewProcess {
            name: command.name.clone(),
            path,
            file,
            identity,
            built_in: false,
        }
    };
    process.execute(
        command
            .pull_request
            .as_ref()
            .context("pull-request input is required")?,
        &Flags {
            harness: command.asked.clone(),
            email: command.email.clone(),
            ..Flags::default()
        },
        Mode::Address,
        checking,
    )
}

struct ReviewProcess {
    name: String,
    path: PathBuf,
    file: ProcessFile,
    identity: Identity,
    built_in: bool,
}

impl ReviewProcess {
    fn built_in() -> Result<Self> {
        let file = load_inherited_builtin("review-run")?;
        ensure!(
            file.steps.len() == 2
                && matches!(file.steps[0].kind, DraftKind::ReviewAndAddress { .. })
                && matches!(file.steps[1].kind, DraftKind::Delivery { .. }),
            "review-run needs review-and-address followed by delivery"
        );
        Ok(Self {
            name: "review-run".into(),
            path: "processes/review-run.toml".into(),
            file,
            identity: Identity::built_in("review-run", "Review run"),
            built_in: true,
        })
    }

    fn execute(
        &self,
        pr: &PullRequestUrl,
        flags: &Flags,
        mode: Mode,
        checking: bool,
    ) -> Result<String> {
        interrupt::install()?;
        let config = UserConfig::load()?;
        let launch = Git::new(std::env::current_dir()?);
        let origin = launch.run(&["config", "remote.origin.url"])?;
        ensure!(
            pr.issue.matches_origin(&origin),
            "origin mismatch: {} is not in {origin}",
            pr.url
        );
        let repo = pr.issue.repo();
        let mut command = CommandChoice::of(&flags.harness, &config.harness);
        if checking {
            let snapshot = GitHub::new().review_pull_request(pr)?;
            launch.fetch(&[
                &snapshot.branch,
                &snapshot.base,
                &snapshot.head,
                &snapshot.base_head,
            ])?;
            let checkout =
                SkillWorktree::on_pull_request(&launch, &snapshot.branch, &snapshot.head)?;
            let steps = self.resolve(checkout.path(), &mut command)?;
            return Ok(steps
                .iter()
                .map(|step| {
                    format!(
                        "{}; {}; harness: {} ({}), model: {} ({}), effort: {} ({})\n",
                        step_plan(step, true),
                        step.choice,
                        step.choice.harness.name(),
                        step.sources.harness,
                        step.choice.model.as_deref().unwrap_or("default"),
                        step.sources.model,
                        step.choice.effort.as_deref().unwrap_or("default"),
                        step.sources.effort
                    )
                })
                .collect());
        }
        let mut notification = RunNotification::new(
            flags.notification(&config),
            &config.email,
            if self.built_in {
                About::ReviewRun
            } else {
                About::Process { name: &self.name }
            },
        )?;
        logs::begin(if self.built_in {
            Begin::ReviewRun
        } else {
            Begin::Process {
                name: &self.name,
                pass: false,
            }
        });
        logs::configured(&config);
        logs::process_loaded(self.identity.clone());
        command_limit::hold_slot(&repo);
        logs::started(
            if self.built_in {
                Work::ReviewRun(&repo)
            } else {
                Work::Process {
                    repo: &repo,
                    name: &self.name,
                }
            },
            &command.choice,
        );
        let mut last_log = None;
        let mut delivered = false;
        let result = (|| {
            let github = GitHub::new();
            let snapshot = github.review_pull_request(pr)?;
            logs::issue_branch(Some(&snapshot.branch));
            launch.fetch(&[
                &snapshot.branch,
                &snapshot.base,
                &snapshot.head,
                &snapshot.base_head,
            ])?;
            let checkout =
                SkillWorktree::on_pull_request(&launch, &snapshot.branch, &snapshot.head)?;
            let steps = self.resolve(checkout.path(), &mut command)?;
            notification.built_with(&command.choice);
            let git = Git::new(checkout.path());
            skills::exclude(&git, "/.thirdshift-review-input.md")?;
            let input = github.review_input(pr, &snapshot)?;
            // The checked-out branch chooses this path. A symlink here would
            // send the packet write outside the worktree.
            write_review_input(
                &checkout.path().join(".thirdshift-review-input.md"),
                &input.markdown,
            )?;
            let results = step_results_dir(None)?;
            let mut earlier = Vec::new();
            let mut began = Vec::new();
            let mut findings = None;
            let mut last = String::new();
            let before_each = || unchanged(&github, pr, &snapshot);
            for (index, step) in steps.iter().enumerate() {
                crate::resources::wait()?;
                if mode == Mode::Address && matches!(step.kind, StepKind::Delivery) {
                    compatible(&github, pr, &snapshot)?;
                } else {
                    before_each()?;
                }
                let started = Instant::now();
                let began_at = Utc::now();
                progress::step(format_args!(
                    "running step {} with {}",
                    step.name, step.choice
                ));
                let outcome = (|| {
                    Ok(match &step.kind {
                        StepKind::ReviewAndAddress | StepKind::Skill { .. } => {
                            let reviewing = matches!(step.kind, StepKind::ReviewAndAddress);
                            if reviewing {
                                unchanged_checkout(&git, &snapshot)?;
                            }
                            let text = if let StepKind::Skill { name, path } = &step.kind {
                                prompt::process_step(
                                    &self.name,
                                    &step.name,
                                    name,
                                    path,
                                    prompt::EarlierSteps {
                                        results: &earlier,
                                        first: index == 0,
                                    },
                                    None,
                                    prompt::ProcessWorktree::PullRequest {
                                        url: pr,
                                        base: &snapshot.base_head,
                                    },
                                )
                            } else {
                                prompt::pull_request_review(
                                    pr,
                                    &snapshot.base_head,
                                    mode,
                                    prompt::EarlierSteps {
                                        results: &earlier,
                                        first: index == 0,
                                    },
                                )
                            };
                            let logs = session::Logs::of_process_review(&repo, index + 1);
                            let (result, log) = session::Sessions::checked_within(
                                &logs,
                                checkout.path(),
                                &step.choice,
                                None,
                                None,
                                &before_each,
                                |sessions| {
                                    sessions.run_to_final_message(
                                        if reviewing {
                                            session::Purpose::CodeReview
                                        } else {
                                            session::Purpose::Ordinary
                                        },
                                        if reviewing {
                                            session::Kind::CodeReview
                                        } else {
                                            session::Kind::ProcessStep(index + 1)
                                        },
                                        &text,
                                    )
                                },
                            );
                            last_log = log;
                            let message = result?
                                .filter(|text| !text.trim().is_empty())
                                .context("session left no final report")?;
                            if reviewing {
                                findings = Some(message.clone());
                            }
                            message
                        }
                        StepKind::Command(text) => run_command(
                            checkout.path(),
                            text,
                            &results,
                            &results.join(format!("{}.txt", step.name)),
                            Some(CommandInput::PullRequest(&pr.url)),
                        )?,
                        StepKind::Delivery => {
                            let findings = findings
                                .as_deref()
                                .context("delivery needs a review report")?;
                            deliver(&git, &github, pr, &snapshot, mode, &input, findings)?;
                            delivered = true;
                            pr.url.clone()
                        }
                        StepKind::Implement
                        | StepKind::ArchitectureReview
                        | StepKind::SpecReview
                        | StepKind::SecurityReview
                        | StepKind::SecurityAudit
                        | StepKind::ReproduceFindings
                        | StepKind::SelfMerge
                        | StepKind::RunPublishedIssue { .. }
                        | StepKind::LandTickets { .. }
                        | StepKind::FixFindings { .. }
                        | StepKind::CloseFindings => {
                            bail!("unsupported pull-request Step {}", step.name)
                        }
                    })
                })();
                finish_step(
                    step,
                    started.elapsed(),
                    (began_at, outcome),
                    &results,
                    &mut earlier,
                    &mut began,
                    &mut last,
                )?;
            }
            Ok(last)
        })();
        skills::remove_written();
        let outcome = if result.is_ok() {
            if delivered {
                "ready for review"
            } else {
                "succeeded"
            }
        } else if interrupt::requested() {
            "interrupted"
        } else {
            "failed"
        };
        let account = Account {
            recovery: None,
            outcome,
            ended: match &result {
                Err(error) => Err(Cause::of(error)),
                Ok(_) => Ok(format!("{outcome}: {}", pr.url)),
            },
            interrupted: outcome == "interrupted",
            pr_url: Some(&pr.url),
            advice: &[],
            base_fix: None,
            log: last_log.as_deref(),
            ticket_lines: &[],
            review: None,
            failed_reproduction: None,
            pre_audit_failure: None,
            security_findings: None,
            security_findings_waiting: false,
            security_fix: None,
            security_close: None,
            urls: vec![&pr.url],
        };
        logs::finished(&account);
        logs::ended(outcome);
        notification.finish(&account);
        result
    }

    fn resolve(&self, checkout: &Path, command: &mut CommandChoice) -> Result<Vec<ResolvedStep>> {
        let mut steps = self
            .file
            .steps
            .iter()
            .map(|step| resolve(&self.path, step, checkout, command))
            .collect::<Result<Vec<_>>>()?;
        command.check_steps(&mut steps);
        for step in &steps {
            if let Some(failure) = &step.start_failure {
                bail!("step {} cannot start: {failure}", step.name);
            }
        }
        Ok(steps)
    }
}

fn write_review_input(path: &Path, markdown: &str) -> Result<()> {
    let replacing = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!(
                "review input {} is a symlink; refusing to follow it",
                path.display()
            );
        }
        Ok(metadata) if metadata.is_file() => true,
        Ok(_) => bail!("review input {} is not a regular file", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(error)
                .with_context(|| format!("could not inspect review input {}", path.display()));
        }
    };
    let mut options = fs::OpenOptions::new();
    options
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    if replacing {
        options.truncate(true);
    } else {
        options.create_new(true).mode(0o600);
    }
    let mut file = options.open(path).with_context(|| {
        format!(
            "refusing to follow a symlink for the review input {}",
            path.display()
        )
    })?;
    file.write_all(markdown.as_bytes())?;
    Ok(())
}

fn unchanged(github: &GitHub, pr: &PullRequestUrl, snapshot: &ReviewPullRequest) -> Result<()> {
    let current = compatible(github, pr, snapshot)?;
    ensure!(
        current.head == snapshot.head,
        "pull request changed during Review run"
    );
    Ok(())
}

/// Delivery may report a moved head, but never publish to a changed base,
/// branch or an ineligible pull request.
fn compatible(
    github: &GitHub,
    pr: &PullRequestUrl,
    snapshot: &ReviewPullRequest,
) -> Result<ReviewPullRequest> {
    let current = github.review_pull_request(pr)?;
    ensure!(
        current.base_head == snapshot.base_head
            && current.base == snapshot.base
            && current.branch == snapshot.branch,
        "pull request changed during Review run"
    );
    Ok(current)
}

fn unchanged_checkout(git: &Git, snapshot: &ReviewPullRequest) -> Result<()> {
    ensure!(
        git.run(&["rev-parse", "HEAD"])? == snapshot.head
            && git.run(&["branch", "--show-current"])? == snapshot.branch
            && git.run(&["status", "--porcelain"])?.is_empty(),
        "recommend-only session changed the branch; no comment posted"
    );
    Ok(())
}

pub(super) fn has_pull_request(file: &ProcessFile) -> bool {
    file.worktrees
        .iter()
        .any(|worktree| worktree.kind == WorktreeKind::PullRequest)
}

pub(super) fn validate(path: &Path, file: &ProcessFile) -> Result<()> {
    if !has_pull_request(file) {
        if let Some(line) = file.steps.iter().find_map(|step| {
            if let DraftKind::ReviewAndAddress { line } = step.kind {
                Some(line)
            } else {
                None
            }
        }) {
            return Err(rule(
                path,
                line,
                "review-and-address",
                "review-and-address needs a pull-request worktree",
            ));
        }
        return Ok(());
    }
    ensure!(
        file.worktrees.len() == 1,
        "{}: a pull-request Process needs exactly one pull-request worktree",
        path.display()
    );
    let worktree = &file.worktrees[0].name;
    let mut reviewed = false;
    for (index, step) in file.steps.iter().enumerate() {
        let reason = if step.worktree.as_ref().map(|(name, _)| name) != Some(worktree)
            || step.optional
            || step.parallel.is_some()
            || step.process.is_some()
        {
            Some(
                "pull-request Steps must use the pull-request worktree, be required and sequential, and cannot dispatch a child Process",
            )
        } else {
            match step.kind {
                DraftKind::ReviewAndAddress { .. } if !reviewed => {
                    reviewed = true;
                    None
                }
                DraftKind::Skill { .. } | DraftKind::Command { .. } => None,
                DraftKind::Delivery { .. } if reviewed && index + 1 == file.steps.len() => None,
                DraftKind::ReviewAndAddress { .. }
                | DraftKind::Delivery { .. }
                | DraftKind::Implement { .. }
                | DraftKind::ArchitectureReview { .. }
                | DraftKind::SpecReview { .. }
                | DraftKind::SecurityReview { .. }
                | DraftKind::SecurityAudit { .. }
                | DraftKind::ReproduceFindings { .. }
                | DraftKind::SelfMerge { .. }
                | DraftKind::RunPublishedIssue { .. }
                | DraftKind::LandTickets { .. }
                | DraftKind::FixFindings { .. }
                | DraftKind::CloseFindings { .. } => Some(
                    "pull-request Processes support skill, command, one review-and-address, and delivery as the final Step after a review",
                ),
            }
        };
        if let Some(reason) = reason {
            return Err(rule(path, step.name_line, "pull-request", reason));
        }
    }
    Ok(())
}
