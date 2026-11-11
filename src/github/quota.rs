//! Shared machine-local Quota waits and Pass admission.
use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::GitHub;

mod identity;
mod store;
#[cfg(test)]
mod tests;
// The wait planner is the seam the following Command-wait Ticket consumes.
#[expect(
    dead_code,
    reason = "wait planning is connected to Commands by a later Ticket"
)]
pub(crate) mod wait;

pub(crate) use store::QuotaWait;
use store::Store;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Resource {
    Core,
    Graphql,
}

impl Resource {
    fn name(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Graphql => "graphql",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Quota {
    remaining: u64,
    reset_at: DateTime<Utc>,
}

impl Quota {
    pub(super) fn graphql(json: &Value) -> Result<Self> {
        let reset_at = json["resetAt"]
            .as_str()
            .context("GitHub quota has no resetAt")?;
        Ok(Self {
            remaining: json["remaining"]
                .as_u64()
                .context("GitHub quota has no remaining")?,
            reset_at: DateTime::parse_from_rfc3339(reset_at)
                .context("invalid GitHub quota resetAt")?
                .with_timezone(&Utc),
        })
    }

    fn rest(json: &Value) -> Result<Self> {
        let reset = json["reset"]
            .as_i64()
            .context("GitHub quota has no reset")?;
        Ok(Self {
            remaining: json["remaining"]
                .as_u64()
                .context("GitHub quota has no remaining")?,
            reset_at: DateTime::from_timestamp(reset, 0).context("invalid GitHub quota reset")?,
        })
    }

    fn low(&self, now: DateTime<Utc>) -> bool {
        self.remaining < 500 && self.reset_at > now
    }
}

impl QuotaWait {
    /// One stable reason, held until the last depleted resource resets.
    pub fn pass_reason(waits: &[Self]) -> Option<String> {
        let wait = waits
            .iter()
            .filter(|wait| wait.reset_at() > Utc::now())
            .max_by_key(|wait| wait.reset_at())?;
        Some(format!(
            "GitHub Quota wait until {} ({}: {} left)",
            wait.reset_at().to_rfc3339_opts(SecondsFormat::Secs, true),
            wait.resource.name(),
            wait.remaining(),
        ))
    }

    pub fn reset_at(&self) -> DateTime<Utc> {
        self.quota.reset_at
    }

    pub fn remaining(&self) -> u64 {
        self.quota.remaining
    }

    pub fn warning(waits: &[Self]) -> Option<String> {
        let reasons: Vec<_> = waits
            .iter()
            .filter(|wait| wait.reset_at() > Utc::now())
            .map(|wait| {
                let name = match wait.resource {
                    Resource::Graphql => "GraphQL points",
                    Resource::Core => "REST requests",
                };
                let reserve = if wait.remaining() < 500 {
                    " (fewer than 500)"
                } else {
                    ""
                };
                format!(
                    "{} {name} remaining{reserve}; Inbox reads paused until {}",
                    wait.remaining(),
                    wait.reset_at().to_rfc3339_opts(SecondsFormat::Secs, true),
                )
            })
            .collect();
        (!reasons.is_empty()).then(|| reasons.join("; "))
    }
}

impl GitHub {
    /// A known wait needs no probe. Non-quota probe failures fail open;
    /// exhaustion recorded by a failed probe still skips this Pass.
    pub(crate) fn pass_quota_wait(&self) -> Option<String> {
        if let Some(reason) = QuotaWait::pass_reason(&self.quota_waits().unwrap_or_default()) {
            return Some(reason);
        }
        let waits = self
            .inbox_quotas()
            .unwrap_or_else(|_| self.quota_waits().unwrap_or_default());
        QuotaWait::pass_reason(&waits)
    }

    // Quota probes deliberately bypass failure recording: learning a reset or
    // identity must never recursively probe another failure.
    fn quota_json(&self, args: &[&str]) -> Result<Value> {
        let output = self.raw_output_with_input(args, None)?;
        anyhow::ensure!(output.status.success(), "GitHub quota probe failed");
        Ok(serde_json::from_slice(&output.stdout)?)
    }

    /// Read-only access for the Inbox and later GitHub callers. With a known
    /// identity this makes no GitHub calls, even across process restarts.
    pub(crate) fn quota_waits(&self) -> Result<Vec<QuotaWait>> {
        let host = identity::host(&[]);
        self.quota_waits_for(&host)
    }

    pub(crate) fn quota_waits_for(&self, host: &str) -> Result<Vec<QuotaWait>> {
        let store = Store::machine()?;
        let host = host.to_ascii_lowercase();
        match identity::cached(&store, &host)? {
            Some(account) => store.waiting(&account, Utc::now()),
            None => Ok(Vec::new()),
        }
    }

    pub(super) fn observe_quota(&self, resource: Resource, quota: Quota) -> Result<()> {
        let store = Store::machine()?;
        let host = identity::host(&[]);
        let account = identity::resolve(self, &store, &host)?;
        store.observe(&account, resource, quota, Utc::now())
    }

    /// This endpoint reports both quotas and does not spend REST requests:
    /// https://docs.github.com/en/rest/rate-limit/rate-limit
    pub(crate) fn inbox_quotas(&self) -> Result<Vec<QuotaWait>> {
        let json = self.gh_json(&["api", "rate_limit"])?;
        for resource in [Resource::Graphql, Resource::Core] {
            self.observe_quota(resource, Quota::rest(&json["resources"][resource.name()])?)?;
        }
        self.quota_waits()
    }

    pub(super) fn record_quota_failure(&self, args: &[&str], stderr: &[u8]) -> Result<()> {
        let diagnostic = String::from_utf8_lossy(stderr).to_ascii_lowercase();
        if diagnostic.contains("secondary rate limit")
            || !(diagnostic.contains("api rate limit exceeded")
                || diagnostic.contains("api rate limit already exceeded")
                || diagnostic.contains("rate_limit_exceeded")
                || diagnostic.contains("rate_limited"))
        {
            return Ok(());
        }
        let resource = if diagnostic.contains("graphql:")
            || args.starts_with(&["api", "graphql"])
            || args.first() != Some(&"api")
                && (diagnostic.contains("rate_limit_exceeded")
                    || diagnostic.contains("rate_limited"))
        {
            Resource::Graphql
        } else {
            Resource::Core
        };
        let host = identity::host(args);
        let store = Store::machine()?;
        let account = identity::resolve(self, &store, &host)?;
        let now = Utc::now();
        let resource_json = self
            .quota_json(&["api", "rate_limit", "--hostname", &host])
            .unwrap_or(Value::Null);
        let resource_json = &resource_json["resources"][resource.name()];
        let quota = Quota {
            remaining: resource_json["remaining"].as_u64().unwrap_or(0),
            reset_at: resource_json["reset"]
                .as_i64()
                .and_then(|reset| DateTime::from_timestamp(reset, 0))
                .unwrap_or(now + chrono::Duration::minutes(15)),
        };
        store.exhausted(&account, resource, quota, now)
    }
}
