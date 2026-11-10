//! Kimi Code's Model aliases and their providers, as `kimi provider list
//! --json` prints its config: `providers` and `models`, keyed by id. A
//! session names an alias with `-m`; the alias names its provider, whose base
//! URL the Trial proxy allows and whose `api_key_env` carries its key.

use std::collections::BTreeMap;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::Kimi;
use crate::harness::{Adapter, said};

/// The managed Kimi Code provider's API, when its config names no base URL.
const MANAGED_BASE_URL: &str = "https://api.kimi.com/coding/v1";

#[derive(Debug)]
pub struct Catalog {
    /// Each alias's entry, by alias, in Kimi's own order of ids.
    models: BTreeMap<String, Value>,
    providers: BTreeMap<String, Value>,
}

/// Where a Model's requests go, and the environment variable its key comes
/// from when the provider names one.
#[derive(Debug, PartialEq, Eq)]
pub struct Provider {
    pub base_url: String,
    pub api_key_env: Option<String>,
}

impl Catalog {
    pub fn read() -> Result<Self> {
        let output = crate::harness::process::output(
            &Kimi,
            Command::new(Kimi.name()).args(["provider", "list", "--json"]),
            None,
        )
        .context("could not run kimi provider list to read Kimi Code's Models")?;
        if !output.status.success() {
            bail!(
                "kimi provider list failed, so Kimi Code's Models can't be read: {}",
                said(&output)
            );
        }
        Self::parse(&String::from_utf8_lossy(&output.stdout))
    }

    /// The JSON Kimi Code 2.5.2 prints: camel-case provider and model
    /// entries, the same shape its `config.toml` loads into.
    pub fn parse(text: &str) -> Result<Self> {
        let config: Value =
            serde_json::from_str(text).context("kimi provider list printed no JSON")?;
        let table = |key: &str| -> Result<BTreeMap<String, Value>> {
            match &config[key] {
                Value::Null => Ok(BTreeMap::new()),
                Value::Object(entries) => Ok(entries
                    .iter()
                    .map(|(id, entry)| (id.clone(), entry.clone()))
                    .collect()),
                Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
                    bail!("kimi provider list printed no {key} table")
                }
            }
        };
        Ok(Self {
            models: table("models")?,
            providers: table("providers")?,
        })
    }

    /// Each alias a session can name with `-m`.
    pub fn listed(&self) -> Vec<&str> {
        self.models.keys().map(String::as_str).collect()
    }

    /// The alias `model` names, exactly, or none to leave it to Kimi Code's
    /// `default_model`. An unknown alias fails, naming the configured ones.
    pub fn settle(&self, model: Option<&str>) -> Result<Option<String>> {
        let Some(model) = model else {
            return Ok(None);
        };
        if self.models.contains_key(model) {
            return Ok(Some(model.to_string()));
        }
        if self.models.is_empty() {
            bail!(
                "the Model {model} is not in Kimi Code's config, which has no Models: \
                 add the provider and its Model with kimi provider, or sign in with kimi login"
            );
        }
        bail!(
            "the Model {model} is not in Kimi Code's config: choose one of {}",
            self.listed().join(", ")
        )
    }

    /// The provider the alias `model` sends its requests to.
    pub fn provider(&self, model: &str) -> Result<Provider> {
        let entry = self
            .models
            .get(model)
            .with_context(|| format!("the Model {model} is not in Kimi Code's config"))?;
        let id = entry["provider"]
            .as_str()
            .or_else(|| entry["providerId"].as_str());
        let provider = id.and_then(|id| self.providers.get(id));
        let text = |value: &Value| {
            value
                .as_str()
                .filter(|text| !text.is_empty())
                .map(String::from)
        };
        let managed = provider
            .is_some_and(|provider| provider["type"] == "kimi" || provider["oauth"].is_object())
            || entry["oauth"].is_object();
        let base_url = text(&entry["baseUrl"])
            .or_else(|| provider.and_then(|provider| text(&provider["baseUrl"])))
            .or_else(|| managed.then(|| MANAGED_BASE_URL.to_string()))
            .with_context(|| {
                format!("Kimi Code's config names no base URL for the Model {model}'s provider")
            })?;
        Ok(Provider {
            base_url,
            api_key_env: provider.and_then(|provider| text(&provider["apiKeyEnv"])),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Fireworks provider whose key comes from the environment, and the
    /// managed Kimi Code provider signed in with `kimi login`.
    fn catalog() -> Catalog {
        Catalog::parse(include_str!("../../../tests/fixtures/kimi-providers.json")).unwrap()
    }

    #[test]
    fn a_configured_alias_settles_as_itself_and_none_is_left_to_kimi() {
        assert_eq!(
            catalog().settle(Some("kimi-k3.1-pro")).unwrap().as_deref(),
            Some("kimi-k3.1-pro")
        );
        assert_eq!(catalog().settle(None).unwrap(), None);
    }

    #[test]
    fn an_unknown_alias_fails_naming_the_configured_ones() {
        assert_eq!(
            catalog().settle(Some("kimi-k9")).unwrap_err().to_string(),
            "the Model kimi-k9 is not in Kimi Code's config: choose one of kimi-code/kimi-for-coding, kimi-k3.1-pro"
        );
        assert!(
            Catalog::parse("{}")
                .unwrap()
                .settle(Some("kimi-k9"))
                .unwrap_err()
                .to_string()
                .contains("which has no Models")
        );
    }

    #[test]
    fn a_fireworks_alias_names_its_base_url_and_key_variable() {
        assert_eq!(
            catalog().provider("kimi-k3.1-pro").unwrap(),
            Provider {
                base_url: "https://api.fireworks.ai/inference/v1".into(),
                api_key_env: Some("FIREWORKS_API_KEY".into()),
            }
        );
    }

    #[test]
    fn the_managed_provider_without_a_base_url_uses_kimi_codes_api() {
        assert_eq!(
            catalog().provider("kimi-code/kimi-for-coding").unwrap(),
            Provider {
                base_url: MANAGED_BASE_URL.into(),
                api_key_env: None,
            }
        );
    }

    #[test]
    fn output_that_is_no_config_fails() {
        for output in ["", "not json", r#"{"models": 3}"#] {
            assert!(Catalog::parse(output).is_err(), "{output}");
        }
    }
}
