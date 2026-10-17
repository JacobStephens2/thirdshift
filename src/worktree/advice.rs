//! Read-only recovery advice. The user chooses and executes any alternative;
//! these observations grant no acquisition or cleanup authority.

use std::fmt;
use std::path::Path;

use crate::args::shell_command;
use crate::git::Git;

use super::acquisition::{Registration, registrations};

#[derive(Debug)]
struct BranchConflict {
    branch: String,
    on_origin: bool,
    advice: String,
}

impl fmt::Display for BranchConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the local branch {} ", self.branch)?;
        if self.on_origin {
            write!(f, "differs from origin/{}", self.branch)?;
        } else {
            write!(f, "is not on origin")?;
        }
        write!(f, "{}", self.advice)
    }
}

impl std::error::Error for BranchConflict {}

pub(super) fn branch_conflict(branch: &str, on_origin: bool) -> anyhow::Error {
    BranchConflict {
        branch: branch.to_string(),
        on_origin,
        advice: String::new(),
    }
    .into()
}

pub(crate) fn local_branch_advice(launch: &Git, mut error: anyhow::Error) -> anyhow::Error {
    let Some(conflict) = error.downcast_mut::<BranchConflict>() else {
        return error;
    };
    let branch = &conflict.branch;
    let push = command(
        launch,
        &[
            "push",
            "origin",
            &format!("refs/heads/{branch}:refs/heads/{branch}"),
        ],
    );
    let rename = command(
        launch,
        &["branch", "-m", "--", branch, &format!("{branch}-preserved")],
    );
    let delete = command(launch, &["branch", "-d", "--", branch]);
    conflict.advice = format!(
        "\nAlternatives (choose one; thirdshift has not run these commands). Commands use git -C to name their working directory and can be run from any directory.\n\
         Inspect and commit any checkout changes before pushing preserved work:\n  {push}\n\
         A push may refuse if origin has different work; inspect and reconcile it first, without forcing. After a successful push, rerun the original thirdshift command from the Launch directory on the Base branch to continue the published Issue branch.\n\
         Or preserve the local branch under an unused name (choose another name if this one exists):\n  {rename}\n\
         Or delete the local branch only when its work is preserved and Git permits it:\n  {delete}\n\
         Deletion is optional. Git may refuse while unmerged work remains; inspect and preserve it first."
    );
    match registrations(launch) {
        Ok(entries) => {
            let reference = format!("refs/heads/{branch}");
            for entry in entries.iter().filter(|entry| entry.branch.as_deref() == Some(&reference)) {
                conflict.advice.push_str(&checkout_advice(launch, entry, None));
            }
        }
        Err(inspection) => conflict.advice.push_str(&format!(
            "\nCould not identify occupied checkouts: {inspection:#}. Inspect before renaming or deleting:\n  {}",
            command(launch, &["worktree", "list", "--porcelain"])
        )),
    }
    error
}

pub(super) fn occupied_advice(
    launch: &Git,
    branch: Option<&str>,
    path: &Path,
    error: anyhow::Error,
) -> anyhow::Error {
    let Some(branch) = branch else {
        return error;
    };
    let Ok(entries) = registrations(launch) else {
        return error;
    };
    let reference = format!("refs/heads/{branch}");
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut advice = String::new();
    for entry in entries
        .iter()
        .filter(|entry| entry.path == path || entry.branch.as_deref() == Some(&reference))
    {
        advice.push_str(&checkout_advice(launch, entry, Some(&path)));
    }
    if advice.is_empty() {
        error
    } else {
        OccupiedAdvice {
            cause: error,
            advice,
        }
        .into()
    }
}

#[derive(Debug)]
struct OccupiedAdvice {
    cause: anyhow::Error,
    advice: String,
}

impl fmt::Display for OccupiedAdvice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#}{}", self.cause, self.advice)
    }
}

impl std::error::Error for OccupiedAdvice {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.cause.as_ref())
    }
}

fn checkout_advice(launch: &Git, entry: &Registration, target: Option<&Path>) -> String {
    let mut advice = format!(
        "\nOccupied checkout {} on {}. Before renaming or deleting its branch, leave or inspect this checkout.\n\
         Inspect and preserve its work first:\n  {}",
        entry.path.display(),
        entry.branch.as_deref().unwrap_or("detached HEAD"),
        command(
            launch,
            &["-C", &entry.path.to_string_lossy(), "status", "--short"]
        )
    );
    if launch.dir().canonicalize().ok().as_ref() == Some(&entry.path) {
        advice.push_str("\nThis is the Launch directory: switch it to the Base branch, then rerun the original thirdshift command to use the normal Continuation path once preserved work is published. Do not remove the Launch directory.");
    } else {
        advice.push_str(&format!(
            "\nTo free the checkout for a retry, remove it only after preserving its work, then rerun the original thirdshift command from the Launch directory on the Base branch. If the selected Issue branch is on origin, the retry uses the normal Continuation path. Commands use git -C and can run from any directory:\n  {}\n\
             Removal is optional and may refuse while work remains or the Worktree is locked; inspect and preserve it first. No force flags are used.",
            command(launch, &["worktree", "remove", "--", &entry.path.to_string_lossy()])
        ));
        if target.is_some_and(|target| target != entry.path) {
            advice.push_str("\nAlternatively, leave this checkout on another branch and retry, keeping the preserved checkout.");
        }
    }
    advice
}

fn command(launch: &Git, args: &[&str]) -> String {
    shell_command(
        ["git", "-C", &launch.dir().to_string_lossy()]
            .into_iter()
            .chain(args.iter().copied())
            .map(String::from)
            .collect(),
    )
}
