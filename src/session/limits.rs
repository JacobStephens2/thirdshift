//! Opt-in session stop policy shared by Eval callers. Harness decoders only
//! report quota evidence; they never signal or pause an Experiment themselves.

use std::path::Path;

/// The private signal keeps the versioned Trial request compatible with
/// older Arm builds. The Experiment also watches those builds' session logs.
pub(crate) const SIGNAL_ENV: &str = "THIRDSHIFT_EVAL_LIMIT_SIGNAL";

#[derive(Debug)]
pub(crate) struct Reached;

impl std::fmt::Display for Reached {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Command paused at a plan-limit warning; run the Command again to resume")
    }
}

impl std::error::Error for Reached {}

pub(crate) fn stop() -> anyhow::Result<()> {
    if let Some(path) = std::env::var_os(SIGNAL_ENV) {
        notify(Path::new(&path))?;
    }
    Err(Reached.into())
}

pub(crate) fn notify(signal: &Path) -> anyhow::Result<()> {
    std::fs::write(signal, "plan-limit warning\n")?;
    Ok(())
}

pub(crate) fn check(signal: &Path) -> anyhow::Result<()> {
    if signal.try_exists()? {
        return Err(Reached.into());
    }
    Ok(())
}
