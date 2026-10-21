//! A bounded capability check, not the sandbox runner for target code.
//! The skill contract and the subset checked here are pinned in tests/setup.rs.

use std::path::Path;
use std::process::ExitCode;

const CHILD: &str = "--thirdshift-setup-sandbox-child";
const PASSED: u8 = 42;
const NETWORK: u8 = 21;
const HOME: u8 = 22;
const SCRATCH: u8 = 23;

#[cfg(target_os = "linux")]
const REMEDIES: &str = "\
On Debian/Ubuntu, install bubblewrap:
  sudo apt install bubblewrap
If Ubuntu AppArmor blocks user namespaces, review this per-binary grant before applying it:
  sudo tee /etc/apparmor.d/bwrap <<'EOF'
abi <abi/4.0>,
include <tunables/global>
profile bwrap /usr/bin/bwrap flags=(unconfined) {
  userns,
  include if exists <local/bwrap>
}
EOF
  sudo apparmor_parser -r /etc/apparmor.d/bwrap
The profile grants user namespaces to /usr/bin/bwrap; use the installed binary's path if different.
Then rerun thirdshift setup. These commands are printed only; Setup never runs them.";

pub(super) fn report(config_path: &Path) -> String {
    #[cfg(target_os = "linux")]
    {
        match probe(config_path) {
            Ok(()) => "Security sandbox probe: PASS (sandbox start, network connection blocked, host home read-only, scratch writable); sessions must still enforce every audit skill control".into(),
            Err(error) => format!("Security sandbox probe: FAIL ({error:#})\n{REMEDIES}"),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = config_path;
        format!(
            "Security sandbox probe: unavailable on {}; validate equivalent platform isolation before executing target code",
            std::env::consts::OS
        )
    }
}

#[cfg(target_os = "linux")]
fn probe(config_path: &Path) -> anyhow::Result<()> {
    use anyhow::{Context, bail};
    use std::net::{TcpListener, TcpStream};
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    crate::interrupt::install()?;
    // A reachable host-only endpoint: no DNS, internet, curl or Python needed.
    let listener = TcpListener::bind("127.0.0.1:0").context("network isolation: host listener")?;
    let address = listener.local_addr()?;
    TcpStream::connect_timeout(&address, Duration::from_millis(250))
        .context("network isolation: host connection baseline")?;
    let scratch = tempfile::tempdir().context("scratch write: temporary directory")?;
    let scratch_path = scratch.path().canonicalize()?;
    let dummy_home = scratch_path.join("home");
    std::fs::create_dir(&dummy_home).context("scratch write: dummy HOME")?;
    let config_path = config_path
        .canonicalize()
        .context("host home read-only: saved User config")?;
    let executable = std::env::current_exe().context("sandbox start: probe executable")?;
    let mut command = Command::new("bwrap");
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .args([
            "--unshare-all",
            "--die-with-parent",
            "--new-session",
            "--ro-bind",
            "/",
            "/",
            "--dev",
            "/dev",
            "--proc",
            "/proc",
            "--bind",
        ])
        .arg(&scratch_path)
        .arg(&scratch_path)
        .args(["--clearenv", "--setenv", "HOME"])
        .arg(&dummy_home)
        .args(["--setenv", "TMPDIR"])
        .arg(&scratch_path)
        .arg("--chdir")
        .arg(&scratch_path)
        .arg("--")
        .arg(executable)
        .arg(CHILD)
        .arg(address.to_string())
        .arg(config_path)
        .arg(&scratch_path)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .context("sandbox start: could not launch bwrap from PATH")?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline && !crate::interrupt::requested() => {
                std::thread::sleep(Duration::from_millis(10));
            }
            waiting => {
                crate::process::stop(&mut child, &[]);
                waiting.context("sandbox start: waiting for bwrap")?;
                bail!("sandbox completion: timed out or interrupted (3 second limit)");
            }
        }
    };
    match status.code() {
        Some(code) if code == i32::from(PASSED) => Ok(()),
        Some(code) if code == i32::from(NETWORK) => {
            bail!("network isolation: host connection succeeded")
        }
        Some(code) if code == i32::from(HOME) => {
            bail!("host home read-only: write-open was not denied by a read-only filesystem")
        }
        Some(code) if code == i32::from(SCRATCH) => {
            bail!("scratch write: could not write inside scratch with a dummy HOME")
        }
        _ => bail!("sandbox start: bwrap did not complete the probe ({status})"),
    }
}

/// Private subprocess protocol. Only trusted built-in checks run here; no target
/// code or shell is involved, and a successful write-open never changes content.
pub(crate) fn child(args: &[String]) -> Option<ExitCode> {
    if args.first().map(String::as_str) != Some(CHILD) {
        return None;
    }
    let [_, address, config, scratch] = args else {
        return Some(ExitCode::FAILURE);
    };
    let address = match address.parse() {
        Ok(address) => address,
        Err(_) => return Some(ExitCode::FAILURE),
    };
    if std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_millis(250)).is_ok()
    {
        return Some(ExitCode::from(NETWORK));
    }
    let denied = std::fs::OpenOptions::new().write(true).open(config).err();
    if denied.and_then(|error| error.raw_os_error()) != Some(libc::EROFS) {
        return Some(ExitCode::from(HOME));
    }
    let scratch = Path::new(scratch);
    if std::env::var_os("HOME").as_deref() != Some(scratch.join("home").as_os_str())
        || std::fs::write(scratch.join("write-check"), b"probe").is_err()
    {
        return Some(ExitCode::from(SCRATCH));
    }
    Some(ExitCode::from(PASSED))
}
