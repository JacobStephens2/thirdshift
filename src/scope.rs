//! A memory cap for each Trial and each model Grader: its own systemd scope.
//!
//! Each Trial and each model Grader runs in a scope of its own, from
//! `systemd-run --user --scope`, with `MemoryHigh`, `MemoryMax` and
//! `MemorySwapMax` from the User config's `[trial]` section. The scope's own
//! process raises its `oom_score_adj` to the maximum first, so a system-wide
//! out-of-memory kill takes the Trial or Grader before the factory.
//!
//! A Trial the cap kills dies alone: the kernel's kill stays inside the
//! scope, and the Trial is void with `memory_cap`. That Void needs the outer
//! launcher's evidence, never the agent's word: an agent's own out-of-memory
//! error, with no kernel kill in the scope, stays a failure. The launcher
//! watches its scope's `memory.events` while the Trial runs, and reads the
//! user journal's `oom-kill` result for the scope after a suspicious exit,
//! which survives the scope itself. Either is enough.
//!
//! Scopes need the calling user's manager: one `loginctl enable-linger`
//! keeps it around after logouts, which cron and other logged-out launchers
//! need. Without a reachable user manager the Trial or Grader runs directly,
//! still with its raised `oom_score_adj`, and a warning names the linger
//! step. That is the documented fallback, not a silent uncapped run.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};

/// The User config's `[trial]` defaults. #1009 measured about 480 MiB for the
/// largest Harness (Claude Code with a very large context), about 200 MiB for
/// Codex, 150 MiB for Grok and 80 MiB for OpenCode, with 6.6 GiB available
/// and no build running. The throttle starts at about twice the largest
/// measurement, the kill at about four times it, so brief spikes slow down
/// instead of dying. Two workers at the kill cap still leave room for the
/// factory's own builds on an 8 GiB host, and per-scope swap stays within the
/// host's 4 GiB swap file.
pub const DEFAULT_MEMORY_HIGH: &str = "1G";
/// See [`DEFAULT_MEMORY_HIGH`].
pub const DEFAULT_MEMORY_MAX: &str = "2G";
/// See [`DEFAULT_MEMORY_HIGH`].
pub const DEFAULT_MEMORY_SWAP_MAX: &str = "1G";

/// What a scoped child raises its own `oom_score_adj` to: the maximum, so a
/// system-wide kill always takes the Trial or Grader before the factory,
/// whose score stays 0. Raising needs no privilege; only lowering does.
pub const OOM_SCORE_ADJ: i32 = 1000;

/// Environment overrides for the three caps, read after the User config.
/// Tests use these for a small cap; operators use the User config.
pub const ENV_MEMORY_HIGH: &str = "THIRDSHIFT_TRIAL_MEMORY_HIGH";
/// See [`ENV_MEMORY_HIGH`].
pub const ENV_MEMORY_MAX: &str = "THIRDSHIFT_TRIAL_MEMORY_MAX";
/// See [`ENV_MEMORY_HIGH`].
pub const ENV_MEMORY_SWAP_MAX: &str = "THIRDSHIFT_TRIAL_MEMORY_SWAP_MAX";

/// The memory caps for Trial and model Grader scopes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caps {
    pub high: String,
    pub max: String,
    pub swap_max: String,
}

impl Caps {
    /// The caps from the User config, with the test environment overrides
    /// applied over it. An invalid value fails loudly: silently running
    /// uncapped, or under a mistyped cap, would be worse.
    pub fn from_config_or_env(config: &crate::config::UserConfig) -> Result<Self> {
        let high = override_or(ENV_MEMORY_HIGH, &config.trial_memory_high)?;
        let max = override_or(ENV_MEMORY_MAX, &config.trial_memory_max)?;
        let swap_max = override_or(ENV_MEMORY_SWAP_MAX, &config.trial_memory_swap_max)?;
        let caps = Self {
            high,
            max,
            swap_max,
        };
        caps.validate()?;
        Ok(caps)
    }

    fn validate(&self) -> Result<()> {
        for (name, value) in [
            ("trial.memory_high", &self.high),
            ("trial.memory_max", &self.max),
            ("trial.memory_swap_max", &self.swap_max),
        ] {
            check_memory_value(name, value)?;
        }
        Ok(())
    }
}

/// `value` as systemd accepts a memory limit: a byte count with an optional
/// `K`, `M`, `G` or `T` suffix, or `infinity`.
pub fn check_memory_value(name: &str, value: &str) -> Result<()> {
    if value == "infinity" {
        return Ok(());
    }
    let digits = value.trim_end_matches(['K', 'M', 'G', 'T', 'k', 'm', 'g', 't']);
    if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Ok(());
    }
    anyhow::bail!(
        "{name} must be a byte count like \"512M\", \"2G\" or \"infinity\", not {value:?}"
    )
}

fn override_or(env: &str, configured: &str) -> Result<String> {
    match std::env::var(env) {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        Ok(_) => Ok(configured.to_string()),
        Err(std::env::VarError::NotPresent) => Ok(configured.to_string()),
        Err(error) => Err(error).with_context(|| format!("could not read {env}")),
    }
}

/// Whether `systemd-run --user --scope` can reach this user's manager here.
/// Cron and other logged-out launchers need lingering for this to hold.
pub fn available() -> bool {
    if find_systemd_run().is_none() {
        return false;
    }
    Command::new("systemctl")
        .args(["--user", "show", "-p", "SystemState", "--value"])
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Find `systemd-run` on `PATH`, if it is executable.
fn find_systemd_run() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join("systemd-run"))
        .find(|candidate| {
            candidate.is_file() && {
                use std::os::unix::fs::PermissionsExt;
                candidate
                    .metadata()
                    .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
            }
        })
}

static WARNED_FALLBACK: AtomicBool = AtomicBool::new(false);

/// Warn once that scopes are unavailable and Trials run directly.
fn warn_fallback() {
    if WARNED_FALLBACK.swap(true, Ordering::SeqCst) {
        return;
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "<user>".to_string());
    crate::progress::step(format_args!(
        "warning: Trials run without a memory cap: no systemd user manager is reachable; \
         run `loginctl enable-linger {user}` once and run the Experiment from a login session"
    ));
}

static UNIT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A scope name unique to this process, for `kind` Trial or Grader `detail`.
fn unit_name(kind: &str, detail: &str) -> String {
    let count = UNIT_COUNTER.fetch_add(1, Ordering::SeqCst);
    let detail: String = detail
        .chars()
        .map(|char| {
            if char.is_ascii_alphanumeric() || char == '-' || char == '_' {
                char
            } else {
                '_'
            }
        })
        .collect();
    format!(
        "thirdshift-{kind}-{}-{count}-{detail}.scope",
        std::process::id()
    )
}

/// A Trial or model Grader launch: its command and, when scoped, its scope.
pub struct Launch {
    pub command: Command,
    /// The scope the Trial or Grader runs in, when one was available.
    pub scope: Option<Scope>,
}

/// A scope a launcher created, named for `journalctl --user -u` after.
pub struct Scope {
    pub unit: String,
}

impl Scope {
    /// Whether the user journal recorded an `oom-kill` result for this
    /// scope: the manager's own line, which survives the scope itself. Only
    /// for suspicious exits; retries briefly, as the journal can lag the
    /// scope's end.
    pub fn journal_oom_killed(&self) -> bool {
        for _ in 0..3 {
            let output = Command::new("journalctl")
                .args(["--user", "-u", &self.unit, "--no-pager", "-o", "cat"])
                .output();
            if output.is_ok_and(|output| {
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .any(|line| line.contains("Failed with result 'oom-kill'"))
            }) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        false
    }
}

/// Watch `unit`'s `memory.events` while it runs: `None` where the scope is
/// already gone, as after a fast kill.
pub fn watch_scope(unit: &str) -> Option<Watch> {
    let group = control_group(unit)?;
    let events = events_path(&group);
    events.is_file().then_some(Watch {
        events,
        kernel_kill: false,
    })
}

/// Where `group`, a cgroup path as the manager reports it, keeps its
/// `memory.events`.
fn events_path(group: &str) -> PathBuf {
    // The manager reports an absolute path, which `join` would take for the
    // whole path; strip its slash to hang it under the cgroup mount.
    Path::new("/sys/fs/cgroup")
        .join(group.strip_prefix('/').unwrap_or(group))
        .join("memory.events")
}

/// A live watch on one scope's `memory.events`.
pub struct Watch {
    events: PathBuf,
    kernel_kill: bool,
}

impl Watch {
    /// Sample the scope's kernel kills. Best effort: a gone scope reads as no
    /// new evidence, never as an error.
    pub fn sample(&mut self) {
        if self.kernel_kill {
            return;
        }
        self.kernel_kill = kill_count(&self.events).is_some_and(|kills| kills > 0);
    }

    /// Whether any sample saw a kernel kill in the scope.
    pub fn kernel_kill_seen(&self) -> bool {
        self.kernel_kill
    }
}

/// This scope's kernel kills: `oom_kill` plus `oom_group_kill`, which counts
/// whole-cgroup kills separately. `None` where the scope has no readable
/// events.
fn kill_count(events: &Path) -> Option<u64> {
    let text = std::fs::read_to_string(events).ok()?;
    let mut kills = 0;
    let mut found = false;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        if matches!(fields.next(), Some("oom_kill" | "oom_group_kill"))
            && let Some(count) = fields.next().and_then(|count| count.parse::<u64>().ok())
        {
            kills += count;
            found = true;
        }
    }
    found.then_some(kills)
}

/// One scope's cgroup path, as the user manager reports it while the scope
/// runs. `None` once the scope is gone.
fn control_group(unit: &str) -> Option<String> {
    let output = Command::new("systemctl")
        .args(["--user", "show", unit, "-p", "ControlGroup", "--value"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let group = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!group.is_empty()).then_some(group)
}

/// `program` without a scope, still under the `--oom-exec` wrapper so the
/// Trial or Grader raises its own `oom_score_adj`. The wrapper `exec`s into
/// `program`, so signals, stdio, and exit status relay transparently.
fn direct<S: AsRef<std::ffi::OsStr>>(program: &Path, args: &[S]) -> Result<Command> {
    let mut command = Command::new(current_exe()?);
    command.arg("--oom-exec").arg(program).args(args);
    Ok(command)
}

/// `systemd-run --user --scope` with `unit` and the caps, before the `--`
/// that starts the scope's command.
fn scope_base(unit: &str, caps: &Caps) -> Command {
    let mut command = Command::new("systemd-run");
    command.args([
        "--user",
        "--scope",
        &format!("--unit={unit}"),
        &format!("--property=MemoryHigh={}", caps.high),
        &format!("--property=MemoryMax={}", caps.max),
        &format!("--property=MemorySwapMax={}", caps.swap_max),
        "--",
    ]);
    command
}

/// Build a Trial's command: scoped under the wrapper and its caps when a user
/// manager is reachable, direct otherwise. The wrapper raises the scope's own
/// `oom_score_adj`, which old Arm builds inherit; new ones raise it
/// themselves too.
pub fn trial_command(
    program: &Path,
    args: &[&str],
    trial: &str,
    attempt: usize,
    caps: &Caps,
) -> Result<Launch> {
    if !available() {
        warn_fallback();
        return Ok(Launch {
            command: direct(program, args)?,
            scope: None,
        });
    }
    let unit = unit_name("trial", &format!("{trial}-{attempt}"));
    let mut command = scope_base(&unit, caps);
    command
        .arg(current_exe()?)
        .arg("--oom-exec")
        .arg(program)
        .args(args);
    Ok(Launch {
        command,
        scope: Some(Scope { unit }),
    })
}

/// Build a model Grader Harness's command: the Harness under the wrapper and
/// the Trial caps when scoped, the Harness alone otherwise. The wrapper
/// raises the scope's own `oom_score_adj`, which the Harness cannot do for
/// itself.
pub fn grader_command(
    program: &Path,
    args: &[String],
    directory: &Path,
    caps: &Caps,
) -> Result<Launch> {
    if !available() {
        warn_fallback();
        let mut command = direct(program, args)?;
        command.current_dir(directory);
        return Ok(Launch {
            command,
            scope: None,
        });
    }
    let unit = unit_name("grader", "model");
    let mut command = scope_base(&unit, caps);
    command
        .arg(current_exe()?)
        .arg("--oom-exec")
        .arg(program)
        .args(args)
        .current_dir(directory);
    Ok(Launch {
        command,
        scope: Some(Scope { unit }),
    })
}

fn current_exe() -> Result<PathBuf> {
    std::env::current_exe().context("could not find the running thirdshift binary")
}

/// Raise this process's own `oom_score_adj` to [`OOM_SCORE_ADJ`], best
/// effort: raising needs no privilege, and where `/proc` has no such file
/// there is nothing to raise. Never lowers: a child that already scores
/// higher keeps its score.
pub fn raise_oom_score_adj() {
    const PATH: &str = "/proc/self/oom_score_adj";
    let Some(current) = std::fs::read_to_string(PATH)
        .ok()
        .and_then(|text| text.trim().parse::<i32>().ok())
    else {
        return;
    };
    if current < OOM_SCORE_ADJ && std::fs::write(PATH, OOM_SCORE_ADJ.to_string()).is_err() {
        crate::progress::step(format_args!(
            "warning: could not raise this scope's oom_score_adj to {OOM_SCORE_ADJ}"
        ));
    }
}

/// The `--oom-exec` body: raise this scope's own `oom_score_adj`, then become
/// `program`. `exec` keeps the raised score, the scope, and every stdio
/// descriptor, so the launcher sees `program` as it was.
pub fn oom_exec(program: &str, args: &[String]) -> Result<()> {
    use std::os::unix::process::CommandExt;
    raise_oom_score_adj();
    let error = Command::new(program).args(args).exec();
    Err(error).with_context(|| format!("could not run {program}"))
}

/// The cause a cap kill carries, in Trials and in Grader errors: the scope's
/// `memory.events` reported an `oom_kill`.
pub const OOM_KILL_CAUSE: &str = "scope memory.events reported an oom_kill";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_values_accept_systemd_byte_counts_and_infinity() {
        for value in ["512", "512M", "2G", "1T", "100K", "infinity"] {
            check_memory_value("trial.memory_max", value).unwrap();
        }
        for value in ["", "much", "1.5G", "-1", "G", "12X"] {
            assert!(
                check_memory_value("trial.memory_max", value).is_err(),
                "{value:?} must be refused"
            );
        }
    }

    #[test]
    fn events_paths_hang_absolute_manager_paths_under_the_cgroup_mount() {
        assert_eq!(
            events_path("/user.slice/user-1002.slice/app.slice/x.scope"),
            PathBuf::from(
                "/sys/fs/cgroup/user.slice/user-1002.slice/app.slice/x.scope/memory.events"
            )
        );
    }

    #[test]
    fn a_live_scope_resolves_its_memory_events() {
        if !available() {
            println!("SKIPPED (a live scope resolves its memory events): no systemd user manager");
            return;
        }
        let unit = unit_name("test", "watch");
        let mut child = Command::new("systemd-run")
            .args([
                "--user",
                "--scope",
                &format!("--unit={unit}"),
                "--",
                "sleep",
                "30",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("could not start a test scope");
        let mut watch = None;
        for _ in 0..30 {
            watch = watch_scope(&unit);
            if watch.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = Command::new("systemctl")
            .args(["--user", "stop", &unit])
            .output();
        let _ = child.wait();
        let mut watch = watch.expect("the live scope must resolve its memory.events");
        watch.sample();
        assert!(
            !watch.kernel_kill_seen(),
            "an idle scope reports no kernel kills"
        );
    }

    #[test]
    fn fallback_grader_command_keeps_a_raised_oom_score() {
        if super::available() {
            println!("SKIPPED: manager reachable, fallback untestable here");
            return;
        }
        let caps = super::Caps {
            high: "1G".to_string(),
            max: "2G".to_string(),
            swap_max: "1G".to_string(),
        };
        let launched = super::grader_command(
            std::path::Path::new("cat"),
            &["/proc/self/oom_score_adj".to_string()],
            std::path::Path::new("/tmp"),
            &caps,
        )
        .unwrap();
        assert!(launched.scope.is_none(), "needs the no-manager fallback");
        // The fallback routes through the scope wrapper, which raises its own
        // score before becoming the Grader; the wrapper itself is covered by
        // the live `--oom-exec` tests. This binary cannot exec itself here,
        // so the routing shape is the assertion.
        assert!(
            format!("{:?}", launched.command).contains("--oom-exec"),
            "a fallback Grader must run under the raising wrapper: {:?}",
            launched.command
        );
    }

    #[test]
    fn unit_names_stay_unique_and_systemd_safe() {
        let first = unit_name("trial", "abc--0-1");
        let second = unit_name("trial", "abc--0-1");
        assert_ne!(first, second);
        assert!(first.ends_with(".scope"));
        assert!(first.strip_suffix(".scope").is_some_and(|name| {
            name.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        }));
    }
}
