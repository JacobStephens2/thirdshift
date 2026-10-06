//! agy models lists model IDs and labels, with supported Efforts encoded
//! in ID suffixes. Bare model aliases require an explicit Effort.
use super::Agy;
use crate::harness::{Adapter, ModelAndEffort, said};
use anyhow::{Context, Result, bail};
use std::process::{Command, Stdio};

#[derive(Debug)]
struct Model {
    id: String,
    label: String,
}

#[derive(Debug)]
pub struct Catalog {
    models: Vec<Model>,
}

/// Effort suffixes agy knows, in increasing order.
const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

fn alias(id: &str) -> (&str, Option<&str>) {
    id.rsplit_once('-')
        .filter(|(_, suffix)| EFFORTS.contains(suffix))
        .map_or((id, None), |(base, suffix)| (base, Some(suffix)))
}

impl Catalog {
    pub fn read() -> Result<Self> {
        let output = Command::new(Agy.name())
            .arg("models")
            .envs(Agy.environment().iter().copied())
            .stdin(Stdio::null())
            .output()
            .context("could not run agy models to read Antigravity CLI's Models")?;
        if !output.status.success() {
            bail!(
                "agy models failed, so Antigravity CLI's Models can't be read: {}",
                said(&output)
            );
        }
        Self::parse(&String::from_utf8_lossy(&output.stdout))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let models: Vec<Model> = text
            .lines()
            .filter_map(|line| {
                let (id, label) = line.trim().split_once('\t')?;
                (!id.is_empty() && !label.trim().is_empty()).then(|| Model {
                    id: id.to_string(),
                    label: label.trim().to_string(),
                })
            })
            .collect();
        if models.is_empty() {
            bail!("agy models printed no catalog of Models");
        }
        Ok(Self { models })
    }

    pub fn listed(&self) -> Vec<String> {
        self.models
            .iter()
            .map(|model| format!("{} ({})", model.id, model.label))
            .collect()
    }

    /// Canonical ID, label or bare alias, matched regardless of case.
    pub fn model_name(&self, name: Option<&str>) -> Result<Option<String>> {
        let Some(name) = name else {
            return Ok(None);
        };
        for model in &self.models {
            if model.id.eq_ignore_ascii_case(name) || model.label.eq_ignore_ascii_case(name) {
                return Ok(Some(model.id.clone()));
            }
        }
        if let Some(base) = self
            .models
            .iter()
            .map(|model| alias(&model.id).0)
            .find(|base| base.eq_ignore_ascii_case(name))
        {
            return Ok(Some(base.to_string()));
        }
        bail!(
            "the Model {name} is not in Antigravity CLI's catalog: choose one of {}",
            self.models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    pub fn efforts(&self, model: Option<&str>) -> Vec<&str> {
        EFFORTS
            .into_iter()
            .filter(|effort| {
                self.models.iter().any(|candidate| {
                    let (base, suffix) = alias(&candidate.id);
                    suffix == Some(*effort) && model.is_none_or(|model| alias(model).0 == base)
                })
            })
            .collect()
    }

    pub fn settle(&self, model: Option<&str>, effort: Option<&str>) -> Result<ModelAndEffort> {
        let model = self.model_name(model)?;
        let efforts = self.efforts(model.as_deref());
        let settled_effort = match effort {
            Some(effort) => Some(
                *efforts
                    .iter()
                    .find(|valid| valid.eq_ignore_ascii_case(effort))
                    .with_context(|| {
                        format!(
                            "the Effort {effort} is not one {} supports: choose one of {}",
                            model.as_deref().unwrap_or("any Antigravity CLI Model"),
                            efforts.join(", ")
                        )
                    })?,
            ),
            None => None,
        };
        if let Some(model) = &model
            && !self.models.iter().any(|candidate| candidate.id == *model)
            && settled_effort.is_none()
        {
            bail!(
                "the Antigravity CLI Model {model} requires an Effort: choose one of {}",
                efforts.join(", ")
            );
        }
        Ok(ModelAndEffort {
            model,
            effort: settled_effort.map(String::from),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MODELS: &str = "Fetching available models...\ngemini-3.8-flash-high\tGemini 3.8 Flash (High)\ngemini-3.8-flash-medium\tGemini 3.8 Flash (Medium)\ngemini-3.8-flash-low\tGemini 3.8 Flash (Low)\ngemini-3.1-pro-high\tGemini 3.1 Pro (High)\ngemini-3.1-pro-low\tGemini 3.1 Pro (Low)\ngpt-oss-120b-medium\tGPT-OSS 120B (Medium)\n";

    #[test]
    fn ids_labels_and_aliases_settle_regardless_of_case() {
        let catalog = Catalog::parse(MODELS).unwrap();
        for name in ["GEMINI-3.8-FLASH-HIGH", "Gemini 3.8 Flash (HIGH)"] {
            assert_eq!(
                catalog.settle(Some(name), None).unwrap().model.as_deref(),
                Some("gemini-3.8-flash-high")
            );
        }
        let choice = catalog
            .settle(Some("Gemini-3.8-Flash"), Some("Medium"))
            .unwrap();
        assert_eq!(choice.model.as_deref(), Some("gemini-3.8-flash"));
        assert_eq!(choice.effort.as_deref(), Some("medium"));
        assert_eq!(
            catalog
                .settle(Some("GPT-OSS-120B"), Some("MEDIUM"))
                .unwrap()
                .model
                .as_deref(),
            Some("gpt-oss-120b")
        );
    }

    #[test]
    fn effort_choices_follow_the_model_family_and_missing_effort_is_caught() {
        let catalog = Catalog::parse(MODELS).unwrap();
        assert_eq!(
            catalog.efforts(Some("gemini-3.1-pro-high")),
            ["low", "high"]
        );
        assert!(
            catalog
                .settle(Some("gemini-3.1-pro"), Some("medium"))
                .unwrap_err()
                .to_string()
                .ends_with("choose one of low, high")
        );
        assert!(
            catalog
                .settle(Some("gemini-3.8-flash"), None)
                .unwrap_err()
                .to_string()
                .contains("requires an Effort")
        );
        assert_eq!(
            catalog
                .settle(None, Some("HIGH"))
                .unwrap()
                .effort
                .as_deref(),
            Some("high")
        );
        assert!(
            catalog
                .settle(Some("gemini-99"), None)
                .unwrap_err()
                .to_string()
                .contains("gemini-3.8-flash-high")
        );
        assert!(catalog.settle(None, Some("MAX")).is_err());
    }

    #[test]
    fn an_empty_or_unreadable_catalog_fails_instead_of_accepting_names() {
        for text in ["", "not a catalog", "\t\n", "model\t\n"] {
            assert!(Catalog::parse(text).is_err());
        }
        assert_eq!(
            Catalog::parse(MODELS).unwrap().listed()[0],
            "gemini-3.8-flash-high (Gemini 3.8 Flash (High))"
        );
    }
}
