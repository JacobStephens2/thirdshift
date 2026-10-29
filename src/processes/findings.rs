//! Security fix and close catalogue Steps share the Security run's privacy,
//! publication and proof-of-concept gates. Only dispatch selects a Process.

use super::*;
use crate::github::SecurityRecord;
use crate::pass::LaunchAndGitHub;

pub(super) struct MergedFix {
    record: SecurityRecord,
    issue: IssueUrl,
    pr_url: String,
}

pub(super) fn fix(run: &StepRun<'_>, process: &str) -> Result<String> {
    let config = UserConfig::load()?;
    if !run.security_flags.security_fix_allowed(&config) {
        return Ok("Security fixing is not allowed".into());
    }
    let launch = Git::new(std::env::current_dir()?);
    let repo = launch::repo()?;
    let base = launch::checked_out_base()?;
    let flags = Flags {
        goal: Some(if run.limits.forbid_self_merge {
            Goal::ReadyForReview
        } else {
            run.goal
        }),
        ..run.security_flags.clone()
    };
    let mut choice = run.choice.clone();
    let mut outside = LaunchAndGitHub {
        launch: &launch,
        base: &base,
        repo: &repo,
        harness: &mut choice,
        flags: &flags,
        config: &config,
        security_review_harness: None,
    };
    let mut records = GitHub::new().security_records(&repo.slug())?;
    let mut urls = Vec::new();
    while let Some((record, finding)) = records.take_next_fix()? {
        interrupt::check()?;
        let (ended, issue) = crate::security::fix_record(
            &mut outside,
            &repo,
            run.base,
            &record,
            &finding,
            |outside, issue| {
                let mut asks = Asks::of_run(issue, &flags, &config);
                asks.notification = NotificationAsk::Skip;
                asks.harness = outside.harness.clone();
                asks.security_review_harness = outside.security_review_harness.clone();
                asks.delivery_limits = delivery::Limits {
                    repair_cap: run.limits.repair_cap,
                    protected_paths: run.limits.protected_paths.clone(),
                };
                logs::dispatching(
                    if record.fix_size().expect("a published fix has a size")
                        == crate::security::reproduction::FixSize::Spec
                    {
                        Work::SpecRun(issue)
                    } else {
                        Work::Run(issue)
                    },
                    outside.harness,
                    std::slice::from_ref(&finding),
                );
                let ended = crate::run::run_process_to_end(
                    issue,
                    &mut asks,
                    crate::run::StartedBy::Dispatch { base: run.base },
                    process,
                );
                logs::dispatch_ended(&ended);
                ended
            },
        );
        let mut completed = run.completed.lock().expect("completed Runs");
        match ended.outcome {
            Ok(reached) => {
                let url = reached.resolution.url().to_string();
                if reached.resolution.goal() == Some(Goal::Merged) {
                    completed.merged_fixes.push(MergedFix {
                        record,
                        issue: issue.expect("a dispatched fix has an issue"),
                        pr_url: url.clone(),
                    });
                }
                urls.push(url);
                completed.resolutions.push(reached.resolution);
            }
            Err(failed) => {
                completed.failed = true;
                return Err(failed.error);
            }
        }
    }
    Ok(if urls.is_empty() {
        "No reproduced findings await fixing".into()
    } else {
        urls.join("\n")
    })
}

pub(super) fn close(run: &StepRun<'_>) -> Result<String> {
    let config = UserConfig::load()?;
    if !run.security_flags.security_close_allowed(&config) {
        return Ok("Security closing is not allowed".into());
    }
    let launch = Git::new(std::env::current_dir()?);
    let repo = launch::repo()?;
    let base = launch::checked_out_base()?;
    let mut choice = run.choice.clone();
    let mut outside = LaunchAndGitHub {
        launch: &launch,
        base: &base,
        repo: &repo,
        harness: &mut choice,
        flags: run.security_flags,
        config: &config,
        security_review_harness: None,
    };
    let fixes = std::mem::take(&mut run.completed.lock().expect("completed Runs").merged_fixes);
    for fix in &fixes {
        interrupt::check()?;
        crate::security::close_after_merge(
            &mut outside,
            run.base,
            &fix.record,
            &fix.issue,
            &fix.pr_url,
        );
    }
    Ok(format!("Rechecked {} merged Security fix(es)", fixes.len()))
}

pub(super) fn validate(path: &Path, file: &ProcessFile) -> Result<()> {
    for step in &file.steps {
        let (line, name) = match step.kind {
            DraftKind::FixFindings { line } => (line, "fix-findings"),
            DraftKind::CloseFindings { line } => (line, "close-findings"),
            _ => continue,
        };
        if step.parallel.is_some() {
            return Err(rule(
                path,
                line,
                name,
                format!("{name} cannot be in a parallel group"),
            ));
        }
        if file.worktrees.iter().any(|worktree| {
            worktree.kind != WorktreeKind::Throwaway
                && step
                    .worktree
                    .as_ref()
                    .is_some_and(|(name, _)| name == &worktree.name)
        }) {
            return Err(rule(
                path,
                line,
                name,
                format!("{name} needs a throwaway worktree"),
            ));
        }
    }
    Ok(())
}
