//! Final-message cases vary at the review interface, without a Run or git.

use super::{Outcome, Review};

const VALID_REPORT: &[u8] = br#"[{"fingerprint":"fp-1","title":"T","description":"D","proof_of_concept":{"test":"t","command":"cargo test","head_exit_code":1,"merge_base_exit_code":1,"notes":"n","severity":"high","fix_size":"single"}}]"#;
const TOP_LEVEL_REPORT: &[u8] = br#"[{"fingerprint":"fp-1","title":"T","description":"D","severity":"high","fix_size":"single","proof_of_concept":{"test":"t","command":"cargo test","head_exit_code":1,"merge_base_exit_code":1,"notes":"n"}}]"#;

#[test]
fn nested_severity_and_fix_size_parse_and_validate() {
    let findings = super::parse_report(VALID_REPORT).unwrap();
    assert_eq!(findings.len(), 1);
    assert!(findings[0].reproduction().is_ok());
}

#[test]
fn top_level_severity_names_the_offending_field_in_the_outcome() {
    let error = match super::parse_report(TOP_LEVEL_REPORT) {
        Ok(_) => panic!("top-level shape should fail strict parse"),
        Err(error) => error,
    };
    assert_eq!(
        format!("{error}"),
        "Security review's private report is invalid: unknown field `severity` at finding 1"
    );
    // No field value from the private report leaks into the message.
    assert!(!format!("{error}").contains("fp-1"));
    let outcome = Outcome::of(Err(error), None);
    let expected =
        "Security review's private report is invalid: unknown field `severity` at finding 1";
    assert_eq!(outcome.hold().unwrap().to_string(), expected);
    assert_eq!(outcome.entry(), format!("- {expected}\n"));
}

#[test]
fn top_level_fix_size_and_invalid_reports_name_the_cause() {
    for (report, expected) in [
        (
            br#"[{"fingerprint":"fp-1","title":"T","description":"D","fix_size":"single","proof_of_concept":{"test":"t","command":"c","head_exit_code":1,"merge_base_exit_code":1,"notes":"n","severity":"high"}}]"#.as_slice(),
            "Security review's private report is invalid: unknown field `fix_size` at finding 1",
        ),
        (b"not json".as_slice(), "Security review's private report is invalid: invalid JSON"),
        (
            br#"[{"fingerprint":"fp-1","title":"T","description":"D","proof_of_concept":{"test":"t","command":"c","head_exit_code":1,"merge_base_exit_code":1,"notes":"n","severity":"high"}}]"#.as_slice(),
            "Security review's private report is invalid: missing field `fix_size` at finding 1",
        ),
    ] {
        let error = match super::parse_report(report) {
            Ok(_) => panic!("report should be invalid: {expected}"),
            Err(error) => error,
        };
        assert_eq!(format!("{error}"), expected);
        let outcome = Outcome::of(Err(error), None);
        assert_eq!(outcome.hold().unwrap().to_string(), expected);
    }
}

#[test]
fn malformed_final_messages_produce_a_public_incomplete_hold() {
    for final_message in [
        "",
        "Review incomplete",
        "Security review: not JSON",
        "Security review: incomplete: ",
        "Security review: incomplete:   ",
        "Security review: incomplete: first line\nsecond line",
        "Security review: incomplete: first\rsecond",
        "Security review: {}",
        r#"Security review: {"unaddressed_count":0,"findings":["Cross-tenant read"],"pre_existing_count":0}"#,
        r#"Security review: {"unaddressed_count":1,"findings":[""],"pre_existing_count":0}"#,
        r#"Security review: {"unaddressed_count":0,"findings":[],"pre_existing_count":0}\nMore text"#,
    ] {
        let result = Review::from_final_message(Some(final_message));
        assert!(result.is_err(), "accepted {final_message:?}");
        let outcome = Outcome::of(result, None);
        let expected = "Security review incomplete: session failed, ended early or omitted a valid final line; see Session trace";
        assert_eq!(outcome.hold().unwrap().to_string(), expected);
        assert_eq!(outcome.entry(), format!("- {expected}\n"));
    }
    let missing = Outcome::of(Review::from_final_message(None), None);
    assert!(missing.hold().is_some());
}

#[test]
fn poc_first_top_level_severity_is_named_unknown() {
    let report = br#"[{"proof_of_concept":{"test":"t","command":"c","head_exit_code":1,"merge_base_exit_code":1,"notes":"n"},"fingerprint":"fp-1","title":"T","description":"D","severity":"high","fix_size":"single"}]"#;
    let error = match super::parse_report(report) {
        Ok(_) => panic!("top-level shape should fail"),
        Err(error) => error,
    };
    assert_eq!(
        format!("{error}"),
        "Security review's private report is invalid: unknown field `severity` at finding 1"
    );
}

#[test]
fn type_error_names_offending_field() {
    let report = br#"[{"fingerprint":"fp-1","title":"T","description":"D","proof_of_concept":{"test":"t","command":"c","head_exit_code":"1","merge_base_exit_code":1,"notes":"n","severity":"high","fix_size":"single"}}]"#;
    let error = match super::parse_report(report) {
        Ok(_) => panic!("type error should fail"),
        Err(error) => error,
    };
    assert!(format!("{error}").contains("`head_exit_code`"), "{error}");
}

#[test]
fn empty_title_validation_names_title_field() {
    let path = std::env::temp_dir().join(format!("ts-2298-{}.json", std::process::id()));
    std::fs::write(&path, br#"[{"fingerprint":"fp-1","title":"","description":"D","proof_of_concept":{"test":"t","command":"c","head_exit_code":1,"merge_base_exit_code":1,"notes":"n","severity":"high","fix_size":"single"}}]"#).unwrap();
    let review = super::Review::from_final_message(Some(
        r#"Security review: {"unaddressed_count":0,"findings":[],"pre_existing_count":1}"#,
    ));
    let error = match super::load_report(&path, &review) {
        Ok(_) => panic!("empty title should fail"),
        Err(error) => error,
    };
    std::fs::remove_file(&path).ok();
    assert!(format!("{error}").contains("`title`"), "{error}");
}
