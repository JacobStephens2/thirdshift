//! Read-only recovery advice for local Issue branch acquisition failures.

use std::fmt;
use std::path::Path;

use anyhow::Result;

use super::acquisition::{Registration, registrations};
use crate::git::Git;

#[derive(Debug)]
pub(super) struct LocalBranchConflict {
    pub branch: String,
    pub on_origin: bool,
}

impl fmt::Display for LocalBranchConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.on_origin {
            write!(
                f,
                "the local branch {} differs from origin/{}",
                self.branch, self.branch
            )
        } else {
            write!(f, "the local branch {} is not on origin", self.branch)
        }
    }
}

impl std::error::Error for LocalBranchConflict {}

pub(crate) fn local_branch(launch: &Git, error: anyhow::Error) -> anyhow::Error {
    let Some(conflict) = error.downcast_ref::<LocalBranchConflict>() else {
        return error;
    };
    let branch = &conflict.branch;
    let git = git_in(launch.dir());
    let mut advice = format!(
        "\nRecovery commands use git -C to name their working directory; they can be run from any directory.\n\
         Inspect and preserve local work first. Choose one alternative; thirdshift has not run these commands.\n\
         Push committed work (a non-fast-forward push may refuse; inspect and reconcile it first):\n  \
         {git} push -u origin {branch_arg}\n\
         Or rename the local branch to an unused name (change the saved name if needed):\n  \
         {git} branch -m {branch_arg} {saved}\n\
         Or delete only after preserving the work (Git may refuse an unmerged branch):\n  \
         {git} branch -d {branch_arg}",
        branch_arg = quote(branch),
        saved = quote(&format!("{branch}-saved")),
    );
    match registrations(launch) {
        Ok(entries) => {
            let reference = format!("refs/heads/{branch}");
            for entry in entries.iter().filter(|entry| entry.branch.as_deref() == Some(reference.as_str())) {
                advice.push_str(&occupied(&entries, entry));
            }
        }
        Err(cause) => advice.push_str(&format!(
            "\nCould not inspect occupied checkouts: {cause:#}. Inspect before renaming or deleting:\n  {git} worktree list --porcelain"
        )),
    }
    advice.push_str(CONTINUATION);
    anyhow::anyhow!("{error:#}{advice}")
}

pub(super) fn occupied_add(
    launch: &Git,
    branch: &str,
    path: &Path,
    error: anyhow::Error,
) -> anyhow::Error {
    // Observe only after acquisition has completed its existing recovery.
    // Advice never grants authority to mutate any of these resources.
    let advice = (|| -> Result<String> {
        let entries = registrations(launch)?;
        let reference = format!("refs/heads/{branch}");
        let actual_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let mut advice = String::new();
        for entry in entries.iter().filter(|entry| {
            entry.branch.as_deref() == Some(reference.as_str()) || entry.path == actual_path
        }) {
            advice.push_str(&occupied(&entries, entry));
        }
        Ok(advice)
    })();
    match advice {
        Ok(advice) if !advice.is_empty() => anyhow::anyhow!("{error:#}{advice}{CONTINUATION}"),
        _ => error,
    }
}

fn occupied(entries: &[Registration], entry: &Registration) -> String {
    let checkout = git_in(&entry.path);
    let identity = entry.branch.as_deref().unwrap_or("detached HEAD");
    let mut advice = format!(
        "\nOccupied checkout {} ({identity}). Inspect and preserve its work first:\n  \
         {checkout} status\n\
         Before renaming/deleting its branch, leave that branch in this checkout (after inspection):\n  \
         {checkout} switch --detach",
        quote(&entry.path.to_string_lossy()),
    );
    if entries.first().is_some_and(|main| main.path != entry.path) {
        advice.push_str(&format!(
            "\nOr, from outside that checkout, remove it without force after preserving its work:\n  \
             {git} worktree remove {path}\n\
             Removal may refuse while work remains, including untracked files, submodules or a lock; inspect/preserve it first.",
            git = git_in(&entries[0].path),
            path = quote(&entry.path.to_string_lossy()),
        ));
    } else {
        advice.push_str("\nThis is the main checkout; Git cannot remove it with worktree remove.");
    }
    advice.push_str("\nEach git -C command names the directory it operates in; thirdshift has not run these commands.");
    advice
}

const CONTINUATION: &str = "\nTo use normal Continuation, preserve the Issue branch's commits on origin and clear any occupied checkout first. Then rerun the same thirdshift command from the Launch directory with the Base branch checked out. Deleting preserved work is not required.";

fn git_in(path: &Path) -> String {
    format!("git -C {}", quote(&path.to_string_lossy()))
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
