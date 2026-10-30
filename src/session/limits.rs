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
        if let Some(reset) = self
            .0
            .resets_at
            .as_deref()
            .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
        {
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
                .unwrap_or("unknown")
                .replace('_', "-");
            write!(f, ": {harness} {window} window")?;
            if let Some(used) = self.0.utilization {
                write!(f, " at {:.0}%", used * 100.0)?;
            }
            write!(
                f,
                ", resets {}",
                reset
                    .with_timezone(&chrono::Local)
                    .format("%a %b %-d %H:%M %Z")
            )?;
        }
        f.write_str("; run the Command again to resume")
    }
}

impl std::error::Error for Reached {}

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
