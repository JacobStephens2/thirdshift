//! Record-derived ancestry and Session attachment for one Command-record
//! subtree. The Waterfall and whole Trace export both project this sequence,
//! so dispatch ancestry and Session ownership change here only.
use std::collections::BTreeSet;
use std::path::PathBuf;

use anyhow::Result;

use super::Context;
use crate::logs::command_record::{CommandRecord, StageTime};
use crate::logs::reading::Command;
use crate::logs::session::original;
use crate::otlp_jsonl::SpanId;

/// One node of a Trace's record-derived ancestry, borrowing its record's raw
/// facts so each adapter keeps its own timing, errors and output.
pub(in crate::logs) struct Node<'a> {
    /// The record presenting this node. The Command root presents the
    /// selected record under its owning Command's identity.
    pub command: &'a Command,
    /// The owning Command's context at the root, otherwise this record's Run.
    pub context: Context,
    pub parent: Option<SpanId>,
    pub fact: Fact<'a>,
}

pub(in crate::logs) enum Fact<'a> {
    Command,
    Run,
    Stage(StageTime<'a>),
    Session(Attachment<'a>),
}

/// A Session's one place in the selected subtree. Display ancestry only: its
/// retained requests keep the parents they recorded.
pub(in crate::logs) struct Attachment<'a> {
    /// The inventoried path, readable and stable for conversation links.
    pub path: PathBuf,
    /// The normalized identity shared by former and current filenames.
    pub original: PathBuf,
    /// The first stage naming this Session; `None` attaches it to the Run.
    pub stage: Option<StageTime<'a>>,
}

impl Node<'_> {
    /// A Session's own ID comes from its trace, not from the records.
    pub fn id(&self) -> Option<SpanId> {
        match &self.fact {
            Fact::Command => Some(self.context.command),
            Fact::Run => Some(self.context.run),
            Fact::Stage(time) => Some(self.context.stage_id(time.stage)),
            Fact::Session(_) => None,
        }
    }
}

/// The Command root first, then in preorder each Run, its stages, its
/// dispatched and child subtrees, and finally the Sessions its descendants
/// did not claim. `owner` is the actual outer Command and fixes the Trace
/// identity even when `selected` is one of its children.
pub(in crate::logs) fn of<'a>(selected: &'a Command, owner: &Command) -> Result<Vec<Node<'a>>> {
    let context = Context::of_session(&owner.command_log);
    let mut walk = Walk {
        claimed: BTreeSet::new(),
        nodes: vec![Node {
            command: selected,
            context: context.clone(),
            parent: None,
            fact: Fact::Command,
        }],
    };
    walk.record(selected, &context, context.command)?;
    Ok(walk.nodes)
}

struct Walk<'a> {
    claimed: BTreeSet<PathBuf>,
    nodes: Vec<Node<'a>>,
}

impl<'a> Walk<'a> {
    fn record(&mut self, command: &'a Command, outer: &Context, parent: SpanId) -> Result<()> {
        let context = outer.for_record(&command.record);
        let node = |parent, fact| Node {
            command,
            context: context.clone(),
            parent: Some(parent),
            fact,
        };
        self.nodes.push(node(parent, Fact::Run));
        for time in command.record.stage_times(None) {
            self.nodes.push(node(context.run, Fact::Stage(time)));
        }
        // Dispatchers can inventory their child's same-issue Sessions. Let
        // descendants claim them first so each Session retains its actual Run.
        for child in command.dispatched.iter().chain(&command.children) {
            let parent = dispatch_parent(&context, &command.record, &child.record);
            self.record(child, outer, parent)?;
        }
        for path in command.sessions()? {
            let identity = original(&path);
            if !self.claimed.insert(identity.clone()) {
                continue;
            }
            // A shared Session belongs to the first stage naming it, even when
            // an old trace points at the later stage that reused it.
            let stage = command.record.stage_times(None).find(|time| {
                time.stage
                    .session_log
                    .as_deref()
                    .is_some_and(|reference| original(reference) == identity)
            });
            let parent = stage
                .as_ref()
                .map_or(context.run, |time| context.stage_id(time.stage));
            self.nodes.push(node(
                parent,
                Fact::Session(Attachment {
                    path,
                    original: identity,
                    stage,
                }),
            ));
        }
        Ok(())
    }
}

/// A child Run belongs to the latest stage begun by its start: the stage that
/// dispatched it. Without one, it belongs to the dispatching Run.
fn dispatch_parent(context: &Context, record: &CommandRecord, child: &CommandRecord) -> SpanId {
    let started = chrono::DateTime::parse_from_rfc3339(&child.started_at).ok();
    record
        .stages
        .iter()
        .rev()
        .find(|stage| {
            chrono::DateTime::parse_from_rfc3339(&stage.began_at)
                .ok()
                .zip(started)
                .is_some_and(|(began, started)| began <= started)
        })
        .map_or(context.run, |stage| context.stage_id(stage))
}

#[cfg(test)]
mod tests;
