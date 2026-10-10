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

#[test]
fn constructed_drafts_preserve_the_existing_layout_and_support_commit_lookup() {
    for provenance in [
        FindingProvenance::Audit,
        FindingProvenance::Review {
            issue_url: "https://github.com/acme/widgets/issues/9",
        },
    ] {
        let draft = DraftAdvisory::new(FindingDraft {
            fingerprint: "bounded-input",
            summary: "Bound input",
            audited_commit: "0123456789abcdef0123456789abcdef01234567",
            provenance,
            original_description: "## Original heading\nPrivate evidence\n",
            evidence: "{\n  \"proof\": true\n}",
            package: Package {
                ecosystem: "rust".into(),
                name: Some("widgets".into()),
            },
        });
        let expected = match provenance {
            FindingProvenance::Audit => {
                "Found by thirdshift's Security run.\n\nFingerprint: `bounded-input`\nAudited commit: `0123456789abcdef0123456789abcdef01234567`\n\n## Original heading\nPrivate evidence\n\n\n```json\n{\n  \"proof\": true\n}\n```\n"
            }
            FindingProvenance::Review { .. } => {
                "Found by thirdshift's Security review.\n\nFingerprint: `bounded-input`\nAudited commit: `0123456789abcdef0123456789abcdef01234567`\nReview issue: https://github.com/acme/widgets/issues/9\n\n## Original heading\nPrivate evidence\n\n\n```json\n{\n  \"proof\": true\n}\n```\n"
            }
        };
        assert_eq!(draft.description, expected);
        assert_eq!(draft.package.name.as_deref(), Some("widgets"));
        for private in [false, true] {
            let mut records = collection(private, Vec::new());
            let created = records
                .record_or_reuse(&draft, |storage, draft| {
                    storage.decode_created(native(private, draft))
                })
                .unwrap();
            assert_eq!(
                created.record.audited_commit().unwrap(),
                "0123456789abcdef0123456789abcdef01234567"
            );
        }
    }
}

fn record(private: bool, text: &str) -> SecurityRecord {
    let mut candidate = draft("fixture");
    candidate.description = text.into();
    collection(private, Vec::new())
        .decode_created(native(private, &candidate))
        .unwrap()
}

#[test]
fn reproduction_replacement_preserves_original_markdown_backticks_and_the_fix_section() {
    use crate::security::reproduction::Outcome;
    let original = "## Reproduction\nOriginal arbitrary heading.\n\n<!-- thirdshift:security-reproduction -->\n## Reproduction\nOutcome: reproduced low single\nOld notes.\n\n<!-- thirdshift:security-fix -->\nFix Ticket: https://github.com/acme/widgets/issues/8\nFix Run: pending\nDay shift trailing edit.\n";
    let reproduction = Reproduction {
        outcome: Outcome::Reproduced {
            severity: Severity::High,
            size: FixSize::Spec,
        },
        notes: "New notes.\nKeep this newline.\n".into(),
        test: "````rust\nbypass_login();\n````".into(),
    };
    for private in [false, true] {
        let updated = record(private, original).with_reproduction(&reproduction);
        assert_eq!(
            updated,
            "## Reproduction\nOriginal arbitrary heading.\n\n<!-- thirdshift:security-reproduction -->\n## Reproduction\n\nOutcome: reproduced high spec\nSeverity: high\nFix size: spec\n\nNew notes.\nKeep this newline.\n\n\n### Proof-of-concept test\n\n`````\n````rust\nbypass_login();\n````\n`````\n\n<!-- thirdshift:security-fix -->\nFix Ticket: https://github.com/acme/widgets/issues/8\nFix Run: pending\nDay shift trailing edit.\n"
        );
        assert!(record(private, &updated).fix_size().unwrap() == FixSize::Spec);
        let mut value = native(private, &draft("fixture"));
        value[if private { "body" } else { "description" }] = json!(updated);
        let records = collection(private, vec![value]);
        assert!(records.next_fix().unwrap().is_none());
        assert_eq!(records.failed_fix().unwrap().unwrap().number, 8);
    }
}

#[test]
fn fix_transitions_preserve_fresh_edits_and_join_the_collection_gates() {
    let original = "Existing write-up.\n\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced high single\n";
    let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/8").unwrap();
    for private in [false, true] {
        let record = record(private, original);
        assert!(
            record
                .with_pending_fix("Changed write-up.", &issue)
                .is_err()
        );
        let pending = record.with_pending_fix(original, &issue).unwrap();
        assert_eq!(
            pending,
            "Existing write-up.\n\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced high single\n<!-- thirdshift:security-fix -->\nFix Ticket: https://github.com/acme/widgets/issues/8\nFix Run: pending\n"
        );
        let fresh = format!("{pending}Day shift trailing edit.\n\n");
        let failed = record
            .with_fix_ending(&fresh, &issue, FixEnding::Failed)
            .unwrap();
        assert!(failed.ends_with("Fix Run: pending\nDay shift trailing edit.\nFix Run: failed\n"));
        let succeeded = record
            .with_fix_ending(&failed, &issue, FixEnding::Succeeded)
            .unwrap();
        assert!(
            succeeded.ends_with("Day shift trailing edit.\nFix Run: failed\nFix Run: succeeded\n")
        );
        for (description, paused) in [(&pending, true), (&failed, true), (&succeeded, false)] {
            let mut value = native(private, &draft("fixture"));
            value[if private { "body" } else { "description" }] = json!(description);
            let records = collection(private, vec![value]);
            assert_eq!(records.failed_fix().unwrap().is_some(), paused);
            assert!(records.next_fix().unwrap().is_none());
            assert!(records.waiting_for_day_shift(false));
            assert!(!records.waiting_for_day_shift(true));
        }
        for wrong in [
            "No fix section.",
            "\n<!-- thirdshift:security-fix -->\nFix Ticket: malformed\nFix Run: pending\n",
            "\n<!-- thirdshift:security-fix -->\nFix Ticket: https://github.com/acme/widgets/issues/9\nFix Run: pending\n",
        ] {
            let error = record
                .with_fix_ending(wrong, &issue, FixEnding::Failed)
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "the private record's fix Ticket changed during its Run; leaving it unchanged"
            );
        }
    }
}

#[test]
fn public_fix_text_requires_its_private_link_and_rejects_literal_short_evidence() {
    let private = "Found by thirdshift's Security review.\n\nFingerprint: `fixture`\nAudited commit: `0123456789abcdef0123456789abcdef01234567`\n## Private heading\nPrivate evidence crosses the bound.\n\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced high single\nSeverity: high\nFix size: single\n```\nbypass_login();\ninput\n```\n";
    let url = "https://github.com/acme/widgets/security/advisories/GHSA-test";
    for storage in [false, true] {
        let record = record(storage, private);
        assert_eq!(
            record
                .check_public_fix_text("Bound input.", url)
                .unwrap_err()
                .to_string(),
            "the Security fix Ticket does not link the private record"
        );
        for copy in [
            "bypass_login();",
            "Private evidence crosses the bound.",
            "Found by thirdshift's Security review.",
        ] {
            let text = format!("Bound input. Private record: {url}\n{copy}");
            assert_eq!(
                record
                    .check_public_fix_text(&text, url)
                    .unwrap_err()
                    .to_string(),
                "the Security fix Ticket includes private write-up text"
            );
        }
        let text = format!(
            "Bound input. Private record: {url}\nFingerprint: `fixture`\nAudited commit: `0123456789abcdef0123456789abcdef01234567`\nOutcome: reproduced high single\nSeverity: high\nFix size: single\n## Private heading\n<!-- thirdshift:security-reproduction -->\n```\n"
        );
        record.check_public_fix_text(&text, url).unwrap();
    }
}

#[test]
fn historical_commit_validation_is_lazy_and_keeps_the_first_matching_field() {
    for private in [false, true] {
        for (description, diagnostic) in [
            (
                "Private evidence without protocol fields.",
                "Security finding record has no audited commit",
            ),
            (
                "Audited commit: `invalid`\nAudited commit: `0123456789abcdef0123456789abcdef01234567`\nPrivate evidence.",
                "Security finding record has an invalid audited commit",
            ),
            (
                "Audited commit: `gggggggggggggggggggggggggggggggggggggggg`",
                "Security finding record has an invalid audited commit",
            ),
            (
                "Audited commit: `0123456789abcdef0123456789abcdef01234567` trailing",
                "Security finding record has no audited commit",
            ),
        ] {
            let record = record(private, description);
            assert!(record.fix_size().is_err());
            assert_eq!(record.audited_commit().unwrap_err().to_string(), diagnostic);
            // Historical records can still be reused without commit validation.
            let mut candidate = draft("historical");
            candidate.description = format!("Fingerprint: `historical`\n{description}");
            let mut records = collection(private, vec![native(private, &candidate)]);
            assert!(
                !records
                    .record_or_reuse(&candidate, |_, _| bail!("historical creation"))
                    .unwrap()
                    .created
            );
        }
        let record = record(
            private,
            "Audited commit: `0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef`\nAudited commit: `invalid`\n",
        );
        assert_eq!(
            record.audited_commit().unwrap(),
            "0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef"
        );
    }
}

#[test]
fn reproduction_writes_join_selection_and_not_reproduced_records_keep_evidence() {
    use crate::security::reproduction::Outcome;
    for private in [false, true] {
        for (outcome, reproduced) in [
            (
                Outcome::Reproduced {
                    severity: Severity::Medium,
                    size: FixSize::Single,
                },
                true,
            ),
            (Outcome::NotReproduced, false),
        ] {
            let text = record(private, "## Reproduction\nOriginal write-up.\n\n")
                .with_reproduction(&Reproduction {
                    outcome,
                    notes: "Local notes.".into(),
                    test: "fixture();\n".into(),
                });
            assert!(text.starts_with(
                "## Reproduction\nOriginal write-up.\n\n<!-- thirdshift:security-reproduction -->\n"
            ));
            assert!(text.ends_with("```\nfixture();\n```\n"));
            let updated = record(private, &text);
            assert_eq!(updated.fix_size().is_ok(), reproduced);
            let mut value = native(private, &draft("fixture"));
            value[if private { "body" } else { "description" }] = json!(text);
            let records = collection(private, vec![value]);
            let selected = records.next_fix().unwrap();
            assert_eq!(selected.is_some(), reproduced);
            if let Some((record, metadata)) = selected {
                assert_eq!(metadata.severity.as_deref(), Some("medium"));
                assert!(record.fix_size().unwrap() == FixSize::Single);
            }
            assert!(records.waiting_for_day_shift(false));
            assert_eq!(records.waiting_for_day_shift(true), !reproduced);
        }
    }
}

#[test]
fn historical_selection_keeps_first_outcome_and_last_fix_link_and_status() {
    for private in [false, true] {
        for first in [
            "not reproduced",
            "reproduced invalid single",
            "reproduced high invalid",
        ] {
            let text = format!(
                "\n<!-- thirdshift:security-reproduction -->\nOutcome: {first}\nOutcome: reproduced high single\n\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced critical spec\n"
            );
            assert!(record(private, &text).fix_size().is_err());
            let mut value = native(private, &draft("fixture"));
            value[if private { "body" } else { "description" }] = json!(text);
            assert!(
                collection(private, vec![value])
                    .next_fix()
                    .unwrap()
                    .is_none()
            );
        }
        let first = "\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced low single\n\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced critical spec\n";
        assert!(record(private, first).fix_size().unwrap() == FixSize::Single);
        for (last, paused) in [
            ("pending", true),
            ("failed", true),
            ("succeeded", false),
            ("unknown", false),
        ] {
            let text = format!(
                "{first}\n<!-- thirdshift:security-fix -->\nFix Ticket: https://github.com/acme/widgets/issues/8\nFix Run: failed\n\n<!-- thirdshift:security-fix -->\nFix Ticket: https://github.com/acme/widgets/issues/9\nFix Run: succeeded\nFix Run: {last}\nDay shift edit.\n"
            );
            let mut value = native(private, &draft("fixture"));
            value[if private { "body" } else { "description" }] = json!(text);
            let records = collection(private, vec![value]);
            assert!(records.next_fix().unwrap().is_none());
            assert_eq!(
                records.failed_fix().unwrap().map(|issue| issue.number),
                if paused { Some(9) } else { None }
            );
        }
        let text = format!("{first}\n<!-- thirdshift:security-fix -->\nFix Ticket: ");
        let mut value = native(private, &draft("fixture"));
        value[if private { "body" } else { "description" }] = json!(text);
        assert!(
            collection(private, vec![value])
                .next_fix()
                .unwrap()
                .is_none()
        );
    }
}
