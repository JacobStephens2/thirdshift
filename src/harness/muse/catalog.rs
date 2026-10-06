//! Muse's cached model catalog costs no model call to read.

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::fs;

pub struct Catalog {
    models: Vec<(String, String)>,
}

impl Catalog {
    pub fn parse(text: &str) -> Result<Self> {
        let json: Value =
            serde_json::from_str(text).context("Muse's cached catalog is not JSON")?;
        let rows = json["rows"]
            .as_array()
            .context("Muse's cached catalog has no rows")?;
        let mut models = Vec::new();
        for row in rows.iter().filter(|row| row["visibility"] != "hidden") {
            let id = row["model_id"]
                .as_str()
                .context("Muse's cached catalog has a row with no model_id")?;
            models.push((
                id.to_string(),
                row["display_label"].as_str().unwrap_or(id).to_string(),
            ));
        }
        Ok(Self { models })
    }

    pub fn read() -> Result<Option<Self>> {
        let Some(dir) = super::data_dir().map(|dir| dir.join("model-catalog")) else {
            return Ok(None);
        };
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("could not read {}", dir.display()));
            }
        };
        let mut catalog = None;
        for entry in entries {
            let path = entry?.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let parsed = Self::parse(
                &fs::read_to_string(&path)
                    .with_context(|| format!("could not read {}", path.display()))?,
            )?;
            catalog
                .get_or_insert_with(|| Self { models: Vec::new() })
                .models
                .extend(parsed.models);
        }
        Ok(catalog)
    }

    pub fn settle_model(&self, name: &str) -> Result<String> {
        self.models
            .iter()
            .find(|(id, label)| id.eq_ignore_ascii_case(name) || label.eq_ignore_ascii_case(name))
            .map(|(id, _)| id.clone())
            .with_context(|| {
                format!(
                    "the Model {name} is not in Muse's cached catalog: choose one of {}",
                    self.models
                        .iter()
                        .map(|(id, _)| id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
    }
}

pub const EFFORTS: [&str; 8] = [
    "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
];

/// Muse rejects unknown Efforts locally; do so before any test call here too.
pub fn settle_effort(effort: Option<&str>) -> Result<Option<String>> {
    let Some(effort) = effort else {
        return Ok(None);
    };
    if let Some(known) = EFFORTS
        .iter()
        .find(|known| known.eq_ignore_ascii_case(effort))
    {
        return Ok(Some((*known).to_string()));
    }
    bail!(
        "the Effort {effort} is not supported by Muse: choose one of {}",
        EFFORTS.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_models_match_by_id_or_label_regardless_of_case() {
        let catalog = Catalog::parse(
            r#"{"rows":[
            {"model_id":"muse-spark-1.3","display_label":"Muse Spark 1.3","visibility":"visible"},
            {"model_id":"muse-image-1.0","visibility":"hidden"}
        ]}"#,
        )
        .unwrap();
        for name in ["MUSE-SPARK-1.3", "muse spark 1.3"] {
            assert_eq!(catalog.settle_model(name).unwrap(), "muse-spark-1.3");
        }
        assert!(catalog.settle_model("muse-image-1.0").is_err());
        assert!(
            catalog
                .settle_model("bad-model")
                .unwrap_err()
                .to_string()
                .contains("choose one of muse-spark-1.3")
        );
    }

    #[test]
    fn malformed_catalogs_are_rejected() {
        for text in ["broken", "{}", "{\"rows\":[{}]}"] {
            assert!(Catalog::parse(text).is_err());
        }
    }
}
