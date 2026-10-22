//! Optional patch evidence from Codex's own session file. This unpublished
//! format is deliberately fail-closed: no match leaves the stream untouched.
//! Wire reference: openai/codex, rust-v0.160.0, protocol/src/protocol.rs
//! (`PatchApplyEndEvent`, `FileChange`) and rollout/src/policy.rs.
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::{Value, json};

use crate::harness::interpretation::event::Event;
use crate::harness::interpretation::{Facts, Retained};

pub(super) struct Completion {
    root: Option<PathBuf>,
    worktree: PathBuf,
}

impl Completion {
    pub(super) fn new(worktree: &Path) -> Self {
        Self {
            root: std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex"))
                }),
            worktree: worktree.to_owned(),
        }
    }

    fn amendments(&self, facts: &Facts) -> Option<Vec<Event>> {
        let edits: Vec<_> = facts
            .report
            .events
            .iter()
            .filter(|event| {
                matches!(event,
                    Event::ToolCallEnded { tool, .. } if tool == "apply_patch"
                )
            })
            .collect();
        if edits.is_empty() {
            return None;
        }
        let id = facts.ended.session_id.as_deref()?;
        if id.is_empty() || id.contains(['/', '\\']) || id == ".." {
            return None;
        }
        let path = find(&self.root.as_ref()?.join("sessions"), id, 3)?;
        let patches = read(&fs::read_to_string(path).ok()?, id)?;
        if edits.len() != patches.len() {
            return None;
        }
        edits
            .into_iter()
            .zip(patches)
            .map(|(event, patch)| {
                let Event::ToolCallEnded { input, .. } = event else {
                    return None;
                };
                let changes = input.as_array()?;
                let native = patch["payload"]["changes"].as_object()?;
                if changes.is_empty() || changes.len() != native.len() {
                    return None;
                }
                let native: BTreeMap<_, _> = native
                    .iter()
                    .map(|(path, change)| (self.absolute(path), change))
                    .collect();
                let mut diffs = Vec::new();
                let mut matched = BTreeMap::new();
                for change in changes {
                    let path = change["path"].as_str()?;
                    let absolute = self.absolute(path);
                    let retained = native.get(&absolute)?;
                    if change["kind"].as_str()? != retained["type"].as_str()?
                        || matched.insert(absolute, ()).is_some()
                    {
                        return None;
                    }
                    diffs.push(
                        json!({"path":path,"kind":change["kind"],"diff":diff(path, retained)?}),
                    );
                }
                let mut amended = event.clone();
                if let Event::ToolCallEnded {
                    result, amended, ..
                } = &mut amended
                {
                    *result = json!({"source":"codex_session_file","record":patch,"changes":diffs});
                    *amended = true;
                }
                Some(amended)
            })
            .collect()
    }

    fn absolute(&self, path: &str) -> PathBuf {
        self.worktree.join(path)
    }
}

impl Retained for Completion {
    fn reconcile(self: Box<Self>, facts: &mut Facts) -> Result<()> {
        if let Some(amendments) = self.amendments(facts) {
            crate::harness::interpretation::event::reconcile(&mut facts.report.events, amendments);
        }
        Ok(())
    }
}

/// Codex uses year/month/day folders. Do not follow directory symlinks or
/// search unrelated homes; ambiguous matching files supply no evidence.
fn find(root: &Path, id: &str, depth: usize) -> Option<PathBuf> {
    let mut found = None;
    let suffix = format!("-{id}.jsonl");
    let mut pending = vec![(root.to_owned(), depth)];
    while let Some((folder, depth)) = pending.pop() {
        for entry in fs::read_dir(folder).ok()?.filter_map(Result::ok) {
            let kind = entry.file_type().ok()?;
            if kind.is_dir() && depth > 0 {
                pending.push((entry.path(), depth - 1));
            } else if kind.is_file()
                && entry.file_name().to_str()?.starts_with("rollout-")
                && entry.file_name().to_str()?.ends_with(&suffix)
            {
                if found.is_some() {
                    return None;
                }
                found = Some(entry.path());
            }
        }
    }
    found
}

/// A Resume shares its thread's file with earlier turns. Only the latest
/// turn can amend this process's streamed edits; never search ahead by path.
fn read(text: &str, id: &str) -> Option<Vec<Value>> {
    let mut records = text.lines().filter(|line| !line.trim().is_empty());
    let meta: Value = serde_json::from_str(records.next()?).ok()?;
    if meta["type"] != "session_meta" || meta["payload"]["id"] != id {
        return None;
    }
    let mut patches = Vec::new();
    for line in records {
        let record: Value = serde_json::from_str(line).ok()?;
        if record["type"] != "event_msg" {
            continue;
        }
        match record["payload"]["type"].as_str() {
            Some("task_started" | "turn_started") => patches.clear(),
            Some("patch_apply_end") => patches.push(record),
            _ => {}
        }
    }
    Some(patches)
}

fn diff(path: &str, change: &Value) -> Option<String> {
    match change["type"].as_str()? {
        "update" => change["unified_diff"].as_str().map(String::from),
        "add" | "delete" => {
            let content = change["content"].as_str()?;
            let count = content.lines().count();
            let start = usize::from(count > 0);
            let (header, prefix) = if change["type"] == "add" {
                (
                    format!("--- /dev/null\n+++ b/{path}\n@@ -0,0 +{start},{count} @@\n"),
                    '+',
                )
            } else {
                (
                    format!("--- a/{path}\n+++ /dev/null\n@@ -{start},{count} +0,0 @@\n"),
                    '-',
                )
            };
            let mut diff = header;
            for line in content.split_inclusive('\n') {
                diff.push(prefix);
                diff.push_str(line);
                if !line.ends_with('\n') {
                    diff.push_str("\n\\ No newline at end of file\n");
                }
            }
            Some(diff)
        }
        _ => None,
    }
}
