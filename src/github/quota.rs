//! Shared machine-local Quota waits, Pass admission and running Command recovery.
use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::GitHub;

pub(crate) mod command;
mod identity;
mod store;
#[cfg(test)]
mod tests;
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

    fn pass_reason(&self, resource: Resource) -> String {
        format!(
            "GitHub Quota wait until {} ({}: {} left)",
            self.reset_at.to_rfc3339_opts(SecondsFormat::Secs, true),
            resource.name(),
            self.remaining,
        )
    }
}

impl QuotaWait {
    /// One stable reason, held until the last depleted resource resets.
    pub fn pass_reason(waits: &[Self]) -> Option<String> {
        let wait = waits
            .iter()
            .filter(|wait| wait.reset_at() > Utc::now())
            .max_by_key(|wait| wait.reset_at())?;
        Some(wait.quota.pass_reason(wait.resource))
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
        // Admission records exhaustion and skips; only admitted Commands
        // wait and retry. Keep the probe's ordinary read-failure handling.
        let probe = Self {
            interruption: self.interruption,
            wait_for_quota: false,
        };
        if let Ok(quotas) = probe.current_quotas() {
            // Admission depends on GitHub's observation even when an
            // exhausted account cannot identify itself for shared storage.
            let reason = quotas
                .iter()
                .filter(|(_, quota)| quota.low(Utc::now()))
                .max_by_key(|(_, quota)| quota.reset_at)
                .map(|(resource, quota)| quota.pass_reason(*resource));
            for (resource, quota) in quotas {
                let _ = probe.observe_quota(resource, quota);
            }
            return reason;
        }
        QuotaWait::pass_reason(&self.quota_waits().unwrap_or_default())
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
    fn current_quotas(&self) -> Result<[(Resource, Quota); 2]> {
        let json = self.gh_json(&["api", "rate_limit"])?;
        Ok([
            (
                Resource::Graphql,
                Quota::rest(&json["resources"]["graphql"])?,
            ),
            (Resource::Core, Quota::rest(&json["resources"]["core"])?),
        ])
    }

    pub(crate) fn inbox_quotas(&self) -> Result<Vec<QuotaWait>> {
        for (resource, quota) in self.current_quotas()? {
            self.observe_quota(resource, quota)?;
        }
        self.quota_waits()
    }

    pub(super) fn record_quota_failure(&self, args: &[&str], resource: Resource) -> DateTime<Utc> {
        let host = identity::host(args);
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
        let reset = quota.reset_at;
        let record = || -> Result<()> {
            let store = Store::machine()?;
            let account = identity::resolve(self, &store, &host)?;
            store.exhausted(&account, resource, quota, now)
        };
        if record().is_err() {
            // Recording failure must not prevent waiting or hide the original
            // GitHub diagnostic if the Command reaches its wait cap.
            crate::progress::step("warning: could not record GitHub Quota wait");
        }
        reset
    }
}

pub(super) enum Limit {
    Primary(Resource),
    Secondary,
}

impl Limit {
    pub(super) fn of(args: &[&str], stderr: &[u8]) -> Option<Self> {
        let diagnostic = String::from_utf8_lossy(stderr).to_ascii_lowercase();
        if diagnostic.contains("secondary rate limit") || diagnostic.contains("abuse detection") {
            return Some(Self::Secondary);
        }
        if !(diagnostic.contains("api rate limit exceeded")
            || diagnostic.contains("api rate limit already exceeded")
            || diagnostic.contains("rate_limit_exceeded")
            || diagnostic.contains("rate_limited"))
        {
            return None;
        }
        let graphql = diagnostic.contains("graphql:")
            || args.starts_with(&["api", "graphql"])
            || args.first() != Some(&"api")
                && (diagnostic.contains("rate_limit_exceeded")
                    || diagnostic.contains("rate_limited"));
        Some(Self::Primary(if graphql {
            Resource::Graphql
        } else {
            Resource::Core
        }))
    }
}
