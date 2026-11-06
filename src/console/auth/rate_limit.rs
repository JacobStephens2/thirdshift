//! Shared, disposable failed-authentication budget for each client IP address.
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};

const COOLDOWN: Duration = Duration::from_secs(60);

#[derive(Clone, Default)]
pub(super) struct Limiter(Arc<Mutex<HashMap<IpAddr, Failures>>>);

struct Failures {
    count: u32,
    last: Instant,
    blocked_until: Option<Instant>,
}

impl Limiter {
    pub fn retry_after(&self, address: IpAddr) -> Option<u64> {
        let now = Instant::now();
        let mut failures = self.0.lock().expect("authentication limiter lock");
        failures.retain(|_, failure| now.duration_since(failure.last) < COOLDOWN);
        failures.get(&address)?.blocked_until.map(|until| {
            until
                .saturating_duration_since(now)
                .as_secs()
                .saturating_add(1)
        })
    }

    pub async fn failed(&self, address: IpAddr) -> Option<u64> {
        let delay = {
            let now = Instant::now();
            let mut failures = self.0.lock().expect("authentication limiter lock");
            let failure = failures.entry(address).or_insert(Failures {
                count: 0,
                last: now,
                blocked_until: None,
            });
            // An attempt already in flight must not extend an existing cool-down.
            if let Some(until) = failure.blocked_until {
                return Some(
                    until
                        .saturating_duration_since(now)
                        .as_secs()
                        .saturating_add(1),
                );
            }
            failure.count += 1;
            failure.last = now;
            if failure.count >= 8 {
                failure.blocked_until = Some(now + COOLDOWN);
                return Some(COOLDOWN.as_secs());
            }
            if failure.count < 3 {
                Duration::ZERO
            } else {
                Duration::from_millis(250 << (failure.count - 3).min(3))
            }
        };
        tokio::time::sleep(delay).await;
        self.retry_after(address)
    }
}

pub(super) fn address(peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
    // Caddy and nginx replace this header with one value. Apache appends the
    // client it saw onto the first header and leaves a later duplicate as the
    // caller sent it, so the last header is not the proxy's address.
    if peer.ip() == Ipv4Addr::LOCALHOST
        && let Some(address) = headers
            .get_all("x-forwarded-for")
            .iter()
            .next()
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.rsplit(',').next())
            .and_then(|value| value.trim().parse::<IpAddr>().ok())
    {
        return address.to_canonical();
    }
    peer.ip().to_canonical()
}

pub(super) fn refusal(retry_after: u64) -> Response {
    let mut response = (
        StatusCode::TOO_MANY_REQUESTS,
        axum::Json(serde_json::json!({
            "error": "Too many failed sign-ins. Try again after the cool-down.",
            "retry_after": retry_after,
        })),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, retry_after.into());
    response
}
