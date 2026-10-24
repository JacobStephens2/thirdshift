//! GitHub's two quota clocks. The Inbox leaves a fixed reserve for the factory.
use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::GitHub;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Quota {
    remaining: u64,
    reset_at: String,
}

impl Quota {
    pub(super) fn graphql(json: &Value) -> Result<Self> {
        let reset_at = json["resetAt"]
            .as_str()
            .context("GitHub quota has no resetAt")?;
        DateTime::parse_from_rfc3339(reset_at).context("invalid GitHub quota resetAt")?;
        Ok(Self {
            remaining: json["remaining"]
                .as_u64()
                .context("GitHub quota has no remaining")?,
            reset_at: reset_at.into(),
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
            reset_at: DateTime::from_timestamp(reset, 0)
                .context("invalid GitHub quota reset")?
                .to_rfc3339_opts(SecondsFormat::Secs, true),
        })
    }

    fn low(&self) -> bool {
        self.remaining < 500
            && DateTime::parse_from_rfc3339(&self.reset_at).is_ok_and(|reset| reset > Utc::now())
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Quotas {
    pub graphql: Quota,
    rest: Quota,
}

impl Quotas {
    pub fn warning(&self) -> Option<String> {
        let reasons: Vec<_> = [
            (&self.graphql, "GraphQL points"),
            (&self.rest, "REST requests"),
        ]
        .into_iter()
        .filter(|(quota, _)| quota.low())
        .map(|(quota, name)| {
            format!(
                "{} {name} remaining (fewer than 500); Inbox reads paused until {}",
                quota.remaining, quota.reset_at,
            )
        })
        .collect();
        (!reasons.is_empty()).then(|| reasons.join("; "))
    }
}

impl GitHub {
    /// This endpoint reports both quotas and does not spend REST requests:
    /// https://docs.github.com/en/rest/rate-limit/rate-limit
    pub(crate) fn inbox_quotas(&self) -> Result<Quotas> {
        let json = self.gh_json(&["api", "rate_limit"])?;
        let resources = &json["resources"];
        Ok(Quotas {
            graphql: Quota::rest(&resources["graphql"])?,
            rest: Quota::rest(&resources["core"])?,
        })
    }
}
