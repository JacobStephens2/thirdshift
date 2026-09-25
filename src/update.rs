//! `thirdshift update`: replace this copy of thirdshift with the latest stable
//! GitHub Release, through the install receipt the shell installer wrote.
//!
//! Only the `update` command calls this; a Run never checks for updates. The
//! release's installer moves the new binary into place with a rename, so a Run
//! still using the old binary keeps running it.

use anyhow::{Context, Result, anyhow, bail};
use axoupdater::{AxoUpdater, AxoupdateError};

const APP: &str = "thirdshift";

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// How to update the kinds of install that `update` won't touch.
const OTHER_INSTALLS: &str = concat!(
    "  built from source:  git pull && cargo install --path .\n",
    "  from crates.io:     cargo install thirdshift\n",
    "  copied by hand:     reinstall with curl -LsSf ",
    "https://github.com/JacobStephens2/thirdshift/releases/latest/download/thirdshift-installer.sh | sh",
);

/// Update to the latest stable release, or confirm this is it. Returns the
/// line that says which.
pub fn update() -> Result<String> {
    let mut updater = AxoUpdater::new_for(APP);
    match updater.load_receipt() {
        Ok(_) => {}
        Err(AxoupdateError::NoReceipt { .. }) => bail!(
            "this thirdshift has no install receipt, so the thirdshift installer didn't \
             install it and update won't replace it. To update it instead:\n{OTHER_INSTALLS}"
        ),
        Err(error) => return Err(anyhow!(error).context("can't read the install receipt")),
    }
    // Without this check, a copy from elsewhere would be reported as up to
    // date whenever the installer's copy is.
    if !updater.check_receipt_is_for_this_executable()? {
        bail!(
            "this thirdshift is not the copy the thirdshift installer put in {}, \
             so update won't replace it. To update it instead:\n{OTHER_INSTALLS}",
            updater.install_prefix_root()?
        );
    }
    // The running binary knows its version better than the receipt does.
    updater.set_current_version(VERSION.parse()?)?;
    // stdout carries nothing for `update`; the installer's stdout comes back
    // in the error if it fails.
    updater.disable_installer_stdout();
    let updated = updater
        .run_sync()
        .context("can't update from GitHub Releases")?;
    Ok(match updated {
        Some(result) => format!("updated thirdshift {VERSION} to {}", result.new_version),
        None => format!("thirdshift {VERSION} is the latest release"),
    })
}
