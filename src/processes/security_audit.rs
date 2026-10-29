//! Security audit catalogue Steps. Privacy and reproduction rules are shared
//! with Security runs; their Pass skip gates stay in front of that run.

use super::*;
use crate::github::{DraftAdvisory, SecurityRecord, SecurityRecords};
use crate::security::{audit, findings, reproduction};

pub(super) fn run(prepared: &Prepared, step: &ResolvedStep, index: usize) -> Result<String> {
    let repo = launch::repo()?;
    let github = GitHub::new();
    // A new write-up can quote an outcome no reproduction wrote.
    // reproduce-findings still scores a record this audit just created.
    let owes_reproduction = matches!(step.kind, StepKind::ReproduceFindings)
        && prepared
            .security_findings
            .lock()
            .expect("Security findings")
            .as_ref()
            .is_some_and(findings::Findings::has_pending);
    if !owes_reproduction
        && prepared.security_fix
        && github.security_records(&repo.slug())?.next_fix()?.is_some()
    {
        let message = format!(
            "{} skipped: a recorded Security finding awaits an allowed fix",
            step.name
        );
        progress::step(&message);
        return Ok(message);
    }
    let mut logs = session::Logs::of_process_review(&repo, index + 1);
    if let Some(identity) = &prepared.dispatched {
        logs = logs.for_child(
            prepared
                .issue
                .as_ref()
                .expect("a dispatched Ticket has an issue"),
            identity.clone(),
        );
    }
    let before_each = |worktree: &Path| {
        link_harnesses(worktree, std::slice::from_ref(step))?;
        if let Some(issue) = &prepared.issue {
            prepare_for_session(issue, prepared.started, worktree)?;
        }
        Ok(())
    };
    let mut outside = Sessions {
        launch: Git::new(std::env::current_dir()?),
        repo: &repo,
        choice: &step.choice,
        session: findings::Session {
            logs: &logs,
            before_each: &before_each,
        },
    };
    let mut state = prepared
        .security_findings
        .lock()
        .expect("Security findings");
    if matches!(step.kind, StepKind::SecurityAudit) {
        audit::check_node()?;
        let attempt = audit::run_with_session(
            &outside.launch,
            &repo,
            &prepared.base,
            &step.choice,
            findings::Session {
                logs: &logs,
                before_each: &before_each,
            },
        );
        let mut findings = findings::Findings::default();
        findings.log = attempt.log;
        findings.record(&mut outside, attempt.outcome?, None)?;
        let message = findings.recorded.to_string();
        *state = Some(findings);
        Ok(message)
    } else {
        let findings = state
            .as_mut()
            .context("reproduce-findings needs a completed security-audit Step")?;
        findings.reproduce(&mut outside, None)?;
        Ok(findings.recorded.to_string())
    }
}

struct Sessions<'a> {
    launch: Git,
    repo: &'a crate::issue::Repo,
    choice: &'a Choice,
    session: findings::Session<'a>,
}

impl findings::Outside for Sessions<'_> {
    fn security_records(&mut self) -> Result<SecurityRecords> {
        GitHub::new().security_records(&self.repo.slug())
    }

    fn create_security_record(
        &mut self,
        records: &SecurityRecords,
        finding: &DraftAdvisory,
    ) -> Result<SecurityRecord> {
        GitHub::new().create_security_record(&self.repo.slug(), records, finding)
    }

    fn reproduce(
        &mut self,
        record: &SecurityRecord,
        number: usize,
    ) -> (Result<reproduction::Reproduction>, Option<PathBuf>) {
        reproduction::run_with_session(
            &self.launch,
            self.repo,
            record,
            number,
            self.choice,
            findings::Session {
                logs: self.session.logs,
                before_each: self.session.before_each,
            },
        )
    }

    fn update_security_record(
        &mut self,
        record: &SecurityRecord,
        reproduction: &reproduction::Reproduction,
    ) -> Result<()> {
        GitHub::new().update_security_record(&self.repo.slug(), record, reproduction)
    }

    fn step(&mut self, line: String) {
        progress::step(line);
    }
}

pub(super) fn validate(path: &Path, file: &ProcessFile) -> Result<()> {
    let mut audited = false;
    for step in &file.steps {
        let (line, name) = match step.kind {
            DraftKind::SecurityAudit { line } => (line, "security-audit"),
            DraftKind::ReproduceFindings { line } => (line, "reproduce-findings"),
            _ => continue,
        };
        let reason = if step.parallel.is_some() {
            Some(format!("{name} cannot be in a parallel group"))
        } else if step.optional {
            Some(format!("{name} must be required"))
        } else if step.worktree.as_ref().is_some_and(|(name, _)| {
            file.worktrees
                .iter()
                .any(|worktree| worktree.name == *name && worktree.kind != WorktreeKind::Throwaway)
        }) {
            Some(format!("{name} needs a throwaway worktree"))
        } else if name == "reproduce-findings" && !audited {
            Some("reproduce-findings must follow security-audit".to_string())
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(rule(path, line, name, reason));
        }
        audited = true;
    }
    Ok(())
}
