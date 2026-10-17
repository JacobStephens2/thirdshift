//! Classification for the CI watch's read-only recovery policy. Unknown
//! failures remain causal errors, never successful empty observations.

use std::fmt;

/// A non-success API response, retaining gh's original diagnostics. This
/// carries evidence only: the CI watch owns whether and when to retry it.
#[derive(Debug)]
pub(super) struct ApiFailure {
    pub path: String,
    pub stderr: String,
}

impl fmt::Display for ApiFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gh api {} failed: {}", self.path, self.stderr)
    }
}

impl std::error::Error for ApiFailure {}

pub(crate) fn is_transient(error: &anyhow::Error) -> bool {
    if let Some(error) = error.downcast_ref::<serde_json::Error>() {
        return error.is_eof();
    }
    let Some(failure) = error.downcast_ref::<ApiFailure>() else {
        return false;
    };
    // gh reports HTTP status in stderr. Prefer it to incidental prose, so
    // an authentication or validation failure mentioning EOF stays permanent.
    if let Some(status) = failure
        .stderr
        .split("HTTP ")
        .skip(1)
        .find_map(|tail| tail.get(..3).and_then(|code| code.parse::<u16>().ok()))
    {
        return status == 429 || (500..600).contains(&status);
    }
    let stderr = failure.stderr.to_ascii_lowercase();
    [
        "unexpected end of json input",
        "unexpected eof",
        "error connecting to ",
        "connection reset",
        "connection refused",
        "network is unreachable",
        "no such host",
        "temporary failure in name resolution",
        "tls handshake timeout",
        "i/o timeout",
        "context deadline exceeded",
    ]
    .iter()
    .any(|message| stderr.contains(message))
}
