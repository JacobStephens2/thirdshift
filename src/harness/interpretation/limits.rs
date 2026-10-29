//! Provider quota facts, independent of the caller's stop policy.

use serde_json::Value;

/// Eval quota stops share the same 85% threshold across Claude and Codex.
const PLAN_LIMIT_UTILIZATION: f64 = 0.85;

pub(super) fn claude(event: &Value) -> bool {
    if event["type"] != "rate_limit_event" {
        return false;
    }
    let info = &event["rate_limit_info"];
    match info["status"].as_str() {
        Some("rejected") => true,
        Some("allowed_warning") => {
            let utilization = info["unifiedWindows"]
                .as_object()
                .into_iter()
                .flat_map(|windows| windows.values())
                .filter_map(|window| window["utilization"].as_f64())
                .chain(info["utilization"].as_f64())
                .reduce(f64::max);
            utilization.is_none_or(|used| used >= PLAN_LIMIT_UTILIZATION)
        }
        _ => false,
    }
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
                .is_some_and(|used| used >= PLAN_LIMIT_UTILIZATION * 100.0)
        })
}
