//! Shared Architect facts for Pass gates, conclusions and Process sequencing.

use chrono::{DateTime, Utc};

use crate::github;
use crate::issue::Repo;
use crate::pass::Call;
use crate::processes::architect::ArchitectRun;

pub(crate) const ORIGIN: &str = "git@github.com:acme/widgets.git";

pub(crate) fn started() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-04T09:00:00.750Z")
        .unwrap()
        .to_utc()
}

pub(crate) fn viewed(open: bool, after: i64, labels: &[&str]) -> github::Issue {
    github::Issue {
        title: None,
        is_open: open,
        labels: labels.iter().copied().collect(),
        created: started() + chrono::Duration::seconds(after),
    }
}

pub(crate) fn url(number: u64) -> String {
    format!("https://github.com/acme/widgets/issues/{number}")
}

pub(crate) fn message(line: &str) -> String {
    format!("Reviewed the codebase.\n\n{line}{}\n", url(8))
}

pub(crate) fn step(line: &str) -> Call {
    Call::Step(line.to_string())
}

pub(crate) fn request(repo: &Repo) -> ArchitectRun<'_> {
    ArchitectRun {
        repo,
        base: "main",
        origin: ORIGIN,
        focus: None,
        started: started(),
        pull: false,
        plan_only: false,
    }
}
