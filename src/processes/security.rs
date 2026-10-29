//! The Built-in Security process, after the Pass lock and skip gates.
//! Existing adapters retain session identity, privacy and one fix per run.

use anyhow::{Result, bail, ensure};

use super::{DraftKind, load_inherited_builtin};
use crate::failed_run::FailedRun;
use crate::github::{RecordedFinding, SecurityRecord};
use crate::issue::{IssueUrl, Repo};
use crate::logs::Work;
use crate::pass::{Dispatch, Outside};
use crate::security::{CloseOutcome, Ended, Outcome, close_after_merge, findings, fix_record};

/// The Command's resolved choices, with a pending fix selected before the file.
pub(crate) struct SecurityRun<'a> {
    pub repo: &'a Repo,
    pub base: &'a str,
    pub fixing: bool,
    pub closing: bool,
    pub offer_command: Option<&'a str>,
    pub pending: Option<(SecurityRecord, RecordedFinding)>,
}

#[derive(Default)]
struct Audit {
    findings: findings::Findings,
    started: bool,
}

impl Audit {
    fn ended(self, result: Result<()>) -> Ended {
        Ended {
            outcome: result
                .map(|()| self.findings.recorded)
                .map_err(|error| FailedRun {
                    log: self.findings.log,
                    ..error.into()
                }),
            audit_started: self.started,
            failed_reproduction: self.findings.failed_reproduction,
            findings: self.findings.findings,
            waiting: self.findings.waiting,
            advice: self.findings.advice,
        }
    }
}

struct Fix {
    record: SecurityRecord,
    finding: RecordedFinding,
    ended: crate::run::Ended,
    issue: Option<IssueUrl>,
    close: CloseOutcome,
}

pub(crate) fn run(outside: &mut impl Outside, run: SecurityRun<'_>) -> Outcome {
    let steps = match steps() {
        Ok(steps) => steps,
        Err(error) => return Outcome::Audited(error.into()),
    };
    let skip_audit = run.pending.is_some();
    let mut pending = run.pending;
    let mut audit = Audit::default();
    let mut fixed: Option<Fix> = None;
    let result = (|| -> Result<()> {
        for step in steps {
            match step {
                DraftKind::SecurityAudit { .. } if !skip_audit => {
                    outside.check_harness(false)?;
                    outside.check_node()?;
                    outside.started(Work::SecurityRun(run.repo));
                    let attempt = outside.audit(run.base);
                    audit.started = attempt.started;
                    audit.findings.log = attempt.log;
                    audit
                        .findings
                        .record(outside, attempt.outcome?, run.offer_command)?;
                }
                DraftKind::ReproduceFindings { .. } if !skip_audit => {
                    audit.findings.reproduce(outside, run.offer_command)?;
                }
                DraftKind::FixFindings { .. } if run.fixing => {
                    if pending.is_none() {
                        pending = outside.security_records()?.next_fix()?;
                    }
                    if let Some((record, finding)) = pending.take() {
                        let (ended, issue) = fix_record(
                            outside,
                            run.repo,
                            run.base,
                            &record,
                            &finding,
                            |outside, issue, size| {
                                outside.dispatch(Dispatch::SecurityFix {
                                    issue,
                                    is_spec: size == crate::security::reproduction::FixSize::Spec,
                                    base: run.base,
                                    finding: &finding,
                                })
                            },
                        );
                        fixed = Some(Fix {
                            record,
                            finding,
                            ended,
                            issue,
                            close: CloseOutcome::NotAttempted,
                        });
                    }
                }
                DraftKind::CloseFindings { .. } if run.closing => {
                    if let Some(fix) = &mut fixed
                        && let (Ok(reached), Some(issue)) = (&fix.ended.outcome, &fix.issue)
                        && reached.resolution.goal() == Some(crate::run::Goal::Merged)
                    {
                        fix.close = close_after_merge(
                            outside,
                            run.base,
                            &fix.record,
                            issue,
                            reached.resolution.url(),
                        );
                    }
                }
                DraftKind::SecurityAudit { .. }
                | DraftKind::ReproduceFindings { .. }
                | DraftKind::FixFindings { .. }
                | DraftKind::CloseFindings { .. } => {}
                DraftKind::Skill { .. }
                | DraftKind::Command { .. }
                | DraftKind::Implement { .. }
                | DraftKind::ArchitectureReview { .. }
                | DraftKind::SpecReview { .. }
                | DraftKind::SecurityReview { .. }
                | DraftKind::Delivery { .. }
                | DraftKind::SelfMerge { .. }
                | DraftKind::RunPublishedIssue { .. }
                | DraftKind::LandTickets { .. } => {
                    unreachable!("the compiled Security Steps were checked")
                }
            }
        }
        Ok(())
    })();
    match fixed {
        Some(fix) => Outcome::Fixed {
            ended: fix.ended,
            finding: fix.finding,
            audit: (!skip_audit).then(|| Box::new(audit.ended(result))),
            close: fix.close,
        },
        None => Outcome::Audited(audit.ended(result)),
    }
}

fn steps() -> Result<Vec<DraftKind>> {
    let file = load_inherited_builtin("security-run")?;
    ensure!(
        file.worktrees.is_empty(),
        "security-run owns its session worktrees"
    );
    file.steps
        .into_iter()
        .map(|step| {
            ensure!(
                step.worktree.is_none() && step.process.is_none(),
                "security-run Step {} must inherit the Command's choices and run in sequence",
                step.name
            );
            match step.kind {
                DraftKind::SecurityAudit { .. }
                | DraftKind::ReproduceFindings { .. }
                | DraftKind::FixFindings { .. }
                | DraftKind::CloseFindings { .. } => Ok(step.kind),
                DraftKind::Skill { .. }
                | DraftKind::Command { .. }
                | DraftKind::Implement { .. }
                | DraftKind::ArchitectureReview { .. }
                | DraftKind::SpecReview { .. }
                | DraftKind::SecurityReview { .. }
                | DraftKind::Delivery { .. }
                | DraftKind::SelfMerge { .. }
                | DraftKind::RunPublishedIssue { .. }
                | DraftKind::LandTickets { .. } => {
                    bail!("unsupported security-run Step {}", step.name)
                }
            }
        })
        .collect()
}
