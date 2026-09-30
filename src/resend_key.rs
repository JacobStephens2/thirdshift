//! The Resend API key, and where it came from: `RESEND_API_KEY` when it is
//! set and not empty. The email sender and Setup both find it here, so they
//! can't disagree about whether there is one.

use anyhow::{Result, anyhow};

/// A Resend API key, and where it came from.
pub struct ResendKey {
    pub key: String,
    pub source: Source,
}

/// Where a Resend API key came from.
#[derive(Debug, PartialEq, Eq)]
pub enum Source {
    /// The `RESEND_API_KEY` environment variable.
    Environment,
}

impl ResendKey {
    /// The Resend API key, or `None` if there is none.
    pub fn find() -> Result<Option<Self>> {
        let key = std::env::var("RESEND_API_KEY")
            .ok()
            .filter(|key| !key.is_empty());
        Ok(key.map(|key| ResendKey {
            key,
            source: Source::Environment,
        }))
    }

    /// The Resend API key, or an error saying there is none.
    pub fn require() -> Result<Self> {
        ResendKey::find()?
            .ok_or_else(|| anyhow!("RESEND_API_KEY is unset or empty: set it to a Resend API key"))
    }
}
