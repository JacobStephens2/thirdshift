//! Opt-in session stop policy shared by Eval callers. Harness decoders only
//! report quota evidence; they never signal or pause an Experiment themselves.

pub(crate) use crate::harness::interpretation::limits::Limit;
use std::path::Path;

/// The private signal keeps the versioned Trial request compatible with
/// older Arm builds. The Experiment also watches those builds' session logs.
pub(crate) const SIGNAL_ENV: &str = "THIRDSHIFT_EVAL_LIMIT_SIGNAL";

#[derive(Debug)]
pub(crate) struct Reached(pub Limit);

impl std::fmt::Display for Reached {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Command paused at a plan-limit warning")?;
        if self.0.resets_at.is_some() {
            let harness = match self.0.harness.as_deref() {
                Some("claude") => "Claude",
                Some("codex") => "Codex",
                _ => "unknown Harness",
            };
            let window = self
                .0
                .window
                .as_deref()
                .and_then(crate::harness::interpretation::limits::operator_token)
                .unwrap_or("unknown");
            write_stop(f, harness, Some(window), &self.0)?;
        }
        f.write_str("; run the Command again to resume")
    }
}

/// `: <harness> <window> window at <n>%, resets <local time>`, leaving out
/// each fact `limit` does not know, as a paused Command's cause names what
/// stopped it.
fn write_stop(
    f: &mut std::fmt::Formatter<'_>,
    harness: &str,
    window: Option<&str>,
    limit: &Limit,
) -> std::fmt::Result {
    write!(f, ": {harness}")?;
    if let Some(window) = window {
        write!(f, " {} window", window.replace('_', "-"))?;
    }
    if let Some(used) = limit.utilization {
        write!(f, " at {:.0}%", used * 100.0)?;
    }
    if let Some(reset) = limit
        .resets_at
        .as_deref()
        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
    {
        write!(
            f,
            ", resets {}",
            reset
                .with_timezone(&chrono::Local)
                .format("%a %b %-d %H:%M %Z")
        )?;
    }
    Ok(())
}

impl std::error::Error for Reached {}

/// Why a Paused Command stopped, and a Pass skipped, as each says it.
pub(crate) const NO_ROUTE: &str = "no allowed route has available Plan usage";

/// A factory Command stopped because no route it may take has available
/// Plan usage: a Paused Command. Only structured Harness wire frames and
/// official non-billed reads build one, never agent-writable text, so an
/// agent cannot pause its own Run or release its Claim.
#[derive(Debug)]
pub(crate) struct PlanUsagePaused {
    /// The stopping Harness and Plan window, as far as they are known.
    pub limit: Limit,
    /// What stopped, when it was not this process's own session: a Spec
    /// run's Ticket.
    pub detail: Option<String>,
}

impl PlanUsagePaused {
    pub(crate) fn new(limit: Limit) -> Self {
        Self {
            limit: limit.accepted(),
            detail: None,
        }
    }
}

impl std::fmt::Display for PlanUsagePaused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(NO_ROUTE)?;
        if let Some(detail) = &self.detail {
            write!(f, " ({detail})")?;
        }
        if let Some(harness) = self.limit.harness.as_deref() {
            write_stop(f, harness, self.limit.window.as_deref(), &self.limit)?;
        }
        f.write_str("; run the Command again to continue once Plan usage allows")
    }
}

impl std::error::Error for PlanUsagePaused {}

/// The Plan-usage pause in `error`'s chain, if it ended a Paused Command.
pub(crate) fn plan_usage_paused(error: &anyhow::Error) -> Option<&PlanUsagePaused> {
    // A context layer is found through anyhow; a source deeper in the chain
    // through the standard one.
    error.downcast_ref::<PlanUsagePaused>().or_else(|| {
        error
            .chain()
            .find_map(|cause| cause.downcast_ref::<PlanUsagePaused>())
    })
}

pub(crate) fn stop(limit: Limit) -> anyhow::Result<()> {
    let limit = limit.accepted();
    if let Some(path) = std::env::var_os(SIGNAL_ENV) {
        notify(Path::new(&path), &limit)?;
    }
    Err(Reached(limit).into())
}

pub(crate) fn notify(signal: &Path, limit: &Limit) -> anyhow::Result<()> {
    let limit = limit.clone().accepted();
    // Publish one complete stopping event across competing Trial workers.
    let mut file = tempfile::NamedTempFile::new_in(signal.parent().unwrap())?;
    serde_json::to_writer(file.as_file_mut(), &limit)?;
    match file.persist_noclobber(signal) {
        Ok(_) => (),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

pub(crate) fn check(signal: &Path) -> anyhow::Result<()> {
    match std::fs::read(signal) {
        Ok(bytes) => {
            return Err(Reached(
                serde_json::from_slice::<Limit>(&bytes)
                    .unwrap_or_default()
                    .accepted(),
            )
            .into());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::check;

    #[test]
    fn a_poisoned_plan_limit_signal_cannot_forge_the_pause_message() {
        let dir = tempfile::tempdir().unwrap();
        let signal = dir.path().join("plan-limit");
        std::fs::write(
            &signal,
            br#"{"harness":"codex","window":"seven_day\recho paused-review","utilization":0.86,"resets_at":"2026-10-30T22:00:00Z"}"#,
        )
        .unwrap();
        let shown = check(&signal).unwrap_err().to_string();
        assert!(
            !shown.chars().any(char::is_control),
            "pause message kept a control character: {shown:?}"
        );
        assert!(
            !shown.contains("echo paused-review"),
            "pause message kept forged window text: {shown:?}"
        );
    }
}
