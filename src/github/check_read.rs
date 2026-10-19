//! Classification for GitHub read-only recovery policies. Unknown
//! failures remain causal errors, never successful empty observations.

use std::fmt;
use std::process::ExitStatus;

/// A non-success API response, retaining gh's original diagnostics. This
/// carries evidence only: the caller owns whether and when to retry it.
#[derive(Debug)]
pub(super) struct ReadFailure {
    pub command: String,
    pub stderr: String,
    /// Private record diagnostics may contain evidence. Classify the raw
    /// stderr, but expose only this status in errors and retry progress.
    pub private_status: Option<ExitStatus>,
}

impl fmt::Display for ReadFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.private_status {
            Some(status) => write!(f, "{} failed ({status})", self.command),
            None => write!(f, "{} failed: {}", self.command, self.stderr),
        }
    }
}

impl std::error::Error for ReadFailure {}

pub(crate) fn is_transient(error: &anyhow::Error) -> bool {
    if let Some(error) = error.downcast_ref::<serde_json::Error>() {
        return error.is_eof();
    }
    let Some(failure) = error.downcast_ref::<ReadFailure>() else {
        return false;
    };
    let stderr = failure.stderr.trim().to_ascii_lowercase();
    // Quota exhaustion belongs to quota-aware deferral, not a short retry.
    // gh can report GraphQL quota errors with or without an HTTP status.
    if stderr.contains("api rate limit")
        || (stderr.contains("graphql") && stderr.contains("rate limit"))
        || stderr.contains("rate_limited")
    {
        return false;
    }
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
    if stderr == "eof" || stderr.ends_with(": eof") {
        return true;
    }
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
