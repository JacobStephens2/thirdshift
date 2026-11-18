//! Host capabilities for Visual evidence and the commands that repair them.
//! Probing never installs packages or contacts the network.

use std::os::fd::AsFd;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::{interrupt, process};

pub(super) const PASS: &str = "Visual evidence: PASS (gh >= 2.103.2, ffmpeg, Playwright Chromium)";
pub(super) const PROMPT: &str = "Install or upgrade Visual evidence tools now? [y/N] ";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gh {
    Missing,
    Outdated,
    Ready,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Platform {
    MacOs,
    Linux,
}

#[derive(Clone, Debug)]
pub(crate) struct Probe {
    pub gh: Gh,
    pub ffmpeg: bool,
    pub chromium: bool,
    pub platform: Platform,
}

impl Probe {
    pub fn ready(&self) -> bool {
        self.gh == Gh::Ready && self.ffmpeg && self.chromium
    }

    pub fn report(&self) -> String {
        if self.ready() {
            return PASS.into();
        }
        let mut missing = Vec::new();
        if self.gh != Gh::Ready {
            missing.push("gh >= 2.103.2 (missing, outdated or unreadable version)");
        }
        if !self.ffmpeg {
            missing.push("ffmpeg on PATH");
        }
        if !self.chromium {
            missing.push("Playwright Chromium executable");
        }
        format!("Visual evidence: WARN (need {})", missing.join(", "))
    }

    pub fn commands(&self) -> Vec<&'static str> {
        let mut commands = Vec::new();
        match self.platform {
            Platform::MacOs => {
                if self.gh != Gh::Ready {
                    commands.push("brew update");
                    commands.push(match self.gh {
                        Gh::Missing => "brew install gh",
                        Gh::Outdated => "brew upgrade gh",
                        Gh::Ready => unreachable!(),
                    });
                }
                if !self.ffmpeg {
                    commands.push("brew install ffmpeg");
                }
                if !self.chromium {
                    commands.push("brew install node");
                    commands.push("npx --yes --package=playwright playwright install chromium");
                }
            }
            Platform::Linux => {
                if !self.ready() {
                    commands.push("sudo apt update");
                }
                if self.gh != Gh::Ready {
                    // GitHub's repository carries versions newer than distro gh packages.
                    // https://github.com/cli/cli/blob/trunk/docs/install_linux.md
                    commands.push("sudo apt install -y curl ca-certificates");
                    commands.push("sudo install -m 0755 -d /etc/apt/keyrings && curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg | sudo tee /etc/apt/keyrings/githubcli-archive-keyring.gpg > /dev/null && sudo chmod go+r /etc/apt/keyrings/githubcli-archive-keyring.gpg && echo \"deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main\" | sudo tee /etc/apt/sources.list.d/github-cli.list > /dev/null && sudo apt update && sudo apt install -y gh");
                }
                if !self.ffmpeg {
                    commands.push("sudo apt install -y ffmpeg");
                }
                if !self.chromium {
                    commands.push("sudo apt install -y nodejs npm");
                    commands.push(
                        "npx --yes --package=playwright playwright install --with-deps chromium",
                    );
                }
            }
        }
        commands
    }
}

pub(super) fn probe(home: &Path) -> Probe {
    let gh = if process::find_executable("gh").is_none() {
        Gh::Missing
    } else {
        let output = process::output_until(
            Command::new("gh").arg("--version"),
            None,
            process::Control {
                name: "gh --version",
                interruption: process::Interruption::Ordinary,
                stop: &|child| process::stop(child, &[]),
            },
            Instant::now() + Duration::from_secs(5),
            &|| Ok(()),
        );
        if output.is_ok_and(|output| {
            output.status.success() && current_gh(&String::from_utf8_lossy(&output.stdout))
        }) {
            Gh::Ready
        } else {
            Gh::Outdated
        }
    };
    let platform = if cfg!(target_os = "macos") {
        Platform::MacOs
    } else {
        Platform::Linux
    };
    let cache = std::env::var_os("PLAYWRIGHT_BROWSERS_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| match platform {
            Platform::MacOs => home.join("Library/Caches/ms-playwright"),
            Platform::Linux => std::env::var_os("XDG_CACHE_HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| home.join(".cache"))
                .join("ms-playwright"),
        });
    Probe {
        gh,
        ffmpeg: process::find_executable("ffmpeg").is_some(),
        chromium: has_chromium(&cache),
        platform,
    }
}

fn current_gh(output: &str) -> bool {
    let Some(version) = output
        .strip_prefix("gh version ")
        .and_then(|s| s.split_whitespace().next())
    else {
        return false;
    };
    let mut parts = version.split('.');
    let mut numbers = [0_u64; 3];
    for number in &mut numbers {
        let Some(value) = parts.next().and_then(|s| s.parse().ok()) else {
            return false;
        };
        *number = value;
    }
    parts.next().is_none() && numbers >= [2, 103, 2]
}

fn has_chromium(cache: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(cache) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        (name.starts_with("chromium-") || name.starts_with("chromium_headless_shell-"))
            && [
                "chrome-linux/headless_shell",
                "chrome-mac/headless_shell",
                "chrome-linux/chrome",
                "chrome-linux64/chrome",
                "chrome-linux-arm64/chrome",
                "chrome-mac/Chromium.app/Contents/MacOS/Chromium",
                "chrome-mac-arm64/Chromium.app/Contents/MacOS/Chromium",
                "chrome-mac-x64/Chromium.app/Contents/MacOS/Chromium",
                "chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
                "chrome-mac-x64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
                "chrome-headless-shell-linux64/chrome-headless-shell",
                "chrome-headless-shell-linux-arm64/chrome-headless-shell",
                "chrome-headless-shell-mac-arm64/chrome-headless-shell",
                "chrome-headless-shell-mac-x64/chrome-headless-shell",
            ]
            .iter()
            .any(|path| process::is_executable(&entry.path().join(path)))
    })
}

/// Inherit the terminal so sudo and package managers can ask their own questions.
pub(super) fn install(command: &str) -> Result<()> {
    interrupt::install()?;
    interrupt::check()?;
    let mut child = Command::new("/bin/sh")
        .args(["-c", command])
        .process_group(0)
        .stdout(std::io::stderr().as_fd().try_clone_to_owned()?)
        .spawn()
        .with_context(|| format!("could not run {command}"))?;
    let terminal = match Foreground::take(child.id() as libc::pid_t) {
        Ok(terminal) => terminal,
        Err(error) => {
            process::stop(&mut child, &[]);
            return Err(error.into());
        }
    };
    // A child that read before the handoff may have received SIGTTIN.
    // SAFETY: the positive child id is the process group just created above.
    unsafe { libc::kill(-(child.id() as libc::pid_t), libc::SIGCONT) };
    let status = loop {
        if interrupt::requested() {
            process::stop(&mut child, &[libc::SIGTERM]);
            bail!("interrupted");
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(error) => {
                process::stop(&mut child, &[]);
                return Err(error.into());
            }
        }
    };
    drop(terminal);
    if status.signal().is_some() {
        bail!("installer interrupted: {status}");
    }
    if !status.success() {
        bail!("{command}: {status}");
    }
    Ok(())
}

/// Return terminal ownership to Setup even when an installer is interrupted.
struct Foreground(libc::pid_t);

impl Foreground {
    fn take(group: libc::pid_t) -> std::io::Result<Self> {
        // SAFETY: tcgetpgrp only inspects the inherited terminal descriptor.
        let previous = unsafe { libc::tcgetpgrp(libc::STDIN_FILENO) };
        if previous == -1 {
            return Err(std::io::Error::last_os_error());
        }
        Self::set(group)?;
        Ok(Self(previous))
    }

    fn set(group: libc::pid_t) -> std::io::Result<()> {
        // SAFETY: initialized signal sets and valid output pointers. Block
        // SIGTTOU on this thread during job-control changes, then restore its
        // mask; restoring from the background must not suspend Setup itself.
        unsafe {
            let mut blocked = std::mem::zeroed();
            let mut previous = std::mem::zeroed();
            libc::sigemptyset(&mut blocked);
            libc::sigaddset(&mut blocked, libc::SIGTTOU);
            let error = libc::pthread_sigmask(libc::SIG_BLOCK, &blocked, &mut previous);
            if error != 0 {
                return Err(std::io::Error::from_raw_os_error(error));
            }
            let changed = libc::tcsetpgrp(libc::STDIN_FILENO, group);
            let result = if changed == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            };
            libc::pthread_sigmask(libc::SIG_SETMASK, &previous, std::ptr::null_mut());
            result
        }
    }
}

impl Drop for Foreground {
    fn drop(&mut self) {
        let _ = Self::set(self.0);
    }
}
