//! Grok's free model list and per-model Efforts from the cache it refreshes.

use std::fs;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::Grok;
use crate::harness::{Adapter, ModelAndEffort, said};

#[derive(Debug)]
struct Model {
    name: String,
    efforts: Vec<String>,
}

#[derive(Debug)]
pub struct Catalog {
    models: Vec<Model>,
    default: String,
}

impl Catalog {
    pub fn read() -> Result<Self> {
        let output = Command::new(Grok.name())
            .arg("models")
            .envs(Grok.environment().iter().copied())
            .stdin(Stdio::null())
            .output()
            .context("could not run grok models to read Grok Build's Models")?;
        if !output.status.success() {
            bail!(
                "grok models failed, so Grok Build's Models can't be read: {}",
                said(&output)
            );
        }
        // `grok models` prints names, but refreshes the effort metadata here.
        let home = match std::env::var_os("GROK_HOME").filter(|home| !home.is_empty()) {
            Some(home) => std::path::PathBuf::from(home),
            None => crate::config::home()?.join(".grok"),
        };
        let path = home.join("models_cache.json");
        let cache = fs::read_to_string(&path).with_context(|| {
            format!(
                "could not read Grok Build's Model and Effort catalog {} after grok models",
                path.display()
            )
        })?;
        Self::parse(&String::from_utf8_lossy(&output.stdout), &cache)
    }

    /// The text list and JSON cache recorded from Grok Build 1.0.46.
    pub fn parse(list: &str, cache: &str) -> Result<Self> {
        let cache: Value =
            serde_json::from_str(cache).context("Grok Build's Model cache is not JSON")?;
        let metadata = cache["models"]
            .as_object()
            .context("Grok Build's cache has no Models")?;
        let default = list
            .lines()
            .find_map(|line| line.trim().strip_prefix("Default model: "))
            .context("grok models printed no default Model")?
            .trim()
            .to_string();
        let available = list
            .split_once("Available models:")
            .context("grok models printed no list of Models")?
            .1;
        let mut models = Vec::new();
        for name in available.lines().filter_map(|line| {
            let line = line.trim();
            line.strip_prefix("* ")
                .or_else(|| line.strip_prefix("- "))?
                .split_whitespace()
                .next()
        }) {
            let info = metadata.get(name).context(format!(
                "Grok Build's cache has no metadata for Model {name}"
            ))?;
            let efforts = info["info"]["reasoning_efforts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|effort| effort["value"].as_str())
                .map(String::from)
                .collect();
            models.push(Model {
                name: name.to_string(),
                efforts,
            });
        }
        if models.is_empty() || !models.iter().any(|model| model.name == default) {
            bail!("grok models printed no available default Model");
        }
        Ok(Self { models, default })
    }

    pub fn listed(&self) -> Vec<&str> {
        self.models
            .iter()
            .map(|model| model.name.as_str())
            .collect()
    }

    fn model(&self, name: &str) -> Result<&Model> {
        self.models
            .iter()
            .find(|model| model.name.eq_ignore_ascii_case(name))
            .with_context(|| {
                format!(
                    "the Model {name} is not in Grok Build's catalog: choose one of {}",
                    self.listed().join(", ")
                )
            })
    }

    pub fn efforts(&self, model: Option<&str>) -> Vec<&str> {
        self.model(model.unwrap_or(&self.default))
            .map(|model| model.efforts.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }

    pub fn settle(&self, model: Option<&str>, effort: Option<&str>) -> Result<ModelAndEffort> {
        let found = self.model(model.unwrap_or(&self.default))?;
        let effort = match effort {
            None => None,
            Some(effort) => {
                let Some(valid) = found
                    .efforts
                    .iter()
                    .find(|valid| valid.eq_ignore_ascii_case(effort))
                else {
                    bail!(
                        "the Effort {effort} is not one the Grok Build Model {} supports: choose one of {}",
                        found.name,
                        found.efforts.join(", ")
                    );
                };
                Some(valid.clone())
            }
        };
        Ok(ModelAndEffort {
            model: model.map(|_| found.name.clone()),
            effort,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = include_str!("../../../tests/fixtures/grok-models.txt");
    const CACHE: &str = include_str!("../../../tests/fixtures/grok-models.json");

    #[test]
    fn recorded_models_have_their_own_efforts_and_default_model_is_checked_when_no_model_is_set() {
        let catalog = Catalog::parse(LIST, CACHE).unwrap();
        assert_eq!(
            catalog.listed(),
            ["grok-4.7", "grok-4.7-build-fast", "grok-4.6", "grok-4.5"]
        );
        assert_eq!(catalog.efforts(Some("grok-4.5")), ["high", "medium", "low"]);
        assert_eq!(
            catalog.settle(None, Some("XHIGH")).unwrap(),
            ModelAndEffort {
                model: None,
                effort: Some("xhigh".to_string())
            }
        );
        let older_default = LIST.replace("Default model: grok-4.7", "Default model: grok-4.5");
        let older = Catalog::parse(&older_default, CACHE).unwrap();
        assert!(
            older
                .settle(None, Some("xhigh"))
                .unwrap_err()
                .to_string()
                .contains("choose one of high, medium, low")
        );
    }

    #[test]
    fn an_unreadable_catalog_fails_instead_of_accepting_unchecked_settings() {
        for (list, cache) in [
            ("", CACHE),
            (LIST, "not json"),
            (LIST, "{}"),
            (LIST, "{\"models\": {}}"),
            (
                "Default model: missing\nAvailable models:\n - missing",
                CACHE,
            ),
        ] {
            assert!(Catalog::parse(list, cache).is_err(), "{list:?} {cache:?}");
        }
    }
}
