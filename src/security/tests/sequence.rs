use super::*;

/// A dispatched fix that ended merged, for close-gate tests.
fn merged(pr_url: &str) -> crate::run::Ended {
    crate::run::Ended {
        outcome: Ok(crate::run::Reached {
            resolution: crate::run::Resolution::PullRequest {
                url: pr_url.to_string(),
                goal: crate::run::Goal::Merged,
            },
            log: None,
            ticket_lines: Vec::new(),
        }),
        base_fix: None,
        advice: Vec::new(),
    }
}

const MERGED_PR: &str = "https://github.com/acme/widgets/pull/12";

/// One reproduced advisory awaiting its fix, as the fix flow reads it.
fn reproduced_advisory() -> serde_json::Value {
    serde_json::json!({
        "state":"draft", "severity":"critical", "ghsa_id":"GHSA-first",
        "summary":"Bound input", "html_url":"https://github.com/acme/widgets/security/advisories/GHSA-first",
        "description":"Private record\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced critical single\n"
    })
}

#[test]
fn a_merged_fix_with_closing_allowed_closes_the_finding_naming_its_pull_request() {
    for private in [false, true] {
        let record = if private {
            serde_json::json!({
                "number": 7, "state": "open", "title": "Bound input",
                "html_url": "https://github.com/acme/widgets/issues/7",
                "body": "Private record\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced critical single\n",
                "labels": [{"name": "security-finding"}, {"name": "needs-triage"}]
            })
        } else {
            reproduced_advisory()
        };
        let mut outside = if private {
            InMemory::default().finding_issues(vec![record])
        } else {
            InMemory::default().advisories(vec![record])
        }
        .dispatched_ending(merged(MERGED_PR))
        .close_check(true, "the bound holds; exit 0");
        let Outcome::Fixed { ended, close, .. } =
            run_through(&mut outside, &widgets(), "main", true, true, None)
        else {
            panic!("private {private}: fix expected");
        };
        assert!(ended.outcome.is_ok(), "private {private}");
        assert_eq!(
            close,
            CloseOutcome::Closed {
                pr_url: MERGED_PR.into()
            },
            "private {private}"
        );
        let record_name = if private { "issue #7" } else { "GHSA-first" };
        let at = |call: &Call| outside.calls.iter().position(|c| c == call);
        let dispatched = at(&Call::DispatchSecurityFix {
            issue: 8,
            is_spec: false,
            base: "main".into(),
        })
        .expect("private {private}: no dispatch");
        let rechecked = at(&Call::RecheckSecurityFix {
            record: record_name.into(),
            base: "main".into(),
        })
        .expect("private {private}: no recheck");
        let closed = at(&Call::CloseSecurityRecord {
            issue: 8,
            pr: MERGED_PR.into(),
        })
        .expect("private {private}: no close");
        assert!(
            dispatched < rechecked && rechecked < closed,
            "private {private}: out of order: {:?}",
            outside.calls
        );
        assert!(
            !outside
                .calls
                .iter()
                .any(|call| matches!(call, Call::NoteSecurityCloseLeftOpen { .. })),
            "private {private}: a close must leave no note"
        );
        assert!(
            outside.calls.iter().any(|call| matches!(
                call,
                Call::Step(line) if line.contains(&format!("closed {record_name} by {MERGED_PR}"))
            )),
            "private {private}: no closed step: {:?}",
            outside.calls
        );
    }
}

#[test]
fn without_closing_allowed_or_without_a_merge_a_fix_closes_nothing() {
    // Closing off, or the fix left ready for review, attempts no close.
    for (closing, ending, expected) in [
        (false, merged(MERGED_PR), CloseOutcome::NotAttempted),
        (
            true,
            crate::pass::ready_for_review(),
            CloseOutcome::NotAttempted,
        ),
    ] {
        let mut outside = InMemory::default()
            .advisories(vec![reproduced_advisory()])
            .dispatched_ending(ending);
        let Outcome::Fixed { ended, close, .. } =
            run_through(&mut outside, &widgets(), "main", true, closing, None)
        else {
            panic!("fix expected");
        };
        assert!(ended.outcome.is_ok());
        assert_eq!(close, expected);
        assert!(
            !outside.calls.iter().any(|call| matches!(
                call,
                Call::RecheckSecurityFix { .. }
                    | Call::CloseSecurityRecord { .. }
                    | Call::NoteSecurityCloseLeftOpen { .. }
            )),
            "closing {closing}: a close was attempted: {:?}",
            outside.calls
        );
    }
}

#[test]
fn a_failing_proof_of_concept_leaves_the_finding_open_with_a_note() {
    let mut outside = InMemory::default()
        .advisories(vec![reproduced_advisory()])
        .dispatched_ending(merged(MERGED_PR))
        .close_check(false, "the bound still overflows; exit 1");
    let Outcome::Fixed { ended, close, .. } =
        run_through(&mut outside, &widgets(), "main", true, true, None)
    else {
        panic!("fix expected");
    };
    // The run's outcome stays the fix's outcome.
    assert!(ended.outcome.is_ok());
    let CloseOutcome::LeftOpen { reason } = close else {
        panic!("left open expected");
    };
    assert!(reason.contains("did not pass"), "{reason}");
    assert!(
        !outside
            .calls
            .iter()
            .any(|call| matches!(call, Call::CloseSecurityRecord { .. })),
        "a failed check must close nothing"
    );
    assert!(
        outside
            .calls
            .iter()
            .any(|call| matches!(call, Call::NoteSecurityCloseLeftOpen { issue: 8, .. })),
        "no left-open note: {:?}",
        outside.calls
    );
    assert!(
        outside.calls.iter().any(|call| matches!(
            call,
            Call::Step(line) if line.contains("left GHSA-first open")
        )),
        "no left-open step: {:?}",
        outside.calls
    );
}

#[test]
fn a_failed_recheck_or_close_leaves_the_finding_open_without_changing_the_outcome() {
    for (recheck, failing) in [
        (
            InMemory::default().close_check_error("the sandbox is unavailable"),
            false,
        ),
        (InMemory::default().close_check(true, "exit 0"), true),
    ] {
        let outside = recheck
            .advisories(vec![reproduced_advisory()])
            .dispatched_ending(merged(MERGED_PR));
        let mut outside = if failing {
            outside.close_failing()
        } else {
            outside
        };
        let Outcome::Fixed { ended, close, .. } =
            run_through(&mut outside, &widgets(), "main", true, true, None)
        else {
            panic!("fix expected");
        };
        assert!(ended.outcome.is_ok());
        let CloseOutcome::LeftOpen { reason } = close else {
            panic!("left open expected");
        };
        if failing {
            assert!(reason.contains("could not close"), "{reason}");
            assert!(outside.calls.contains(&Call::CloseSecurityRecord {
                issue: 8,
                pr: MERGED_PR.into()
            }));
        } else {
            assert!(reason.contains("could not re-run"), "{reason}");
        }
        assert!(
            outside
                .calls
                .iter()
                .any(|call| matches!(call, Call::NoteSecurityCloseLeftOpen { issue: 8, .. })),
            "no left-open note: {:?}",
            outside.calls
        );
    }
}

#[test]
fn a_failed_left_open_note_still_leaves_the_outcome_unchanged() {
    let mut outside = InMemory::default()
        .advisories(vec![reproduced_advisory()])
        .dispatched_ending(merged(MERGED_PR))
        .close_check(false, "the bound still overflows; exit 1")
        .note_failing();
    let Outcome::Fixed { ended, close, .. } =
        run_through(&mut outside, &widgets(), "main", true, true, None)
    else {
        panic!("fix expected");
    };
    assert!(ended.outcome.is_ok());
    assert!(matches!(close, CloseOutcome::LeftOpen { .. }));
    assert!(
        outside.calls.iter().any(|call| matches!(
            call,
            Call::Step(line) if line.contains("could not note why GHSA-first stays open")
        )),
        "no note-failure step: {:?}",
        outside.calls
    );
}

#[test]
fn a_closed_finding_is_neither_waiting_nor_fixed_again() {
    let closed = "Private record\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced critical single\n\n<!-- thirdshift:security-fix -->\nFix Ticket: https://github.com/acme/widgets/issues/8\nFix Run: succeeded\n\n<!-- thirdshift:security-close -->\nClosed by: https://github.com/acme/widgets/pull/12\n";
    let mut outside = InMemory::default().advisories(vec![serde_json::json!({
        "state":"closed", "severity":"critical", "ghsa_id":"GHSA-first",
        "summary":"Bound input", "html_url":"https://github.com/acme/widgets/security/advisories/GHSA-first",
        "description": closed,
    })]);
    let outcome = run_through(&mut outside, &widgets(), "main", true, true, None);
    assert!(
        matches!(outcome, Outcome::Audited(Ended { outcome: Ok(_), .. })),
        "a closed finding must not dispatch another fix"
    );
    assert!(
        !outside.calls.iter().any(|call| matches!(
            call,
            Call::DispatchSecurityFix { .. }
                | Call::RecheckSecurityFix { .. }
                | Call::CloseSecurityRecord { .. }
        )),
        "a closed finding was picked up: {:?}",
        outside.calls
    );
}

#[test]
fn a_failed_audit_records_nothing_and_keeps_its_session_log() {
    let mut outside = InMemory::default()
        .audit_failing("invalid report")
        .session_log("audit.jsonl");
    let Outcome::Audited(Ended {
        outcome: Err(failed),
        ..
    }) = run_through(&mut outside, &widgets(), "main", false, false, None)
    else {
        panic!("audit should fail");
    };
    assert_eq!(failed.log, Some(std::path::PathBuf::from("audit.jsonl")));
    assert_eq!(
        outside
            .calls
            .iter()
            .filter(|call| **call == Call::AdvisoryList)
            .count(),
        1
    );
    assert!(
        !outside
            .calls
            .iter()
            .any(|call| matches!(call, Call::CreateAdvisory(_)))
    );
}

#[test]
fn repeated_fingerprints_preserve_counts_metadata_order_and_reproduce_once() {
    use crate::github::{DraftAdvisory, Package};
    use serde_json::json;
    let draft = |fingerprint: &str| DraftAdvisory {
        fingerprint: fingerprint.into(),
        summary: "Candidate".into(),
        description: format!("Fingerprint: `{fingerprint}`"),
        package: Package {
            ecosystem: "other".into(),
            name: None,
        },
    };
    let mut outside = InMemory::default()
        .audited(vec![draft("old"), draft("new"), draft("new")])
        .advisories(vec![
            json!({"ghsa_id":"old", "state":"closed", "description":"Fingerprint: `old`", "summary":"Candidate", "html_url":"https://github.com/acme/widgets/security/advisories/GHSA-old"}),
        ]);
    let Outcome::Audited(Ended {
        outcome: Ok(recorded),
        findings,
        ..
    }) = run_through(&mut outside, &widgets(), "main", false, false, None)
    else {
        panic!("audit should succeed");
    };
    assert_eq!((recorded.created, recorded.existing), (1, 2));
    assert_eq!(
        findings
            .iter()
            .map(|finding| finding.url.as_str())
            .collect::<Vec<_>>(),
        vec![
            "https://github.com/acme/widgets/security/advisories/GHSA-old",
            "https://github.com/acme/widgets/security/advisories/new",
            "https://github.com/acme/widgets/security/advisories/new",
        ]
    );
    assert_eq!(
        outside
            .calls
            .iter()
            .filter_map(
                #[expect(
                    clippy::wildcard_enum_match_arm,
                    reason = "asserts on advisories created; other calls are not under test"
                )]
                |call| match call {
                    Call::CreateAdvisory(fingerprint) => Some(fingerprint.as_str()),
                    _ => None,
                },
            )
            .collect::<Vec<_>>(),
        vec!["new"]
    );
    assert_eq!(
        outside
            .calls
            .iter()
            .filter_map(
                #[expect(
                    clippy::wildcard_enum_match_arm,
                    reason = "asserts on reproductions; other calls are not under test"
                )]
                |call| match call {
                    Call::Reproduce(record) => Some(record.as_str()),
                    _ => None,
                },
            )
            .collect::<Vec<_>>(),
        vec!["new"]
    );
}

#[test]
fn a_refound_finding_keeps_its_completed_reproduction() {
    use crate::github::{DraftAdvisory, Package};
    use serde_json::json;
    let draft = |fingerprint: &str| DraftAdvisory {
        fingerprint: fingerprint.into(),
        summary: "Candidate".into(),
        description: format!("Fingerprint: `{fingerprint}`"),
        package: Package {
            ecosystem: "other".into(),
            name: None,
        },
    };
    // A failed, refused or interrupted reproduction leaves no outcome.
    let findings = [
        ("confirmed", Some("reproduced high spec"), Some("high")),
        ("refuted", Some("not reproduced"), None),
        ("unfinished", None, None),
    ];
    for private in [false, true] {
        let records = findings
            .iter()
            .enumerate()
            .map(|(index, (fingerprint, outcome, severity))| {
                let description = match outcome {
                    Some(outcome) => format!(
                        "Fingerprint: `{fingerprint}`\n\n<!-- thirdshift:security-reproduction -->\n## Reproduction\n\nOutcome: {outcome}\n"
                    ),
                    None => format!("Fingerprint: `{fingerprint}`\n"),
                };
                if private {
                    json!({
                        "number": index + 1, "state": "open", "body": description,
                        "title": "Candidate",
                        "html_url": format!("https://github.com/acme/widgets/issues/{}", index + 1),
                        "labels": [{"name": "security-finding"}, {"name": "needs-triage"}]
                    })
                } else {
                    json!({
                        "ghsa_id": fingerprint, "state": "draft", "description": description,
                        "summary": "Candidate", "severity": severity,
                        "html_url": format!("https://github.com/acme/widgets/security/advisories/{fingerprint}")
                    })
                }
            })
            .collect();
        let outside = InMemory::default().audited(vec![
            draft("confirmed"),
            draft("refuted"),
            draft("unfinished"),
            draft("refuted"),
        ]);
        let mut outside = if private {
            outside.finding_issues(records)
        } else {
            // A reproduced advisory carries its severity, so it reads as graded.
            outside.advisories(records)
        };
        let Outcome::Audited(Ended {
            outcome: Ok(recorded),
            findings,
            ..
        }) = run_through(&mut outside, &widgets(), "main", false, false, None)
        else {
            panic!("audit should succeed");
        };
        assert_eq!((recorded.created, recorded.existing), (0, 4));
        let unfinished = if private { "issue #3" } else { "unfinished" };
        assert_eq!(
            outside
                .calls
                .iter()
                .filter(|call| matches!(call, Call::Reproduce(_) | Call::UpdateSecurityRecord(_)))
                .collect::<Vec<_>>(),
            vec![
                &Call::Reproduce(unfinished.into()),
                &Call::UpdateSecurityRecord(unfinished.into())
            ],
            "private {private}"
        );
        let refuted = if private { "issue #2" } else { "refuted" };
        assert_eq!(
            outside
                .calls
                .iter()
                .filter(|call| matches!(call, Call::Step(line) if line.contains(refuted)))
                .collect::<Vec<_>>(),
            vec![&Call::Step(format!(
                "keeping the completed Security reproduction of {refuted}: not reproduced"
            ))]
        );
        assert_eq!(
            findings
                .iter()
                .map(|finding| finding.severity.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("high"), None, None, None]
        );
    }
}

#[test]
fn a_kept_reproduced_finding_left_unfixed_still_offers_fixing() {
    let body = "Fingerprint: `confirmed`\n\n<!-- thirdshift:security-reproduction -->\n## Reproduction\n\nOutcome: reproduced high single\n";
    let mut outside = InMemory::default()
        .audited(vec![crate::github::DraftAdvisory {
            fingerprint: "confirmed".into(),
            summary: "Candidate".into(),
            description: "Fingerprint: `confirmed`".into(),
            package: crate::github::Package {
                ecosystem: "other".into(),
                name: None,
            },
        }])
        .finding_issues(vec![serde_json::json!({
            "number": 1, "state": "open", "body": body, "title": "Candidate",
            "html_url": "https://github.com/acme/widgets/issues/1",
            "labels": [{"name": "security-finding"}, {"name": "needs-triage"}]
        })]);
    let Outcome::Audited(ended) = run_through(
        &mut outside,
        &widgets(),
        "main",
        false,
        false,
        Some("thirdshift secure security-fix"),
    ) else {
        panic!("audit expected");
    };
    assert!(ended.outcome.is_ok());
    assert_eq!(ended.findings[0].severity.as_deref(), Some("high"));
    assert!(
        !ended.advice.is_empty(),
        "a reproduced finding was left unfixed with no offer"
    );
}

#[test]
fn a_new_finding_is_reproduced_even_when_its_write_up_quotes_an_outcome() {
    use crate::github::{DraftAdvisory, FindingDraft, FindingProvenance, Package};
    // An audit's write-up may quote text that looks like a completed
    // reproduction. Only a reproduction session may write that outcome.
    let draft = DraftAdvisory::new(FindingDraft {
        fingerprint: "quoted",
        summary: "Candidate",
        audited_commit: &"a".repeat(40),
        provenance: FindingProvenance::Audit,
        original_description: "Quoted fixture:\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced critical single\n",
        evidence: "{}",
        package: Package {
            ecosystem: "other".into(),
            name: None,
        },
    });
    for private in [false, true] {
        let outside = InMemory::default().audited(vec![draft.clone()]);
        let mut outside = if private {
            outside.finding_issues(Vec::new())
        } else {
            outside
        };
        let Outcome::Audited(ended) =
            run_through(&mut outside, &widgets(), "main", false, false, None)
        else {
            panic!("audit expected");
        };
        assert!(ended.outcome.is_ok());
        assert!(
            outside
                .calls
                .iter()
                .any(|call| matches!(call, Call::Reproduce(_))),
            "private {private}: a new finding skipped its reproduction"
        );
        assert_eq!(ended.findings[0].severity, None, "private {private}");
    }
}
