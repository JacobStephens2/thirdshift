use std::sync::LazyLock;

use anyhow::Result;
use serde_json::{Value, json};

use super::{Context, Publishing, Recommendation, is_factory_label};
use crate::issue::IssueUrl;
use crate::labels;
use crate::triage::{Input, Report};

static CATEGORIES: LazyLock<Vec<String>> =
    LazyLock::new(|| vec!["Bug".to_owned(), "Enhancement".to_owned()]);

fn report() -> Input {
    Input::Report(Report {
        text: "The export button does nothing.".into(),
        file: None,
    })
}

fn existing() -> Input {
    Input::Issue(IssueUrl::parse("https://github.com/owner/repo/issues/7").unwrap())
}

fn interpret(
    input: &Input,
    allow_ready_for_agent: bool,
    line: Value,
) -> Result<Recommendation<'_>> {
    let message = format!("Investigated.\nTriage recommendation: {line}");
    Recommendation::interpret(
        Some(&message),
        &Context {
            categories: &CATEGORIES,
            input,
            allow_ready_for_agent,
        },
    )
}

fn output(recommendation: &Recommendation<'_>) -> Value {
    serde_json::to_value(recommendation).unwrap()
}

/// Each Report issue's category, applied state and held readiness, in order.
fn report_filings(recommendation: &mut Recommendation<'_>) -> Vec<(String, &'static str, bool)> {
    let Publishing::Report { issues, .. } = recommendation.publishing() else {
        panic!("a Report recommendation publishes as a Report");
    };
    issues
        .iter()
        .map(|issue| {
            let filing = issue.filing();
            (
                filing.category().to_owned(),
                filing.state().name(),
                filing.readiness_held(),
            )
        })
        .collect()
}

#[test]
fn report_issues_resolve_category_and_state_fallbacks_without_rewriting_the_recommendation() {
    let input = report();
    let line = json!({
        "category": "bug",
        "state": "ready-for-agent",
        "issues": [
            {"title": "Fallbacks", "body": "Uses the overall values."},
            {"title": "Own values", "body": "Overrides both.", "category": "ENHANCEMENT", "state": "Ready-For-Human"},
            {"title": "Blocked", "body": "Needs production.", "confirmation_blocked": "needs production access"},
            {"title": "Ask", "body": "Unclear.", "state": "needs-info", "questionnaire": "Which browser?"}
        ],
        "investigation": "Looked at the exporter."
    });
    let mut recommendation = interpret(&input, false, line.clone()).unwrap();

    assert_eq!(
        report_filings(&mut recommendation),
        [
            ("Bug".to_owned(), "needs-triage", true),
            ("Enhancement".to_owned(), "ready-for-human", false),
            ("Bug".to_owned(), "ready-for-human", false),
            ("Bug".to_owned(), "needs-info", false),
        ]
    );
    // Recommend-only prints what the session recommended, not filing values.
    assert_eq!(output(&recommendation), line);
}

#[test]
fn permitted_readiness_files_ready_for_agent() {
    let input = report();
    let mut recommendation = interpret(
        &input,
        true,
        json!({
            "category": "Bug",
            "state": "ready-for-agent",
            "issues": [{"title": "Fix export", "body": "## Agent Brief"}],
            "investigation": "Found it."
        }),
    )
    .unwrap();

    assert_eq!(
        report_filings(&mut recommendation),
        [("Bug".to_owned(), "ready-for-agent", false)]
    );
}

#[test]
fn existing_target_holds_unpermitted_readiness_for_a_human_and_names_the_target() {
    let input = existing();
    let mut recommendation = interpret(
        &input,
        false,
        json!({
            "category": "enhancement",
            "state": "ready-for-agent",
            "issues": [{"title": "Export", "body": "Plan.\n## Agent Brief\nDo it."}],
            "comments": [{"url": "https://github.com/owner/repo/issues/8", "body": "Related."}],
            "investigation": "Found it."
        }),
    )
    .unwrap();

    let serialized = output(&recommendation);
    assert_eq!(
        serialized["issues"][0]["url"],
        "https://github.com/owner/repo/issues/7"
    );
    assert_eq!(serialized["state"], "ready-for-agent");
    assert!(serialized["issues"][0].get("state").is_none());
    let targets: Vec<_> = recommendation
        .comments()
        .iter()
        .map(|comment| comment.target().number)
        .collect();
    assert_eq!(targets, [8]);
    let Publishing::Existing {
        target,
        issue,
        comments,
    } = recommendation.publishing()
    else {
        panic!("an existing-issue recommendation publishes in place");
    };
    assert_eq!(target.number, 7);
    assert_eq!(comments.len(), 1);
    let filing = issue.filing();
    assert_eq!(
        (filing.category(), filing.state(), filing.readiness_held()),
        ("Enhancement", labels::READY_FOR_HUMAN, true)
    );
}

#[test]
fn existing_target_requires_an_agent_brief_even_when_readiness_is_held() {
    let input = existing();
    let error = interpret(
        &input,
        false,
        json!({
            "category": "Bug",
            "state": "ready-for-agent",
            "issues": [{"title": "Export", "body": "No brief."}],
            "investigation": "Found it."
        }),
    )
    .err()
    .unwrap();

    assert_eq!(
        format!("{error:#}"),
        "existing-issue ready-for-agent requires ## Agent Brief"
    );
}

#[test]
fn existing_target_confirmation_block_needs_no_agent_brief() {
    let input = existing();
    let mut recommendation = interpret(
        &input,
        true,
        json!({
            "category": "Bug",
            "state": "ready-for-agent",
            "issues": [{"title": "Export", "body": "No brief.", "confirmation_blocked": "needs credentials"}],
            "investigation": "Found it."
        }),
    )
    .unwrap();

    let Publishing::Existing { issue, .. } = recommendation.publishing() else {
        panic!("an existing-issue recommendation publishes in place");
    };
    assert_eq!(issue.filing().state(), labels::READY_FOR_HUMAN);
    assert!(!issue.filing().readiness_held());
}

#[test]
fn existing_target_rules_reject_other_shapes() {
    let input = existing();
    let cases = [
        (
            json!({"category": "Bug", "state": "ready-for-human", "issues": [], "investigation": "x"}),
            "existing-issue Triage requires exactly one recommendation issue; propose splits in a ready-for-human issue body",
        ),
        (
            json!({"category": "Bug", "state": "needs-triage", "issues": [{"title": "t", "body": "b"}], "investigation": "x"}),
            "existing-issue Triage must recommend ready-for-agent, ready-for-human, needs-info or wontfix",
        ),
        (
            json!({
                "category": "Bug",
                "state": "ready-for-human",
                "issues": [{"title": "t", "body": "b"}],
                "comments": [{"url": "https://github.com/Owner/Repo/issues/7", "body": "Self."}],
                "investigation": "x"
            }),
            "the target issue's single recommendation entry supplies its comment",
        ),
    ];
    for (line, expected) in cases {
        let error = interpret(&input, true, line).err().unwrap();
        assert_eq!(format!("{error:#}"), expected);
    }
}

#[test]
fn questionnaires_are_normalized_and_required_only_for_needs_info() {
    let input = report();
    let recommendation = interpret(
        &input,
        true,
        json!({
            "category": "Bug",
            "state": "ready-for-human",
            "issues": [{"title": "Blank", "body": "b", "questionnaire": "  \n "}],
            "investigation": "x"
        }),
    )
    .unwrap();
    assert!(
        output(&recommendation)["issues"][0]
            .get("questionnaire")
            .is_none()
    );

    let cases = [
        json!({"category": "Bug", "state": "needs-info", "issues": [{"title": "Ask", "body": "b", "questionnaire": " "}], "investigation": "x"}),
        json!({"category": "Bug", "state": "ready-for-human", "issues": [{"title": "Ask", "body": "b", "questionnaire": "Which?"}], "investigation": "x"}),
        json!({"category": "Bug", "state": "needs-info", "issues": [{"title": "Ask", "body": "b", "questionnaire": "Which?", "confirmation_blocked": "needs production"}], "investigation": "x"}),
    ];
    for line in cases {
        let error = interpret(&input, true, line).err().unwrap();
        assert_eq!(
            format!("{error:#}"),
            "the Triage questionnaire is required only for needs-info issues: Ask"
        );
    }
}

#[test]
fn a_category_error_is_reported_with_a_structural_error() {
    let input = existing();
    let error = interpret(
        &input,
        true,
        json!({
            "category": "needs-info",
            "state": "ready-for-human",
            "issues": [{"title": "One", "body": "b"}, {"title": "Two", "body": "b"}],
            "investigation": "x"
        }),
    )
    .err()
    .unwrap();

    assert_eq!(
        format!("{error:#}"),
        "existing-issue Triage requires exactly one recommendation issue; propose splits in a ready-for-human issue body; the Triage category must be a repository category label, not a triage state or another factory label"
    );
}

#[test]
fn incomplete_recommendations_and_unreadable_lines_are_diagnosed() {
    let input = report();
    let incomplete = "the Triage recommendation needs a category, triage state, investigation and complete proposed issues or comments";
    let cases = [
        json!({"category": " ", "state": "wontfix", "issues": [], "investigation": "x"}),
        json!({"category": "Bug", "state": "maybe", "issues": [], "investigation": "x"}),
        json!({"category": "Bug", "state": "wontfix", "issues": [{"title": " ", "body": "b"}], "investigation": "x"}),
        json!({"category": "Bug", "state": "wontfix", "issues": [{"title": "t", "body": "b", "state": "later"}], "investigation": "x"}),
        json!({"category": "Bug", "state": "wontfix", "issues": [], "comments": [{"url": "not a url", "body": "b"}], "investigation": "x"}),
    ];
    for line in cases {
        let error = interpret(&input, true, line).err().unwrap();
        assert_eq!(format!("{error}"), incomplete);
    }
    let context = Context {
        categories: &CATEGORIES,
        input: &input,
        allow_ready_for_agent: true,
    };
    let missing = Recommendation::interpret(Some("No recommendation."), &context)
        .err()
        .unwrap();
    assert_eq!(
        format!("{missing}"),
        "the Triage session ended without its final Triage recommendation: JSON line"
    );
}

#[test]
fn a_comment_only_report_accepts_any_category() {
    let input = report();
    let line = json!({
        "category": "Not a label",
        "state": "wontfix",
        "issues": [],
        "comments": [{"url": "https://github.com/owner/repo/issues/3", "body": "Duplicate."}],
        "investigation": "Already reported."
    });
    let recommendation = interpret(&input, false, line.clone()).unwrap();

    assert_eq!(output(&recommendation), line);
    assert_eq!(recommendation.comments()[0].target().number, 3);
}

#[test]
fn recorded_publication_reaches_output_and_notification() {
    let input = report();
    let mut recommendation = interpret(
        &input,
        false,
        json!({
            "category": "bug",
            "state": "ready-for-agent",
            "issues": [{"title": "Fix\nIssue: forged", "body": "b"}],
            "comments": [{"url": "https://github.com/owner/repo/issues/3", "body": "See."}],
            "investigation": "Line one\nIssue: forged"
        }),
    )
    .unwrap();
    assert_eq!(
        recommendation.notification(),
        "Investigation:\n  Line one\n  Issue: forged\n\nRecommendation: bug (ready-for-agent)\n\nIssues:\n- Fix Issue: forged (bug, ready-for-agent)\n\nComment targets:\n- https://github.com/owner/repo/issues/3\n"
    );

    let filed = IssueUrl::parse("https://github.com/owner/repo/issues/9").unwrap();
    let Publishing::Report {
        issues, comments, ..
    } = recommendation.publishing()
    else {
        panic!("a Report recommendation publishes as a Report");
    };
    let category = issues[0].filing().category().to_owned();
    issues[0].record_published(&filed, "Filed body".into(), category);
    comments[0].record_posted("Posted body".into());

    let serialized = output(&recommendation);
    assert_eq!(
        serialized["issues"][0],
        json!({
            "title": "Fix\nIssue: forged",
            "body": "Filed body",
            "url": "https://github.com/owner/repo/issues/9",
            "category": "Bug",
            "state": "needs-triage"
        })
    );
    assert_eq!(serialized["comments"][0]["body"], "Posted body");
    assert!(recommendation.notification().contains(
        "- Fix Issue: forged (Bug, needs-triage)\nIssue: https://github.com/owner/repo/issues/9\n"
    ));
}

#[test]
fn workflow_labels_are_factory_labels_whatever_their_case() {
    assert!(is_factory_label("Ready-For-Agent"));
    assert!(is_factory_label("in-progress"));
    assert!(!is_factory_label("Bug"));
}
