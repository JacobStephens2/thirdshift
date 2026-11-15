//! Plan usage facts read from structured Harness wire frames and official
//! non-billed reads, never from agent-writable text. Every string kept is a
//! short operator token, so a stored or printed fact cannot carry a forged
//! message or control characters.

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::harness::Harness;
use crate::harness::interpretation::limits::operator_token;

/// One Plan window, such as Claude's `five_hour` or Codex's `primary`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Window {
    pub name: String,
    /// Percentage of the window used, 0 to 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_percent: Option<f64>,
    /// The provider's own status for the window, such as `allowed_warning`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_minutes: Option<u64>,
}

impl Window {
    pub(crate) fn named(name: &str) -> Option<Self> {
        Some(Self {
            name: operator_token(name)?.to_owned(),
            used_percent: None,
            status: None,
            resets_at: None,
            duration_minutes: None,
        })
    }

    /// Whether the window has reset since it was observed, so its
    /// utilization no longer says anything.
    pub fn reset_by(&self, now: DateTime<Utc>) -> bool {
        self.resets_at.is_some_and(|reset| reset <= now)
    }
}

/// A confirmed limit: the provider refused, or will refuse, included work.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Exhausted {
    pub window: Option<String>,
    pub resets_at: Option<DateTime<Utc>>,
    /// The provider's overage status, such as `rejected` for a spend cap.
    pub overage: Option<String>,
}

/// What one wire frame said about a subscription Harness's Plan usage.
#[derive(Clone, Debug, PartialEq)]
pub struct Observation {
    pub harness: Harness,
    pub windows: Vec<Window>,
    pub exhausted: Option<Exhausted>,
}

impl Observation {
    /// The refusal this observation reports, as a Paused Command's record
    /// keeps it: the Harness, the Plan window that stopped work, its
    /// utilization as a fraction, and its reset. None when it reports no
    /// refusal.
    pub(crate) fn limit(&self) -> Option<crate::session::limits::Limit> {
        let exhausted = self.exhausted.as_ref()?;
        let window = exhausted
            .window
            .as_deref()
            .and_then(|name| self.windows.iter().find(|window| window.name == name));
        let window = window.or_else(|| {
            self.windows.iter().max_by(|a, b| {
                a.used_percent
                    .unwrap_or_default()
                    .total_cmp(&b.used_percent.unwrap_or_default())
            })
        });
        Some(crate::session::limits::Limit::of(
            self.harness,
            exhaustion_window(exhausted, window),
            window.and_then(|window| window.used_percent),
            exhausted
                .resets_at
                .or_else(|| window.and_then(|window| window.resets_at)),
        ))
    }
}

/// The name of the Plan window that stopped work: the refusal's own, else
/// the fullest window reported beside it.
fn exhaustion_window(exhausted: &Exhausted, window: Option<&Window>) -> Option<String> {
    exhausted
        .window
        .clone()
        .or_else(|| window.map(|window| window.name.clone()))
}

/// The Harnesses whose subscriptions Plan usage watches. OpenCode and Kimi
/// Code run on API keys, not subscriptions.
pub const SUBSCRIPTION: [Harness; 5] = [
    Harness::Claude,
    Harness::Codex,
    Harness::Muse,
    Harness::Agy,
    Harness::Grok,
];

pub fn subscription(harness: Harness) -> bool {
    SUBSCRIPTION.contains(&harness)
}

/// A token, as providers name windows and statuses.
pub(crate) fn token(value: &Value) -> Option<String> {
    value.as_str().and_then(operator_token).map(str::to_owned)
}

/// A percentage from a fraction, as Claude reports utilization.
fn from_fraction(value: &Value) -> Option<f64> {
    percent(value.as_f64()? * 100.0)
}

/// A percentage, as Codex and Muse report them.
pub(crate) fn from_percent(value: &Value) -> Option<f64> {
    percent(value.as_f64()?)
}

fn percent(used: f64) -> Option<f64> {
    (used.is_finite() && (0.0..=1000.0).contains(&used)).then_some(used)
}

/// A time given as Unix seconds or RFC 3339.
pub(crate) fn seconds(value: &Value) -> Option<DateTime<Utc>> {
    match value {
        Value::Number(number) => DateTime::from_timestamp(number.as_i64()?, 0),
        Value::String(text) => DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|time| time.to_utc()),
        Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => None,
    }
}

/// A time given as Unix milliseconds.
pub(crate) fn milliseconds(value: &Value) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp_millis(value.as_i64()?)
}

/// Claude's `rate_limit_event`, whatever its status or utilization.
pub(crate) fn claude_rate_limit(event: &Value) -> Option<Observation> {
    if event["type"] != "rate_limit_event" {
        return None;
    }
    let info = &event["rate_limit_info"];
    let status = token(&info["status"]);
    let named = token(&info["rateLimitType"]);
    let mut windows: Vec<Window> = Vec::new();
    for (name, unified) in info["unifiedWindows"].as_object().into_iter().flatten() {
        let Some(mut window) = Window::named(name) else {
            continue;
        };
        window.used_percent = from_fraction(&unified["utilization"]);
        window.status = token(&unified["status"]);
        window.resets_at = seconds(&unified["resetsAt"]);
        windows.push(window);
    }
    if let Some(name) = &named {
        if !windows.iter().any(|window| &window.name == name) {
            windows.extend(Window::named(name));
        }
        let window = windows
            .iter_mut()
            .find(|window| &window.name == name)
            .expect("the named window was just added");
        window.used_percent = window.used_percent.or(from_fraction(&info["utilization"]));
        window.status = window.status.take().or(status.clone());
        window.resets_at = window.resets_at.or(seconds(&info["resetsAt"]));
    }
    if windows.is_empty() && status.is_none() {
        return None;
    }
    let exhausted = (status.as_deref() == Some("rejected")).then(|| Exhausted {
        resets_at: named
            .as_ref()
            .and_then(|name| windows.iter().find(|window| &window.name == name))
            .and_then(|window| window.resets_at)
            .or(seconds(&info["resetsAt"])),
        window: named,
        overage: token(&info["overageStatus"]),
    });
    Some(Observation {
        harness: Harness::Claude,
        windows,
        exhausted,
    })
}

/// The rate-limit facts on a failed Claude API turn, from its `api_error`
/// and `api_error_params.rate_limit_info`: a rejected status, or, with no
/// status, a usage-limit `api_error` or a rejected overage (a spend cap).
pub(crate) fn claude_api_error(event: &Value) -> Option<Observation> {
    let info = &event["api_error_params"]["rate_limit_info"];
    let usage_limit = event["api_error"]
        .as_str()
        .is_some_and(|kind| kind.to_ascii_lowercase().contains("usage_limit"));
    let refused = match info["status"].as_str() {
        Some(status) => status == "rejected",
        None => usage_limit || info["overageStatus"] == "rejected",
    };
    if !refused {
        return None;
    }
    let window = token(&info["rateLimitType"]);
    let resets_at = seconds(&info["resetsAt"]).or_else(|| seconds(&info["overageResetsAt"]));
    let windows = window
        .as_deref()
        .and_then(Window::named)
        .map(|mut named| {
            named.status = Some("rejected".into());
            named.resets_at = resets_at;
            named
        })
        .into_iter()
        .collect();
    Some(Observation {
        harness: Harness::Claude,
        windows,
        exhausted: Some(Exhausted {
            window,
            resets_at,
            overage: token(&info["overageStatus"]),
        }),
    })
}

/// Codex's `token_count` report, from its session file or its stream.
pub(crate) fn codex_token_count(event: &Value, now: DateTime<Utc>) -> Option<Observation> {
    let event = if event["type"] == "event_msg" && event["payload"]["type"] == "token_count" {
        &event["payload"]
    } else {
        event
    };
    let limits = &event["rate_limits"];
    if !limits.is_object() {
        return None;
    }
    let mut windows = Vec::new();
    for name in ["primary", "secondary"] {
        let reported = &limits[name];
        if !reported.is_object() {
            continue;
        }
        let mut window = Window::named(name).expect("a fixed window name");
        window.used_percent = from_percent(&reported["used_percent"]);
        window.duration_minutes = reported["window_minutes"].as_u64();
        window.resets_at = seconds(&reported["resets_at"]).or_else(|| {
            reported["resets_in_seconds"]
                .as_i64()
                .and_then(TimeDelta::try_seconds)
                .map(|delta| now + delta)
        });
        windows.push(window);
    }
    let reached = token(&limits["rate_limit_reached_type"]);
    let exhausted = reached.map(|window| Exhausted {
        resets_at: windows
            .iter()
            .find(|named| named.name == window)
            .and_then(|named| named.resets_at),
        window: Some(window),
        overage: None,
    });
    if windows.is_empty() && exhausted.is_none() {
        return None;
    }
    Some(Observation {
        harness: Harness::Codex,
        windows,
        exhausted,
    })
}

/// A Harness's own terminal failure, from a top-level failed-turn frame,
/// when its reason names a usage limit rather than a per-minute or
/// per-second request rate, which is transient throttling. Callers pass
/// only frames the agent cannot write: never a tool's or sub-agent's
/// failure, nor an interrupted turn's final message.
pub(crate) fn terminal_failure(harness: Harness, reason: &str) -> Option<Observation> {
    let usage = crate::harness::interpretation::event::ErrorKind::from_text(reason)
        == crate::harness::interpretation::event::ErrorKind::UsageLimit;
    (usage && !throttling(reason))
        .then(|| usage_limit(harness))
        .flatten()
}

/// Whether `reason` names a short request-rate quota, such as Gemini's
/// "requests per minute", whose 429 clears in moments.
fn throttling(reason: &str) -> bool {
    let squeezed: String = reason
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_lowercase())
        .collect();
    squeezed.contains("perminute") || squeezed.contains("persecond")
}

/// A terminal usage-limit error a Harness's stream decoder reported.
pub(crate) fn usage_limit(harness: Harness) -> Option<Observation> {
    subscription(harness).then(|| Observation {
        harness,
        windows: Vec::new(),
        exhausted: Some(Exhausted::default()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_per_minute_quota_throttle_is_not_plan_usage() {
        assert!(terminal_failure(Harness::Agy,
            "429 RESOURCE_EXHAUSTED: Quota exceeded for quota metric 'Generate Content API requests per minute'").is_none());
        assert!(
            terminal_failure(
                Harness::Agy,
                "quota exceeded: GenerateRequestsPerMinutePerProject"
            )
            .is_none()
        );
        assert!(terminal_failure(Harness::Grok, "quota exceeded").is_some());
    }

    #[test]
    fn claude_rate_limit_events_are_kept_at_every_status_and_utilization() {
        let allowed = claude_rate_limit(&json!({
            "type": "rate_limit_event",
            "rate_limit_info": {
                "status": "allowed",
                "rateLimitType": "five_hour",
                "utilization": 0.12,
                "resetsAt": 1_800_000_000,
                "unifiedWindows": {"seven_day": {"utilization": 0.4, "resetsAt": 1_800_500_000}}
            }
        }))
        .unwrap();
        assert_eq!(allowed.exhausted, None);
        let five = allowed
            .windows
            .iter()
            .find(|w| w.name == "five_hour")
            .unwrap();
        assert_eq!(five.used_percent, Some(12.0));
        assert_eq!(five.status.as_deref(), Some("allowed"));
        assert_eq!(five.resets_at.unwrap().timestamp(), 1_800_000_000);
        let seven = allowed
            .windows
            .iter()
            .find(|w| w.name == "seven_day")
            .unwrap();
        assert_eq!(seven.used_percent, Some(40.0));

        let rejected = claude_rate_limit(&json!({
            "type": "rate_limit_event",
            "rate_limit_info": {"status": "rejected", "rateLimitType": "seven_day",
                "resetsAt": 1_800_000_000, "overageStatus": "rejected"}
        }))
        .unwrap();
        let exhausted = rejected.exhausted.unwrap();
        assert_eq!(exhausted.window.as_deref(), Some("seven_day"));
        assert_eq!(exhausted.overage.as_deref(), Some("rejected"));
        assert_eq!(exhausted.resets_at.unwrap().timestamp(), 1_800_000_000);
    }

    #[test]
    fn forged_window_names_are_dropped() {
        let observation = claude_rate_limit(&json!({
            "type": "rate_limit_event",
            "rate_limit_info": {"status": "allowed_warning",
                "unifiedWindows": {"five_hour\necho forged": {"utilization": 0.9}}}
        }))
        .unwrap();
        assert!(observation.windows.is_empty());
    }

    #[test]
    fn codex_token_counts_are_kept_below_any_threshold_and_name_a_reached_window() {
        let now = Utc::now();
        let observation = codex_token_count(
            &json!({"type": "event_msg", "payload": {"type": "token_count", "rate_limits": {
                "primary": {"used_percent": 7.0, "window_minutes": 300, "resets_in_seconds": 60},
                "secondary": {"used_percent": 100.0, "resets_at": 1_800_000_000},
                "rate_limit_reached_type": "secondary"
            }}}),
            now,
        )
        .unwrap();
        assert_eq!(observation.windows[0].used_percent, Some(7.0));
        assert_eq!(observation.windows[0].duration_minutes, Some(300));
        assert_eq!(
            observation.windows[0].resets_at,
            Some(now + TimeDelta::seconds(60))
        );
        let exhausted = observation.exhausted.unwrap();
        assert_eq!(exhausted.window.as_deref(), Some("secondary"));
        assert_eq!(exhausted.resets_at.unwrap().timestamp(), 1_800_000_000);
    }

    #[test]
    fn api_key_harnesses_are_not_watched() {
        assert!(usage_limit(Harness::OpenCode).is_none());
        assert!(usage_limit(Harness::Kimi).is_none());
        for harness in SUBSCRIPTION {
            assert!(usage_limit(harness).is_some(), "{harness:?}");
        }
    }
}
