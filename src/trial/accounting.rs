//! Frozen token prices and the accounting facts retained with each attempt.
use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::session::usage::{Measurement, Summary};

/// Durable ownership and clock observation, separate from ended trace spans.
#[derive(Serialize, Deserialize)]
pub(crate) struct SessionStart {
    pub role: String,
    pub at_unix_nano: u64,
}

impl SessionStart {
    pub fn path(log: &Path) -> std::path::PathBuf {
        crate::logs::session::original(log).with_extension("trial-start.json")
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Prices {
    pub date: String,
    pub source: String,
    pub models: BTreeMap<String, Rate>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rate {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

impl Default for Prices {
    fn default() -> Self {
        serde_json::from_str(include_str!("prices.json")).expect("embedded Trial prices")
    }
}

impl Prices {
    pub fn load(path: &Path) -> Result<Self> {
        let prices: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        chrono::NaiveDate::parse_from_str(&prices.date, "%Y-%m-%d")
            .context("price table date must be YYYY-MM-DD")?;
        ensure!(
            !prices.source.trim().is_empty(),
            "price table needs a source"
        );
        ensure!(!prices.models.is_empty(), "price table needs Models");
        for (model, rate) in &prices.models {
            ensure!(!model.trim().is_empty(), "price table has an empty Model");
            ensure!(
                [rate.input, rate.output, rate.cache_read, rate.cache_write]
                    .iter()
                    .all(|rate| rate.is_finite() && *rate >= 0.0),
                "price table rates must be finite and nonnegative: {model}"
            );
        }
        Ok(prices)
    }

    pub fn cost(&self, usage: &Summary) -> Result<f64> {
        ensure!(!usage.models.is_empty(), "missing usage");
        let mut total = 0.0;
        for model in &usage.models {
            let name = model.model.as_deref().context("missing observed Model")?;
            let rate = self
                .models
                .get(name)
                .with_context(|| format!("missing frozen price for Model {name}"))?;
            let tokens = model.measurement;
            let input = tokens.input_tokens.context("missing usage: input tokens")?;
            let output = tokens
                .output_tokens
                .context("missing usage: output tokens")?;
            let read = tokens.cache_read_tokens.unwrap_or(0);
            let write = tokens.cache_write_tokens.unwrap_or(0);
            let uncached = input
                .checked_sub(read)
                .and_then(|n| n.checked_sub(write))
                .context("cache tokens exceed inclusive input usage")?;
            total += (uncached as f64 * rate.input
                + output as f64 * rate.output
                + read as f64 * rate.cache_read
                + write as f64 * rate.cache_write)
                / 1_000_000.0;
        }
        ensure!(total.is_finite(), "priced cost overflow");
        Ok(total)
    }
}

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Attempt {
    pub sessions: Vec<Session>,
    /// A global stop retained Session facts before this attempt completed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub incomplete: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Session {
    pub path: String,
    pub role: String,
    pub time_ms: u128,
    pub models: BTreeMap<String, Measurement>,
    pub cost_usd: Option<f64>,
    pub gap: Option<String>,
}

impl Attempt {
    pub fn gap(&self) -> Option<&str> {
        self.sessions
            .iter()
            .find_map(|session| session.gap.as_deref())
    }
}
