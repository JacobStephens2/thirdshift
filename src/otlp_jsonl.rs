//! OTLP JSON Lines for logs and spans, independent of application conventions.
//!
//! Requests also implement serde's traits for use as OTLP/HTTP bodies. A Writer's
//! request type fixes its signal for its lifetime. Use separate output streams
//! for logs and spans, opened in append mode when extending an existing file.
//! Calls require exclusive access and flush each complete line. An I/O failure
//! can leave a partial line; repair the tail before appending again. Flush moves
//! buffered bytes to the OS, not durable storage: file owners choose fsync policy.
//! serde_json's `float_roundtrip` and `arbitrary_precision` features are required
//! to read floating-point and integer values without changing their precision.

// This module is introduced before the Session recorder that will consume it.
#![allow(dead_code)]

use std::io::{self, BufRead, Write};
use std::marker::PhantomData;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

mod integer;
mod value;
pub use value::AnyValue;

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    value == &T::default()
}

// Proto messages share default-field omission and lowerCamelCase names.
macro_rules! message {
    ($name:ident { $($(#[$attr:meta])* $field:ident: $ty:ty),* $(,)? }) => {
        #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
        #[serde(default, rename_all = "camelCase")]
        pub struct $name {
            $(#[serde(skip_serializing_if = "is_default")]
              $(#[$attr])*
              pub $field: $ty,)*
        }
    };
}

message!(Logs { resource_logs: Vec<ResourceLogs> });
message!(Spans { resource_spans: Vec<ResourceSpans> });
message!(ResourceSpans {
    resource: Option<Resource>,
    scope_spans: Vec<ScopeSpans>,
    schema_url: String,
});
message!(ScopeSpans {
    scope: Option<Scope>,
    spans: Vec<Span>,
    schema_url: String,
});
message!(ResourceLogs {
    resource: Option<Resource>,
    scope_logs: Vec<ScopeLogs>,
    schema_url: String,
});
message!(ScopeLogs {
    scope: Option<Scope>,
    log_records: Vec<LogRecord>,
    schema_url: String,
});
message!(Resource {
    attributes: Vec<KeyValue>,
    #[serde(deserialize_with = "integer::deserialize")]
    dropped_attributes_count: u32,
});
message!(Scope {
    name: String,
    version: String,
    attributes: Vec<KeyValue>,
    #[serde(deserialize_with = "integer::deserialize")]
    dropped_attributes_count: u32,
});
message!(KeyValue { key: String, value: Option<AnyValue> });
message!(LogRecord {
    #[serde(with = "integer")]
    time_unix_nano: u64,
    #[serde(with = "integer")]
    observed_time_unix_nano: u64,
    // OTLP enums are numeric; unknown future numbers are preserved.
    severity_number: i32,
    severity_text: String,
    body: Option<AnyValue>,
    attributes: Vec<KeyValue>,
    #[serde(deserialize_with = "integer::deserialize")]
    dropped_attributes_count: u32,
    #[serde(deserialize_with = "integer::deserialize")]
    flags: u32,
    #[serde(deserialize_with = "optional_id")]
    trace_id: Option<TraceId>,
    #[serde(deserialize_with = "optional_id")]
    span_id: Option<SpanId>,
    event_name: String,
});

message!(Span {
    #[serde(deserialize_with = "optional_id")]
    trace_id: Option<TraceId>,
    #[serde(deserialize_with = "optional_id")]
    span_id: Option<SpanId>,
    trace_state: String,
    #[serde(deserialize_with = "optional_id")]
    parent_span_id: Option<SpanId>,
    #[serde(deserialize_with = "integer::deserialize")]
    flags: u32,
    name: String,
    // 0 unspecified, 1 internal, 2 server, 3 client, 4 producer, 5 consumer.
    kind: i32,
    #[serde(with = "integer")]
    start_time_unix_nano: u64,
    #[serde(with = "integer")]
    end_time_unix_nano: u64,
    attributes: Vec<KeyValue>,
    #[serde(deserialize_with = "integer::deserialize")]
    dropped_attributes_count: u32,
    events: Vec<SpanEvent>,
    #[serde(deserialize_with = "integer::deserialize")]
    dropped_events_count: u32,
    links: Vec<SpanLink>,
    #[serde(deserialize_with = "integer::deserialize")]
    dropped_links_count: u32,
    status: Option<Status>,
});
message!(SpanEvent {
    #[serde(with = "integer")]
    time_unix_nano: u64,
    name: String,
    attributes: Vec<KeyValue>,
    #[serde(deserialize_with = "integer::deserialize")]
    dropped_attributes_count: u32,
});
message!(SpanLink {
    #[serde(deserialize_with = "optional_id")]
    trace_id: Option<TraceId>,
    #[serde(deserialize_with = "optional_id")]
    span_id: Option<SpanId>,
    trace_state: String,
    attributes: Vec<KeyValue>,
    #[serde(deserialize_with = "integer::deserialize")]
    dropped_attributes_count: u32,
    #[serde(deserialize_with = "integer::deserialize")]
    flags: u32,
});
message!(Status {
    message: String,
    // 0 unset, 1 OK, 2 error. Preserve unknown future codes.
    code: i32,
});

/// Fixed-width IDs encode as lowercase hex, never protobuf's usual base64.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Id<const N: usize>(pub [u8; N]);
pub type TraceId = Id<16>;
pub type SpanId = Id<8>;

impl<const N: usize> Serialize for Id<N> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let hex: String = self.0.iter().map(|byte| format!("{byte:02x}")).collect();
        s.serialize_str(&hex)
    }
}

impl<'de, const N: usize> Deserialize<'de> for Id<N> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let hex = String::deserialize(d)?;
        if hex.len() != N * 2 {
            return Err(D::Error::custom(format!("expected {} hex digits", N * 2)));
        }
        let mut bytes = [0; N];
        for (byte, pair) in bytes.iter_mut().zip(hex.as_bytes().as_chunks::<2>().0) {
            let high = (pair[0] as char).to_digit(16);
            let low = (pair[1] as char).to_digit(16);
            match (high, low) {
                (Some(h), Some(l)) => *byte = (h * 16 + l) as u8,
                _ => return Err(D::Error::custom("invalid hex ID")),
            }
        }
        Ok(Id(bytes))
    }
}

fn optional_id<'de, D: serde::Deserializer<'de>, const N: usize>(
    d: D,
) -> Result<Option<Id<N>>, D::Error> {
    use serde::de::IntoDeserializer;
    match Option::<String>::deserialize(d)? {
        Some(s) if !s.is_empty() => Id::deserialize(s.into_deserializer()).map(Some),
        _ => Ok(None),
    }
}

mod sealed {
    pub trait Signal {
        const KEY: &'static str;
        fn is_empty(&self) -> bool;
    }
    impl Signal for super::Logs {
        const KEY: &'static str = "resourceLogs";
        fn is_empty(&self) -> bool {
            self.resource_logs
                .iter()
                .flat_map(|resource| &resource.scope_logs)
                .all(|scope| scope.log_records.is_empty())
        }
    }
    impl Signal for super::Spans {
        const KEY: &'static str = "resourceSpans";
        fn is_empty(&self) -> bool {
            self.resource_spans
                .iter()
                .flat_map(|resource| &resource.scope_spans)
                .all(|scope| scope.spans.is_empty())
        }
    }
}

/// A complete export request. Sealed to the supported OTLP signals.
pub trait Request: sealed::Signal + Serialize + DeserializeOwned {}
impl Request for Logs {}
impl Request for Spans {}

pub struct Writer<W, R> {
    output: W,
    request: PhantomData<R>,
}

impl<W: Write, R: Request> Writer<W, R> {
    pub fn new(output: W) -> Self {
        Self {
            output,
            request: PhantomData,
        }
    }

    pub fn write(&mut self, request: &R) -> io::Result<()> {
        if request.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "empty OTLP export request",
            ));
        }
        // Serialize before touching the stream: an encoding error cannot corrupt it.
        let mut line = serde_json::to_vec(request)?;
        line.push(b'\n');
        self.output.write_all(&line)?;
        self.output.flush()
    }
}

/// One request per line. Errors are returned at the offending line, including
/// incomplete tails; create a fresh reader at that offset when tailing live data.
pub struct Reader<B, R> {
    input: B,
    line: String,
    request: PhantomData<R>,
}

impl<B: BufRead, R: Request> Reader<B, R> {
    pub fn new(input: B) -> Self {
        Self {
            input,
            line: String::new(),
            request: PhantomData,
        }
    }
}

impl<B: BufRead, R: Request> Iterator for Reader<B, R> {
    type Item = io::Result<R>;

    fn next(&mut self) -> Option<Self::Item> {
        self.line.clear();
        match self.input.read_line(&mut self.line) {
            Ok(0) => None,
            Ok(_) => Some(
                (|| {
                    use serde::de::Error;
                    let mut value = serde_json::from_str(&self.line)?;
                    let fields = match &value {
                        serde_json::Value::Object(fields) => fields,
                        _ => {
                            return Err(serde_json::Error::custom(
                                "expected OTLP export request object",
                            ));
                        }
                    };
                    if [
                        "resourceLogs",
                        "resourceSpans",
                        "resourceMetrics",
                        "resourceProfiles",
                    ]
                    .iter()
                    .any(|key| *key != R::KEY && fields.contains_key(*key))
                    {
                        return Err(serde_json::Error::custom("wrong signal in OTLP file"));
                    }
                    // ProtoJSON null means "unset" for message fields. Leave array
                    // elements intact: null elements of repeated fields are invalid.
                    remove_null_fields(&mut value);
                    serde_json::from_value(value)
                })()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            ),
            Err(e) => Some(Err(e)),
        }
    }
}

fn remove_null_fields(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            fields.retain(|_, value| !value.is_null());
            for value in fields.values_mut() {
                remove_null_fields(value);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                remove_null_fields(value);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
