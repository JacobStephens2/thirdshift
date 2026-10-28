//! Retained Process identity. Missing fields preserve records from older binaries.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Identity {
    #[serde(
        rename = "process_name",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub name: Option<String>,
    #[serde(
        rename = "process_kind",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub kind: Option<Kind>,
    #[serde(
        rename = "process_hash",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub hash: Option<String>,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub(crate) enum Kind {
    #[serde(rename = "built-in")]
    BuiltIn,
    #[serde(rename = "custom")]
    Custom,
}

impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::BuiltIn => "built-in",
            Self::Custom => "custom",
        })
    }
}

impl Identity {
    pub fn built_in(file: &str, run_type: &str) -> Self {
        let source = super::catalog::compiled(file);
        Self {
            name: Some(if source.is_some() { file } else { run_type }.into()),
            kind: Some(Kind::BuiltIn),
            hash: source.map(hash),
        }
    }

    pub fn custom(name: &str, source: Option<&str>) -> Self {
        Self {
            name: Some(name.into()),
            kind: Some(Kind::Custom),
            hash: source.map(|text| hash(text.as_bytes())),
        }
    }
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
