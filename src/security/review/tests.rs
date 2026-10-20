//! Final-message cases vary at the review interface, without a Run or git.

use super::{Outcome, Review};

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
        let outcome = Outcome::of(result);
        let expected = "Security review incomplete: session failed, ended early or omitted a valid final line; see Session log";
        assert_eq!(outcome.hold().unwrap().to_string(), expected);
        assert_eq!(outcome.entry(), format!("- {expected}\n"));
    }
    let missing = Outcome::of(Review::from_final_message(None));
    assert!(missing.hold().is_some());
}
