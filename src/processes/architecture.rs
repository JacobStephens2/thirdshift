//! Architecture review as a catalogue Step. The session and conclusion share
//! the Architect run's rules; its skip gates remain in front of that run.

use super::*;
use crate::worktree::ReviewWorktree;

pub(super) fn run(run: &StepRun<'_>, step: &ResolvedStep) -> Result<String> {
    let launch = Git::new(std::env::current_dir()?);
    let repo = launch::repo()?;
    let checkout = ReviewWorktree::create(&launch, &repo.name, run.base)?;
    link_harnesses(checkout.path(), std::slice::from_ref(step))?;
    let origin = launch.run(&["config", "remote.origin.url"])?;
    let started = Utc::now();
    progress::step(format_args!(
        "starting the Architecture review of {}",
        run.base
    ));
    run_session_step(run, checkout.path(), step, |sessions, index| {
        let message = sessions.run_to_final_message(
            session::Purpose::Ordinary,
            session::Kind::ProcessStep(index),
            &prompt::architecture_review(run.base, None),
        )?;
        let reviewed =
            crate::architect::conclude(&mut GitHub::new(), message.as_deref(), &origin, started)?;
        let ending = match &reviewed {
            crate::architect::Reviewed::PlanReady(..) if run.plan_only => {
                Some(format!("{reviewed}; --plan-only requested"))
            }
            crate::architect::Reviewed::PlanReady(..) => None,
            _ => Some(reviewed.to_string()),
        };
        if let Some(ending) = ending {
            run.completed
                .lock()
                .expect("completed Runs")
                .architecture_end = Some(ending);
        }
        Ok(reviewed.url().to_string())
    })
}

pub(super) fn validate(path: &Path, file: &ProcessFile) -> Result<()> {
    for step in &file.steps {
        let DraftKind::ArchitectureReview { line } = step.kind else {
            continue;
        };
        let reason = if step.parallel.is_some() {
            Some("architecture-review cannot be in a parallel group")
        } else if file.worktrees.iter().any(|worktree| {
            worktree.issue_branch_line.is_some()
                && step
                    .worktree
                    .as_ref()
                    .is_some_and(|(name, _)| name == &worktree.name)
        }) {
            Some("architecture-review needs a throwaway worktree")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(rule(path, line, "architecture-review", reason));
        }
    }
    Ok(())
}
