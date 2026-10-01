//! SIGINT, SIGTERM and SIGHUP: recorded rather than fatal, so an interrupted
//! Run can still go through the Failed run path and clean up.

use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};

static REQUESTED: OnceLock<Arc<AtomicBool>> = OnceLock::new();

/// Start recording SIGINT, SIGTERM and SIGHUP instead of dying on them.
pub fn install() -> Result<()> {
    let flag = REQUESTED.get_or_init(|| Arc::new(AtomicBool::new(false)));
    for signal in [SIGINT, SIGTERM, SIGHUP] {
        signal_hook::flag::register(signal, Arc::clone(flag))
            .context("could not install the signal handler")?;
    }
    Ok(())
}

/// Has the Run been interrupted?
pub fn requested() -> bool {
    REQUESTED
        .get()
        .is_some_and(|flag| flag.load(Ordering::SeqCst))
}

/// Run `step`, and once more if it failed with the Run interrupted: Ctrl-C in
/// a terminal also kills the git or gh the step was running.
pub fn retry_if_interrupted(step: impl Fn() -> Result<()>) -> Result<()> {
    step().or_else(|error| if requested() { step() } else { Err(error) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sigint_sigterm_and_sighup_are_each_recorded_as_an_interrupt() {
        install().unwrap();
        let flag = REQUESTED.get().unwrap();
        for signal in [SIGINT, SIGTERM, SIGHUP] {
            flag.store(false, Ordering::SeqCst);

            signal_hook::low_level::raise(signal).unwrap();

            assert!(requested(), "signal {signal}");
        }
        flag.store(false, Ordering::SeqCst);
    }
}
