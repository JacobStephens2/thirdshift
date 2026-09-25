//! SIGINT, SIGTERM and SIGHUP: recorded rather than fatal, so an interrupted Run can
//! still go through the Failed run path and clean up.

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
