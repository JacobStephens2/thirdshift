//! Security fix and close catalogue Steps share the Security run's privacy,
//! publication and proof-of-concept gates. Only dispatch selects a Process.

use super::*;
use crate::pass::LaunchAndGitHub;

/// Keep diagnostics in the local Command log, including when an outer
/// Process dispatches this one and would email its child's failure cause.
pub(super) fn private_failure(error: anyhow::Error) -> anyhow::Error {
    progress::step(format_args!("Security finding Step failed: {error:#}"));
    anyhow!("Security finding Step failed; see the private Command log")
}

pub(super) fn fix(run: &StepRun<'_>, process: &str) -> Result<String> {
    let config = UserConfig::load()?;
    if !run.security_flags.security_fix_allowed(&config) {
        return Ok("Security fixing is not allowed".into());
    }
    // Held for publication, the Ticket run and every child it starts.
    let _suppress_otlp = logs::SuppressOtlp::new();
    run.completed
        .lock()
        .expect("completed Runs")
        .security_findings
        .get_or_insert_default();
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
    let github = GitHub::new();
    let mut records = github.security_records(&repo.slug())?;
    let mut urls = Vec::new();
    while let Some((record, finding)) = records.take_next_fix()? {
        interrupt::check()?;
        run.completed
            .lock()
            .expect("completed Runs")
            .security_findings
            .as_mut()
            .expect("Security Step")
            .push(finding.clone());
        let (ended, _) = crate::security::fix_record(
            &mut outside,
            &repo,
            run.base,
            &record,
            &finding,
            |outside, issue, size| {
                let mut asks = Asks::of_run(issue, &flags, &config);
                asks.notification = NotificationAsk::Skip;
                asks.harness = outside.harness.clone();
                asks.security_review_harness = outside.security_review_harness.clone();
                asks.delivery_limits = delivery::Limits {
                    repair_cap: run.limits.repair_cap,
                    protected_paths: run.limits.protected_paths.clone(),
                };
                logs::dispatching(
                    if size == crate::security::reproduction::FixSize::Spec {
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
                // A finding stays open until close-findings proves the fix,
                // even if Delivery automatically closed its private issue.
                if reached.resolution.goal() == Some(Goal::Merged) {
                    github.reopen_security_finding(&repo.slug(), &record)?;
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
    // The close check's prompt contains the private record.
    let _suppress_otlp = logs::SuppressOtlp::new();
    run.completed
        .lock()
        .expect("completed Runs")
        .security_findings
        .get_or_insert_default();
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
    let github = GitHub::new();
    let fixes = github.security_records(&repo.slug())?.fixes_to_close()?;
    let mut rechecked = 0;
    for (record, issue) in fixes {
        interrupt::check()?;
        let merged = github
            .pull_requests_with_head_prefix(&issue, &format!("issue-{}", issue.number))?
            .into_iter()
            .filter(|pr| pr.state == crate::github::PrState::Merged && pr.base == run.base)
            .filter(|pr| branch::branch_number(&issue, &pr.head).is_some())
            .max_by_key(|pr| branch::branch_number(&issue, &pr.head));
        let Some(pr) = merged else {
            continue;
        };
        {
            let mut completed = run.completed.lock().expect("completed Runs");
            let findings = completed.security_findings.as_mut().expect("Security Step");
            if !findings
                .iter()
                .any(|finding| finding.url == record.metadata().url)
            {
                findings.push(record.metadata().clone());
            }
        }
        let outcome =
            crate::security::close_after_merge(&mut outside, run.base, &record, &issue, &pr.url);
        if matches!(outcome, crate::security::CloseOutcome::LeftOpen { .. }) {
            github.reopen_security_finding(&repo.slug(), &record)?;
        }
        rechecked += 1;
    }
    Ok(format!("Rechecked {rechecked} merged Security fix(es)"))
}

pub(super) fn validate(path: &Path, file: &ProcessFile) -> Result<()> {
    for step in &file.steps {
        let (line, name) = match step.kind {
            DraftKind::FixFindings { line } => (line, "fix-findings"),
            DraftKind::CloseFindings { line } => (line, "close-findings"),
            DraftKind::Skill { .. }
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
            | DraftKind::LandTickets { .. } => continue,
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
