//! Provision a repository's Rust pin on the host, once for both Trial workers.

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, ensure};

use crate::process::{self, Control, Interruption};

type PinResults = BTreeMap<(&'static str, Vec<u8>), Result<(), String>>;

pub(super) struct Toolchains {
    directory: PathBuf,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    // Include the file name: only rust-toolchain accepts the legacy format.
    // Cache failures too, so retries and the other Arm see the same environment.
    pins: PinResults,
    proxies: Option<tempfile::TempDir>,
}

impl Toolchains {
    pub fn new(directory: &Path) -> Self {
        Self {
            directory: directory.into(),
            state: Mutex::default(),
        }
    }

    pub fn ensure(&self, repository: &Path) -> Result<Option<PathBuf>> {
        // rustup prefers rust-toolchain when both files exist. Let rustup own
        // parsing, profiles, components and targets rather than duplicating it.
        for name in ["rust-toolchain", "rust-toolchain.toml"] {
            let contents = match fs::read(repository.join(name)) {
                Ok(contents) => contents,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => return Err(error).context("could not read Trial Rust pin"),
            };
            let mut state = self
                .state
                .lock()
                .map_err(|_| anyhow!("Rust pin lock poisoned"))?;
            let key = (name, contents);
            if let Some(result) = state.pins.get(&key) {
                result.clone().map_err(anyhow::Error::msg)?;
            } else {
                let result = install(repository).and_then(|()| {
                    if state.proxies.is_none() {
                        state.proxies = Some(proxies(&self.directory)?);
                    }
                    Ok(())
                });
                crate::interrupt::check()?;
                state.pins.insert(
                    key,
                    result
                        .as_ref()
                        .map(|_| ())
                        .map_err(|error| format!("{error:#}")),
                );
                result?;
            }
            return Ok(state.proxies.as_ref().map(|root| root.path().to_path_buf()));
        }
        Ok(None)
    }
}

/// Older Arms clear RUSTUP_HOME inside their own sandbox. Their existing
/// CARGO_HOME bind and PATH discovery still admit these read-only proxies,
/// which restore the installed root when the session invokes a Rust tool.
fn proxies(directory: &Path) -> Result<tempfile::TempDir> {
    let root = tempfile::Builder::new()
        .prefix("rust-toolchain-")
        .tempdir_in(directory)?;
    // Leave CARGO_HOME/bin absent. Old Arms add that directory before their
    // PATH discovery, whose duplicate check would keep the host shims ahead.
    let bin = root.path().join("session/bin");
    let dispatch = root.path().join("proxies");
    fs::create_dir_all(&bin)?;
    fs::create_dir(&dispatch)?;
    let rustup = process::find_executable("rustup").context("rustup is not on PATH")?;
    fs::copy(rustup, dispatch.join("rustup"))?;
    let installed = std::env::var_os("RUSTUP_HOME")
        .map(PathBuf::from)
        .unwrap_or(crate::config::home()?.join(".rustup"))
        .canonicalize()?;
    let installed = crate::args::shell_command(vec![
        installed
            .to_str()
            .context("Rust toolchain path must be UTF-8")?
            .into(),
    ]);
    for tool in [
        "rustup",
        "cargo",
        "rustc",
        "rustdoc",
        "rustfmt",
        "cargo-fmt",
        "cargo-clippy",
        "clippy-driver",
        "rust-analyzer",
        "cargo-miri",
        "rust-gdb",
        "rust-gdbgui",
        "rust-lldb",
        "rls",
    ] {
        if tool != "rustup" {
            symlink("rustup", dispatch.join(tool))?;
        }
        let target = crate::args::shell_command(vec![
            dispatch
                .join(tool)
                .to_str()
                .context("Rust proxy path must be UTF-8")?
                .into(),
        ]);
        let wrapper = bin.join(tool);
        fs::write(
            &wrapper,
            format!("#!/bin/sh\nexport RUSTUP_HOME={installed}\nexec {target} \"$@\"\n"),
        )?;
        fs::set_permissions(wrapper, fs::Permissions::from_mode(0o755))?;
    }
    Ok(root)
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
