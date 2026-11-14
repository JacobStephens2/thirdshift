//! Official non-billed reads of a Harness's Plan usage and account, run on
//! demand: `codex app-server`'s `account/rateLimits/read`, `muse serve`'s
//! `usage/read` and `account/read`, and `claude auth status --json`. None
//! starts a model turn, scrapes a web endpoint or reads a private token.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use super::observation::{Exhausted, Window, from_percent, milliseconds, seconds, token};
use super::store::{Credits, State};
use crate::harness::Harness;

/// How long one read may take before thirdshift gives up on it.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The Harnesses with a non-billed read.
pub const READABLE: [Harness; 3] = [Harness::Claude, Harness::Codex, Harness::Muse];

/// The Harnesses whose read says how much of their Plan windows is used,
/// which routing refreshes before choosing a route. Claude Code's says only
/// who is signed in.
pub const WINDOWED: [Harness; 2] = [Harness::Codex, Harness::Muse];

/// What one Harness's read answered, before it is taken into the saved
/// state under the snapshot's lock.
pub enum Reading {
    Codex(Result<Value>, Option<bool>),
    Muse(Option<BTreeMap<i64, Value>>),
    Claude(Option<Value>),
}

/// Run `harness`'s read, if it has one.
pub fn read(harness: Harness) -> Option<Reading> {
    match harness {
        Harness::Codex => Some(match codex() {
            Ok(result) => Reading::Codex(Ok(result), None),
            Err(error) => Reading::Codex(Err(error), codex_signed_in()),
        }),
        Harness::Muse => Some(Reading::Muse(muse().ok())),
        Harness::Claude => Some(Reading::Claude(claude().ok())),
        Harness::Agy | Harness::Grok | Harness::OpenCode | Harness::Kimi => None,
    }
}

impl Reading {
    /// Take the answer into `state`. A Harness whose read failed keeps what
    /// was saved, with whether it is signed in when the CLI can say.
    pub fn apply(self, state: &mut State, now: DateTime<Utc>) {
        state.read_at = Some(now);
        match self {
            Self::Codex(Ok(result), _) => take_codex(state, &result, now),
            Self::Codex(Err(_), signed) => signed_in(state, signed),
            Self::Muse(Some(responses)) => take_muse(state, &responses, now),
            Self::Claude(Some(status)) => take_claude(state, &status),
            Self::Muse(None) | Self::Claude(None) => {}
        }
    }
}

fn signed_in(state: &mut State, signed_in: Option<bool>) {
    if signed_in.is_some() {
        state.account.get_or_insert_default().signed_in = signed_in;
    }
}

/// Short display text from a read, without control characters.
fn text(value: &Value) -> Option<String> {
    let text = match value {
        Value::String(text) => text.trim().to_owned(),
        Value::Number(number) => number.to_string(),
        Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => return None,
    };
    (!text.is_empty() && text.len() <= 128 && !text.chars().any(char::is_control)).then_some(text)
}

fn codex() -> Result<Value> {
    let mut responses = rpc(
        "codex",
        &["app-server"],
        &[
            json!({"method": "initialize", "id": 0, "params": {"clientInfo": client()}}),
            json!({"method": "initialized"}),
            json!({"method": "account/rateLimits/read", "id": 1}),
        ],
        &[1],
    )?;
    let response = responses.remove(&1).context("no rate-limits answer")?;
    if !response["error"].is_null() {
        bail!("codex app-server refused the read");
    }
    Ok(response["result"].clone())
}

fn codex_signed_in() -> Option<bool> {
    let mut child = Command::new("codex")
        .args(["login", "status"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().ok()? {
            return Some(status.success());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn codex_windows(limits: &Value, prefix: &str) -> Vec<Window> {
    ["primary", "secondary"]
        .into_iter()
        .filter(|name| limits[*name].is_object())
        .filter_map(|name| {
            let reported = &limits[name];
            let mut window = Window::named(&format!("{prefix}{name}"))?;
            window.used_percent = from_percent(&reported["usedPercent"]);
            window.duration_minutes = reported["windowDurationMins"].as_u64();
            window.resets_at = seconds(&reported["resetsAt"]);
            Some(window)
        })
        .collect()
}

/// The first of `key` found on the answer or its `rateLimits`.
fn either<'a>(result: &'a Value, key: &str) -> &'a Value {
    if result[key].is_null() {
        &result["rateLimits"][key]
    } else {
        &result[key]
    }
}

fn take_codex(state: &mut State, result: &Value, now: DateTime<Utc>) {
    let limits = if result["rateLimits"].is_object() {
        &result["rateLimits"]
    } else {
        result
    };
    let mut windows = codex_windows(limits, "");
    for (id, limits) in result["rateLimitsByLimitId"]
        .as_object()
        .into_iter()
        .flatten()
    {
        let more = codex_windows(limits, &format!("{id}-"));
        let same = more.len() == windows.len()
            && more.iter().zip(&windows).all(|(more, window)| {
                (more.used_percent, more.resets_at) == (window.used_percent, window.resets_at)
            });
        if !same {
            windows.extend(more);
        }
    }
    state.windows = windows;
    state.observed_at = Some(now);
    state.source = Some("codex app-server".into());
    let account = state.account.get_or_insert_default();
    account.signed_in = Some(true);
    account.plan = token(either(result, "planType")).or(account.plan.take());
    account.id = text(&result["accountId"]).or(account.id.take());
    let credits = either(result, "credits");
    if credits.is_object() {
        state.credits = Some(Credits {
            has_credits: credits["hasCredits"].as_bool(),
            balance: text(&credits["balance"]),
        });
    }
    state.ordinary_usage_allowed = either(result, "ordinaryUsageAllowed").as_bool();
    state.spend_control_reached =
        either(result, "spendControlReached")
            .as_bool()
            .or(either(result, "individualLimit")["spendControlReached"].as_bool());
    let full = state
        .windows
        .iter()
        .find(|window| window.used_percent.is_some_and(|used| used >= 100.0))
        .cloned();
    // Codex asks clients not to infer recovery from percentages or reset
    // times: only its own word clears a recorded limit.
    let refused =
        state.ordinary_usage_allowed == Some(false) || state.spend_control_reached == Some(true);
    if refused || (state.ordinary_usage_allowed.is_none() && full.is_some()) {
        state.exhaust(
            &Exhausted {
                window: full.as_ref().map(|window| window.name.clone()),
                resets_at: full.and_then(|window| window.resets_at),
                overage: None,
            },
            now,
        );
    } else if state.ordinary_usage_allowed == Some(true) {
        state.cooldown = None;
    }
}

fn muse() -> Result<BTreeMap<i64, Value>> {
    rpc(
        "muse",
        &["serve"],
        &[
            json!({"method": "initialize", "id": 0, "params": {
                "clientInfo": client(), "capabilities": {"experimentalApi": true}
            }}),
            json!({"method": "initialized"}),
            json!({"method": "usage/read", "id": 1}),
            json!({"method": "account/read", "id": 2}),
        ],
        &[1, 2],
    )
}

fn take_muse(state: &mut State, responses: &BTreeMap<i64, Value>, now: DateTime<Utc>) {
    let usage = responses
        .get(&1)
        .map(|response| &response["result"]["usage"])
        .filter(|usage| usage.is_object());
    if let Some(usage) = usage {
        let mut windows = Vec::new();
        for name in ["window", "weekly"] {
            let reported = &usage[name];
            if !reported.is_object() {
                continue;
            }
            let mut window = Window::named(name).expect("a fixed window name");
            window.used_percent = from_percent(&reported["usedPercent"]);
            window.resets_at = milliseconds(&reported["resetsAtMs"]);
            window.duration_minutes = reported["windowDurationMins"].as_u64();
            windows.push(window);
        }
        let observed = milliseconds(&usage["observedAtMs"])
            .filter(|observed| *observed <= now)
            .unwrap_or(now);
        let newer = state.observed_at.is_none_or(|saved| observed >= saved);
        if newer {
            state.windows = windows;
            state.observed_at = Some(observed);
            state.source = Some("muse serve".into());
            let full = state
                .windows
                .iter()
                .find(|window| {
                    window.used_percent.is_some_and(|used| used >= 100.0) && !window.reset_by(now)
                })
                .cloned();
            match full {
                Some(window) => state.exhaust(
                    &Exhausted {
                        window: Some(window.name),
                        resets_at: window.resets_at,
                        overage: None,
                    },
                    now,
                ),
                None if state
                    .cooldown
                    .as_ref()
                    .is_some_and(|cooldown| cooldown.since < observed) =>
                {
                    state.cooldown = None
                }
                None => {}
            }
        }
        if let Some(tier) = token(&usage["tier"]) {
            state.account.get_or_insert_default().plan = Some(tier);
        }
    }
    if let Some(response) = responses.get(&2) {
        let result = &response["result"];
        let account = if result["account"].is_object() {
            &result["account"]
        } else {
            result
        };
        if account.is_object() {
            let saved = state.account.get_or_insert_default();
            saved.label = text(&account["label"]).or(saved.label.take());
            saved.signed_in = account["credentialRequired"]
                .as_bool()
                .map(|required| !required)
                .or_else(|| muse_state_signed_in(&account["state"]))
                .or(saved.signed_in);
        }
    }
}

/// What Muse's account `state` says about signing in, when it says
/// anything: a signed-out or credential-wanting state is signed out.
fn muse_state_signed_in(state: &Value) -> Option<bool> {
    let state: String = token(state)?
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_lowercase())
        .collect();
    match state.as_str() {
        "signedout" | "loggedout" | "unauthenticated" | "credentialrequired" | "none" => {
            Some(false)
        }
        "signedin" | "loggedin" | "authenticated" | "ready" => Some(true),
        _ => None,
    }
}

fn claude() -> Result<Value> {
    let output = output_within("claude", &["auth", "status", "--json"])?;
    Ok(serde_json::from_slice(&output)?)
}

fn take_claude(state: &mut State, status: &Value) {
    let Some(logged_in) = status["loggedIn"].as_bool() else {
        return;
    };
    let account = state.account.get_or_insert_default();
    account.signed_in = Some(logged_in);
    if logged_in {
        account.plan = token(&status["subscriptionType"]).or(account.plan.take());
        account.tier = token(&status["planTier"]).or(account.tier.take());
        account.label = text(&status["email"]).or(account.label.take());
        account.organization = text(&status["orgId"]).or(account.organization.take());
    }
}

fn client() -> Value {
    json!({"name": "thirdshift", "title": "thirdshift", "version": env!("CARGO_PKG_VERSION")})
}

/// Run `cli` with `args` and keep its stdout, whatever its exit status, if
/// it ends within the timeout.
fn output_within(cli: &str, args: &[&str]) -> Result<Vec<u8>> {
    let mut child = Command::new(cli)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("could not run {cli}"))?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut bytes);
        let _ = sender.send(bytes);
    });
    let bytes = receiver.recv_timeout(TIMEOUT);
    let _ = child.kill();
    let _ = child.wait();
    bytes.with_context(|| format!("{cli} did not answer in time"))
}

/// Send JSON-RPC `requests`, one per line without the `jsonrpc` member, to
/// `cli args` on stdin, and collect the responses whose ids are in `wanted`
/// until they have all come, the server ends or the timeout passes.
fn rpc(
    cli: &str,
    args: &[&str],
    requests: &[Value],
    wanted: &[i64],
) -> Result<BTreeMap<i64, Value>> {
    let mut child = Command::new(cli)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("could not run {cli}"))?;
    let stdout = child.stdout.take().expect("piped stdout");
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let mut stdin = child.stdin.take().expect("piped stdin");
    for request in requests {
        if writeln!(stdin, "{request}").is_err() {
            break;
        }
    }
    let _ = stdin.flush();
    let deadline = Instant::now() + TIMEOUT;
    let mut responses = BTreeMap::new();
    while wanted.iter().any(|id| !responses.contains_key(id)) {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok(line) = receiver.recv_timeout(left) else {
            break;
        };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(id) = message["id"].as_i64().filter(|id| wanted.contains(id))
            && (message.get("result").is_some() || message.get("error").is_some())
        {
            responses.insert(id, message);
        }
    }
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    if responses.is_empty() {
        bail!("{cli} {} gave no answer", args.join(" "));
    }
    Ok(responses)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_refusal_records_a_limit_and_its_permission_clears_one() {
        let now = Utc::now();
        let mut state = State::default();
        take_codex(
            &mut state,
            &json!({
                "accountId": "acct-1", "planType": "pro", "ordinaryUsageAllowed": false,
                "rateLimits": {
                    "primary": {"usedPercent": 100, "windowDurationMins": 300, "resetsAt": 1_900_000_000},
                    "secondary": {"usedPercent": 40, "windowDurationMins": 10080}
                },
                "credits": {"hasCredits": true, "balance": "12.50"}
            }),
            now,
        );
        let cooldown = state.cooldown.clone().unwrap();
        assert_eq!(cooldown.window.as_deref(), Some("primary"));
        assert_eq!(cooldown.until.timestamp(), 1_900_000_000);
        assert_eq!(state.account.as_ref().unwrap().plan.as_deref(), Some("pro"));
        assert_eq!(
            state.credits.as_ref().unwrap().balance.as_deref(),
            Some("12.50")
        );
        take_codex(
            &mut state,
            &json!({"ordinaryUsageAllowed": true, "rateLimits": {"primary": {"usedPercent": 3}}}),
            now,
        );
        assert_eq!(state.cooldown, None);
    }

    #[test]
    fn muse_usage_and_account_are_taken_and_an_older_reading_never_replaces_a_newer() {
        let now = Utc::now();
        let mut state = State::default();
        let responses = BTreeMap::from([
            (
                1,
                json!({"id": 1, "result": {"usage": {
                    "tier": "pro", "observedAtMs": now.timestamp_millis() - 1000,
                    "window": {"usedPercent": 30, "resetsAtMs": now.timestamp_millis() + 60_000, "windowDurationMins": 300},
                    "weekly": {"usedPercent": 70}
                }}}),
            ),
            (
                2,
                json!({"id": 2, "result": {"state": "ready", "label": "me", "credentialRequired": false}}),
            ),
        ]);
        take_muse(&mut state, &responses, now);
        assert_eq!(state.windows.len(), 2);
        assert_eq!(state.windows[0].used_percent, Some(30.0));
        let account = state.account.clone().unwrap();
        assert_eq!(
            (
                account.plan.as_deref(),
                account.label.as_deref(),
                account.signed_in
            ),
            (Some("pro"), Some("me"), Some(true))
        );
        state.observed_at = Some(now);
        state.windows.clear();
        take_muse(&mut state, &responses, now);
        assert!(state.windows.is_empty());
    }

    #[test]
    fn claude_auth_status_says_who_is_signed_in_and_on_what_plan() {
        let mut state = State::default();
        take_claude(
            &mut state,
            &json!({"loggedIn": true, "subscriptionType": "max", "planTier": "default_claude_max_20x",
                "orgId": "org-1", "email": "me@example.com"}),
        );
        let account = state.account.clone().unwrap();
        assert_eq!(account.plan.as_deref(), Some("max"));
        assert_eq!(account.label.as_deref(), Some("me@example.com"));
        take_claude(&mut state, &json!({"loggedIn": false}));
        assert_eq!(state.account.unwrap().signed_in, Some(false));
    }

    #[test]
    fn muse_account_state_alone_says_signed_out() {
        let mut state = State::default();
        take_muse(
            &mut state,
            &BTreeMap::from([(
                2,
                json!({"id": 2, "result": {"state": "signed_out", "label": "me"}}),
            )]),
            Utc::now(),
        );
        assert_eq!(state.account.unwrap().signed_in, Some(false));
    }
}
