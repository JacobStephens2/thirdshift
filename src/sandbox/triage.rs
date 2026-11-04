//! Triage's opt-in host reads, with writes confined to its throwaway worktree.
//! A read-only mount still allows `connect` to a host Unix socket, so each
//! launch also loads a seccomp filter that refuses those sockets.

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
        seccomp: Some(host_socket_filter()?),
    };
    launch.set_env("TMPDIR", &tmp);
    prepare_home(&mut launch, &tmp.join("home"), harness)
        .context("Triage sandbox failed to prepare its writable Harness home")?;
    let mut probe_command = Command::new(bwrap);
    probe_command.args(&launch.argv);
    launch.pass_seccomp(&mut probe_command);
    probe_command.args(["--", "/bin/true"]);
    let probe = crate::process::output_until(
        &mut probe_command,
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

/// Refuse Unix-domain sockets. The read-only bind does not stop `connect` to
/// a host service (a session bus, systemd, or a container engine) that then
/// writes outside the worktree. TCP and connected stream socketpairs stay
/// available for runtimes. Datagram pairs are refused because their endpoints
/// can reconnect to host sockets. `io_uring_setup` is refused as well: a ring
/// can issue the blocked calls from a worker this filter does not see.
fn host_socket_filter() -> Result<Vec<u8>> {
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    {
        Ok(unix_socket_program())
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        anyhow::bail!(
            "Triage sandbox failed to start: Unix-socket confinement is not built for this architecture"
        )
    }
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn unix_socket_program() -> Vec<u8> {
    // linux/audit.h: EM_* with the 64-bit and little-endian flags. These are
    // the seccomp ABI values, not something libc exports.
    #[cfg(target_arch = "x86_64")]
    let arch: u32 = 0xC000_003E;
    #[cfg(target_arch = "aarch64")]
    let arch: u32 = 0xC000_00B7;
    let deny_arch = libc::SECCOMP_RET_ERRNO | (libc::EPERM as u32);
    let deny_unix = libc::SECCOMP_RET_ERRNO | (libc::EAFNOSUPPORT as u32);
    // Jump offsets skip that many following instructions. Zero falls through.
    // A wrong architecture returns EPERM for every syscall. io_uring_setup,
    // AF_UNIX sockets and datagram socketpairs return EAFNOSUPPORT.
    // Mask creation flags before comparing a socketpair's type.
    [
        sock_filter(LD_W_ABS, 0, 0, 4),
        sock_filter(JMP_JEQ_K, 1, 0, arch),
        sock_filter(RET_K, 0, 0, deny_arch),
        sock_filter(LD_W_ABS, 0, 0, 0),
        sock_filter(JMP_JEQ_K, 9, 0, libc::SYS_io_uring_setup as u32),
        sock_filter(JMP_JEQ_K, 5, 0, libc::SYS_socket as u32),
        sock_filter(JMP_JEQ_K, 1, 0, libc::SYS_socketpair as u32),
        sock_filter(RET_K, 0, 0, libc::SECCOMP_RET_ALLOW),
        sock_filter(LD_W_ABS, 0, 0, 24),
        sock_filter(
            (libc::BPF_ALU | libc::BPF_AND | libc::BPF_K) as u16,
            0,
            0,
            !(libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK) as u32,
        ),
        sock_filter(JMP_JEQ_K, 3, 2, libc::SOCK_DGRAM as u32),
        sock_filter(LD_W_ABS, 0, 0, 16),
        sock_filter(JMP_JEQ_K, 1, 0, libc::AF_UNIX as u32),
        sock_filter(RET_K, 0, 0, libc::SECCOMP_RET_ALLOW),
        sock_filter(RET_K, 0, 0, deny_unix),
    ]
    .concat()
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
const LD_W_ABS: u16 = (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16;
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
const JMP_JEQ_K: u16 = (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16;
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
const RET_K: u16 = (libc::BPF_RET | libc::BPF_K) as u16;

/// One classic BPF `sock_filter`, native endian: code, jt, jf, k.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn sock_filter(code: u16, jt: u8, jf: u8, k: u32) -> [u8; 8] {
    let mut bytes = [0; 8];
    bytes[..2].copy_from_slice(&code.to_ne_bytes());
    bytes[2] = jt;
    bytes[3] = jf;
    bytes[4..].copy_from_slice(&k.to_ne_bytes());
    bytes
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

#[cfg(test)]
mod review_regression {
    #[test]
    fn read_machine_blocks_socketpair_reconnection_to_host_service() {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixDatagram;
        const CHILD_PATH: &str = "THIRDSHIFT_REVIEW_SOCKETPAIR_HOST_PATH";
        if let Some(path) = std::env::var_os(CHILD_PATH) {
            let filter = super::host_socket_filter().unwrap();
            let program = libc::sock_fprog {
                len: (filter.len() / std::mem::size_of::<libc::sock_filter>()) as u16,
                filter: filter.as_ptr().cast_mut().cast(),
            };
            // This is an isolated test process. Install the exact production
            // filter without needing bwrap or unprivileged namespaces.
            assert_eq!(
                unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) },
                0
            );
            assert_eq!(
                unsafe { libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER, &program) },
                0,
                "{}",
                std::io::Error::last_os_error()
            );
            let (mut sender, mut receiver) = std::os::unix::net::UnixStream::pair().unwrap();
            sender.write_all(b"runtime").unwrap();
            let mut received = [0; 7];
            receiver.read_exact(&mut received).unwrap();
            assert_eq!(&received, b"runtime");
            if let Ok((endpoint, _peer)) = UnixDatagram::pair()
                && endpoint.connect(path).is_ok()
            {
                let _ = endpoint.send(b"delegate");
            }
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("host-service.sock");
        let marker = temp.path().join("host-wrote");
        let host = UnixDatagram::bind(&path).unwrap();
        host.set_nonblocking(true).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "sandbox::triage::review_regression::read_machine_blocks_socketpair_reconnection_to_host_service",
                "--nocapture",
            ])
            .env(CHILD_PATH, &path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let mut message = [0u8; 32];
        if let Ok(length) = host.recv(&mut message) {
            assert_eq!(&message[..length], b"delegate");
            std::fs::write(&marker, "written by host service").unwrap();
        }
        assert!(
            !marker.exists(),
            "production seccomp allowed a host-service write through socketpair(AF_UNIX, SOCK_DGRAM)"
        );
    }
}
