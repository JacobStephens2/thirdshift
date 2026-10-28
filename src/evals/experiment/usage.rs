//! Arm grouping and presentation of shared Session usage. Discovery and reads
//! belong to the captured Experiment reader.
use std::collections::BTreeMap;
use std::io::BufRead;

use anyhow::Result;
use serde::Serialize;

use super::Arm as TrialArm;
use crate::session::usage::{self, Measurement, Model, Summary};

#[derive(Default, Serialize)]
pub(crate) struct Arms {
    pub baseline: Arm,
    pub candidate: Arm,
}

#[derive(Default, Serialize)]
pub(crate) struct Arm {
    models: BTreeMap<String, Measurement>,
    pub cost_usd: Option<f64>,
}

impl Arms {
    pub(super) fn include(&mut self, arm: TrialArm, spans: impl BufRead) -> Result<()> {
        let arm = match arm {
            TrialArm::Baseline => &mut self.baseline,
            TrialArm::Candidate => &mut self.candidate,
        };
        arm.include(usage::from_spans(spans)?);
        Ok(())
    }

    pub fn baseline(&self) -> String {
        self.baseline.to_string()
    }
    pub fn candidate(&self) -> String {
        self.candidate.to_string()
    }
}

impl Arm {
    pub fn tokens(&self) -> Option<u64> {
        self.summary().tokens()
    }

    fn summary(&self) -> Summary {
        Summary {
            models: self
                .models
                .iter()
                .map(|(model, measurement)| Model {
                    model: Some(model.clone()),
                    measurement: *measurement,
                })
                .collect(),
            cost_usd: self.cost_usd,
        }
    }

    fn include(&mut self, mut session: Summary) {
        // Unknown identity keeps the Experiment's existing display/JSON key.
        // Project before grouping so a literal observed "unknown" shares it.
        for model in &mut session.models {
            model.model = Some(model.model.take().unwrap_or_else(|| "unknown".into()));
        }
        let mut summary = self.summary();
        summary.add(&session);
        self.cost_usd = summary.cost_usd;
        self.models = summary
            .models
            .into_iter()
            .map(|model| {
                (
                    model.model.unwrap_or_else(|| "unknown".into()),
                    model.measurement,
                )
            })
            .collect();
    }
}

impl std::fmt::Display for Arm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (model, measurement) in &self.models {
            write!(
                f,
                " · {model}: {} input, {} output tokens",
                measurement
                    .input_tokens
                    .map_or("unknown".into(), |v| v.to_string()),
                measurement
                    .output_tokens
                    .map_or("unknown".into(), |v| v.to_string())
            )?;
        }
        if let Some(cost) = self.cost_usd {
            write!(f, " · ${cost:.6}")?;
        }
        Ok(())
    }
}
