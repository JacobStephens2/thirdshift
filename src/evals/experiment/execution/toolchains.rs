//! Provision a repository's Rust pin on the host, once for both Trial workers.

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, ensure};

use crate::process::{self, Control, Interruption};

type PinResults = BTreeMap<(&'static str, Vec<u8>), Result<(), String>>;

#[derive(Default)]
pub(super) struct Toolchains {
    // Include the file name: only rust-toolchain accepts the legacy format.
    // Cache failures too, so retries and the other Arm see the same environment.
    pins: Mutex<PinResults>,
}

impl Toolchains {
    pub fn ensure(&self, repository: &Path) -> Result<()> {
        // rustup prefers rust-toolchain when both files exist. Let rustup own
        // parsing, profiles, components and targets rather than duplicating it.
        for name in ["rust-toolchain", "rust-toolchain.toml"] {
            let contents = match fs::read(repository.join(name)) {
                Ok(contents) => contents,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => return Err(error).context("could not read Trial Rust pin"),
            };
            let mut pins = self
                .pins
                .lock()
                .map_err(|_| anyhow!("Rust pin lock poisoned"))?;
            let key = (name, contents);
            if let Some(result) = pins.get(&key) {
                return result.clone().map_err(anyhow::Error::msg);
            }
            let result = install(repository);
            crate::interrupt::check()?;
            pins.insert(
                key,
                result
                    .as_ref()
                    .map(|_| ())
                    .map_err(|error| format!("{error:#}")),
            );
            return result;
        }
        Ok(())
    }
}

fn install(repository: &Path) -> Result<()> {
    crate::progress::step("Preparing Trial repository's pinned Rust toolchain");
    let output = process::output_until(
        Command::new("rustup")
            .args(["toolchain", "install", "--no-self-update"])
            .env_remove("RUSTUP_TOOLCHAIN")
            .current_dir(repository),
        None,
        Control {
            name: "Trial Rust toolchain installation",
            interruption: Interruption::Ordinary,
            stop: &|child| process::stop(child, &[libc::SIGTERM, libc::SIGKILL]),
        },
        Instant::now() + Duration::from_secs(900),
        &|| Ok(()),
    )
    .context("could not provision Trial Rust toolchain")?;
    ensure!(
        output.status.success(),
        "Trial Rust toolchain installation failed ({}): {}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
