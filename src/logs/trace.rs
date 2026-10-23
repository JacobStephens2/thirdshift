//! Deterministic Trace ancestry. No second copy of Command or stage times.
use serde::Serialize;
use sha2::{Digest, Sha256};

/// The eight kinds in a Command's trace, including record-derived ancestors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SpanKind {
    Cmd,
    Run,
    Stage,
    Sess,
    Model,
    Turn,
    Tool,
    Agent,
}

impl std::fmt::Display for SpanKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Cmd => "cmd",
            Self::Run => "run",
            Self::Stage => "stage",
            Self::Sess => "sess",
            Self::Model => "model",
            Self::Turn => "turn",
            Self::Tool => "tool",
            Self::Agent => "agent",
        })
    }
}

use super::{
    command_record::{CommandRecord, Stage},
    owner::Owner,
};
use crate::otlp_jsonl::{Id, SpanId, TraceId};

/// Length-prefix every part so identities cannot alias by concatenation.
pub(crate) fn id<const N: usize>(parts: &[&[u8]]) -> Id<N> {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    let digest = hash.finalize();
    Id(digest[..N].try_into().expect("ID fits SHA-256"))
}

#[derive(Clone)]
pub(crate) struct Context {
    pub trace: TraceId,
    pub command: SpanId,
    pub run: SpanId,
    pub stage: SpanId,
    pub identity: String,
    pub repository: String,
}

impl Context {
    pub(super) fn from_record(owner: &Owner, record: &CommandRecord) -> Self {
        Self::of_owner(owner).for_record(record)
    }

    pub(super) fn for_record(&self, record: &CommandRecord) -> Self {
        let mut context = self.clone();
        context.repository = record.repository.clone();
        context.run = id(&[
            &context.command.0,
            b"run",
            record.kind.as_bytes(),
            record.name.as_bytes(),
            record.started_at.as_bytes(),
            record.parent_dispatch.as_deref().unwrap_or("").as_bytes(),
            record
                .parent_record
                .as_deref()
                .unwrap_or(std::path::Path::new(""))
                .as_os_str()
                .as_encoded_bytes(),
        ]);
        context.stage = record
            .stages
            .last()
            .map_or(context.run, |stage| context.stage_id(stage));
        context
    }

    /// Historical readers use the same identity for any retained stage.
    pub(super) fn stage_id(&self, stage: &Stage) -> SpanId {
        id(&[
            &self.run.0,
            b"stage",
            stage.name.as_bytes(),
            stage.began_at.as_bytes(),
        ])
    }

    /// Both retained views place a child Run in the stage that dispatched it.
    pub(super) fn child_parent(&self, record: &CommandRecord, child: &CommandRecord) -> SpanId {
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
            .map_or(self.run, |stage| self.stage_id(stage))
    }

    pub(crate) fn of_owner(owner: &Owner) -> Self {
        let identity = owner.identity().to_string();
        Self::with_identity(identity)
    }

    /// Unowned callers still retain a Session trace; preserve that behavior.
    /// A stamped path recovers grouping without borrowing another Command.
    pub(crate) fn of_session(path: &std::path::Path) -> Self {
        let identity = super::owner::decode(path)
            .map(|decoded| decoded.identity.to_string())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        Self::with_identity(identity)
    }

    fn with_identity(identity: String) -> Self {
        let trace = id(&[b"thirdshift.trace.v1", identity.as_bytes()]);
        let command = id(&[&trace.0, b"command"]);
        Self {
            trace,
            command,
            run: command,
            stage: command,
            identity,
            repository: String::new(),
        }
    }
}
