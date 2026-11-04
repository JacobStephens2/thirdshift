//! A Report investigated without publishing or dispatching work.

use std::io::{IsTerminal, Read};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::failed_run::FailedRun;
use crate::git::Git;
use crate::harness::Choice;
use crate::{command_limit, launch, logs, prompt, session, worktree};

/// Read before command startup, so Setup cannot consume a piped Report.
pub fn read_report(file: Option<&str>) -> Result<String> {
    let report = if let Some(path) = file {
        std::fs::read_to_string(path).with_context(|| format!("could not read Report {path}"))?
    } else {
        ensure!(
            !std::io::stdin().is_terminal(),
            "give a Report on standard input or with --file <path>"
        );
        let mut report = String::new();
        std::io::stdin().read_to_string(&mut report)?;
        report
    };
    ensure!(
        !report.trim().is_empty(),
        "give a Report on standard input or with --file <path>"
    );
    Ok(report)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Recommendation {
    category: String,
    state: String,
    issues: Vec<ProposedIssue>,
    investigation: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProposedIssue {
    title: String,
    body: String,
}

/// Keep the session's prose out of the machine-readable stdout contract.
fn recommendation(message: Option<&str>) -> Result<String> {
    let json = message
        .and_then(|message| {
            message
                .lines()
                .rev()
                .find_map(|line| line.strip_prefix("Triage recommendation: "))
        })
        .context("the Triage session ended without its final Triage recommendation: JSON line")?;
    let recommendation: Recommendation = serde_json::from_str(json)
        .context("the Triage recommendation is not valid recommendation JSON")?;
    ensure!(
        !recommendation.category.trim().is_empty()
            && [
                "needs-triage",
                "needs-info",
                "ready-for-agent",
                "ready-for-human",
                "wontfix"
            ]
            .contains(&recommendation.state.as_str())
            && !recommendation.investigation.trim().is_empty()
            && recommendation
                .issues
                .iter()
                .all(|issue| !issue.title.trim().is_empty() && !issue.body.trim().is_empty()),
        "the Triage recommendation needs a category, triage state, investigation and complete proposed issues"
    );
    Ok(serde_json::to_string(&recommendation)?)
}

/// A Triage run always takes a Command slot, even above the Command limit.
pub fn run(report: &str, harness: &mut Choice, read_machine: bool) -> Result<String, FailedRun> {
    let repo = launch::repo()?;
    command_limit::hold_slot(&repo);
    logs::started(logs::Work::TriageRun(&repo), harness);
    harness.check_without_probe()?;
    let base = launch::checked_out_base()?;
    let launch = Git::new(std::env::current_dir().context("could not read the Launch directory")?);
    let checkout = worktree::ReviewWorktree::for_triage(&launch, &repo.name, base.origin_commit())?;
    let session_logs = session::Logs::of_triage_run(&repo);
    let investigate = |sessions: &session::Sessions<'_>| {
        let skill = crate::skills::find(checkout.path(), "thirdshift-triage")?;
        let message = sessions.run_to_final_message(
            session::Purpose::Ordinary,
            session::Kind::Triage,
            &prompt::triage(&skill.path, report, read_machine),
        )?;
        recommendation(message.as_deref())
    };
    let (result, log) = if read_machine {
        let sandbox = crate::sandbox::triage::prepare(checkout.path(), harness.harness)?;
        session::Sessions::within_sandboxed(
            &session_logs,
            checkout.path(),
            harness,
            &sandbox,
            investigate,
        )
    } else {
        session::Sessions::within(&session_logs, checkout.path(), harness, investigate)
    };
    result.map_err(|error| {
        let mut failed = FailedRun::from(error);
        failed.log = log;
        failed
    })
}
