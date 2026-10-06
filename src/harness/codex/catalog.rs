//! Codex's free Model and Effort catalog.

use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use serde_json::Value;

use super::Codex;
use crate::harness::{Adapter, ModelAndEffort, said};

/// One Model in Codex's catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CatalogModel {
    /// The name Codex takes.
    slug: String,
    display_name: Option<String>,
    /// The Efforts it supports.
    efforts: Vec<String>,
    /// Whether the catalog lists it to choose from.
    listed: bool,
}

/// The Models Codex can run, as `codex debug models` prints them.
#[derive(Debug)]
pub struct Catalog {
    models: Vec<CatalogModel>,
}

impl Catalog {
    /// The catalog `codex debug models` prints, which costs no tokens to
    /// read.
    pub fn read() -> Result<Catalog> {
        let output = Command::new(Codex.name())
            .args(["debug", "models"])
            .envs(Codex.environment().iter().copied())
            .stdin(Stdio::null())
            .output()
            .context("could not run codex debug models to read Codex's Models")?;
        if !output.status.success() {
            bail!(
                "codex debug models failed, so Codex's Models can't be read: {}",
                said(&output)
            );
        }
        Catalog::parse(&String::from_utf8_lossy(&output.stdout))
    }

    /// The catalog `codex debug models` printed as `json`.
    pub fn parse(json: &str) -> Result<Catalog> {
        let parsed: Value = serde_json::from_str(json)
            .context("codex debug models printed no JSON catalog of Models")?;
        let models = parsed["models"]
            .as_array()
            .context("codex debug models printed no list of Models")?
            .iter()
            .filter_map(|model| {
                Some(CatalogModel {
                    slug: model["slug"].as_str()?.to_string(),
                    display_name: model["display_name"].as_str().map(String::from),
                    efforts: model["supported_reasoning_levels"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|level| level["effort"].as_str().map(String::from))
                        .collect(),
                    listed: model["visibility"] != "hide",
                })
            })
            .collect();
        Ok(Catalog { models })
    }

    /// The Models it lists to choose from, each as its slug and, where it
    /// has one, its display name, as in `gpt-6.1-sol (GPT-6.1-Sol)`.
    pub fn listed(&self) -> Vec<String> {
        self.models
            .iter()
            .filter(|model| model.listed)
            .map(|model| match &model.display_name {
                Some(display) => format!("{} ({display})", model.slug),
                None => model.slug.clone(),
            })
            .collect()
    }

    /// The Efforts the Model `slug` supports, or with no Model, those any
    /// Model supports, each once, in the catalog's order.
    pub fn efforts(&self, slug: Option<&str>) -> Vec<&str> {
        let mut efforts = Vec::new();
        let models = self
            .models
            .iter()
            .filter(|model| slug.is_none_or(|slug| model.slug == slug));
        for each in models.flat_map(|model| &model.efforts) {
            if !efforts.contains(&each.as_str()) {
                efforts.push(each.as_str());
            }
        }
        efforts
    }

    /// `model` and `effort` as Codex names them: the slug of the Model whose
    /// slug or display name is `model`, regardless of case, and the Effort,
    /// of those that Model supports, or with no Model of those any Model
    /// supports, that is `effort`, regardless of case. Fails naming the
    /// valid choices for a Model or Effort the catalog doesn't have.
    pub fn settle(&self, model: Option<&str>, effort: Option<&str>) -> Result<ModelAndEffort> {
        let found = match model {
            Some(model) => Some(self.model(model)?),
            None => None,
        };
        let Some(effort) = effort else {
            return Ok(ModelAndEffort {
                model: found.map(|found| found.slug.clone()),
                effort: None,
            });
        };
        let efforts = self.efforts(found.map(|found| found.slug.as_str()));
        let Some(settled) = efforts
            .iter()
            .find(|supported| supported.eq_ignore_ascii_case(effort))
        else {
            let of = match found {
                Some(found) => format!("the Codex Model {}", found.slug),
                None => "any Codex Model".to_string(),
            };
            bail!(
                "the Effort {effort} is not one {of} supports: choose one of {}",
                efforts.join(", ")
            );
        };
        Ok(ModelAndEffort {
            model: found.map(|found| found.slug.clone()),
            effort: Some(settled.to_string()),
        })
    }

    /// The Model whose slug or display name is `name`, regardless of case.
    fn model(&self, name: &str) -> Result<&CatalogModel> {
        let named = |candidate: &&CatalogModel| {
            candidate.slug.eq_ignore_ascii_case(name)
                || candidate
                    .display_name
                    .as_deref()
                    .is_some_and(|display| display.eq_ignore_ascii_case(name))
        };
        if let Some(found) = self.models.iter().find(named) {
            return Ok(found);
        }
        let listed: Vec<&str> = self
            .models
            .iter()
            .filter(|model| model.listed)
            .map(|model| model.slug.as_str())
            .collect();
        bail!(
            "the Model {name} is not in Codex's catalog: choose one of {}",
            listed.join(", ")
        )
    }
}
