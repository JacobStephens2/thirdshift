//! Session ownership and Trial/Grader memory caps through systemd scopes.
//!
//! Each Trial and each Grader runs in a scope of its own, from
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
//! need. A logged-out launcher discovers the user's runtime directory when
//! its environment omits it. Without a reachable manager dispatch fails with
//! the linger step, before any uncapped Trial or Grader starts.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};

/// The User config's `[trial]` defaults. #1009 measured about 480 MiB for the
/// largest Harness (Claude Code with a very large context), about 200 MiB for
/// Codex, 150 MiB for Grok and 80 MiB for OpenCode, with 6.6 GiB available
/// and no build running. The throttle starts at about twice the largest
/// measurement, the kill at about four times it, so brief spikes slow down
/// instead of dying. A Trial's footprint beside two building Runs remains
/// unmeasured; these defaults use the available snapshot, not that live check.
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

/// Verified against current membership before a Trial's sequential sessions
/// borrow their dedicated capped scope. Never adopt a login scope.
const TRIAL_SCOPE: &str = "THIRDSHIFT_TRIAL_SCOPE";

/// The memory caps for Trial and Grader scopes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caps {
    pub high: String,
    pub max: String,
    pub swap_max: String,
}

impl Default for Caps {
    fn default() -> Self {
        Self {
            high: DEFAULT_MEMORY_HIGH.to_string(),
            max: DEFAULT_MEMORY_MAX.to_string(),
            swap_max: DEFAULT_MEMORY_SWAP_MAX.to_string(),
        }
    }
}

impl Caps {
    /// The caps from the User config, with the test environment overrides
    /// applied over it. An invalid value fails loudly: silently running
    /// uncapped, or under a mistyped cap, would be worse.
    pub fn with_env(&self) -> Result<Self> {
        let high = override_or(ENV_MEMORY_HIGH, &self.high)?;
        let max = override_or(ENV_MEMORY_MAX, &self.max)?;
        let swap_max = override_or(ENV_MEMORY_SWAP_MAX, &self.swap_max)?;
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
    let multiplier = match value.as_bytes().last() {
        Some(b'K' | b'k') => 1024_u64,
        Some(b'M' | b'm') => 1024_u64.pow(2),
        Some(b'G' | b'g') => 1024_u64.pow(3),
        Some(b'T' | b't') => 1024_u64.pow(4),
        _ => 1,
    };
    let digits = if multiplier == 1 {
        value
    } else {
        &value[..value.len() - 1]
    };
    if digits.bytes().all(|byte| byte.is_ascii_digit())
        && digits
            .parse::<u64>()
            .ok()
            .and_then(|bytes| bytes.checked_mul(multiplier))
            .is_some()
    {
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
    if !cfg!(target_os = "linux") {
        return false;
    }
    if crate::process::find_executable("systemd-run").is_none() {
        return false;
    }
    manager_command("systemctl")
        .args(["--user", "show", "-p", "SystemState", "--value"])
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Cron need not provide a login-session environment even with lingering on.
/// Fill in only a missing runtime directory; explicit bus settings still win.
fn manager_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    if cfg!(target_os = "linux")
        && std::env::var_os("XDG_RUNTIME_DIR").is_none_or(|value| value.is_empty())
    {
        // SAFETY: geteuid has no arguments or memory preconditions.
        let runtime = PathBuf::from(format!("/run/user/{}", unsafe { libc::geteuid() }));
        command.env("XDG_RUNTIME_DIR", runtime);
    }
    command
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

/// A Trial or Grader launch and its required memory scope.
pub struct Launch {
    command: Command,
    scope: Scope,
}

impl Launch {
    pub fn command(&mut self) -> &mut Command {
        &mut self.command
    }

    pub fn unit(&self) -> &str {
        self.scope.unit()
    }

    pub fn oom_killed(&self) -> bool {
        self.scope.oom_killed()
    }
}

impl crate::process::Invocation for Launch {
    fn command(&mut self) -> &mut Command {
        &mut self.command
    }

    fn parts(&mut self) -> (&mut Command, Option<&Scope>) {
        (&mut self.command, Some(&self.scope))
    }
}

/// A scope a launcher created, named for `journalctl --user -u` after.
pub struct Scope {
    unit: String,
    watch: RefCell<Option<Watch>>,
    observation: Box<dyn Observation>,
}

impl Scope {
    pub fn unit(&self) -> &str {
        &self.unit
    }

    pub(crate) fn processes(&self) -> Result<Vec<libc::pid_t>> {
        self.observation.processes(&self.unit)
    }

    pub(crate) fn signal(&self, signal: libc::c_int) -> Result<()> {
        self.observation.signal(&self.unit, signal)
    }

    pub(crate) fn borrows_trial(&self) -> bool {
        self.observation.borrows_trial()
    }

    /// Resolve and sample the live scope lazily: it only exists after launch.
    pub fn sample(&self) {
        let mut watch = self.watch.borrow_mut();
        if watch.is_none() {
            *watch = self.observation.events(&self.unit).map(|events| Watch {
                events,
                kernel_kill: false,
            });
        }
        if let Some(watch) = watch.as_mut() {
            watch.sample();
            if watch.kernel_kill_seen() {
                self.observation.acknowledge();
            }
        }
    }

    /// Inspect a suspicious completion, retaining live kernel evidence even
    /// after the scope disappears, then falling back to the manager's journal.
    pub fn oom_killed(&self) -> bool {
        self.watch
            .borrow()
            .as_ref()
            .is_some_and(Watch::kernel_kill_seen)
            || self.observation.journal_oom_killed(&self.unit)
    }
}

/// Native manager/filesystem access, substituted privately by deterministic
/// execution tests. Observation never participates in execution error policy.
trait Observation {
    fn events(&self, unit: &str) -> Option<PathBuf>;
    fn acknowledge(&self) {}
    fn journal_oom_killed(&self, unit: &str) -> bool;
    fn borrows_trial(&self) -> bool {
        false
    }
    // Memory-only fixtures own no real cgroup. Native observation supplies
    // the same scope's process ownership alongside its memory evidence.
    fn processes(&self, _unit: &str) -> Result<Vec<libc::pid_t>> {
        Ok(Vec::new())
    }
    fn signal(&self, _unit: &str, _signal: libc::c_int) -> Result<()> {
        Ok(())
    }
}

struct Native;

/// The supervisor stays inside its Trial cap; each sequential Harness session
/// owns only members added after its launch, including detached descendants.
struct TrialMembers {
    directory: PathBuf,
    protected: BTreeSet<libc::pid_t>,
}

impl Observation for TrialMembers {
    fn borrows_trial(&self) -> bool {
        true
    }

    fn events(&self, _unit: &str) -> Option<PathBuf> {
        Some(self.directory.join("memory.events"))
    }

    fn journal_oom_killed(&self, unit: &str) -> bool {
        Native.journal_oom_killed(unit)
    }

    fn processes(&self, _unit: &str) -> Result<Vec<libc::pid_t>> {
        let mut pids = Vec::new();
        read_group_processes(&self.directory, &mut pids)?;
        pids.retain(|pid| !self.protected.contains(pid));
        Ok(pids)
    }

    fn signal(&self, unit: &str, signal: libc::c_int) -> Result<()> {
        for pid in self.processes(unit)? {
            if let Err(error) = crate::process::signal(pid, signal)
                && error.raw_os_error() != Some(libc::ESRCH)
            {
                return Err(error).context("could not signal a Trial session survivor");
            }
        }
        Ok(())
    }
}

impl Observation for Native {
    fn processes(&self, unit: &str) -> Result<Vec<libc::pid_t>> {
        let Some(group) = read_control_group(unit)? else {
            return Ok(Vec::new());
        };
        let directory = events_path(&group).parent().unwrap().to_path_buf();
        let mut pids = Vec::new();
        read_group_processes(&directory, &mut pids)?;
        Ok(pids)
    }

    fn signal(&self, unit: &str, signal: libc::c_int) -> Result<()> {
        let output = manager_command("systemctl")
            .args([
                "--user",
                "kill",
                "--kill-who=all",
                &format!("--signal={signal}"),
                unit,
            ])
            .output()
            .context("could not signal the session scope")?;
        if !output.status.success() && !self.processes(unit)?.is_empty() {
            anyhow::bail!("could not signal session scope {unit}: {}", output.status);
        }
        Ok(())
    }

    fn events(&self, unit: &str) -> Option<PathBuf> {
        let events = events_path(&control_group(unit)?);
        events.is_file().then_some(events)
    }

    /// Whether the user journal recorded an `oom-kill` result for this
    /// scope: the manager's own line, which survives the scope itself. Only
    /// for suspicious exits; retries briefly, as the journal can lag the
    /// scope's end.
    fn journal_oom_killed(&self, unit: &str) -> bool {
        for _ in 0..3 {
            let output = manager_command("journalctl")
                .args(["--user", "-u", unit, "--no-pager", "-o", "cat"])
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
#[cfg(test)]
fn watch_scope(unit: &str) -> Option<Watch> {
    Some(Watch {
        events: Native.events(unit)?,
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
struct Watch {
    events: PathBuf,
    kernel_kill: bool,
}

impl Watch {
    /// Sample the scope's kernel kills. Best effort: a gone scope reads as no
    /// new evidence, never as an error.
    fn sample(&mut self) {
        if self.kernel_kill {
            return;
        }
        self.kernel_kill = kill_count(&self.events).is_some_and(|kills| kills > 0);
    }

    /// Whether any sample saw a kernel kill in the scope.
    fn kernel_kill_seen(&self) -> bool {
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
    read_control_group(unit).ok().flatten()
}

fn read_control_group(unit: &str) -> Result<Option<String>> {
    let output = manager_command("systemctl")
        .args([
            "--user",
            "show",
            unit,
            "-p",
            "ControlGroup",
            "-p",
            "LoadState",
        ])
        .output()
        .context("could not read the session scope")?;
    let properties = String::from_utf8_lossy(&output.stdout);
    if properties.lines().any(|line| line == "LoadState=not-found") {
        return Ok(None);
    }
    if !output.status.success() {
        anyhow::bail!("could not read scope {unit}: {}", output.status);
    }
    let group = properties
        .lines()
        .find_map(|line| line.strip_prefix("ControlGroup="))
        .context("scope response has no ControlGroup")?;
    Ok((!group.is_empty()).then(|| group.to_owned()))
}

fn read_group_processes(directory: &Path, pids: &mut Vec<libc::pid_t>) -> Result<()> {
    let contents = match std::fs::read_to_string(directory.join("cgroup.procs")) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).context("could not read the session cgroup's processes"),
    };
    for pid in contents.lines() {
        let pid: libc::pid_t = pid.parse().context("invalid cgroup process ID")?;
        if pid > 0 {
            pids.push(pid);
        }
    }
    match std::fs::read_dir(directory) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    read_group_processes(&entry.path(), pids)?;
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("could not inspect the session cgroup"),
    }
    Ok(())
}

/// `systemd-run --user --scope` with `unit` and the caps, before the `--`
/// that starts the scope's command.
fn scope_base(unit: &str, caps: Option<&Caps>) -> Command {
    // Resolve before a sandbox applies its own PATH to the command. That
    // PATH belongs to the Harness, not to the outer scope launcher.
    let program = crate::process::find_executable("systemd-run")
        .unwrap_or_else(|| PathBuf::from("systemd-run"));
    let mut command = manager_command(program);
    command.args([
        "--user",
        "--scope",
        "--quiet",
        "--collect",
        &format!("--unit={unit}"),
    ]);
    if let Some(caps) = caps {
        command.args([
            &format!("--property=MemoryHigh={}", caps.high),
            &format!("--property=MemoryMax={}", caps.max),
            &format!("--property=MemorySwapMax={}", caps.swap_max),
            // A killed tool must take its whole Trial or Grader with it, even
            // when its parent catches the failure or ignores SIGTERM.
            "--property=OOMPolicy=kill",
        ]);
    }
    command.arg("--");
    command
}

/// Ordinary sessions get ownership without Trial resource limits. Without a
/// user manager the shared execution owner uses the environment-marker sweep.
pub(crate) fn session_command(
    program: impl AsRef<std::ffi::OsStr>,
) -> Box<dyn crate::process::Invocation> {
    let program = program.as_ref();
    let mut invocation: Box<dyn crate::process::Invocation> = if available() {
        let unit = unit_name("session", "harness");
        let mut command = scope_base(&unit, None);
        command.arg(program);
        Box::new(Launch {
            command,
            scope: Scope {
                unit,
                watch: RefCell::new(None),
                observation: Box::new(Native),
            },
        })
    } else {
        Box::new(Command::new(program))
    };
    crate::process::session::mark(invocation.command());
    invocation
}

/// Metadata probes must still name the Harness after the scope launcher is
/// prepended. Trial launches also carry thirdshift's oom-exec wrapper.
/// The second value is the first underlying argument's index in `get_args`.
pub(crate) fn unwrapped_command(command: &Command) -> (&std::ffi::OsStr, usize) {
    let original = command.get_program();
    let mut args = command.get_args();
    if Path::new(original).file_name() != Some(std::ffi::OsStr::new("systemd-run")) {
        return (original, 0);
    }
    if args.find(|arg| *arg == "--").is_none() {
        return (original, 0);
    }
    let Some(mut program) = args.next() else {
        return (original, 0);
    };
    let mut index = command.get_args().len() - args.len();
    if args.next() == Some(std::ffi::OsStr::new("--oom-exec")) {
        program = args.next().unwrap_or(program);
        index = command.get_args().len() - args.len();
    }
    (program, index)
}

/// Keep Trial Harnesses in the existing cap and retain the outer launcher's
/// OOM evidence. Only a declared, kernel-confirmed dedicated scope is borrowed.
/// A standalone Trial can create its own scope unless that would bypass caps
/// imposed by its caller; those callers retain the marker fallback.
pub(crate) fn trial_session_command(
    program: impl AsRef<std::ffi::OsStr>,
) -> Box<dyn crate::process::Invocation> {
    let mut command = Command::new(program);
    crate::process::session::mark(&mut command);
    let inherited = (|| -> Result<Option<Scope>> {
        if !cfg!(target_os = "linux") {
            return Ok(None);
        }
        let Some(unit) = std::env::var(TRIAL_SCOPE)
            .ok()
            .filter(|unit| unit.starts_with("thirdshift-trial-") && unit.ends_with(".scope"))
        else {
            return Ok(None);
        };
        let groups = std::fs::read_to_string("/proc/self/cgroup")?;
        let Some(group) = groups.lines().find_map(|line| line.strip_prefix("0::")) else {
            return Ok(None);
        };
        if !Path::new(group)
            .components()
            .any(|component| component.as_os_str() == unit.as_str())
        {
            return Ok(None);
        }
        let directory = events_path(group).parent().unwrap().to_path_buf();
        let mut pids = Vec::new();
        read_group_processes(&directory, &mut pids)?;
        Ok(Some(Scope {
            unit,
            watch: RefCell::new(None),
            observation: Box::new(TrialMembers {
                directory,
                protected: pids.into_iter().collect(),
            }),
        }))
    })();
    match inherited {
        Ok(Some(scope)) => Box::new(Launch { command, scope }),
        Ok(None) => {
            if available() {
                match uncapped_current_cgroup() {
                    Ok(true) => return session_command(command.get_program()),
                    Ok(false) => {}
                    Err(error) => crate::progress::step(format_args!(
                        "warning: could not inspect the Trial's inherited caps: {error:#}; using the session marker"
                    )),
                }
            }
            Box::new(command)
        }
        Err(error) => {
            crate::progress::step(format_args!(
                "warning: could not adopt the Trial's session scope: {error:#}; using the session marker"
            ));
            Box::new(command)
        }
    }
}

fn uncapped_current_cgroup() -> Result<bool> {
    let groups = std::fs::read_to_string("/proc/self/cgroup")?;
    let group = groups
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .context("no unified cgroup for the Trial")?;
    let root = Path::new("/sys/fs/cgroup");
    let mut directory = events_path(group).parent().unwrap().to_path_buf();
    loop {
        for property in ["memory.high", "memory.max", "memory.swap.max"] {
            match std::fs::read_to_string(directory.join(property)) {
                Ok(value) if value.trim() != "max" => return Ok(false),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("could not read an inherited Trial cap"),
            }
        }
        if directory == root {
            return Ok(true);
        }
        if !directory.pop() || !directory.starts_with(root) {
            anyhow::bail!("Trial cgroup is outside the cgroup mount");
        }
    }
}

/// Build a Trial's command under the caps. The wrapper raises the scope's own
/// `oom_score_adj`, which old Arm builds inherit; new ones raise it
/// themselves too.
pub fn trial_command(
    program: &Path,
    args: &[&str],
    trial: &str,
    attempt: usize,
    caps: &Caps,
) -> Result<Launch> {
    let unit = unit_name("trial", &format!("{trial}-{attempt}"));
    let mut launched = launch(program, args, unit.clone(), caps)?;
    launched.command.env(TRIAL_SCOPE, unit);
    Ok(launched)
}

/// Build a code or model Grader's command under the Trial caps and the wrapper
/// that raises its own `oom_score_adj`.
pub fn grader_command(
    program: &Path,
    args: &[String],
    directory: &Path,
    caps: &Caps,
) -> Result<Launch> {
    #[cfg(test)]
    if let Some(launched) = testing::grader_command(program, args, directory) {
        return Ok(launched);
    }
    let unit = unit_name("grader", "check");
    let mut launched = launch(program, args, unit, caps)?;
    launched.command.current_dir(directory);
    Ok(launched)
}

fn launch<S: AsRef<std::ffi::OsStr>>(
    program: &Path,
    args: &[S],
    unit: String,
    caps: &Caps,
) -> Result<Launch> {
    if !available() {
        anyhow::bail!(
            "Trial and Grader memory scopes require a reachable systemd user manager; \
             run `loginctl enable-linger <user>` once and check `systemctl --user status`"
        );
    }
    let mut command = scope_base(&unit, Some(caps));
    command
        .arg(current_exe()?)
        .arg("--oom-exec")
        .arg(program)
        .args(args);
    crate::process::session::mark(&mut command);
    Ok(Launch {
        command,
        scope: Scope {
            unit,
            watch: RefCell::new(None),
            observation: Box::new(Native),
        },
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
pub(crate) mod testing;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_probes_the_manager_on_every_platform() {
        use std::os::unix::fs::PermissionsExt;

        const FIXTURE: &str = "THIRDSHIFT_SCOPE_AVAILABILITY_FIXTURE";
        if let Some(directory) = std::env::var_os(FIXTURE) {
            assert!(available(), "the fixture's manager is reachable");
            let _session = trial_session_command("unused");
            let manager = Path::new(&directory).join("systemctl");
            std::fs::write(&manager, "#!/bin/sh\nexit 1\n").unwrap();
            assert!(!available(), "an unreachable manager must still be refused");
            return;
        }

        let directory = tempfile::tempdir().unwrap();
        for name in ["systemd-run", "systemctl"] {
            let program = directory.path().join(name);
            std::fs::write(&program, "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "scope::tests::availability_probes_the_manager_on_every_platform",
                "--nocapture",
            ])
            .env(FIXTURE, directory.path())
            .env("PATH", directory.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr.contains("warning:"), "{stderr}");
    }

    #[test]
    fn memory_values_accept_systemd_byte_counts_and_infinity() {
        for value in ["512", "512M", "2G", "1T", "100K", "infinity"] {
            check_memory_value("trial.memory_max", value).unwrap();
        }
        for value in [
            "",
            "much",
            "1.5G",
            "-1",
            "G",
            "12X",
            "2GG",
            "512KK",
            "1MK",
            "18446744073709551616",
            "18446744073709551615T",
        ] {
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
        let mut child = manager_command("systemd-run")
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
        let _ = manager_command("systemctl")
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
