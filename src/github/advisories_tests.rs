use super::*;

fn draft(fingerprint: &str) -> DraftAdvisory {
    DraftAdvisory {
        fingerprint: fingerprint.into(),
        summary: "Bound input".into(),
        description: format!("Private evidence\nFingerprint: `{fingerprint}`\n"),
        package: Package {
            ecosystem: "rust".into(),
            name: None,
        },
    }
}

fn advisory(draft: &DraftAdvisory) -> Value {
    json!({
        "ghsa_id": "GHSA-test", "description": draft.description,
        "state": "draft", "severity": null, "summary": draft.summary,
        "html_url": "https://github.com/acme/widgets/security/advisories/GHSA-test"
    })
}

#[test]
fn repeated_fingerprints_create_one_validated_record() {
    let mut records = SecurityRecords::advisories(Vec::new());
    let draft = draft("bounded-input");
    let mut creations = 0;
    let created = records
        .record_or_reuse(&draft, |storage, draft| {
            creations += 1;
            storage.decode_created(advisory(draft))
        })
        .unwrap();
    assert!(created.created);
    assert!(created.record.untriaged());
    assert_eq!(
        created.record.metadata(),
        &RecordedFinding {
            severity: None,
            title: "Bound input".into(),
            url: "https://github.com/acme/widgets/security/advisories/GHSA-test".into(),
        }
    );
    let existing = records
        .record_or_reuse(&draft, |_, _| {
            creations += 1;
            bail!("a duplicate must not create another record")
        })
        .unwrap();
    assert!(!existing.created);
    assert_eq!(
        existing.record.description(),
        "Private evidence\nFingerprint: `bounded-input`\n"
    );
    assert_eq!(existing.record.metadata(), created.record.metadata());
    assert_eq!(creations, 1);
}

fn private_issue(draft: &DraftAdvisory) -> Value {
    json!({
        "number": 42, "body": draft.description, "state": "OPEN",
        "labels": [{"name": "security-finding"}, {"name": "Needs-Triage"}],
        "title": draft.summary, "html_url": "https://github.com/acme/widgets/issues/42"
    })
}

fn collection(private: bool, values: Vec<Value>) -> SecurityRecords {
    if private {
        SecurityRecords::issues(values)
    } else {
        SecurityRecords::advisories(values)
    }
}

fn native(private: bool, draft: &DraftAdvisory) -> Value {
    if private {
        private_issue(draft)
    } else {
        advisory(draft)
    }
}

#[test]
fn private_issue_duplicates_return_typed_identity_and_safe_metadata() {
    let draft = draft("private-bound");
    let mut records = collection(true, Vec::new());
    let first = records
        .record_or_reuse(&draft, |storage, draft| {
            storage.decode_created(private_issue(draft))
        })
        .unwrap();
    assert!(first.created);
    assert!(first.record.untriaged());
    assert_eq!(first.record.private_issue_number(), Some(42));
    assert_eq!(first.record.name(), "issue #42");
    assert_eq!(
        first.record.metadata(),
        &RecordedFinding {
            severity: None,
            title: "Bound input".into(),
            url: "https://github.com/acme/widgets/issues/42".into(),
        }
    );
    let reused = records
        .record_or_reuse(&draft, |_, _| bail!("duplicate creation"))
        .unwrap();
    assert!(!reused.created);
    assert_eq!(
        reused.record.description(),
        "Private evidence\nFingerprint: `private-bound`\n"
    );
    assert_eq!(reused.record.metadata(), first.record.metadata());
}

#[test]
fn existing_records_in_every_state_keep_day_shift_grades_and_write_ups() {
    for (private, state, severity, untriaged) in [
        (false, "draft", None, true),
        (false, "draft", Some("low"), false),
        (false, "published", Some("high"), false),
        (false, "closed", None, false),
        (false, "triage", None, false),
        (true, "OPEN", None, true),
        (true, "open", Some("informational"), false),
        (true, "CLOSED", None, false),
    ] {
        let mut candidate = draft("graded");
        let mut value = native(private, &candidate);
        value["state"] = json!(state);
        value["severity"] = json!(severity);
        if private && !untriaged {
            value["labels"] = json!([{"name": "security-finding"}]);
        }
        let field = if private { "body" } else { "description" };
        value[field] = json!("Day shift write-up\nFingerprint: `graded`\n");
        let mut records = collection(private, vec![value]);
        candidate.description = "A repeated finding must not replace the Day shift's work".into();
        for _ in 0..2 {
            let resolved = records
                .record_or_reuse(&candidate, |_, _| bail!("existing creation"))
                .unwrap();
            assert!(!resolved.created, "{private} {state}");
            assert_eq!(resolved.record.untriaged(), untriaged, "{private} {state}");
            assert_eq!(
                resolved.record.description(),
                "Day shift write-up\nFingerprint: `graded`\n"
            );
            assert_eq!(resolved.record.metadata().severity.as_deref(), severity);
            assert_eq!(resolved.record.metadata().title, "Bound input");
        }
    }
}

#[test]
fn unused_malformed_records_and_inexact_markers_do_not_block_recording() {
    for private in [false, true] {
        let candidate = draft("exact");
        let field = if private { "body" } else { "description" };
        let mut records = collection(
            private,
            vec![
                json!({"irrelevant": "malformed historical record"}),
                json!({field: "Fingerprint: `exact-extra`\ninline Fingerprint: `exact`\nFingerprint: `exact` trailing"}),
            ],
        );
        let resolved = records
            .record_or_reuse(&candidate, |storage, draft| {
                storage.decode_created(native(private, draft))
            })
            .unwrap();
        assert!(resolved.created);
        let repeated = records
            .record_or_reuse(&candidate, |_, _| bail!("duplicate creation"))
            .unwrap();
        assert!(!repeated.created);
    }
}

#[test]
fn malformed_selected_records_fail_without_attempting_creation() {
    for private in [false, true] {
        for field in if private {
            ["number", "title", "html_url"]
        } else {
            ["ghsa_id", "summary", "html_url"]
        } {
            let candidate = draft("selected");
            let mut value = native(private, &candidate);
            value.as_object_mut().unwrap().remove(field);
            let mut records = collection(private, vec![value]);
            let mut creations = 0;
            let error = records
                .record_or_reuse(&candidate, |_, _| {
                    creations += 1;
                    bail!("creation must not hide invalid matching records")
                })
                .err()
                .unwrap();
            assert_eq!(creations, 0);
            assert!(!format!("{error:#}").contains("Private evidence"));
        }
    }
}

#[test]
fn failed_creations_and_decoding_do_not_claim_successful_insertion_or_undo_remote_effects() {
    for private in [false, true] {
        for missing in [
            None,
            Some(if private { "number" } else { "ghsa_id" }),
            Some(if private { "body" } else { "description" }),
            Some(if private { "title" } else { "summary" }),
            Some("html_url"),
        ] {
            let candidate = draft("retry");
            let mut records = collection(private, Vec::new());
            let mut remote_creations = Vec::new();
            let failed = records.record_or_reuse(&candidate, |storage, draft| {
                if let Some(field) = missing {
                    let mut value = native(private, draft);
                    remote_creations.push(value.clone());
                    value.as_object_mut().unwrap().remove(field);
                    storage.decode_created(value)
                } else {
                    bail!("scripted creation failure")
                }
            });
            assert!(failed.is_err());
            assert_eq!(remote_creations.len(), usize::from(missing.is_some()));
            let retry = records
                .record_or_reuse(&candidate, |storage, draft| {
                    let value = native(private, draft);
                    remote_creations.push(value.clone());
                    storage.decode_created(value)
                })
                .unwrap();
            assert!(retry.created, "{private} {missing:?}");
            assert_eq!(
                remote_creations.len(),
                if missing.is_some() { 2 } else { 1 }
            );
            assert!(
                !records
                    .record_or_reuse(&candidate, |_, _| bail!("duplicate creation"))
                    .unwrap()
                    .created
            );
        }
    }
}

#[test]
fn the_pass_adapter_creates_typed_records_in_the_selected_storage() {
    use crate::pass::{InMemory, Outside};
    for private in [false, true] {
        let mut outside = if private {
            InMemory::default().finding_issues(Vec::new())
        } else {
            InMemory::default()
        };
        let mut records = outside.security_records().unwrap();
        let resolved = records
            .record_or_reuse(&draft("pass-adapter"), |storage, draft| {
                outside.create_security_record(storage, draft)
            })
            .unwrap();
        assert!(resolved.created);
        assert_eq!(resolved.record.private_issue_number().is_some(), private);
        let repeated = records
            .record_or_reuse(&draft("pass-adapter"), |_, _| bail!("duplicate creation"))
            .unwrap();
        assert!(!repeated.created);
        assert_eq!(repeated.record.metadata(), resolved.record.metadata());
    }
}
