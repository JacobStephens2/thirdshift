//! Independent Grader choices and the population a Scoring run samples.
use std::num::NonZeroUsize;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::harness::{Choice, ChosenBy, Harness};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub owners: Vec<String>,
    pub sample_size: NonZeroUsize,
    pub harness: String,
    pub model: String,
    pub fallback_harness: String,
    pub fallback_model: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            owners: Vec::new(),
            sample_size: NonZeroUsize::new(10).unwrap(),
            harness: "claude".into(),
            model: String::new(),
            fallback_harness: "codex".into(),
            fallback_model: String::new(),
        }
    }
}

impl Settings {
    pub fn fallback_grader(&self) -> Choice {
        grader_choice(&self.fallback_harness, &self.fallback_model)
    }
    pub fn validate(&self) -> Result<()> {
        for owner in &self.owners {
            if owner.is_empty()
                || !owner
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                bail!("scoring.owners must contain GitHub owner names: {owner:?}");
            }
        }
        for (key, name) in [
            ("harness", &self.harness),
            ("fallback_harness", &self.fallback_harness),
        ] {
            if Harness::named(name).is_none() {
                bail!("scoring.{key} must name a Harness: {name:?}");
            }
        }
        Ok(())
    }

    pub fn grader(&self) -> Choice {
        grader_choice(&self.harness, &self.model)
    }
}

fn grader_choice(harness: &str, model: &str) -> Choice {
    Choice {
        harness: Harness::named(harness).expect("validated Scoring Harness"),
        model: (!model.trim().is_empty()).then(|| model.to_owned()),
        effort: None,
        chosen_by: ChosenBy::UserConfig,
    }
}
