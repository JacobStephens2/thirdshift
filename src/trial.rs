//! The hidden, versioned Trial protocol. Only session execution is shared
//! with a Run: a Trial has no Command, Claim, Worktree acquisition or Delivery.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::harness::interpretation::SessionFailure;
use crate::harness::interpretation::event::ErrorKind;
use crate::harness::{Choice, ChosenBy, Harness};
use crate::issue::IssueUrl;
use crate::session::{Kind, Logs, Purpose, Sessions};
use crate::{config::UserConfig, git::Git, interrupt, prompt, skills};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u64,
    repository: PathBuf,
    session_kind: SessionKind,
    inputs: Inputs,
    harness: String,
    model: Option<String>,
    effort: Option<String>,
    output_dir: PathBuf,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum SessionKind {
    SpecReview,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Inputs {
    pub issue_url: String,
    pub base: String,
    pub branch: String,
    pub pr_url: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Output {
    pub version: u64,
    pub outcome: Outcome,
    pub timings: Timings,
    pub void_cause: Option<VoidCause>,
    pub cause: Option<String>,
    pub session_id: Option<String>,
    pub log: Option<PathBuf>,
    pub model_evidence: Option<PathBuf>,
    pub reports: Reports,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    Succeeded,
    Failed,
    Void,
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Void => "void",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum VoidCause {
    Interrupted,
    SessionUnavailable,
    UsageLimit,
    RateLimit,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Timings {
    pub total_ms: u128,
    pub session_ms: u128,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Reports {
    pub standards: Option<PathBuf>,
    pub spec: Option<PathBuf>,
}

impl Output {
    pub fn timed_out(total_ms: u128) -> Self {
        Self {
            version: 1,
            outcome: Outcome::Failed,
            timings: Timings {
                total_ms,
                session_ms: total_ms,
            },
            void_cause: None,
            cause: Some("Trial timed out".into()),
            session_id: None,
            log: None,
            model_evidence: None,
            reports: Reports {
                standards: None,
                spec: None,
            },
        }
    }
}

/// Invalid requests are CLI failures. An accepted Trial returns exactly one
/// JSON line, including when its session fails; progress stays on stderr.
pub fn run(path: &Path) -> Result<String> {
    let started = Instant::now();
    let document: serde_json::Value = serde_json::from_slice(
        &fs::read(path).with_context(|| format!("could not read {}", path.display()))?,
    )
    .context("invalid Trial request")?;
    let version = document["version"]
        .as_u64()
        .context("Trial request version must be an unsigned integer")?;
    ensure!(
        version == 1,
        "unsupported Trial request version {version}; supported version: 1"
    );
    let request: Request = serde_json::from_value(document).context("invalid Trial request")?;
    ensure!(
        request.repository.is_absolute(),
        "Trial repository path must be absolute"
    );
    let harness = Harness::named(&request.harness).context("unknown Trial Harness")?;
    let issue = IssueUrl::parse(&request.inputs.issue_url)?;
    ensure!(
        request.repository.join(prompt::ISSUE_FILE).is_file(),
        "Trial repository must contain {}",
        prompt::ISSUE_FILE
    );
    let output_dir = prepare_output(&request.output_dir)?;
    let repository = request.repository.canonicalize()?;
    let choice = Choice {
        harness,
        model: request.model,
        effort: request.effort,
        chosen_by: ChosenBy::Command,
    };
    let (kind, prompt) = match request.session_kind {
        SessionKind::SpecReview => (
            Kind::SpecReview,
            prompt::spec_review(
                &issue,
                &request.inputs.base,
                &request.inputs.branch,
                &request.inputs.pr_url,
            ),
        ),
    };
    interrupt::install()?;
    let logs = Logs::of_trial(&output_dir);
    let session_started = Instant::now();
    let (ended, log) = Sessions::within(&logs, &repository, &choice, |sessions| {
        skills::exclude(&Git::new(&repository), &format!("/{}", prompt::ISSUE_FILE))?;
        sessions.run_to_end(Purpose::CodeReview, kind, &prompt)
    });
    skills::remove_written();
    let session_ms = session_started.elapsed().as_millis();
    let artifact = |extension| {
        log.as_ref()
            .map(|log| log.with_extension(extension))
            .filter(|path| path.is_file())
    };
    let recorded_session_id = || {
        let file = fs::File::open(log.as_ref()?).ok()?;
        let mut stream = choice
            .harness
            .adapter()
            .interpretation(&repository, &prompt)
            .without_retained();
        for line in BufReader::new(file).lines() {
            let Ok(line) = line else {
                break;
            };
            drop(stream.condense(&line));
        }
        stream.into_session_id()
    };
    let (outcome, session_id, cause, void_cause) = match ended {
        Ok(ended) => (Outcome::Succeeded, ended.session_id, None, None),
        Err(error) => {
            let void_cause = void_cause(&error);
            (
                if void_cause.is_some() {
                    Outcome::Void
                } else {
                    Outcome::Failed
                },
                error
                    .downcast_ref::<SessionFailure>()
                    .and_then(|failure| failure.session_id.clone())
                    .or_else(recorded_session_id),
                Some(format!("{error:#}")),
                void_cause,
            )
        }
    };
    let output = Output {
        version: request.version,
        outcome,
        timings: Timings {
            total_ms: started.elapsed().as_millis(),
            session_ms,
        },
        void_cause,
        cause,
        session_id,
        model_evidence: artifact("models.json"),
        reports: Reports {
            standards: artifact("standards"),
            spec: artifact("spec"),
        },
        log,
    };
    serde_json::to_string(&output).context("could not serialize Trial result")
}

fn void_cause(error: &anyhow::Error) -> Option<VoidCause> {
    if interrupt::requested() {
        return Some(VoidCause::Interrupted);
    }
    if error.is::<crate::session::limits::Reached>() {
        return Some(VoidCause::UsageLimit);
    }
    let Some(failure) = error.downcast_ref::<SessionFailure>() else {
        return Some(VoidCause::SessionUnavailable);
    };
    match failure.diagnostic.as_deref().map(ErrorKind::from_text) {
        Some(ErrorKind::UsageLimit) => Some(VoidCause::UsageLimit),
        Some(ErrorKind::RateLimit) => Some(VoidCause::RateLimit),
        _ => {
            let diagnostic = failure
                .diagnostic
                .as_deref()
                .unwrap_or("")
                .to_ascii_lowercase();
            let unavailable = [
                "not logged in",
                "login failed",
                "login expired",
                "authentication failed",
                "authentication required",
                "token expired",
                "invalid api key",
                "connection refused",
                "failed to connect",
                "could not connect",
                "network unreachable",
            ]
            .iter()
            .any(|text| diagnostic.contains(text));
            let startup = failure.before_first_turn
                && (diagnostic.is_empty()
                    || [
                        "server error",
                        "http 500",
                        "http 502",
                        "http 503",
                        "http 504",
                    ]
                    .iter()
                    .any(|text| diagnostic.contains(text)));
            (unavailable || startup).then_some(VoidCause::SessionUnavailable)
        }
    }
}

fn prepare_output(path: &Path) -> Result<PathBuf> {
    ensure!(path.is_absolute(), "Trial output_dir must be absolute");
    let output = resolved(path)?;
    let config = UserConfig::load()?;
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    for root in [
        config.logs_dir,
        PathBuf::from(home).join(".thirdshift/logs"),
    ] {
        ensure!(
            !output.starts_with(resolved(&root)?),
            "Trial output_dir must be outside factory logs"
        );
    }
    if output.exists() {
        ensure!(
            fs::read_dir(&output)?.next().transpose()?.is_none(),
            "Trial output_dir must be empty"
        );
    }
    fs::create_dir_all(&output).context("could not create Trial output directory")?;
    // Claim even an existing empty directory exclusively, so two Trials
    // cannot pass the emptiness check together and overwrite each other.
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join(".trial"))
        .context("Trial output_dir is already in use")?;
    Ok(output)
}

/// Resolve existing symlink prefixes and lexical parents before creating
/// anything, including when the configured logs root does not exist yet.
fn resolved(path: &Path) -> Result<PathBuf> {
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            component => {
                resolved.push(component);
                if resolved.exists() {
                    resolved = resolved.canonicalize()?;
                }
            }
        }
    }
    Ok(resolved)
}
