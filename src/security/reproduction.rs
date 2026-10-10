//! One proof-of-concept session in an owned disposable checkout. Its test
//! and final message are collected before the checkout is removed.

use std::fmt;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::git::Git;
use crate::github::SecurityRecord;
use crate::harness::Choice;
use crate::issue::Repo;
use crate::logs;
use crate::prompt;
use crate::session::{Logs, Purpose, Sessions};
use crate::worktree::ReviewWorktree;

// Declaration order is severity order, most severe first.
#[derive(Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Informational,
}

impl Severity {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        match name {
            "critical" => Some(Self::Critical),
            "high" => Some(Self::High),
            "medium" => Some(Self::Medium),
            "low" => Some(Self::Low),
            "informational" => Some(Self::Informational),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Informational => "informational",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FixSize {
    Single,
    Spec,
}

impl FixSize {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Spec => "spec",
        }
    }
}

pub enum Outcome {
    Reproduced { severity: Severity, size: FixSize },
    NotReproduced,
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reproduced { severity, size } => {
                write!(f, "reproduced {} {}", severity.name(), size.name())
            }
            Self::NotReproduced => write!(f, "not reproduced"),
        }
    }
}

pub struct Reproduction {
    pub outcome: Outcome,
    pub notes: String,
    pub test: String,
}

impl Reproduction {
    pub fn severity(&self) -> Option<Severity> {
        match self.outcome {
            Outcome::Reproduced { severity, .. } => Some(severity),
            Outcome::NotReproduced => None,
        }
    }
}

pub fn run(
    launch: &Git,
    repo: &Repo,
    record: &SecurityRecord,
    number: usize,
    harness: &Choice,
) -> (Result<Reproduction>, Option<PathBuf>) {
    let prepared = (|| -> Result<_> {
        let commit = record.audited_commit()?;
        let worktree = ReviewWorktree::at_commit(launch, &repo.name, commit)?;
        let root = logs::root(repo).join("reproductions");
        fs::create_dir_all(&root)?;
        let output = tempfile::Builder::new()
            .prefix("reproduction-")
            .tempdir_in(root)?;
        let test = output.path().join("test.txt");
        let prompt = prompt::security_reproduction(commit, record.description(), &test);
        Ok((worktree, output, test, prompt))
    })();
    let (worktree, _output, test, prompt) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => return (Err(error), None),
    };
    let logs = Logs::of_security_run(repo);
    Sessions::within(&logs, worktree.path(), harness, |sessions| {
        let message = sessions.run_to_final_message(
            Purpose::Security,
            &format!("security-reproduction-{number}"),
            &prompt,
        )?;
        let message = message.as_deref().unwrap_or_default().trim_end();
        let (notes, line) = message.rsplit_once('\n').unwrap_or(("", message));
        let outcome = match line.split_whitespace().collect::<Vec<_>>().as_slice() {
            ["Security", "reproduction:", "not", "reproduced"] => Outcome::NotReproduced,
            ["Security", "reproduction:", "reproduced", severity, size] => {
                let severity = Severity::parse(severity)
                    .context("the Security reproduction ended with an invalid severity")?;
                let size = match *size {
                    "single" => FixSize::Single,
                    "spec" => FixSize::Spec,
                    _ => bail!("the Security reproduction ended with an invalid fix size"),
                };
                Outcome::Reproduced { severity, size }
            }
            _ => {
                bail!("the Security reproduction ended without the final line its prompt asks for")
            }
        };
        let test = fs::read_to_string(test)
            .context("could not read the Security reproduction's test file")?;
        if notes.trim().is_empty() || test.trim().is_empty() {
            bail!("the Security reproduction ended without reproduction notes or test text");
        }
        Ok(Reproduction {
            outcome,
            notes: notes.trim().to_string(),
            test,
        })
    })
}
