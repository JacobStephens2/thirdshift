//! Versioned Bar levels and their intervals retain the saved single-level shape.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use super::{Decision, Interval, interval};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Levels {
    Separate {
        #[serde(deserialize_with = "decimal")]
        pass_level: f64,
        #[serde(deserialize_with = "decimal")]
        fail_level: f64,
        /// Saved two-level Bars without this field keep ADR-0024's rule.
        #[serde(
            default = "PassRule::reaches_zero",
            skip_serializing_if = "PassRule::is_reaches_zero"
        )]
        pass_rule: PassRule,
    },
    Legacy {
        #[serde(deserialize_with = "decimal")]
        credible_level: f64,
    },
}

impl Default for Levels {
    fn default() -> Self {
        Self::Separate {
            pass_level: 0.70,
            fail_level: 0.95,
            pass_rule: PassRule::Margin,
        }
    }
}

/// What a pass needs once the pass-level lower bound clears the margin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PassRule {
    /// Nothing more (`paired-dirichlet-v3`, ADR-0025).
    Margin,
    /// The pass-level upper bound must also reach zero (`paired-dirichlet-v1`
    /// and `-v2`, ADR-0024).
    ReachesZero,
}

impl PassRule {
    fn reaches_zero() -> Self {
        Self::ReachesZero
    }

    fn is_reaches_zero(&self) -> bool {
        *self == Self::ReachesZero
    }
}

impl Levels {
    pub fn pass(self) -> f64 {
        match self {
            Self::Separate { pass_level, .. } => pass_level,
            Self::Legacy { credible_level } => credible_level,
        }
    }

    pub fn fail(self) -> f64 {
        match self {
            Self::Separate { fail_level, .. } => fail_level,
            Self::Legacy { credible_level } => credible_level,
        }
    }

    pub fn pass_rule(self) -> PassRule {
        match self {
            Self::Separate { pass_rule, .. } => pass_rule,
            Self::Legacy { .. } => PassRule::ReachesZero,
        }
    }

    pub fn validate(self) -> Result<()> {
        for (name, value) in [("pass", self.pass()), ("fail", self.fail())] {
            ensure!(
                value.is_finite() && value > 0.0 && value < 1.0,
                "{name} level must be strictly between zero and one"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Intervals {
    Separate {
        pass_interval: Interval,
        fail_interval: Interval,
    },
    Legacy {
        interval: Interval,
    },
}

impl Intervals {
    pub(super) fn from_draws(draws: &mut [f64], levels: Levels) -> Self {
        let pass_interval = interval(draws, levels.pass());
        match levels {
            Levels::Separate { .. } => Self::Separate {
                pass_interval,
                fail_interval: if levels.pass() == levels.fail() {
                    pass_interval
                } else {
                    interval(draws, levels.fail())
                },
            },
            Levels::Legacy { .. } => Self::Legacy {
                interval: pass_interval,
            },
        }
    }

    pub fn pass(self) -> Interval {
        match self {
            Self::Separate { pass_interval, .. } => pass_interval,
            Self::Legacy { interval } => interval,
        }
    }

    pub fn fail(self) -> Interval {
        match self {
            Self::Separate { fail_interval, .. } => fail_interval,
            Self::Legacy { interval } => interval,
        }
    }

    pub(super) fn decide(self, margin: f64, rule: PassRule) -> Decision {
        if self.fail().upper < 0.0 {
            Decision::Fail
        } else if self.pass().lower > margin
            && (rule == PassRule::Margin || self.pass().upper >= 0.0)
        {
            Decision::Pass
        } else {
            Decision::Inconclusive
        }
    }
}

// serde's flattened/untagged buffers see arbitrary-precision JSON decimals as
// Number maps. Value preserves that representation before conversion to f64.
pub(crate) fn decimal<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<f64, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    value
        .as_f64()
        .ok_or_else(|| serde::de::Error::custom("expected a decimal"))
}
