use super::*;
use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use serde_json::json;

#[test]
fn finite_double_reads_back_without_changing_bits() {
    let number = f64::from_bits(0x20fd5db43a776693);
    let request = Logs {
        resource_logs: vec![ResourceLogs {
            scope_logs: vec![ScopeLogs {
                log_records: vec![LogRecord {
                    body: Some(AnyValue::Double(number)),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    let mut bytes = Vec::new();
    Writer::new(&mut bytes).write(&request).unwrap();
    let actual = Reader::<_, Logs>::new(bytes.as_slice())
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(actual, request);
}

#[test]
fn reader_accepts_protojson_integer_spellings_without_rounding() {
    fn read_record(fields: &str) -> std::io::Result<LogRecord> {
        let line =
            format!(r#"{{"resourceLogs":[{{"scopeLogs":[{{"logRecords":[{{{fields}}}]}}]}}]}}"#);
        Reader::<_, Logs>::new(line.as_bytes())
            .next()
            .unwrap()
            .map(|request| request.resource_logs[0].scope_logs[0].log_records[0].clone())
    }

    for (encoded, expected) in [
        (r#""4.2e1""#, 42),
        ("4.2e1", 42),
        ("42.0", 42),
        (r#""42.0""#, 42),
        ("18446744073709551615", u64::MAX),
        (r#""18446744073709551615""#, u64::MAX),
        ("18446744073709551615e0", u64::MAX),
        (r#""18446744073709551615e0""#, u64::MAX),
    ] {
        let record = read_record(&format!(r#""timeUnixNano":{encoded}"#))
            .unwrap_or_else(|error| panic!("timeUnixNano {encoded}: {error}"));
        assert_eq!(record.time_unix_nano, expected, "{encoded}");
    }
    for (encoded, expected) in [
        (r#""-4.2e1""#, -42),
        ("-4.2e1", -42),
        ("-42.0", -42),
        ("9223372036854775807", i64::MAX),
        (r#""9223372036854775807""#, i64::MAX),
        ("9223372036854775807e0", i64::MAX),
        (r#""9223372036854775807e0""#, i64::MAX),
        ("-9223372036854775808", i64::MIN),
        (r#""-9223372036854775808""#, i64::MIN),
        ("-9223372036854775808e0", i64::MIN),
        (r#""-9223372036854775808e0""#, i64::MIN),
    ] {
        let record = read_record(&format!(r#""body":{{"intValue":{encoded}}}"#))
            .unwrap_or_else(|error| panic!("intValue {encoded}: {error}"));
        assert_eq!(record.body, Some(AnyValue::Int(expected)), "{encoded}");
    }
    for encoded in ["1", r#""1""#, "1e0", r#""1e0""#, "1.0", r#""1.0""#] {
        let record = read_record(&format!(r#""flags":{encoded}"#))
            .unwrap_or_else(|error| panic!("flags {encoded}: {error}"));
        assert_eq!(record.flags, 1, "{encoded}");
    }
    for encoded in [
        "-1",
        "18446744073709551616",
        r#""18446744073709551616e0""#,
        "42.5",
    ] {
        assert!(
            read_record(&format!(r#""timeUnixNano":{encoded}"#)).is_err(),
            "{encoded}"
        );
    }
    for encoded in [
        "9223372036854775808",
        "-9223372036854775809",
        r#""9223372036854775808e0""#,
        "42.5",
    ] {
        assert!(
            read_record(&format!(r#""body":{{"intValue":{encoded}}}"#)).is_err(),
            "{encoded}"
        );
    }
    for encoded in ["-1", "4294967296", r#""4294967296e0""#, "1.5"] {
        assert!(
            read_record(&format!(r#""flags":{encoded}"#)).is_err(),
            "{encoded}"
        );
    }
}

#[test]
fn log_requests_are_official_otlp_and_read_back_unchanged() {
    let request = Logs {
        resource_logs: vec![ResourceLogs {
            resource: Some(Resource {
                attributes: vec![KeyValue {
                    key: "service.name".into(),
                    value: Some(AnyValue::String("example".into())),
                }],
                ..Default::default()
            }),
            scope_logs: vec![ScopeLogs {
                scope: Some(Scope {
                    name: "test".into(),
                    ..Default::default()
                }),
                log_records: vec![LogRecord {
                    time_unix_nano: 1_701_234_567_000_000_001,
                    observed_time_unix_nano: 1_701_234_567_000_000_002,
                    severity_number: 9,
                    severity_text: "INFO".into(),
                    event_name: "example.event".into(),
                    body: Some(AnyValue::String("hello\nworld".into())),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    let official: ExportLogsServiceRequest = check_roundtrip(&request);
    let record = &official.resource_logs[0].scope_logs[0].log_records[0];
    assert_eq!(record.time_unix_nano, 1_701_234_567_000_000_001);
    assert_eq!(record.event_name, "example.event");
    assert_eq!(record.severity_number, 9);
}

#[test]
fn span_requests_preserve_parents_status_events_and_links() {
    let span = Span {
        trace_id: Some(Id([0xab; 16])),
        span_id: Some(Id([0xcd; 8])),
        parent_span_id: Some(Id([0xef; 8])),
        trace_state: "vendor=state".into(),
        name: "operation".into(),
        kind: 2,
        start_time_unix_nano: 1_701_234_567_000_000_001,
        end_time_unix_nano: u64::MAX,
        flags: 257,
        attributes: vec![KeyValue {
            key: "key".into(),
            value: Some(AnyValue::String("value".into())),
        }],
        dropped_attributes_count: 3,
        events: vec![SpanEvent {
            time_unix_nano: 42,
            name: "event".into(),
            dropped_attributes_count: 1,
            ..Default::default()
        }],
        dropped_events_count: 2,
        links: vec![SpanLink {
            trace_id: Some(Id([0xab; 16])),
            span_id: Some(Id([0xef; 8])),
            trace_state: "vendor=link".into(),
            flags: 1,
            dropped_attributes_count: 4,
            ..Default::default()
        }],
        dropped_links_count: 5,
        status: Some(Status {
            code: 2,
            message: "cancelled".into(),
        }),
    };
    let request = Spans {
        resource_spans: vec![ResourceSpans {
            resource: Some(Resource::default()),
            scope_spans: vec![ScopeSpans {
                scope: Some(Scope {
                    name: "instrumentation".into(),
                    version: "1.0".into(),
                    ..Default::default()
                }),
                spans: vec![span],
                schema_url: "https://example.org/scope".into(),
            }],
            schema_url: "https://example.org/resource".into(),
        }],
    };
    let official: ExportTraceServiceRequest = check_roundtrip(&request);
    let span = &official.resource_spans[0].scope_spans[0].spans[0];
    assert_eq!(span.trace_id, vec![0xab; 16]);
    assert_eq!(span.parent_span_id, vec![0xef; 8]);
    assert_eq!(span.end_time_unix_nano, u64::MAX);
    assert_eq!(span.status.as_ref().unwrap().code, 2);
    assert_eq!(span.events[0].time_unix_nano, 42);
    assert_eq!(span.links[0].span_id, vec![0xef; 8]);
}

#[test]
fn every_attribute_value_type_matches_the_official_protocol() {
    let cases = [
        (AnyValue::Empty, json!({})),
        (AnyValue::String("".into()), json!({"stringValue":""})),
        (AnyValue::Bool(false), json!({"boolValue":false})),
        (
            AnyValue::Int(i64::MIN),
            json!({"intValue":"-9223372036854775808"}),
        ),
        (
            AnyValue::Int(i64::MAX),
            json!({"intValue":"9223372036854775807"}),
        ),
        (AnyValue::Double(1.25), json!({"doubleValue":1.25})),
        (
            AnyValue::Bytes(vec![0, 255, 16, 128]),
            json!({"bytesValue":"AP8QgA=="}),
        ),
        (
            AnyValue::Array(vec![AnyValue::Bool(true), AnyValue::Empty]),
            json!({"arrayValue":{"values":[{"boolValue":true},{}]}}),
        ),
        (
            AnyValue::Map(vec![KeyValue {
                key: "nested".into(),
                value: Some(AnyValue::Int(42)),
            }]),
            json!({"kvlistValue":{"values":[{"key":"nested","value":{"intValue":"42"}}]}}),
        ),
    ];
    for (value, expected_json) in cases {
        assert_eq!(serde_json::to_value(&value).unwrap(), expected_json);
        let official: opentelemetry_proto::tonic::common::v1::AnyValue =
            serde_json::from_value(expected_json.clone()).unwrap();
        let from_official: AnyValue =
            serde_json::from_value(serde_json::to_value(official).unwrap()).unwrap();
        assert_eq!(from_official, value);
        assert_eq!(
            serde_json::from_value::<AnyValue>(expected_json).unwrap(),
            value
        );
    }
}

#[test]
fn special_double_values_use_protojson_strings() {
    for (number, text) in [
        (f64::INFINITY, "Infinity"),
        (f64::NEG_INFINITY, "-Infinity"),
        (f64::NAN, "NaN"),
    ] {
        let expected = json!({"doubleValue":text});
        assert_eq!(
            serde_json::to_value(AnyValue::Double(number)).unwrap(),
            expected
        );
        let AnyValue::Double(decoded) = serde_json::from_value(expected).unwrap() else {
            panic!("expected double")
        };
        assert!(decoded == number || (decoded.is_nan() && number.is_nan()));
    }
}

fn check_fixture<R, P>(text: &str) -> R
where
    R: Request + PartialEq + std::fmt::Debug,
    P: serde::de::DeserializeOwned + Serialize + PartialEq + std::fmt::Debug,
{
    let expected: P = decode_official(text.as_bytes());
    let request: R = serde_json::from_str(text).unwrap();
    assert_eq!(check_roundtrip::<_, P>(&request), expected);
    request
}

fn check_roundtrip<R, P>(request: &R) -> P
where
    R: Request + PartialEq + std::fmt::Debug,
    P: serde::de::DeserializeOwned + Serialize,
{
    let mut bytes = Vec::new();
    Writer::new(&mut bytes).write(request).unwrap();
    assert_eq!(bytes.iter().filter(|b| **b == b'\n').count(), 1);
    let official: P = decode_official(&bytes);
    assert_eq!(
        &Reader::<_, R>::new(bytes.as_slice())
            .next()
            .unwrap()
            .unwrap(),
        request
    );
    let from_official: R =
        serde_json::from_value(serde_json::to_value(&official).unwrap()).unwrap();
    assert_eq!(&from_official, request);
    official
}

fn decode_official<P: serde::de::DeserializeOwned>(bytes: &[u8]) -> P {
    // The official AnyValue uses serde flatten, which cannot consume serde_json's
    // arbitrary-precision number token directly. A JSON Value first exposes
    // ordinary f64 values to that visitor while retaining the official schema.
    let json: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    serde_json::from_value(json).unwrap()
}

#[test]
fn official_json_examples_read_back_to_the_values_they_show() {
    let logs = check_fixture::<Logs, ExportLogsServiceRequest>(include_str!("fixtures/logs.json"));
    let record = &logs.resource_logs[0].scope_logs[0].log_records[0];
    assert_eq!(record.time_unix_nano, 1_544_712_660_300_000_000);
    assert_eq!(record.severity_number, 10);
    assert_eq!(
        record.body,
        Some(AnyValue::String("Example log record".into()))
    );
    assert_eq!(record.attributes[2].value, Some(AnyValue::Int(10)));
    let events =
        check_fixture::<Logs, ExportLogsServiceRequest>(include_str!("fixtures/events.json"));
    assert_eq!(
        events.resource_logs[0].scope_logs[0].log_records[0].event_name,
        "browser.page_view"
    );
    let spans =
        check_fixture::<Spans, ExportTraceServiceRequest>(include_str!("fixtures/trace.json"));
    let span = &spans.resource_spans[0].scope_spans[0].spans[0];
    assert_eq!(span.kind, 2);
    assert_eq!(span.name, "I'm a server span");
    assert_eq!(
        span.parent_span_id,
        Some(Id([0xee, 0xe1, 0x9b, 0x7e, 0xc3, 0xc1, 0xb1, 0x73]))
    );
    for line in include_str!("fixtures/file-logs.jsonl").lines() {
        check_fixture::<Logs, ExportLogsServiceRequest>(line);
    }
    for line in include_str!("fixtures/file-spans.jsonl").lines() {
        check_fixture::<Spans, ExportTraceServiceRequest>(line);
    }
}

#[test]
fn ids_are_hex_enums_numeric_keys_camel_case_and_64_bit_integers_strings() {
    let record = LogRecord {
        trace_id: Some(Id([0xab; 16])),
        span_id: Some(Id([0xcd; 8])),
        time_unix_nano: u64::MAX,
        observed_time_unix_nano: 42,
        severity_number: 9,
        flags: 1,
        dropped_attributes_count: 2,
        body: Some(AnyValue::Int(i64::MIN)),
        ..Default::default()
    };
    let json = serde_json::to_value(record).unwrap();
    assert_eq!(json["traceId"], "abababababababababababababababab");
    assert_eq!(json["spanId"], "cdcdcdcdcdcdcdcd");
    assert_eq!(json["timeUnixNano"], "18446744073709551615");
    assert_eq!(json["observedTimeUnixNano"], "42");
    assert_eq!(json["body"]["intValue"], "-9223372036854775808");
    assert_eq!(json["severityNumber"], 9);
    assert_eq!(json["flags"], 1);
    assert_eq!(json["droppedAttributesCount"], 2);
    assert!(json.get("time_unix_nano").is_none());
    let span = serde_json::to_value(Span {
        kind: 3,
        status: Some(Status {
            code: 2,
            ..Default::default()
        }),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(span["kind"], 3);
    assert_eq!(span["status"]["code"], 2);
    assert!(serde_json::from_value::<Span>(json!({"kind":"SPAN_KIND_SERVER"})).is_err());
    for invalid in ["abcd", "zzzzzzzzzzzzzzzz", "éééééééé", "abcdABCDabcdABC_"] {
        assert!(
            serde_json::from_value::<SpanId>(json!(invalid)).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn reader_accepts_numeric_times_nulls_and_unknown_message_fields() {
    let input = br#"{"future":{"anything":[1,2]},"resourceLogs":[{"resource":null,"scopeLogs":[{"scope":{"name":null,"future":true},"logRecords":[{"timeUnixNano":42,"observedTimeUnixNano":"43","severityText":null,"traceId":"ABABABABABABABABABABABABABABABAB","spanId":"","body":{"future":{"x":1},"intValue":-42},"attributes":null}]}]}]}"#;
    let request = Reader::<_, Logs>::new(input.as_slice())
        .next()
        .unwrap()
        .unwrap();
    let record = &request.resource_logs[0].scope_logs[0].log_records[0];
    assert_eq!(record.time_unix_nano, 42);
    assert_eq!(record.observed_time_unix_nano, 43);
    assert_eq!(record.body, Some(AnyValue::Int(-42)));
    assert_eq!(record.trace_id, Some(Id([0xab; 16])));
    assert_eq!(record.span_id, None);
    assert!(record.severity_text.is_empty());
    assert!(record.attributes.is_empty());
    assert_eq!(
        serde_json::from_value::<AnyValue>(json!({"future": [1]})).unwrap(),
        AnyValue::Empty
    );
    assert!(
        serde_json::from_value::<AnyValue>(json!({"intValue": "1", "boolValue":true})).is_err()
    );
}

#[test]
fn files_keep_one_signal_and_report_invalid_lines() {
    let logs: Logs = serde_json::from_str(include_str!("fixtures/logs.json")).unwrap();
    let spans: Spans = serde_json::from_str(include_str!("fixtures/trace.json")).unwrap();
    let mut bytes = Vec::new();
    let mut writer = Writer::new(&mut bytes);
    writer.write(&logs).unwrap();
    writer.write(&logs).unwrap();
    assert_eq!(
        Reader::<_, Logs>::new(bytes.as_slice())
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
        vec![logs.clone(), logs]
    );
    assert!(Writer::new(Vec::new()).write(&Logs::default()).is_err());
    let mut span_bytes = Vec::new();
    Writer::new(&mut span_bytes).write(&spans).unwrap();
    assert!(
        Reader::<_, Logs>::new(span_bytes.as_slice())
            .next()
            .unwrap()
            .is_err()
    );
    assert!(
        Reader::<_, Spans>::new(bytes.as_slice())
            .next()
            .unwrap()
            .is_err()
    );
    assert!(
        Reader::<_, Logs>::new(br#"{"resourceLogs":[],"resourceSpans":[]}"#.as_slice())
            .next()
            .unwrap()
            .is_err()
    );
    for invalid in ["\n", "[]\n", "null\n", "{\"resourceLogs\":"] {
        assert!(
            Reader::<_, Logs>::new(invalid.as_bytes())
                .next()
                .unwrap()
                .is_err(),
            "{invalid}"
        );
    }
    assert!(Reader::<_, Logs>::new(b"".as_slice()).next().is_none());
}

#[test]
fn bytes_accept_protojson_base64_variants_and_reject_malformed_data() {
    for text in ["+/8=", "+/8", "-_8=", "-_8"] {
        let value: AnyValue = serde_json::from_value(json!({"bytesValue":text})).unwrap();
        assert_eq!(value, AnyValue::Bytes(vec![251, 255]));
        assert_eq!(
            serde_json::to_value(value).unwrap(),
            json!({"bytesValue":"+/8="})
        );
    }
    for text in ["A", "AA=", "A===", "AA=A", "AA==AAAA", "!AAA", "AB=="] {
        assert!(
            serde_json::from_value::<AnyValue>(json!({"bytesValue":text})).is_err(),
            "{text}"
        );
    }
    for (bytes, encoded) in [
        (vec![], ""),
        (vec![102], "Zg=="),
        (vec![102, 111], "Zm8="),
        (vec![102, 111, 111], "Zm9v"),
    ] {
        assert_eq!(
            serde_json::to_value(AnyValue::Bytes(bytes.clone())).unwrap(),
            json!({"bytesValue":encoded})
        );
        assert_eq!(
            serde_json::from_value::<AnyValue>(json!({"bytesValue":encoded})).unwrap(),
            AnyValue::Bytes(bytes)
        );
    }
}

#[test]
fn collector_input_files_roundtrip_through_disk() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = std::env::var_os("OTLP_JSONL_CHECK_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| temporary.path().to_path_buf());
    std::fs::create_dir_all(&directory).unwrap();
    let mut logs: Logs = serde_json::from_str(include_str!("fixtures/logs.json")).unwrap();
    let record = &mut logs.resource_logs[0].scope_logs[0].log_records[0];
    record.event_name = "example.event".into();
    record.attributes.push(KeyValue {
        key: "bytes".into(),
        value: Some(AnyValue::Bytes(vec![0, 255, 16, 128])),
    });
    record.attributes.push(KeyValue {
        key: "negative".into(),
        value: Some(AnyValue::Int(i64::MIN)),
    });
    let mut spans: Spans = serde_json::from_str(include_str!("fixtures/trace.json")).unwrap();
    let span = &mut spans.resource_spans[0].scope_spans[0].spans[0];
    span.events.push(SpanEvent {
        time_unix_nano: 1_544_712_660_300_000_000,
        name: "finished".into(),
        attributes: record.attributes.clone(),
        ..Default::default()
    });
    span.status = Some(Status {
        code: 2,
        message: "example error".into(),
    });
    let log_path = directory.join("input.logs.jsonl");
    let span_path = directory.join("input.spans.jsonl");
    {
        let mut writer = Writer::new(std::io::BufWriter::new(
            std::fs::File::create(&log_path).unwrap(),
        ));
        writer.write(&logs).unwrap();
        writer.write(&logs).unwrap();
        let mut writer = Writer::new(std::io::BufWriter::new(
            std::fs::File::create(&span_path).unwrap(),
        ));
        writer.write(&spans).unwrap();
        writer.write(&spans).unwrap();
    }
    let log_records = Reader::<_, Logs>::new(std::io::BufReader::new(
        std::fs::File::open(log_path).unwrap(),
    ))
    .collect::<Result<Vec<_>, _>>()
    .unwrap();
    let span_records = Reader::<_, Spans>::new(std::io::BufReader::new(
        std::fs::File::open(span_path).unwrap(),
    ))
    .collect::<Result<Vec<_>, _>>()
    .unwrap();
    assert_eq!(log_records, vec![logs.clone(), logs]);
    assert_eq!(span_records, vec![spans.clone(), spans]);
}

#[test]
fn stream_io_errors_reach_the_caller() {
    struct FailedFlush;
    impl std::io::Write for FailedFlush {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::other("flush failed"))
        }
    }
    let logs: Logs = serde_json::from_str(include_str!("fixtures/logs.json")).unwrap();
    assert_eq!(
        Writer::new(FailedFlush)
            .write(&logs)
            .unwrap_err()
            .to_string(),
        "flush failed"
    );
    struct FailedWrite;
    impl std::io::Write for FailedWrite {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("write failed"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert_eq!(
        Writer::new(FailedWrite)
            .write(&logs)
            .unwrap_err()
            .to_string(),
        "write failed"
    );
    assert!(
        Reader::<_, Logs>::new(b"\xff\n".as_slice())
            .next()
            .unwrap()
            .is_err()
    );
}
