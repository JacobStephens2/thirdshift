//! SIGINT, SIGTERM and SIGHUP: recorded rather than fatal, so an interrupted
//! Command or terminal Setup can return through normal cleanup.

use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};

static REQUESTED: OnceLock<Arc<AtomicBool>> = OnceLock::new();
static INSTALLED: OnceLock<std::io::Result<()>> = OnceLock::new();

/// Start recording SIGINT, SIGTERM and SIGHUP once for the process. Reusing
/// the installation neither clears a request nor adds duplicate actions.
pub fn install() -> Result<()> {
    INSTALLED
        .get_or_init(|| {
            let flag = REQUESTED.get_or_init(|| Arc::new(AtomicBool::new(false)));
            for signal in [SIGINT, SIGTERM, SIGHUP] {
                signal_hook::flag::register(signal, Arc::clone(flag))?;
            }
            Ok(())
        })
        .as_ref()
        .copied()
        .map_err(|error| anyhow::anyhow!("{error}"))
        .context("could not install the signal handler")
}

/// Propagate an observed request before retrying or committing settings.
pub fn check() -> Result<()> {
    if requested() {
        anyhow::bail!("interrupted");
    }
    Ok(())
}

/// Has the Command or terminal Setup been interrupted?
pub fn requested() -> bool {
    REQUESTED
        .get()
        .is_some_and(|flag| flag.load(Ordering::SeqCst))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sigint_sigterm_and_sighup_are_each_recorded_as_an_interrupt() {
        if let Ok(signal) = std::env::var("THIRDSHIFT_TEST_INTERRUPT") {
            install().unwrap();
            signal_hook::low_level::raise(signal.parse().unwrap()).unwrap();
            assert!(requested());
            install().unwrap();
            assert!(requested(), "reusing installation cleared interruption");
            assert_eq!(check().unwrap_err().to_string(), "interrupted");
            return;
        }
        for signal in [SIGINT, SIGTERM, SIGHUP] {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "interrupt::tests::sigint_sigterm_and_sighup_are_each_recorded_as_an_interrupt",
                ])
                .env("THIRDSHIFT_TEST_INTERRUPT", signal.to_string())
                .status()
                .unwrap();
            assert!(status.success(), "signal {signal}");
        }
    }
}
