use super::*;

#[test]
fn semantic_sources_create_compatible_records_with_lazy_commit_validation() {
    let commit = "0123456789abcdef0123456789abcdef01234567";
    let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
    for private in [false, true] {
        for source in [
            FindingSource::Audit { commit },
            FindingSource::Review {
                commit,
                issue: &issue,
            },
        ] {
            let draft = DraftAdvisory::new(
                source,
                "bounded-input".into(),
                "Bound input".into(),
                "Original evidence.",
                "{\n  \"evidence\": true\n}",
                Package {
                    ecosystem: "rust".into(),
                    name: Some("widgets".into()),
                },
            );
            let expected = match source {
                FindingSource::Audit { .. } => {
                    "Found by thirdshift's Security run.\n\nFingerprint: `bounded-input`\nAudited commit: `0123456789abcdef0123456789abcdef01234567`\n\nOriginal evidence.\n\n```json\n{\n  \"evidence\": true\n}\n```\n"
                }
                FindingSource::Review { .. } => {
                    "Found by thirdshift's Security review.\n\nFingerprint: `bounded-input`\nAudited commit: `0123456789abcdef0123456789abcdef01234567`\nReview issue: https://github.com/acme/widgets/issues/7\n\nOriginal evidence.\n\n```json\n{\n  \"evidence\": true\n}\n```\n"
                }
            };
            assert_eq!(draft.description, expected);
            assert_eq!(draft.package.name.as_deref(), Some("widgets"));
            let mut records = collection(private, vec![json!({"unused": "malformed"})]);
            let created = records
                .record_or_reuse(&draft, |storage, draft| {
                    storage.decode_created(native(private, draft))
                })
                .unwrap();
            assert!(created.created);
            assert_eq!(created.record.audited_commit().unwrap(), commit);
            let reused = records
                .record_or_reuse(&draft, |_, _| bail!("duplicate creation"))
                .unwrap();
            assert!(!reused.created);
            assert_eq!(reused.record.description(), expected);
        }
    }
}

#[test]
fn repeated_reproduction_and_fix_completion_preserve_evidence_fences_and_private_edits() {
    use crate::security::reproduction::Outcome;
    let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/8").unwrap();
    for private in [false, true] {
        let mut candidate = draft("legacy");
        candidate.description = "Fingerprint: `legacy`\nAudited commit: `ABCDEF0123456789ABCDEF0123456789ABCDEF0123`\nOriginal evidence.\n## Reproduction\nOrdinary Markdown heading.  \n".into();
        let storage = collection(private, Vec::new());
        let record = storage.decode_created(native(private, &candidate)).unwrap();
        let pending = record.link_fix(record.description(), &issue).unwrap();
        assert!(record.link_fix(&format!("{pending}edit"), &issue).is_err());
        candidate.description = pending;
        let record = storage.decode_created(native(private, &candidate)).unwrap();
        let reproduction = Reproduction {
            outcome: Outcome::Reproduced {
                severity: Severity::High,
                size: FixSize::Spec,
            },
            notes: "Private reproduction notes.".into(),
            test: "`````rust\nbounded_fixture();\n`````".into(),
        };
        candidate.description = record.with_reproduction(&reproduction);
        let expected = "Fingerprint: `legacy`\nAudited commit: `ABCDEF0123456789ABCDEF0123456789ABCDEF0123`\nOriginal evidence.\n## Reproduction\nOrdinary Markdown heading.\n\n<!-- thirdshift:security-reproduction -->\n## Reproduction\n\nOutcome: reproduced high spec\nSeverity: high\nFix size: spec\n\nPrivate reproduction notes.\n\n### Proof-of-concept test\n\n``````\n`````rust\nbounded_fixture();\n`````\n``````\n\n<!-- thirdshift:security-fix -->\nFix Ticket: https://github.com/acme/widgets/issues/8\nFix Run: pending\n";
        assert_eq!(candidate.description, expected);
        let record = storage.decode_created(native(private, &candidate)).unwrap();
        assert!(record.fix_size().unwrap() == FixSize::Spec);
        assert_eq!(record.with_reproduction(&reproduction), expected);
        let current = format!("{expected}Day shift note: keep the input contract.\n");
        let failed = record.complete_fix(&current, &issue, false).unwrap();
        assert_eq!(failed, format!("{current}Fix Run: failed\n"));
        candidate.description = failed;
        let records = collection(private, vec![native(private, &candidate)]);
        assert_eq!(records.failed_fix().unwrap().unwrap().url, issue.url);
        let succeeded = record
            .complete_fix(&candidate.description, &issue, true)
            .unwrap();
        assert_eq!(
            succeeded,
            format!("{}Fix Run: succeeded\n", candidate.description)
        );
        candidate.description = succeeded;
        let records = collection(private, vec![native(private, &candidate)]);
        assert!(records.failed_fix().unwrap().is_none());
        assert!(records.next_fix().unwrap().is_none());
        let changed = current.replace("issues/8", "issues/9");
        assert!(record.complete_fix(&changed, &issue, true).is_err());
        assert!(
            record
                .complete_fix(&current.replace("issues/8", "issues/8 "), &issue, false)
                .is_err()
        );
    }
}

#[test]
fn public_fix_checks_preserve_copied_line_exclusions_and_short_statement_detection() {
    for private in [false, true] {
        let mut candidate = draft("private-lines");
        candidate.description = "Private write-up.\n   bypass_login();   \nneedle\n___\n!!!\n```rust\nFingerprint: `private-lines`\nAudited commit: `bad`\nOutcome: reproduced high single\nSeverity: high\nFix size: single\n## Heading\n<!-- comment -->\n".into();
        let storage = collection(private, Vec::new());
        let record = storage.decode_created(native(private, &candidate)).unwrap();
        assert!(record.contains_private_text("Bound input; Private write-up."));
        assert!(record.contains_private_text("Remove bypass_login(); from the input path."));
        assert!(!record.contains_private_text("needle ___ !!! ```rust Fingerprint: `private-lines` Audited commit: `bad` Outcome: reproduced high single Severity: high Fix size: single ## Heading <!-- comment -->"));
        assert!(record.audited_commit().is_err());
    }
}

#[test]
fn legacy_records_keep_first_outcomes_and_last_fix_endings_without_eager_decoding() {
    use crate::security::reproduction::Outcome;
    for private in [false, true] {
        let field = if private { "body" } else { "description" };
        let mut candidate = draft("legacy-selection");
        candidate.description = "Fingerprint: `legacy-selection`\nAudited commit: no backticks\nAudited commit: `0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF`\nAudited commit: `bad`\nUnrelated text.\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced low spec\nOutcome: reproduced critical single\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced high single".into();
        let mut records = collection(
            private,
            vec![
                json!({field: "Unselected malformed history"}),
                native(private, &candidate),
            ],
        );
        let record = records
            .record_or_reuse(&candidate, |_, _| bail!("duplicate creation"))
            .unwrap()
            .record;
        assert_eq!(
            record.audited_commit().unwrap(),
            "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF"
        );
        assert!(record.fix_size().unwrap() == FixSize::Spec);
        assert_eq!(
            records.next_fix().unwrap().unwrap().1.severity.as_deref(),
            Some("low")
        );
        let suffix = "\n<!-- thirdshift:security-fix -->\nFix Ticket: https://github.com/acme/widgets/issues/8\nFix Run: succeeded\n<!-- thirdshift:security-fix -->\nFix Ticket: https://github.com/acme/widgets/issues/9\nFix Run: pending\nDay shift edits without final newline";
        candidate.description.push_str(suffix);
        let records = collection(private, vec![native(private, &candidate)]);
        assert_eq!(records.failed_fix().unwrap().unwrap().number, 9);
        assert!(records.next_fix().unwrap().is_none());
        let record = records.decode_created(native(private, &candidate)).unwrap();
        let replacement = record.with_reproduction(&Reproduction {
            outcome: Outcome::NotReproduced,
            notes: "Not reproduced.".into(),
            test: "harmless_fixture();\n".into(),
        });
        assert_eq!(
            replacement,
            "Fingerprint: `legacy-selection`\nAudited commit: no backticks\nAudited commit: `0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF`\nAudited commit: `bad`\nUnrelated text.\n\n<!-- thirdshift:security-reproduction -->\n## Reproduction\n\nOutcome: not reproduced\n\nNot reproduced.\n\n### Proof-of-concept test\n\n```\nharmless_fixture();\n```\n\n<!-- thirdshift:security-fix -->\nFix Ticket: https://github.com/acme/widgets/issues/9\nFix Run: pending\nDay shift edits without final newline"
        );
        for ending in ["Fix Run: succeeded", "Fix Run: unknown", "Fix Run: failed "] {
            let mut value = native(private, &candidate);
            value[field] = json!(format!("{}\n{ending}", candidate.description));
            assert!(
                collection(private, vec![value])
                    .failed_fix()
                    .unwrap()
                    .is_none()
            );
        }
        for (description, error) in [
            ("Fingerprint: `legacy-selection`", "no audited commit"),
            (
                "Audited commit: `bad`\nAudited commit: `ABCDEF0123456789ABCDEF0123456789ABCDEF0123`",
                "invalid audited commit",
            ),
            (
                "Audited commit: `éééééééééééééééééééé`",
                "invalid audited commit",
            ),
        ] {
            candidate.description = description.into();
            let record = records.decode_created(native(private, &candidate)).unwrap();
            assert!(
                record
                    .audited_commit()
                    .unwrap_err()
                    .to_string()
                    .contains(error)
            );
        }
        for outcome in [
            "not reproduced",
            "reproduced invalid spec",
            "reproduced high invalid",
        ] {
            candidate.description = format!(
                "Private evidence.\n<!-- thirdshift:security-reproduction -->\nOutcome: {outcome}\nOutcome: reproduced high single"
            );
            let records = collection(private, vec![native(private, &candidate)]);
            assert!(records.next_fix().unwrap().is_none());
            assert!(records.waiting_for_day_shift(true));
            assert!(
                records
                    .decode_created(native(private, &candidate))
                    .unwrap()
                    .fix_size()
                    .is_err()
            );
        }
    }
}

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
