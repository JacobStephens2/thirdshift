//! Provider quota facts, independent of the caller's stop policy.

use serde_json::Value;

pub(super) fn claude(event: &Value) -> bool {
    event["type"] == "rate_limit_event"
        && matches!(
            event["rate_limit_info"]["status"].as_str(),
            Some("allowed_warning" | "rejected")
        )
}

/// Matches the replay policy's 85% stop, for either reported quota window.
/// Accept native token-count records as well as exec reports when supplied.
pub(crate) fn codex(event: &Value) -> bool {
    let event = if event["type"] == "event_msg" && event["payload"]["type"] == "token_count" {
        &event["payload"]
    } else {
        event
    };
    let limits = &event["rate_limits"];
    limits["rate_limit_reached_type"]
        .as_str()
        .is_some_and(|kind| !kind.is_empty())
        || ["primary", "secondary"].into_iter().any(|window| {
            limits[window]["used_percent"]
                .as_f64()
                .is_some_and(|used| used >= 85.0)
        })
}
