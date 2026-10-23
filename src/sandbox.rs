//! A Trial session's sandbox: bwrap, its own processes, a fresh home and logins.
//!
//! A Trial's session runs in bwrap with a process namespace of its own, so it
//! sees neither an unbound path nor the host's processes, and everything it
//! starts ends with it. Only these are bound: the prepared repository, the
//! Harness's binary (Codex's whole release folder, since Codex looks for
//! `codex-code-mode-host` beside it), the Factory skills folder, a fresh home,
//! and the toolchains the session needs, read-only. The session otherwise
//! inherits the Trial's environment, with its home, path and logins set for
//! the sandbox. The network stays the host's here; #1013 gives the sandbox
//! its own.
//!
//! The fresh home holds the login, and the user-level instructions, rules,
//! settings and hooks a Run's session reads, those read-only. Harness flags
//! stay as ADR 0013 has them: the sandbox wraps the session, it does not
//! change its arguments.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};

use crate::harness::Harness;
use crate::resend_key::Credentials;
use crate::{config, interrupt};

/// The environment variable carrying an OpenCode Trial's provider key.
pub(crate) const OPENCODE_API_KEY_ENV: &str = "THIRDSHIFT_OPENCODE_API_KEY";
/// The environment variable carrying an OpenCode Trial's provider config.
const OPENCODE_CONFIG_ENV: &str = "OPENCODE_CONFIG_CONTENT";
/// Claude's subscription token variable.
const CLAUDE_TOKEN_ENV: &str = "CLAUDE_CODE_OAUTH_TOKEN";
/// Grok's per-Trial home, bound as the Trial's `~/.grok`.
const GROK_HOME_NAME: &str = ".grok";
/// The Trials Grok home's login files, kept across the wipe before each Trial.
const GROK_KEPT: [&str; 2] = ["auth.json", "auth.json.lock"];

/// A sandbox that could not start: bwrap missing, a bind source unusable, or
/// the Harness binary not found. A Trial that fails this way is void with
/// `sandbox_unavailable`, like a sandbox that could not start mid-session.
#[derive(Debug)]
pub(crate) struct Unavailable {
    diagnostic: String,
}

impl std::fmt::Display for Unavailable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "sandbox failed to start: {}", self.diagnostic)
    }
}

impl std::error::Error for Unavailable {}

/// A Trial whose Harness has no login: a missing Grok Trials-home login or a
/// missing OpenCode provider key. A Trial that fails this way is void with
/// `session_unavailable`, like a login that fails mid-session.
#[derive(Debug)]
pub(crate) struct NoLogin {
    diagnostic: String,
}

impl std::fmt::Display for NoLogin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Trial login failed: {}", self.diagnostic)
    }
}

impl std::error::Error for NoLogin {}

/// The Trial secrets a sandbox may pass to its session. Ordinary Runs never
/// receive them.
#[derive(Default)]
pub(crate) struct Secrets {
    pub claude_setup_token: Option<String>,
    pub opencode_api_key: Option<String>,
}

impl From<&Credentials> for Secrets {
    fn from(credentials: &Credentials) -> Self {
        Secrets {
            claude_setup_token: credentials.claude_setup_token(),
            opencode_api_key: credentials.opencode_api_key(),
        }
    }
}

/// What a sandbox is prepared from. Paths are absolute; `home` is the
/// production home the logins come from, `fresh_home` the host directory the
/// launcher prepares as the session's home.
pub(crate) struct Request<'a> {
    pub harness: Harness,
    pub model: Option<&'a str>,
    pub repository: &'a Path,
    pub skills_dir: &'a Path,
    pub harness_binary: &'a Path,
    pub home: &'a Path,
    pub fresh_home: &'a Path,
    pub secrets: Secrets,
    pub adapter_env: Vec<(&'a str, &'a str)>,
}

/// A prepared sandbox: its bwrap arguments up to `--`, and the Harness's
/// in-sandbox path. A held Grok Trial keeps its Trials-home lock until this
/// is dropped.
pub(crate) struct Prepared {
    argv: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    program: PathBuf,
    _grok: Option<GrokGuard>,
}

/// Prepare a Trial's sandbox: the Harness's fixed binary, the Factory skills
/// folder, the production home, a fresh home under its output directory, and
/// the Trial secrets the Credentials hold, if they hold any. A broken
/// Credentials file leaves the secrets empty rather than blocking a Trial
/// whose Harness needs none; an OpenCode Trial without its key still fails
/// before its session starts.
///
/// When `THIRDSHIFT_TRIAL_NO_SANDBOX` is set and not empty, there is no
/// sandbox: the Trial's sessions run as they did before it. The Experiment
/// suite sets it so its fake Harnesses stay observable at fixed paths, and
/// it is the emergency escape when bwrap breaks on a machine.
pub(crate) fn for_trial(
    choice: &crate::harness::Choice,
    repository: &Path,
    output_dir: &Path,
) -> Result<Option<Prepared>> {
    if std::env::var_os("THIRDSHIFT_TRIAL_NO_SANDBOX").is_some_and(|flag| !flag.is_empty()) {
        return Ok(None);
    }
    let home = config::home().map_err(|error| {
        anyhow::Error::new(Unavailable {
            diagnostic: format!("HOME is not set: {error:#}"),
        })
    })?;
    let secrets = Credentials::read()
        .map(|credentials| Secrets::from(&credentials))
        .unwrap_or_default();
    prepare(&Request {
        harness: choice.harness,
        model: choice.model.as_deref(),
        repository,
        skills_dir: &crate::skills::written_out()?,
        harness_binary: &resolve_binary(choice.harness.name())?,
        home: &home,
        fresh_home: &output_dir.join("home"),
        secrets,
        adapter_env: choice.harness.adapter().environment().to_vec(),
    })
    .map(Some)
}

impl Prepared {
    /// What a session runner holds to launch inside the sandbox. The guard
    /// stays with the `Prepared` it came from, which outlives the sessions.
    pub(crate) fn launch(&self) -> Launch {
        Launch {
            argv: self.argv.clone(),
            env: self.env.clone(),
            program: self.program.clone(),
        }
    }
}

/// How one session is launched inside a prepared sandbox.
#[derive(Clone)]
pub(crate) struct Launch {
    pub argv: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    pub program: PathBuf,
}

impl Launch {
    /// The bwrap command running the Harness's `args` inside the sandbox.
    /// The caller sets the working directory, which must be the bound
    /// repository. The session inherits the caller's environment, with the
    /// sandbox's overrides applied over it.
    pub fn command(&self, args: &[String]) -> Command {
        let mut command = Command::new("bwrap");
        command.args(&self.argv);
        command.arg("--");
        command.arg(&self.program);
        command.args(args);
        command.envs(self.env.iter().map(|(name, value)| (name, value)));
        command
    }
}

/// Find bwrap on `PATH`, if it is executable.
pub(crate) fn find_bwrap() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join("bwrap"))
        .find(|path| is_executable(path))
}

/// Resolve the Harness CLI named `name` the way a session launch would: the
/// first executable on `PATH`, canonicalized so the sandbox binds its real
/// file rather than a link.
pub(crate) fn resolve_binary(name: &str) -> Result<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let found = std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|path| is_executable(path))
        .ok_or_else(|| {
            anyhow::Error::new(Unavailable {
                diagnostic: format!("{name} is not on PATH"),
            })
        })?;
    found.canonicalize().map_err(|error| {
        anyhow::Error::new(Unavailable {
            diagnostic: format!("could not resolve {}: {error:#}", found.display()),
        })
    })
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

/// Prepare a Trial sandbox as `request` says: wipe and bind the homes, bind
/// the repository, skills, Harness and toolchains, and set the session's
/// environment. Fails before any session starts when bwrap is missing, the
/// Grok Trials home has no login, or an OpenCode Trial has no provider key.
pub(crate) fn prepare(request: &Request<'_>) -> Result<Prepared> {
    ensure!(
        request.repository.is_absolute(),
        "sandbox failed to start: Trial repository path must be absolute"
    );
    ensure!(
        request.home.is_absolute(),
        "sandbox failed to start: home path must be absolute"
    );
    if find_bwrap().is_none() {
        return Err(anyhow::Error::new(Unavailable {
            diagnostic: "bwrap is not on PATH".into(),
        }));
    }
    let binary = request.harness_binary.canonicalize().with_context(|| {
        format!(
            "sandbox failed to start: could not resolve {}",
            request.harness_binary.display()
        )
    })?;
    fs::create_dir_all(request.fresh_home).with_context(|| {
        format!(
            "sandbox failed to start: could not create {}",
            request.fresh_home.display()
        )
    })?;
    let mut binds = Binds::default();
    // The prepared repository, read-write, at its own path so the session's
    // working directory is unchanged.
    binds.read_write(request.repository, request.repository);
    binds.read_only(request.skills_dir, request.skills_dir);
    for toolchain in toolchains(request.home) {
        binds.read_only(&toolchain, &toolchain);
    }
    let mut environment = Environment::default();
    let _grok = home_into(request, &mut binds, &mut environment)?;
    let program = harness_into(request, &mut binds, &mut environment, &binary);
    let mut argv = base_argv();
    binds.argv(&mut argv);
    let env = environment.finish(&request.adapter_env);
    Ok(Prepared {
        argv,
        env,
        program,
        _grok,
    })
}

/// The sandbox's mounts, in order: later binds overlay earlier ones.
#[derive(Default)]
struct Binds {
    binds: Vec<(bool, PathBuf, PathBuf)>,
}

impl Binds {
    fn read_write(&mut self, source: &Path, dest: &Path) {
        self.binds
            .push((false, source.to_path_buf(), dest.to_path_buf()));
    }

    /// Bind `source` read-only at `dest`, when it exists. Missing sources are
    /// skipped: a toolchain or user-level file the machine does not have is
    /// not one the session needs.
    fn read_only(&mut self, source: &Path, dest: &Path) {
        if source.exists() {
            self.binds
                .push((true, source.to_path_buf(), dest.to_path_buf()));
        }
    }

    fn argv(&self, argv: &mut Vec<OsString>) {
        for (read_only, source, dest) in &self.binds {
            argv.push(if *read_only { "--ro-bind" } else { "--bind" }.into());
            argv.push(source.into());
            argv.push(dest.into());
        }
    }
}

/// The session's environment overrides. The session otherwise inherits the
/// Trial's environment, as an unsandboxed session would: the sandbox bounds
/// what the session can see on disk and which processes it can see, not which
/// variables it starts with. Secrets travel only as environment values, never
/// in the Harness's arguments: the caller appends those after `--` itself.
#[derive(Default)]
struct Environment {
    pairs: Vec<(OsString, OsString)>,
    path_dirs: Vec<PathBuf>,
}

impl Environment {
    fn set(&mut self, name: &str, value: &Path) {
        self.pairs.push((name.into(), value.as_os_str().to_owned()));
    }

    fn set_str(&mut self, name: &str, value: &str) {
        self.pairs.push((name.into(), value.into()));
    }

    /// Prepend a directory to the session's `PATH`.
    fn prepend_path(&mut self, directory: &Path) {
        if !self.path_dirs.contains(&directory.to_path_buf()) {
            self.path_dirs.insert(0, directory.to_path_buf());
        }
    }

    /// The overrides as command environment: the fresh home, a `PATH` of the
    /// Harness's directories and the base system, the sandbox's `/tmp`, and
    /// the login variables, over whatever the Trial's environment holds.
    fn finish(mut self, adapter_env: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        for directory in [
            "/usr/local/bin",
            "/usr/bin",
            "/bin",
            "/usr/local/sbin",
            "/usr/sbin",
            "/sbin",
        ] {
            self.path_dirs.push(directory.into());
        }
        let path = std::env::join_paths(&self.path_dirs)
            .unwrap_or_else(|_| OsString::from("/usr/bin:/bin"));
        let mut finished = vec![
            (OsString::from("PATH"), path),
            (OsString::from("TMPDIR"), OsString::from("/tmp")),
        ];
        finished.extend(self.pairs);
        finished.extend(
            adapter_env
                .iter()
                .map(|(name, value)| (OsString::from(*name), OsString::from(*value))),
        );
        finished
    }
}

/// The sandbox's own namespaces, devices and base system: its processes see
/// only each other, and the base userland is read-only. The network stays the
/// host's here; #1013 gives the sandbox its own.
fn base_argv() -> Vec<OsString> {
    let mut argv = vec![
        "--unshare-pid",
        "--unshare-ipc",
        "--unshare-uts",
        "--die-with-parent",
        "--new-session",
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--tmpfs",
        "/tmp",
    ]
    .into_iter()
    .map(OsString::from)
    .collect::<Vec<_>>();
    for base in [
        "/usr",
        "/bin",
        "/sbin",
        "/lib",
        "/lib64",
        "/etc",
        "/usr/local",
        "/opt",
    ] {
        if Path::new(base).exists() {
            argv.push("--ro-bind".into());
            argv.push(base.into());
            argv.push(base.into());
        }
    }
    argv
}

/// The toolchain roots a session may need, read-only: each is bound only
/// when it exists, at its own path. Environment overrides win over the
/// defaults, as they do outside the sandbox.
fn toolchains(home: &Path) -> Vec<PathBuf> {
    let roots = [
        std::env::var_os("RUSTUP_HOME").map(PathBuf::from),
        Some(home.join(".rustup")),
        std::env::var_os("CARGO_HOME").map(PathBuf::from),
        Some(home.join(".cargo")),
        std::env::var_os("NVM_DIR").map(PathBuf::from),
        Some(home.join(".nvm")),
    ];
    let mut seen = Vec::new();
    for root in roots.into_iter().flatten() {
        if root.exists() && !seen.contains(&root) {
            seen.push(root);
        }
    }
    seen
}

/// Bind the fresh home and everything it holds: the Harness's login, and the
/// user-level instructions, rules, settings and hooks a Run's session reads,
/// those read-only. Returns the held Grok Trial, if this is one.
fn home_into(
    request: &Request<'_>,
    binds: &mut Binds,
    environment: &mut Environment,
) -> Result<Option<GrokGuard>> {
    binds.read_write(request.fresh_home, request.fresh_home);
    environment.set("HOME", request.fresh_home);
    fn user_file(request: &Request<'_>, binds: &mut Binds, relative: &str) {
        binds.read_only(
            &request.home.join(relative),
            &request.fresh_home.join(relative),
        );
    }

    let mut guard = None;
    match request.harness {
        Harness::Claude => {
            // The subscription token travels in the environment: there is no
            // login file to bind and nothing to refresh.
            if let Some(token) = &request.secrets.claude_setup_token {
                environment.set_str(CLAUDE_TOKEN_ENV, token);
            }
            user_file(request, binds, ".claude.json");
            user_file(request, binds, ".claude/CLAUDE.md");
            user_file(request, binds, ".claude/settings.json");
        }
        Harness::Codex => {
            // The login is bound read-write in place, so a refresh reaches
            // the file production uses. Codex writes it in place, never by
            // rename, so a single-file bind carries the refresh back.
            login_into(
                request,
                binds,
                environment,
                "CODEX_HOME",
                ".codex",
                "auth.json",
            )?;
            user_file(request, binds, ".codex/config.toml");
            user_file(request, binds, ".codex/AGENTS.md");
        }
        Harness::Muse => {
            // Each file is bound read-write in place, since Muse writes its
            // login in place (#1009).
            login_into(
                request,
                binds,
                environment,
                "XDG_CONFIG_HOME",
                ".config",
                "muse/auth.json",
            )?;
            login_into(
                request,
                binds,
                environment,
                "XDG_CONFIG_HOME",
                ".config",
                "muse/auth.json.lock",
            )?;
            user_file(request, binds, ".config/muse/settings.json");
        }
        Harness::Agy => {
            // The token file is bound read-write in place; never the D-Bus
            // session bus, which would expose every user service on it.
            login_into(
                request,
                binds,
                environment,
                "",
                ".gemini/antigravity-cli",
                "antigravity-oauth-token",
            )?;
            user_file(request, binds, ".gemini/GEMINI.md");
            user_file(request, binds, ".gemini/settings.json");
        }
        Harness::Grok => {
            guard = Some(grok_into(request, binds)?);
        }
        Harness::OpenCode => {
            opencode_into(request, environment)?;
            user_file(request, binds, ".config/opencode/opencode.json");
        }
    }
    Ok(guard)
}

/// Bind one login file read-write in place: `override_env` names the
/// environment override when there is one (`CODEX_HOME`, `XDG_CONFIG_HOME`,
/// none for agy), `default_dir` its home-relative default, and `file` the
/// path under it. A missing login is left for the session: its login failure
/// voids the Trial, as it does outside the sandbox. An override always keeps
/// its meaning inside, pointed at the fresh home, so session state the
/// Harness writes beside a missing login still lands where its reader looks.
fn login_into(
    request: &Request<'_>,
    binds: &mut Binds,
    environment: &mut Environment,
    override_env: &str,
    default_dir: &str,
    file: &str,
) -> Result<()> {
    let (source_dir, overridden) = match (!override_env.is_empty())
        .then(|| std::env::var_os(override_env))
        .flatten()
        .filter(|dir| !dir.is_empty())
    {
        Some(dir) => (PathBuf::from(dir), true),
        None => (request.home.join(default_dir), false),
    };
    let source = source_dir.join(file);
    let dest = if overridden {
        // Keep the override's meaning, pointed at the fresh home.
        request.fresh_home.join(default_dir).join(file)
    } else {
        fresh_mirror(request, &source)
    };
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "sandbox failed to start: could not create {}",
                parent.display()
            )
        })?;
    }
    if source.is_file() {
        binds.read_write(&source, &dest);
    }
    if overridden {
        environment.set(override_env, dest.parent().unwrap_or(request.fresh_home));
    }
    Ok(())
}

/// Mirror a home-relative `source` under the fresh home.
fn fresh_mirror(request: &Request<'_>, source: &Path) -> PathBuf {
    match source.strip_prefix(request.home) {
        Ok(relative) => request.fresh_home.join(relative),
        Err(_) => request
            .fresh_home
            .join(source.file_name().unwrap_or_default()),
    }
}

/// The Trials Grok home: signed in once with
/// `GROK_HOME=~/.thirdshift/trials/grok grok login`, wiped before each Trial
/// down to its login and lock, and bound as the Trial's `~/.grok`. Grok
/// renews its 6-hour key by renaming a new `auth.json` into place, which a
/// single-file bind refuses: binding the folder carries the rename back to
/// the Trials home, never to production's `~/.grok`.
fn trials_home(home: &Path) -> PathBuf {
    home.join(".thirdshift/trials/grok")
}

/// Bind the Trials Grok home for one Trial, holding it against every other
/// Grok Trial until the session ends. Fails before the session when the home
/// has no login.
fn grok_into(request: &Request<'_>, binds: &mut Binds) -> Result<GrokGuard> {
    let home = trials_home(request.home);
    let guard = GrokGuard::acquire(request.home)?;
    wipe(&home)?;
    if !home.join("auth.json").is_file() {
        return Err(anyhow::Error::new(NoLogin {
            diagnostic: format!(
                "Grok Trial is not logged in; sign the Trials home in once with \
                 GROK_HOME={} grok login",
                home.display()
            ),
        }));
    }
    binds.read_write(&home, &request.fresh_home.join(GROK_HOME_NAME));
    Ok(guard)
}

/// Delete everything in `home` but its login and lock.
fn wipe(home: &Path) -> Result<()> {
    fs::create_dir_all(home).with_context(|| {
        format!(
            "sandbox failed to start: could not create {}",
            home.display()
        )
    })?;
    let entries = fs::read_dir(home)
        .with_context(|| format!("sandbox failed to start: could not read {}", home.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| {
            format!("sandbox failed to start: could not read {}", home.display())
        })?;
        let name = entry.file_name();
        if GROK_KEPT.iter().any(|kept| name == *kept) {
            continue;
        }
        let path = entry.path();
        let is_dir = fs::symlink_metadata(&path)
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false);
        if is_dir {
            fs::remove_dir_all(&path).with_context(|| {
                format!("sandbox failed to start: could not wipe {}", path.display())
            })?;
        } else {
            fs::remove_file(&path).with_context(|| {
                format!("sandbox failed to start: could not wipe {}", path.display())
            })?;
        }
    }
    Ok(())
}

/// One Grok Trial at a time: the lock is a directory beside the Trials home,
/// held until the guard drops. A lock whose owner is gone is stale and taken.
struct GrokGuard {
    lock: PathBuf,
}

impl GrokGuard {
    fn acquire(home: &Path) -> Result<Self> {
        let lock = home.join(".thirdshift/trials/grok.lock");
        if let Some(parent) = lock.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "sandbox failed to start: could not create {}",
                    parent.display()
                )
            })?;
        }
        let me = std::process::id().to_string();
        loop {
            match fs::create_dir(&lock) {
                Ok(()) => {
                    fs::write(lock.join("pid"), &me).ok();
                    return Ok(GrokGuard { lock });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if !owner_alive(&lock) {
                        fs::remove_dir_all(&lock).ok();
                        continue;
                    }
                    interrupt::check().context("sandbox failed to start: interrupted")?;
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("sandbox failed to start: could not lock {}", lock.display())
                    });
                }
            }
        }
    }
}

/// Whether the process holding `lock` still runs.
fn owner_alive(lock: &Path) -> bool {
    let pid = fs::read_to_string(lock.join("pid"))
        .ok()
        .and_then(|pid| pid.trim().parse::<i32>().ok());
    let Some(pid) = pid else { return true };
    // SAFETY: kill with signal 0 sends nothing; it only reports an error.
    let alive = unsafe { libc::kill(pid, 0) == 0 };
    alive || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

impl Drop for GrokGuard {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.lock).ok();
    }
}

/// An OpenCode Trial's provider key: through its environment, never
/// production's `opencode.db`, which holds its credentials beside its
/// sessions. The key comes from `opencode.api_key` in the Credentials, and
/// the Trial's config sets the provider's `apiKey` to `{env:…}` naming the
/// variable that carries it. Without a key, the Trial fails before its
/// session starts.
fn opencode_into(request: &Request<'_>, environment: &mut Environment) -> Result<()> {
    let Some(key) = request.secrets.opencode_api_key.as_deref() else {
        let path = config::home()
            .map(|home| home.join(".thirdshift/credentials.toml"))
            .unwrap_or_else(|_| PathBuf::from("~/.thirdshift/credentials.toml"));
        return Err(anyhow::Error::new(NoLogin {
            diagnostic: format!(
                "OpenCode Trial login failed: no opencode.api_key in {}; \
                 add the provider key as api_key under [opencode] (mode 600)",
                path.display()
            ),
        }));
    };
    environment.set_str(OPENCODE_API_KEY_ENV, key);
    environment.set_str(OPENCODE_CONFIG_ENV, &provider_config(request.model)?);
    Ok(())
}

/// The Trial's OpenCode config for `model`: the provider's `apiKey` set to
/// `{env:…}` naming the variable that carries the key.
fn provider_config(model: Option<&str>) -> Result<String> {
    let provider = model
        .and_then(|model| model.split('#').next())
        .and_then(|model| model.split_once('/').map(|(provider, _)| provider))
        .filter(|provider| !provider.is_empty());
    let Some(provider) = provider else {
        bail!(
            "OpenCode Trial login failed: the Model must name its provider as \
             <provider>/<model>, so the Trial's config can set that provider's apiKey"
        );
    };
    Ok(serde_json::json!({
        "provider": {
            provider: { "apiKey": format!("{{env:{OPENCODE_API_KEY_ENV}}}") }
        }
    })
    .to_string())
}

/// The Harness binaries' directory inside the sandbox, outside every home.
const HARNESS_DEST_DIR: &str = "/sandbox/harness";

/// Bind the Harness's binary, returning its in-sandbox path. Each binary is
/// bound under its Harness's name, so the session finds the name it launched:
/// a link on `PATH` resolves to its real file, whose own name may differ.
/// Codex's release folder is bound, not the binary alone, since Codex looks
/// for `codex-code-mode-host` beside it; anything else binds as one file, so
/// a stand-in without that layout never exposes its whole directory. Grok's
/// binary lives under `~/.grok`, which the Trials home hides, so it is bound
/// read-only at a path outside the home, after the home bind.
fn harness_into(
    request: &Request<'_>,
    binds: &mut Binds,
    environment: &mut Environment,
    binary: &Path,
) -> PathBuf {
    let dest_dir = Path::new(HARNESS_DEST_DIR).join(request.harness.name());
    environment.prepend_path(&dest_dir);
    match request.harness {
        Harness::Codex => match binary
            .parent()
            .filter(|release| release.join("codex-code-mode-host").exists())
        {
            Some(release) => {
                binds.read_only(release, &dest_dir);
                dest_dir.join(binary.file_name().unwrap_or_default())
            }
            None => {
                binds.read_only(binary, &dest_dir);
                dest_dir
            }
        },
        Harness::Grok if binary.starts_with(request.home.join(GROK_HOME_NAME)) => {
            let dest = dest_dir.join(binary.file_name().unwrap_or_default());
            binds.read_only(binary, &dest);
            dest
        }
        _ => {
            binds.read_only(binary, &dest_dir);
            dest_dir
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// The sandbox tests run where bwrap works; anywhere else they report
    /// themselves skipped rather than passing silently.
    fn bwrap_or_skip(reason: &str) -> bool {
        if find_bwrap().is_none() {
            println!("SKIPPED ({reason}): bwrap is not on PATH");
            return false;
        }
        true
    }

    struct Layout {
        _root: tempfile::TempDir,
        home: PathBuf,
        repo: PathBuf,
        skills: PathBuf,
        fresh: PathBuf,
        harness: PathBuf,
    }

    impl Layout {
        fn new() -> Self {
            let root = tempfile::TempDir::new().unwrap();
            let home = root.path().join("home");
            let repo = root.path().join("repo");
            let skills = root.path().join("skills");
            let fresh = root.path().join("fresh/home");
            for dir in [&home, &repo, &skills] {
                fs::create_dir_all(dir).unwrap();
            }
            fs::write(repo.join("marker.txt"), "repository\n").unwrap();
            let harness = root.path().join("bin/fakeh");
            fs::create_dir_all(harness.parent().unwrap()).unwrap();
            fs::write(&harness, "#!/bin/sh\nexec sh -c \"$SANDBOX_TEST_SCRIPT\"\n").unwrap();
            fs::set_permissions(&harness, fs::Permissions::from_mode(0o755)).unwrap();
            Layout {
                _root: root,
                home,
                repo,
                skills,
                fresh,
                harness,
            }
        }

        fn request<'a>(
            &'a self,
            harness: Harness,
            script: &'a str,
            secrets: Secrets,
            model: Option<&'a str>,
            adapter_env: &'a [(&'a str, &'a str)],
        ) -> Request<'a> {
            Request {
                harness,
                model,
                repository: &self.repo,
                skills_dir: &self.skills,
                harness_binary: &self.harness,
                home: &self.home,
                fresh_home: &self.fresh,
                secrets,
                adapter_env: [("SANDBOX_TEST_SCRIPT", script)]
                    .into_iter()
                    .chain(adapter_env.iter().copied())
                    .collect(),
            }
        }
    }

    /// Run the sandbox's Harness with `script` as its body, in the bound
    /// repository, and return what the script printed.
    fn run_script(prepared: &Prepared, repository: &Path) -> std::process::Output {
        prepared
            .launch()
            .command(&[])
            .current_dir(repository)
            .output()
            .expect("sandboxed session did not run")
    }

    #[test]
    fn a_sandboxed_process_sees_neither_an_unbound_path_nor_host_processes() {
        if !bwrap_or_skip("isolation") {
            return;
        }
        let layout = Layout::new();
        let outside = layout.home.join("elsewhere");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), "host only\n").unwrap();
        let script = format!(
            "test ! -e {} || exit 11; test -f marker.txt || exit 12; \
             test \"$(ls /proc | grep -E '^[0-9]+$' | wc -l)\" -lt 20 || exit 13; \
             echo isolated",
            outside.display()
        );
        let request = layout.request(Harness::Codex, &script, Secrets::default(), None, &[]);
        let prepared = prepare(&request).unwrap();

        let output = run_script(&prepared, &layout.repo);

        assert!(
            output.status.success(),
            "isolation script failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "isolated");
    }

    #[test]
    fn everything_a_sandboxed_session_starts_ends_with_it() {
        if !bwrap_or_skip("orphans") {
            return;
        }
        let layout = Layout::new();
        let script =
            "sleep 60 & echo $! > orphan.pid; kill -0 $(cat orphan.pid); exit 0".to_string();
        let request = layout.request(Harness::Codex, &script, Secrets::default(), None, &[]);
        let prepared = prepare(&request).unwrap();

        let output = run_script(&prepared, &layout.repo);

        assert!(output.status.success());
        let pid: i32 = fs::read_to_string(layout.repo.join("orphan.pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // SAFETY: kill with signal 0 sends nothing; it only reports an error.
        let alive = unsafe { libc::kill(pid, 0) == 0 };
        assert!(!alive, "orphaned sleep {pid} outlived its sandbox");
        fs::remove_file(layout.repo.join("orphan.pid")).unwrap();
    }

    #[test]
    fn a_codex_login_refresh_reaches_the_file_production_uses() {
        if !bwrap_or_skip("codex refresh") {
            return;
        }
        let layout = Layout::new();
        let auth = layout.home.join(".codex/auth.json");
        fs::create_dir_all(auth.parent().unwrap()).unwrap();
        fs::write(&auth, "{\"refresh\": \"original\"}\n").unwrap();
        let script = "echo '{\"refresh\": \"renewed\"}' > \"$HOME/.codex/auth.json\"; cat \"$HOME/.codex/auth.json\"";
        let request = layout.request(Harness::Codex, script, Secrets::default(), None, &[]);
        let prepared = prepare(&request).unwrap();

        let output = run_script(&prepared, &layout.repo);

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read_to_string(&auth).unwrap(),
            "{\"refresh\": \"renewed\"}\n"
        );
    }

    #[test]
    fn a_grok_login_renewed_by_rename_is_saved_in_the_trials_home() {
        if !bwrap_or_skip("grok rename") {
            return;
        }
        let layout = Layout::new();
        let production = layout.home.join(".grok/auth.json");
        fs::create_dir_all(production.parent().unwrap()).unwrap();
        fs::write(&production, "production login\n").unwrap();
        let trials = trials_home(&layout.home);
        fs::create_dir_all(trials.join("sessions/stale")).unwrap();
        fs::write(trials.join("auth.json"), "trials login v1\n").unwrap();
        fs::write(trials.join("auth.json.lock"), "lock\n").unwrap();
        let script = "echo 'trials login v2' > \"$HOME/.grok/fresh.json\" && \
            mv \"$HOME/.grok/fresh.json\" \"$HOME/.grok/auth.json\" && \
            cat \"$HOME/.grok/auth.json\"";
        let request = layout.request(Harness::Grok, script, Secrets::default(), None, &[]);
        let prepared = prepare(&request).unwrap();

        let output = run_script(&prepared, &layout.repo);

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "trials login v2"
        );
        assert_eq!(
            fs::read_to_string(trials.join("auth.json")).unwrap(),
            "trials login v2\n"
        );
        assert_eq!(
            fs::read_to_string(&production).unwrap(),
            "production login\n"
        );
        let mut kept: Vec<_> = fs::read_dir(&trials)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        kept.sort();
        assert_eq!(kept, ["auth.json", "auth.json.lock"]);
    }

    #[test]
    fn a_grok_trial_without_a_login_fails_before_its_session_starts() {
        if !bwrap_or_skip("grok login") {
            return;
        }
        let layout = Layout::new();
        let request = layout.request(Harness::Grok, "exit 99", Secrets::default(), None, &[]);

        let error = format!("{:#}", prepare(&request).err().unwrap());

        assert!(error.contains("not logged in"), "{error}");
        assert!(error.contains("grok login"), "{error}");
        assert!(prepare(&request).is_err());
    }

    #[test]
    fn two_grok_trials_never_run_at_once() {
        let home = tempfile::TempDir::new().unwrap();
        let repo = home.path().join("repo");
        let skills = home.path().join("skills");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&skills).unwrap();
        fs::create_dir_all(trials_home(home.path())).unwrap();
        fs::write(trials_home(home.path()).join("auth.json"), "login\n").unwrap();
        let harness = home.path().join("fakeh");
        fs::write(&harness, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&harness, fs::Permissions::from_mode(0o755)).unwrap();
        let first = home.path().join("first/home");
        let second = home.path().join("second/home");
        let held = prepare(&Request {
            harness: Harness::Grok,
            model: None,
            repository: &repo,
            skills_dir: &skills,
            harness_binary: &harness,
            home: home.path(),
            fresh_home: &first,
            secrets: Secrets::default(),
            adapter_env: Vec::new(),
        })
        .unwrap();

        let contested = std::thread::scope(|scope| {
            let waiting = scope.spawn(|| {
                prepare(&Request {
                    harness: Harness::Grok,
                    model: None,
                    repository: &repo,
                    skills_dir: &skills,
                    harness_binary: &harness,
                    home: home.path(),
                    fresh_home: &second,
                    secrets: Secrets::default(),
                    adapter_env: Vec::new(),
                })
            });
            std::thread::sleep(Duration::from_millis(300));
            let blocked = !waiting.is_finished();
            drop(held);
            let unblocked = waiting.join().unwrap().is_ok();
            (blocked, unblocked)
        });

        assert_eq!(contested, (true, true), "the second Grok Trial must wait");
        assert!(!home.path().join(".thirdshift/trials/grok.lock").exists());
    }

    #[test]
    fn an_opencode_trial_gets_its_key_only_from_the_credentials() {
        if !bwrap_or_skip("opencode key") {
            return;
        }
        let layout = Layout::new();
        fs::create_dir_all(layout.home.join(".local/share/opencode")).unwrap();
        fs::write(
            layout.home.join(".local/share/opencode/opencode.db"),
            "production credentials\n",
        )
        .unwrap();
        let secrets = Secrets {
            claude_setup_token: None,
            opencode_api_key: Some("opencode-provider-key".into()),
        };
        let script = "test -n \"$THIRDSHIFT_OPENCODE_API_KEY\" || exit 21; \
            test ! -e \"$HOME/.local/share/opencode/opencode.db\" || exit 22; \
            echo \"$OPENCODE_CONFIG_CONTENT\"";
        let request = layout.request(Harness::OpenCode, script, secrets, Some("acme/model"), &[]);
        let prepared = prepare(&request).unwrap();

        let output = run_script(&prepared, &layout.repo);

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let config: serde_json::Value = serde_json::from_slice(output.stdout.trim_ascii()).unwrap();
        assert_eq!(
            config["provider"]["acme"]["apiKey"],
            format!("{{env:{OPENCODE_API_KEY_ENV}}}")
        );
    }

    #[test]
    fn an_opencode_trial_without_a_key_fails_before_its_session_starts() {
        if !bwrap_or_skip("opencode key") {
            return;
        }
        let layout = Layout::new();
        let request = layout.request(
            Harness::OpenCode,
            "exit 99",
            Secrets::default(),
            Some("acme/model"),
            &[],
        );

        let error = format!("{:#}", prepare(&request).err().unwrap());

        assert!(error.contains("login failed"), "{error}");
        assert!(error.contains("opencode.api_key"), "{error}");
    }

    #[test]
    fn the_trial_config_names_the_model_provider_for_its_apikey() {
        for (model, provider) in [
            (Some("acme/model"), Some("acme")),
            (Some("acme/model#high"), Some("acme")),
            (None, None),
            (Some("model-without-provider"), None),
            (Some("/model"), None),
        ] {
            match (provider_config(model), provider) {
                (Ok(config), Some(provider)) => {
                    let config: serde_json::Value = serde_json::from_str(&config).unwrap();
                    assert_eq!(
                        config["provider"][provider]["apiKey"],
                        format!("{{env:{OPENCODE_API_KEY_ENV}}}")
                    );
                }
                (Err(_), None) => (),
                (result, provider) => panic!("{model:?} -> {result:?}, want {provider:?}"),
            }
        }
    }

    #[test]
    fn a_claude_trial_gets_its_setup_token_through_its_environment() {
        if !bwrap_or_skip("claude token") {
            return;
        }
        let layout = Layout::new();
        let secrets = Secrets {
            claude_setup_token: Some("trial-token".into()),
            opencode_api_key: None,
        };
        let request = layout.request(
            Harness::Claude,
            "echo \"token=${CLAUDE_CODE_OAUTH_TOKEN-unset}\"",
            secrets,
            None,
            &[],
        );
        let prepared = prepare(&request).unwrap();

        let output = run_script(&prepared, &layout.repo);

        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "token=trial-token"
        );
    }

    #[test]
    fn the_fresh_home_holds_user_files_read_only() {
        if !bwrap_or_skip("user files") {
            return;
        }
        let layout = Layout::new();
        fs::create_dir_all(layout.home.join(".codex")).unwrap();
        fs::write(layout.home.join(".codex/config.toml"), "user config\n").unwrap();
        let script = "cat \"$HOME/.codex/config.toml\"; \
            echo changed > \"$HOME/.codex/config.toml\" 2>/dev/null && exit 31 || exit 0";
        let request = layout.request(Harness::Codex, script, Secrets::default(), None, &[]);
        let prepared = prepare(&request).unwrap();

        let output = run_script(&prepared, &layout.repo);

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "user config"
        );
    }

    #[test]
    fn a_grok_binary_hidden_by_the_trials_home_is_bound_outside_it() {
        if !bwrap_or_skip("grok binary") {
            return;
        }
        let layout = Layout::new();
        let hidden = layout.home.join(".grok/downloads/grok-1/fakeh");
        fs::create_dir_all(hidden.parent().unwrap()).unwrap();
        fs::write(
            &hidden,
            "#!/bin/sh\necho \"argv0=$0\"\nexec sh -c \"$SANDBOX_TEST_SCRIPT\"\n",
        )
        .unwrap();
        fs::set_permissions(&hidden, fs::Permissions::from_mode(0o755)).unwrap();
        fs::create_dir_all(trials_home(&layout.home)).unwrap();
        fs::write(trials_home(&layout.home).join("auth.json"), "login\n").unwrap();
        let script = "echo \"argv0=$0\"; echo \"path=$PATH\"; ls \"$HOME/.grok\"";
        let mut request = layout.request(Harness::Grok, script, Secrets::default(), None, &[]);
        request.harness_binary = &hidden;
        let prepared = prepare(&request).unwrap();

        let output = run_script(&prepared, &layout.repo);

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(
            stdout.contains("argv0=/sandbox/harness/grok/fakeh"),
            "{stdout}"
        );
        assert!(stdout.contains("/sandbox/harness/grok"), "{stdout}");
        assert!(!stdout.contains("downloads"), "{stdout}");
    }

    #[test]
    fn a_wipe_keeps_only_the_login_and_its_lock() {
        let root = tempfile::TempDir::new().unwrap();
        let home = root.path().join("trials-home");
        fs::create_dir_all(home.join("sessions/stale")).unwrap();
        fs::write(home.join("auth.json"), "login\n").unwrap();
        fs::write(home.join("auth.json.lock"), "lock\n").unwrap();
        fs::write(home.join("stray.txt"), "stray\n").unwrap();

        wipe(&home).unwrap();

        let mut kept: Vec<_> = fs::read_dir(&home)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        kept.sort();
        assert_eq!(kept, ["auth.json", "auth.json.lock"]);
    }
}
