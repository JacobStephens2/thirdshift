//! A Session trace owns streaming logs and ended spans. No Harness protocol parsing lives here
//! beyond envelope metadata (event name and clock).
use std::collections::BTreeMap;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::harness::Choice;
use crate::harness::interpretation::event::{Event, Outcome};
use crate::harness::version::Version;
use crate::logs::trace::{Context, SpanKind, id};
use crate::otlp_jsonl::*;

mod attributes;
use attributes::attribute;
pub(crate) use attributes::event_attributes;
mod calls;
use calls::Call;
pub(crate) use calls::{Calls, Retained};
mod export;
use export::Export;

// Pin the development conventions rather than silently following latest.
const SCHEMA: &str = "https://opentelemetry.io/schemas/1.44.0";

pub(crate) struct Trace {
    logs: Writer<File, Logs>,
    spans: Writer<File, Spans>,
    export: Option<Export>,
    resource: Resource,
    version: Option<Version>,
    metadata_logged: bool,
    context: Context,
    session: Span,
    calls: Calls,
    spans_path: PathBuf,
    seen: Vec<Event>,
    model_usage: BTreeMap<String, Value>,
    model_scopes: BTreeMap<Option<String>, SpanId>,
    input_excludes_cache: bool,
}

impl Trace {
    pub fn create(
        log: &Path,
        kind: &str,
        choice: &Choice,
        prompt: &str,
        context: Context,
        command: &std::process::Command,
        command_log: Option<&Path>,
    ) -> io::Result<Self> {
        let log = crate::logs::session::original(log);
        let started = now();
        let session_id = id(&[
            &context.trace.0,
            b"session",
            log.as_os_str().as_encoded_bytes(),
        ]);
        let input_excludes_cache = choice.harness.input_excludes_cache();
        let mut attrs = vec![
            attribute("service.name", "thirdshift"),
            attribute("service.version", env!("CARGO_PKG_VERSION")),
            attribute("thirdshift.harness", choice.harness.name()),
            attribute("thirdshift.session.kind", kind),
            attribute("thirdshift.repository", &context.repository),
            attribute("thirdshift.command.identity", &context.identity),
            attribute("thirdshift.gen_ai.conventions.version", "1.44.0"),
        ];
        if let Some(model) = &choice.model {
            attrs.push(attribute("gen_ai.request.model", model));
        }
        if let Some(effort) = &choice.effort {
            attrs.push(attribute("thirdshift.request.effort", effort));
        }
        if let Some(dir) = command.get_current_dir() {
            attrs.push(attribute("thirdshift.harness.directory", dir.display()));
        }
        Ok(Self {
            logs: Writer::new(File::create(log.with_extension("logs.jsonl"))?),
            spans: Writer::new(File::create(log.with_extension("spans.jsonl"))?),
            export: crate::logs::otlp_endpoint().and_then(Export::start),
            resource: Resource {
                attributes: {
                    let mut resource = attrs.clone();
                    resource.push(attribute("thirdshift.harness.version", "unknown"));
                    resource
                },
                ..Default::default()
            },
            version: Some(Version::new(choice.harness.adapter(), command_log, command)),
            metadata_logged: false,
            context,
            session: Span {
                span_id: Some(session_id),
                name: format!("invoke_agent {kind}"),
                kind: 1,
                start_time_unix_nano: started,
                attributes: {
                    attrs.push(attribute("gen_ai.operation.name", "invoke_agent"));
                    attrs.push(attribute("thirdshift.session.prompt", prompt));
                    attrs.push(attribute(
                        "thirdshift.harness.arguments",
                        serde_json::to_string(&recorded_arguments(command)).unwrap(),
                    ));
                    attrs
                },
                ..Default::default()
            },
            calls: Calls::new(session_id, input_excludes_cache, []),
            spans_path: log.with_extension("spans.jsonl"),
            seen: Vec::new(),
            model_usage: BTreeMap::new(),
            model_scopes: BTreeMap::new(),
            input_excludes_cache,
        })
    }

    pub fn record(&mut self, raw: impl AsRef<[u8]>, events: &[Event]) -> io::Result<()> {
        self.record_at(raw.as_ref(), events, now())
    }

    pub(crate) fn record_at(
        &mut self,
        bytes: &[u8],
        events: &[Event],
        observed: u64,
    ) -> io::Result<()> {
        let raw = String::from_utf8_lossy(bytes);
        let envelope: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
        self.record_version(&envelope);
        let time = event_time(&envelope);
        let mut attrs = Vec::new();
        // The Session span is written on exit. Retain its prompt and start in
        // the first event too, so a live Peek can show them without repeating
        // the prompt in every resource envelope.
        if !self.metadata_logged {
            attrs.extend(
                self.session
                    .attributes
                    .iter()
                    .filter(|attr| attr.key == "thirdshift.session.prompt")
                    .cloned(),
            );
            attrs.push(attribute(
                "thirdshift.session.start_time_unix_nano",
                self.session.start_time_unix_nano,
            ));
        }
        self.metadata_logged = true;
        // Partial Messages frames can carry only usage/deltas, with no new
        // normalized fact. Their envelope retains the parent tool scope.
        let mut span_id = if envelope["type"] == "stream_event" {
            let parent = envelope["parent_tool_use_id"].as_str().map(str::to_owned);
            self.model_scopes
                .get(&parent)
                .copied()
                .or(self.session.span_id)
        } else {
            self.session.span_id
        };
        for event in events {
            span_id = self.apply(event, time.unwrap_or(observed))?.or(span_id);
            for attr in event_attributes(event, self.input_excludes_cache) {
                set(&mut attrs, attr);
            }
            self.seen.push(event.clone());
        }
        // Multiple normalized facts can describe one raw Harness event. Keep
        // them together in exactly one log record, without dropping the raw body.
        if !events.is_empty() {
            attrs.push(attribute(
                "thirdshift.events",
                serde_json::to_string(events).unwrap(),
            ));
        }
        let name = ["type", "event", "payload_type"]
            .iter()
            .find_map(|key| envelope[*key].as_str())
            .or_else(|| envelope["payload"]["event"]["kind"].as_str())
            .unwrap_or("output");
        self.logs.write(&Logs {
            resource_logs: vec![ResourceLogs {
                resource: Some(self.resource.clone()),
                schema_url: SCHEMA.into(),
                scope_logs: vec![ScopeLogs {
                    scope: Some(scope()),
                    schema_url: SCHEMA.into(),
                    log_records: vec![LogRecord {
                        time_unix_nano: time.unwrap_or_default(),
                        observed_time_unix_nano: observed,
                        body: Some(match std::str::from_utf8(bytes) {
                            Ok(raw) => AnyValue::String(raw.trim_end_matches(['\r', '\n']).into()),
                            Err(_) => AnyValue::Bytes(bytes.to_vec()),
                        }),
                        event_name: name.into(),
                        trace_id: Some(self.context.trace),
                        span_id,
                        attributes: attrs,
                        ..Default::default()
                    }],
                }],
            }],
        })
    }

    /// Completion may add retained calls, usage or authoritative amendments.
    /// Preserve those facts as logs, never invent a second ended call span.
    pub fn reconcile(&mut self, events: &[Event]) -> io::Result<()> {
        for event in events {
            if !self.seen.contains(event) {
                let raw = serde_json::to_string(event)?;
                self.record(&raw, std::slice::from_ref(event))?;
            }
        }
        Ok(())
    }

    pub fn finish(&mut self, error: Option<&str>) -> io::Result<()> {
        self.finish_at(error, now())
    }

    pub(crate) fn finish_at(&mut self, error: Option<&str>, ended: u64) -> io::Result<()> {
        self.record_version(&Value::Null);
        let ended = ended.max(self.session.end_time_unix_nano);
        for call in self.calls.close(ended) {
            let span = self.span(&call, error);
            self.write_span(span)?;
        }
        let mut session = self.session.clone();
        session.trace_id = Some(self.context.trace);
        session.parent_span_id = Some(self.context.stage);
        session.end_time_unix_nano = ended.max(session.start_time_unix_nano);
        session.status = Some(Status {
            code: if error.is_some() { 2 } else { 1 },
            message: error.unwrap_or("").into(),
        });
        self.write_span(session.clone())?;
        // Calls are flushed when they end. Retained facts and late permission
        // metadata refine the same span. Atomically retain one final version
        // per ID, preserving the already-flushed file if rewriting fails.
        let mut file = tempfile::NamedTempFile::new_in(self.spans_path.parent().unwrap())?;
        let mut spans: Vec<_> = self
            .calls
            .calls()
            .map(|call| self.span(call, error))
            .chain([session])
            .collect();
        spans.sort_by_key(|span| span.end_time_unix_nano);
        let mut writer = Writer::new(file.as_file_mut());
        for span in spans {
            writer.write(&self.request(span))?;
        }
        file.persist(&self.spans_path)
            .map_err(|error| error.error)?;
        Ok(())
    }

    fn apply(&mut self, event: &Event, time: u64) -> io::Result<Option<SpanId>> {
        use Event::*;
        let Some(applied) = self.calls.apply(event, Some(time)) else {
            if let Usage {
                scope: crate::harness::interpretation::event::UsageScope::Model,
                model,
                ..
            } = event
            {
                self.model_usage.insert(
                    model.as_deref().unwrap_or("").into(),
                    serde_json::to_value(event).unwrap(),
                );
                set(
                    &mut self.session.attributes,
                    attribute(
                        "thirdshift.model_usage",
                        serde_json::to_string(&self.model_usage).unwrap(),
                    ),
                );
            } else {
                for attr in event_attributes(event, self.input_excludes_cache) {
                    set(&mut self.session.attributes, attr);
                }
            }
            #[expect(
                clippy::wildcard_enum_match_arm,
                reason = "only messages and errors name a call or parent to place them under"
            )]
            let (call, parent) = match event {
                Message { id, parent_id, .. } => (id.as_deref(), parent_id.as_deref()),
                Error { parent_id, .. } => (None, parent_id.as_deref()),
                _ => (None, None),
            };
            return Ok(call
                .and_then(|id| self.calls.locate(id))
                .or_else(|| {
                    if matches!(event, Message { .. }) {
                        self.model_scopes.get(&parent.map(str::to_owned)).copied()
                    } else {
                        None
                    }
                })
                .or_else(|| parent.and_then(|id| self.calls.locate(id))));
        };
        let call = applied.call;
        if call.kind == SpanKind::Model {
            self.model_scopes.insert(call.parent_id.clone(), call.span);
        }
        // Later completions refine the call in the final rewrite only.
        if applied.first_completion {
            let span = self.span(&call, None);
            self.write_span(span)?;
        }
        Ok(Some(call.span))
    }

    /// Project one call to its OTLP span. A call closed at Session finish has
    /// no outcome and takes the Session's error, if any.
    fn span(&self, call: &Call, closure: Option<&str>) -> Span {
        let mut attributes = call.attributes.clone();
        for attr in &self.session.attributes {
            if attr.key == "gen_ai.request.model" {
                set(&mut attributes, attr.clone());
            }
        }
        let start = call.started.unwrap_or_default();
        Span {
            trace_id: Some(self.context.trace),
            span_id: Some(call.span),
            parent_span_id: Some(call.parent_span),
            name: match &call.tool {
                Some(tool) => format!("execute_tool {tool}"),
                None => format!("{} {}", call.kind, call.id),
            },
            kind: if call.kind == SpanKind::Model { 3 } else { 1 },
            start_time_unix_nano: start,
            end_time_unix_nano: call.ended.map_or(0, |ended| ended.max(start)),
            status: call.completed.then(|| {
                call.outcome.map(status).unwrap_or_else(|| Status {
                    code: 2,
                    message: closure.unwrap_or("call did not end").into(),
                })
            }),
            attributes,
            ..Default::default()
        }
    }

    fn record_version(&mut self, envelope: &Value) {
        let Some(version) = &mut self.version else {
            return;
        };
        set(
            &mut self.resource.attributes,
            attribute("thirdshift.harness.version", version.observe(envelope)),
        );
    }

    fn write_span(&mut self, span: Span) -> io::Result<()> {
        let request = self.request(span);
        self.spans.write(&request)?;
        if let Some(export) = &self.export {
            export.send(request);
        }
        Ok(())
    }

    fn request(&self, span: Span) -> Spans {
        Spans {
            resource_spans: vec![ResourceSpans {
                resource: Some(self.resource.clone()),
                schema_url: SCHEMA.into(),
                scope_spans: vec![ScopeSpans {
                    scope: Some(scope()),
                    schema_url: SCHEMA.into(),
                    spans: vec![span],
                }],
            }],
        }
    }
}

/// Launcher environment values can be credentials. Keep the command's shape
/// in evidence without copying those values into local spans or OTLP exports.
fn recorded_arguments(command: &std::process::Command) -> Vec<String> {
    let mut arguments: Vec<_> = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    if Path::new(command.get_program())
        .file_name()
        .is_some_and(|name| name == "bwrap")
    {
        let mut index = 0;
        while index < arguments.len() {
            match arguments[index].as_str() {
                "--" => break,
                "--setenv" => {
                    if let Some(value) = arguments.get_mut(index + 2) {
                        *value = "[redacted]".into();
                    }
                    index += 3;
                }
                _ => index += 1,
            }
        }
    }
    arguments
}

fn status(outcome: Outcome) -> Status {
    Status {
        code: if outcome == Outcome::Succeeded { 1 } else { 2 },
        message: format!("{outcome:?}"),
    }
}

pub(super) fn set(attrs: &mut Vec<KeyValue>, attr: KeyValue) {
    if let Some(old) = attrs.iter_mut().find(|old| old.key == attr.key) {
        *old = attr;
    } else {
        attrs.push(attr);
    }
}

fn scope() -> Scope {
    Scope {
        name: "thirdshift.session".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        ..Default::default()
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .try_into()
        .unwrap_or(u64::MAX)
}

pub(crate) fn event_time(event: &Value) -> Option<u64> {
    for key in ["timestamp", "recorded_at", "time"] {
        let value = &event[key];
        if let Some(text) = value.as_str()
            && let Ok(time) = chrono::DateTime::parse_from_rfc3339(text)
        {
            return time
                .timestamp_nanos_opt()
                .and_then(|time| time.try_into().ok());
        }
        // Muse recorded_at and OpenCode timestamp are Unix milliseconds.
        if let Some(ms) = value.as_u64() {
            return ms.checked_mul(1_000_000);
        }
    }
    None
}

#[cfg(test)]
mod tests;
