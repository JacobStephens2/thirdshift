//! Triage's opt-in host reads, with writes confined to its throwaway worktree.

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};

use super::{Launch, find_bwrap, resolve_binary};
use crate::harness::Harness;

/// Copy the ordinary login and keep the network, but enforce the filesystem scope on
/// every launch, a Resume included. Trial sandbox opt-outs do not apply.
pub(crate) fn prepare(worktree: &Path, harness: Harness) -> Result<Launch> {
    let bwrap = find_bwrap().context("Triage sandbox failed to start: bwrap is not on PATH")?;
    let worktree = worktree.canonicalize()?;
    let mut argv: Vec<OsString> = ["--ro-bind", "/", "/", "--bind"]
        .map(OsString::from)
        .to_vec();
    argv.extend([
        worktree.as_os_str().to_owned(),
        worktree.as_os_str().to_owned(),
    ]);
    argv.extend(
        [
            "--unshare-user",
            "--unshare-pid",
            "--unshare-ipc",
            "--disable-userns",
            "--proc",
            "/proc",
            "--remount-ro",
            "/proc",
            "--dev",
            "/dev",
            "--die-with-parent",
            "--new-session",
            "--cap-drop",
            "ALL",
        ]
        .map(OsString::from),
    );
    let tmp = worktree.join(".thirdshift-tmp");
    std::fs::create_dir(&tmp)?;
    let mut launch = Launch {
        argv,
        env: Vec::new(),
        program: resolve_binary(harness.name())?,
        prefix: Vec::new(),
        stderr: None,
    };
    launch.set_env("TMPDIR", &tmp);
    prepare_home(&mut launch, &tmp.join("home"), harness)
        .context("Triage sandbox failed to prepare its writable Harness home")?;
    let probe = crate::process::output_until(
        Command::new(bwrap)
            .args(&launch.argv)
            .args(["--", "/bin/true"]),
        None,
        crate::process::Control {
            name: "Triage sandbox startup check",
            interruption: crate::process::Interruption::Ordinary,
            stop: &|child| crate::process::stop(child, &[libc::SIGTERM, libc::SIGKILL]),
        },
        Instant::now() + Duration::from_secs(5),
        &|| Ok(()),
    )
    .context("Triage sandbox failed to start")?;
    ensure!(
        probe.status.success(),
        "Triage sandbox failed to start ({}): {}",
        probe.status,
        String::from_utf8_lossy(&probe.stderr).trim(),
    );
    Ok(launch)
}

/// Harnesses write sessions and refresh logins. Give them private copies of
/// their settings and credentials; every such write stays in the worktree.
fn prepare_home(launch: &mut Launch, home: &Path, harness: Harness) -> Result<()> {
    fs::create_dir_all(home)?;
    fs::set_permissions(home, fs::Permissions::from_mode(0o700))?;
    let original = crate::config::home()?;
    let config = from_environment("XDG_CONFIG_HOME", original.join(".config"));
    let data = from_environment("XDG_DATA_HOME", original.join(".local/share"));
    let claude = from_environment("CLAUDE_CONFIG_DIR", original.join(".claude"));
    for (name, relative) in [
        ("HOME", ""),
        ("XDG_CONFIG_HOME", ".config"),
        ("XDG_DATA_HOME", ".local/share"),
        ("XDG_CACHE_HOME", ".cache"),
        ("XDG_STATE_HOME", ".local/state"),
    ] {
        let path = home.join(relative);
        fs::create_dir_all(&path)?;
        launch.set_env(name, &path);
    }
    for (source, dest) in [
        (original.join(".ssh"), home.join(".ssh")),
        (original.join(".gitconfig"), home.join(".gitconfig")),
        (config.join("gh"), home.join(".config/gh")),
    ] {
        link_if_present(&source, &dest)?;
    }
    if matches!(harness, Harness::Claude | Harness::Grok | Harness::Muse) {
        copy_files(
            &claude,
            &home.join(".claude"),
            &["CLAUDE.md", "settings.json"],
        )?;
        for relative in ["rules", "hooks", "skills", "plugins"] {
            link_if_present(&claude.join(relative), &home.join(".claude").join(relative))?;
        }
    }
    match harness {
        Harness::Claude => {
            copy_files(&original, home, &[".claude.json"])?;
            copy_files(&claude, &home.join(".claude"), &[".credentials.json"])?;
            launch.set_env("CLAUDE_CONFIG_DIR", &home.join(".claude"));
        }
        Harness::Codex => {
            let source = from_environment("CODEX_HOME", original.join(".codex"));
            let dest = home.join(".codex");
            copy_files(&source, &dest, &["auth.json", "config.toml", "AGENTS.md"])?;
            for relative in ["skills", "rules"] {
                link_if_present(&source.join(relative), &dest.join(relative))?;
            }
            launch.set_env("CODEX_HOME", &dest);
        }
        Harness::Agy => {
            copy_files(
                &original,
                home,
                &[
                    ".gemini/GEMINI.md",
                    ".gemini/settings.json",
                    ".gemini/antigravity-cli/antigravity-oauth-token",
                ],
            )?;
        }
        Harness::Grok => {
            copy_files(&original, home, &[".grok/auth.json", ".grok/settings.json"])?;
            for relative in ["rules", "hooks"] {
                link_if_present(
                    &original.join(".grok").join(relative),
                    &home.join(".grok").join(relative),
                )?;
            }
        }
        Harness::Muse => copy_files(
            &config,
            &home.join(".config"),
            &[
                "muse/auth.json",
                "muse/auth.json.lock",
                "muse/settings.json",
            ],
        )?,
        Harness::OpenCode => {
            copy_files(
                &config,
                &home.join(".config"),
                &[
                    "opencode/opencode.json",
                    "opencode/opencode.jsonc",
                    "opencode/AGENTS.md",
                ],
            )?;
            copy_files(&data, &home.join(".local/share"), &["opencode/auth.json"])?;
        }
    }
    Ok(())
}

fn from_environment(name: &str, default: PathBuf) -> PathBuf {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map_or(default, PathBuf::from)
}

fn copy_files(source: &Path, dest: &Path, paths: &[&str]) -> Result<()> {
    fs::create_dir_all(dest)?;
    for relative in paths {
        let source = source.join(relative);
        if source.exists() {
            let dest = dest.join(relative);
            fs::create_dir_all(dest.parent().expect("copied file has a parent"))?;
            fs::copy(&source, &dest)
                .with_context(|| format!("could not copy {}", source.display()))?;
        }
    }
    Ok(())
}

fn link_if_present(source: &Path, dest: &Path) -> Result<()> {
    if source.exists() {
        fs::create_dir_all(dest.parent().expect("linked file has a parent"))?;
        symlink(source, dest)?;
    }
    Ok(())
}
