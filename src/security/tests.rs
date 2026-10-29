use super::*;
use crate::pass::{Call, InMemory, widgets};

mod sequence;

#[test]
fn a_reproduced_fix_precedes_the_unchanged_base_but_yields_to_a_ready_issue() {
    for ready in [false, true] {
        let mut outside = InMemory::default().advisories(vec![
            serde_json::json!({"state":"draft", "severity":null}),
            serde_json::json!({
                "state":"draft", "severity":"critical", "ghsa_id":"GHSA-first",
                "summary":"Bound input", "html_url":"https://github.com/acme/widgets/security/advisories/GHSA-first",
                "description":"Private record\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced critical single\n"
            }),
            serde_json::json!({
                "state":"draft", "severity":"critical", "ghsa_id":"GHSA-second",
                "summary":"Other bound", "html_url":"https://github.com/acme/widgets/security/advisories/GHSA-second",
                "description":"Private record\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced critical single\n"
            }),
        ]).unchanged_base().dispatched_ending(crate::pass::ready_for_review());
        if ready {
            outside = outside.ready(7, false);
        }
        let outcome = run_through(&mut outside, &widgets(), "main", true, false, None);
        if ready {
            assert!(matches!(outcome, Outcome::Skipped(Skipped::ReadyIssue(_))));
            assert!(matches!(
                &outside.calls[..],
                [Call::ReadySearch, Call::Skipped(_)]
            ));
        } else {
            assert!(matches!(
                outcome,
                Outcome::Fixed {
                    ended: crate::run::Ended { outcome: Ok(_), .. },
                    ..
                }
            ));
            assert!(
                outside
                    .calls
                    .contains(&Call::PublishSecurityFix("GHSA-first".into()))
            );
            assert!(outside.calls.contains(&Call::DispatchSecurityFix {
                issue: 8,
                is_spec: false,
                base: "main".into()
            }));
            assert!(!outside.calls.contains(&Call::AuditHistory));
            assert!(!outside.calls.contains(&Call::NodeCheck));
        }
    }
}

#[test]
fn a_finding_waiting_for_the_day_shift_does_not_hold_off_the_audit() {
    let reproduced = "Private record\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced high single\n";
    for fixing in [false, true] {
        for private in [false, true] {
            // With fixing allowed, a reproduced finding is fixed instead.
            for description in ["Private record\n", reproduced] {
                if fixing && description == reproduced {
                    continue;
                }
                let mut outside = if private {
                    InMemory::default().finding_issues(vec![serde_json::json!({
                        "state": "open", "body": description,
                        "labels": [{"name": "security-finding"}, {"name": "needs-triage"}]
                    })])
                } else {
                    InMemory::default().advisories(vec![serde_json::json!({
                        "state": "draft", "description": description,
                        "severity": if description == reproduced { Some("high") } else { None }
                    })])
                };
                let outcome = run_through(&mut outside, &widgets(), "main", fixing, false, None);
                assert!(
                    matches!(outcome, Outcome::Audited(Ended { outcome: Ok(_), .. })),
                    "fixing {fixing}, private {private}: {description}"
                );
                assert_eq!(
                    outside.calls[..7],
                    [
                        Call::ReadySearch,
                        Call::AdvisoryList,
                        Call::AuditHistory,
                        Call::HarnessCheck,
                        Call::NodeCheck,
                        Call::Started(None),
                        Call::SecurityAudit("main".to_string())
                    ]
                );
            }
        }
    }
}

#[test]
fn checks_prerequisites_then_audits_without_pulling() {
    let mut outside = InMemory::default();
    let outcome = run_through(&mut outside, &widgets(), "main", false, false, None);
    assert!(matches!(
        outcome,
        Outcome::Audited(Ended { outcome: Ok(_), .. })
    ));
    assert_eq!(
        outside.calls,
        vec![
            Call::ReadySearch,
            Call::AdvisoryList,
            Call::AuditHistory,
            Call::HarnessCheck,
            Call::NodeCheck,
            Call::Started(None),
            Call::SecurityAudit("main".to_string())
        ]
    );
}
#[test]
fn a_ready_issue_precedes_every_prerequisite_and_audit() {
    let mut outside = InMemory::default()
        .ready(7, false)
        .advisories(vec![serde_json::json!({"state":"draft", "severity":null})])
        .unchanged_base()
        .node_failing()
        .harness_failing();
    assert!(matches!(
        run_through(&mut outside, &widgets(), "main", false, false, None),
        Outcome::Skipped(Skipped::ReadyIssue(_))
    ));
    assert!(matches!(
        &outside.calls[..],
        [Call::ReadySearch, Call::Skipped(_)]
    ));
}

#[test]
fn security_gates_choose_ready_then_failed_fix_then_unchanged_before_work() {
    let failed = "Private record\n<!-- thirdshift:security-fix -->\nFix Ticket: https://github.com/acme/widgets/issues/8\nFix Run: failed\n";
    for ready in [false, true] {
        for failed_fix in [false, true] {
            for unchanged in [false, true] {
                // A finding waiting for the Day shift is no gate.
                let mut advisories = vec![serde_json::json!({"state":"draft", "severity":null})];
                if failed_fix {
                    advisories.push(serde_json::json!({
                        "state": "draft", "severity": null, "description": failed,
                    }));
                }
                let mut outside = InMemory::default().advisories(advisories);
                if ready {
                    outside = outside.ready(7, false);
                }
                if unchanged {
                    outside = outside.unchanged_base();
                }
                let outcome = run_through(&mut outside, &widgets(), "main", false, false, None);
                let expected = if ready {
                    "Ready issue #7"
                } else if failed_fix {
                    "failed Security fix #8 is still open"
                } else if unchanged {
                    "Base branch main hasn't changed since the last completed Security audit"
                } else {
                    assert!(matches!(
                        outcome,
                        Outcome::Audited(Ended { outcome: Ok(_), .. })
                    ));
                    assert!(outside.calls.contains(&Call::SecurityAudit("main".into())));
                    continue;
                };
                let Outcome::Skipped(skipped) = outcome else {
                    panic!("Security run should skip");
                };
                assert!(skipped.to_string().starts_with(expected), "{skipped}");
                assert!(!outside.calls.contains(&Call::HarnessCheck));
                assert_eq!(outside.calls.contains(&Call::AdvisoryList), !ready);
                assert_eq!(
                    outside.calls.contains(&Call::AuditHistory),
                    !ready && !failed_fix
                );
            }
        }
    }
}

#[test]
fn a_failed_fix_yields_to_ready_work_but_precedes_audit_history() {
    for private in [false, true] {
        for ready in [false, true] {
            for closed in [false, true] {
                for fixing in [false, true] {
                    let body = "Private record\n<!-- thirdshift:security-fix -->\nFix Ticket: https://github.com/acme/widgets/issues/8\nFix Run: failed\n";
                    let values = vec![
                        serde_json::json!({
                            "state": if private { "open" } else { "draft" },
                            "severity": null, "description": body, "body": body,
                            "security_fix_closed": closed,
                        }),
                        serde_json::json!({
                            "state": if private { "open" } else { "draft" },
                            "severity": null, "labels": [{"name":"needs-triage"}],
                        }),
                    ];
                    let mut outside = if private {
                        InMemory::default().finding_issues(values)
                    } else {
                        InMemory::default().advisories(values)
                    }
                    .unchanged_base();
                    if ready {
                        outside = outside.ready(7, false);
                    }
                    let Outcome::Skipped(skipped) =
                        run_through(&mut outside, &widgets(), "main", fixing, false, None)
                    else {
                        panic!("Security run should skip");
                    };
                    let expected = if ready {
                        "Ready issue #7"
                    } else if !closed {
                        "failed Security fix #8 is still open"
                    } else {
                        "Base branch main hasn't changed"
                    };
                    assert!(skipped.to_string().starts_with(expected), "{skipped}");
                    assert!(!outside.calls.contains(&Call::HarnessCheck));
                    assert_eq!(
                        outside.calls.contains(&Call::AuditHistory),
                        !ready && closed
                    );
                    assert_eq!(outside.calls.contains(&Call::AdvisoryList), !ready);
                }
            }
        }
    }
}

#[test]
fn each_failed_prerequisite_prevents_later_operations() {
    for (mut outside, expected) in [
        (
            InMemory::default().harness_failing(),
            vec![
                Call::ReadySearch,
                Call::AdvisoryList,
                Call::AuditHistory,
                Call::HarnessCheck,
            ],
        ),
        (
            InMemory::default().node_failing(),
            vec![
                Call::ReadySearch,
                Call::AdvisoryList,
                Call::AuditHistory,
                Call::HarnessCheck,
                Call::NodeCheck,
            ],
        ),
    ] {
        assert!(matches!(
            run_through(&mut outside, &widgets(), "main", false, false, None),
            Outcome::Audited(Ended {
                outcome: Err(_),
                ..
            })
        ));
        assert_eq!(outside.calls, expected);
    }
}
