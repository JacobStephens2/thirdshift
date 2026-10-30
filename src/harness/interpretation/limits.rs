//! Provider quota facts, independent of the caller's stop policy.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The window that stopped work. Unknown facts remain explicit in Command JSON.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Limit {
    pub harness: Option<String>,
    pub window: Option<String>,
    pub utilization: Option<f64>,
    pub resets_at: Option<String>,
}

impl Limit {
    fn reported(
        harness: &str,
        window: Option<&str>,
        utilization: Option<f64>,
        reset: &Value,
    ) -> Self {
        let resets_at = reset
            .as_i64()
            .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
            .or_else(|| {
                reset
                    .as_str()
                    .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
                    .map(|time| time.to_utc())
            })
            .map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        Self {
            harness: Some(harness.into()),
            window: window.map(str::to_owned),
            utilization,
            resets_at,
        }
        .accepted()
    }

    /// Keep only facts safe to store and print. A sandboxed Trial can write
    /// the Codex session file this parses, and the plan-limit signal repeats
    /// those strings into the operator's terminal and Command record.
    pub(crate) fn accepted(mut self) -> Self {
        if !matches!(self.harness.as_deref(), Some("claude" | "codex")) {
            self.harness = None;
        }
        let window = self
            .window
            .as_deref()
            .and_then(operator_token)
            .map(str::to_owned);
        self.window = window;
        let resets_at = self
            .resets_at
            .as_deref()
            .filter(|text| chrono::DateTime::parse_from_rfc3339(text).is_ok())
            .map(str::to_owned);
        self.resets_at = resets_at;
        if self.utilization.is_some_and(|used| !used.is_finite()) {
            self.utilization = None;
        }
        self
    }
}

/// Window names from the providers are short tokens such as `seven_day`.
pub(crate) fn operator_token(value: &str) -> Option<&str> {
    if (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        Some(value)
    } else {
        None
    }
}

/// Eval quota stops share the same 85% threshold across Claude and Codex.
const PLAN_LIMIT_UTILIZATION: f64 = 0.85;

pub(super) fn claude(event: &Value) -> Option<Limit> {
    if event["type"] != "rate_limit_event" {
        return None;
    }
    let info = &event["rate_limit_info"];
    if !matches!(
        info["status"].as_str(),
        Some("rejected" | "allowed_warning")
    ) {
        return None;
    }
    let mut selected = (info["rateLimitType"].as_str(), info);
    let named_rejection = info["status"] == "rejected" && selected.0.is_some();
    for (name, window) in info["unifiedWindows"].as_object().into_iter().flatten() {
        if !named_rejection
            && window["utilization"].as_f64().is_some_and(|used| {
                selected.1["utilization"]
                    .as_f64()
                    .is_none_or(|current| used > current)
            })
        {
            selected = (Some(name), window);
        }
    }
    let utilization = selected.1["utilization"].as_f64().or_else(|| {
        selected
            .0
            .and_then(|name| info["unifiedWindows"][name]["utilization"].as_f64())
    });
    if info["status"] != "rejected" && utilization.is_some_and(|used| used < PLAN_LIMIT_UTILIZATION)
    {
        return None;
    }
    // Matching top-level and unified facts can each supply the missing reset.
    let reset = if selected.1["resetsAt"].is_null() && selected.0 == info["rateLimitType"].as_str()
    {
        &info["resetsAt"]
    } else {
        &selected.1["resetsAt"]
    };
    let reset = if reset.is_null() {
        selected
            .0
            .map(|name| &info["unifiedWindows"][name]["resetsAt"])
            .unwrap_or(&Value::Null)
    } else {
        reset
    };
    Some(Limit::reported("claude", selected.0, utilization, reset))
}

/// Matches the replay policy's 85% stop, for either reported quota window.
/// Accept native token-count records as well as exec reports when supplied.
pub(crate) fn codex(event: &Value) -> Option<Limit> {
    let event = if event["type"] == "event_msg" && event["payload"]["type"] == "token_count" {
        &event["payload"]
    } else {
        event
    };
    let limits = &event["rate_limits"];
    let reached = limits["rate_limit_reached_type"]
        .as_str()
        .filter(|kind| !kind.is_empty());
    let window = reached.or_else(|| {
        ["primary", "secondary"].into_iter().find(|window| {
            limits[*window]["used_percent"]
                .as_f64()
                .is_some_and(|used| used >= PLAN_LIMIT_UTILIZATION * 100.0)
        })
    });
    window.map(|window| {
        Limit::reported(
            "codex",
            Some(window),
            limits[window]["used_percent"]
                .as_f64()
                .map(|used| used / 100.0),
            &limits[window]["resets_at"],
        )
    })
}
