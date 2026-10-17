//! Safe operational categories, derived only from GitHub response evidence.

use std::fmt;

#[derive(Debug)]
pub(super) struct QueryFailure {
    pub message: String,
    pub stderr: Option<String>,
}

impl fmt::Display for QueryFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for QueryFailure {}

/// Never return diagnostics: GitHub may include credentials or private data.
pub(crate) fn safe_cause(error: &anyhow::Error) -> Option<&'static str> {
    let stderr = if let Some(failure) = error.downcast_ref::<QueryFailure>() {
        failure.stderr.as_deref().unwrap_or_default()
    } else if let Some(failure) = error.downcast_ref::<super::check_read::ApiFailure>() {
        &failure.stderr
    } else {
        return None;
    };
    let stderr = stderr.to_ascii_lowercase();
    Some(
        if ["api rate limit exceeded", "api rate limit already exceeded"]
            .iter()
            .any(|message| stderr.contains(message))
        {
            "GitHub rate-limit exhaustion"
        } else {
            "GitHub dependency failure"
        },
    )
}
