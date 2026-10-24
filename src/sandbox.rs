//! Backported model-session sandbox from #1014 (origin/main ee7fe11).
//! Trial and Grader sandboxes: bwrap, their own processes and network, a fresh
//! home and logins.
//!
//! A Trial's session runs in bwrap with process and network namespaces of its
//! own, so it sees neither an unbound path nor the host's processes, and
//! everything it starts ends with it. Its network holds only its own
//! loopback: a Harness that ignores the proxy finds no route, and its Trial
//! is void. Only these are bound: the prepared repository, the Harness's
//! binary (Codex's whole release folder, since Codex looks for
//! `codex-code-mode-host` beside it), the Factory skills folder, a fresh
//! home, the toolchains the session needs and socat, read-only, and the
//! proxy's socket. The session's environment is only what this launcher sets:
//! home, path, the Harness login, and the proxy variables. Ambient secrets
//! stay outside.
//!
//! An allowlisting CONNECT proxy runs outside the sandbox, listening on a
//! Unix socket bound into it. A socat relay inside turns the proxy port into
//! that socket, and the Harness reaches its provider through `HTTPS_PROXY`.
//! The proxy allows only that Harness's provider hosts, from #1009's lists,
//! and refuses names that resolve to loopback or to the host's own addresses.
//!
//! The fresh home holds the login, and the user-level instructions, rules,
//! settings and hooks a Run's session reads, those read-only. Harness flags
//! stay as ADR 0013 has them: the sandbox wraps the session, it does not
//! change its arguments.
//!
//! A Model Grader uses the same session sandbox with only its packet workspace
//! bound. Code Graders run offline with report copies and the prepared
//! repository at neutral paths; neither Grader binds raw Trial logs or the
//! Experiment's storage.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};

use crate::harness::Harness;
use crate::proxy;
use crate::{config, interrupt};

/// The environment variable carrying an OpenCode Trial's provider key.
pub(crate) const OPENCODE_API_KEY_ENV: &str = "THIRDSHIFT_OPENCODE_API_KEY";
/// The environment variable carrying an OpenCode Trial's provider config.
const OPENCODE_CONFIG_ENV: &str = "OPENCODE_CONFIG_CONTENT";
/// Claude's subscription token variable.
const CLAUDE_TOKEN_ENV: &str = "CLAUDE_CODE_OAUTH_TOKEN";
/// Grok's per-Trial home, bound as the Trial's `~/.grok`.
const GROK_HOME_NAME: &str = ".grok";
/// The Trials Grok home's login files, copied into the fresh home.
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
/// is dropped, and the proxy serves its socket until this is dropped.
pub(crate) struct Prepared {
    argv: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    program: PathBuf,
    prefix: Vec<OsString>,
    stderr: PathBuf,
    _grok: Option<GrokGuard>,
    _proxy: proxy::Guard,
}

/// A session sandbox with an explicit executable: a Trial uses its Arm's
/// binary; a Model Grader uses its pinned binary and packet workspace.
pub(crate) fn for_session(
    choice: &crate::harness::Choice,
    binary: &Path,
    workspace: &Path,
    output_dir: &Path,
) -> Result<Option<Prepared>> {
    if disabled() {
        return Ok(None);
    }
    let home = config::home().map_err(|error| {
        anyhow::Error::new(Unavailable {
            diagnostic: format!("HOME is not set: {error:#}"),
        })
    })?;
    let credentials: toml::Table = fs::read_to_string(home.join(".thirdshift/credentials.toml"))
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or_default();
    let secret = |section: &str, key: &str| {
        credentials
            .get(section)
            .and_then(|section| section.get(key))
            .and_then(toml::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let secrets = Secrets {
        claude_setup_token: secret("claude", "setup_token"),
        opencode_api_key: secret("opencode", "api_key"),
    };
    prepare(&Request {
        harness: choice.harness,
        model: choice.model.as_deref(),
        repository: workspace,
        skills_dir: &crate::skills::written_out()?,
        harness_binary: binary,
        home: &home,
        fresh_home: &output_dir.join("home"),
        secrets,
        adapter_env: choice.harness.adapter().environment().to_vec(),
    })
    .map(Some)
}

pub(crate) fn disabled() -> bool {
    std::env::var_os("THIRDSHIFT_TRIAL_NO_SANDBOX").is_some_and(|flag| !flag.is_empty())
}

impl Prepared {
    /// A later launch (including Resume) may fail even after the startup
    /// check succeeded. Retain bwrap's stderr outside the Harness protocol.
    pub(crate) fn startup_failure(&self, error: &anyhow::Error) -> Option<Unavailable> {
        if error.is::<crate::process::NotStarted>() {
            return Some(Unavailable {
                diagnostic: format!("{error:#}"),
            });
        }
        let failure = error.downcast_ref::<crate::harness::interpretation::SessionFailure>()?;
        if !failure.before_first_turn {
            return None;
        }
        let stderr = fs::read_to_string(&self.stderr).ok()?;
        (stderr.starts_with("bwrap:") || stderr.contains("sandbox failed to start:"))
            .then_some(Unavailable { diagnostic: stderr })
    }
}

impl Prepared {
    /// The bwrap command running the Harness's `args` inside the sandbox.
    /// The caller sets the working directory, which must be the bound
    /// repository. `argv` clears the environment and sets only the sandbox's
    /// own variables, so a Grader cannot read the operator's ambient secrets.
    /// The Harness runs behind the relay: `prefix` starts the relay, then
    /// runs the program as `$0` with `args` as `$@`, so the relay dies with
    /// the session.
    pub fn command(&self, args: &[String]) -> Result<Command> {
        let mut command = Command::new(find_bwrap().unwrap_or_else(|| PathBuf::from("bwrap")));
        command.args(&self.argv);
        command.arg("--");
        command.args(&self.prefix);
        command.arg(&self.program);
        command.args(args);
        command.envs(self.env.iter().map(|(name, value)| (name, value)));
        command.stderr(fs::File::create(&self.stderr)?);
        Ok(command)
    }
}

/// Find bwrap on `PATH`, if it is executable.
pub(crate) fn find_bwrap() -> Option<PathBuf> {
    crate::process::find_executable("bwrap")
}

/// Resolve the Harness CLI named `name` the way a session launch would: the
/// first executable on `PATH`, canonicalized so the sandbox binds its real
/// file rather than a link.
pub(crate) fn resolve_binary(name: &str) -> Result<PathBuf> {
    let found = crate::process::find_executable(name).ok_or_else(|| {
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

/// The proxy port inside the sandbox: the relay listens on loopback here,
/// and the Harness reaches its provider through it.
pub(crate) const RELAY_PORT: u16 = 3128;
/// The proxy's socket inside the sandbox, bound from the proxy outside it.
const SOCKET_DEST: &str = "/sandbox/proxy.sock";
/// The shell starting every sandboxed session: the relay first, then the
/// Harness as `$0` with its arguments as `$@`.
const SHELL: &str = "/bin/sh";

/// Prepare a Trial sandbox as `request` says: wipe and bind the homes, bind
/// the repository, skills, Harness, toolchains and proxy socket, start the
/// proxy, and set the session's environment. Fails before any session starts
/// when bwrap or socat is missing, the proxy cannot start, the Grok Trials
/// home has no login, or an OpenCode Trial has no provider key or names an
/// unknown provider.
pub(crate) fn prepare(request: &Request<'_>) -> Result<Prepared> {
    ensure!(
        request.repository.is_absolute(),
        "sandbox failed to start: Trial repository path must be absolute"
    );
    ensure!(
        request.home.is_absolute(),
        "sandbox failed to start: home path must be absolute"
    );
    let bwrap = find_bwrap().ok_or_else(|| {
        anyhow::Error::new(Unavailable {
            diagnostic: "bwrap is not on PATH".into(),
        })
    })?;
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
    if !Path::new(SHELL).exists() {
        return Err(anyhow::Error::new(Unavailable {
            diagnostic: format!("{SHELL} is missing"),
        }));
    }
    let socat = resolve_binary("socat")?;
    let mut binds = Binds::default();
    let mut environment = Environment::default();
    // The prepared repository, read-write, at its own path so the session's
    // working directory is unchanged.
    binds.read_write(request.repository, request.repository);
    binds.read_only(
        &request.repository.join("repository"),
        &request.repository.join("repository"),
    );
    binds.read_only(request.skills_dir, request.skills_dir);
    toolchains_into(request.home, &mut binds, &mut environment);
    let _grok = home_into(request, &mut binds, &mut environment)?;
    // The proxy allows only this Trial Harness's provider hosts. An OpenCode
    // Trial with no key fails in the homes above, as before; one naming an
    // unknown provider fails here, before its session.
    let allowed = proxy::allowed_hosts(request.harness, request.model).map_err(|error| {
        anyhow::Error::new(NoLogin {
            diagnostic: format!("{error:#}"),
        })
    })?;
    let proxy = proxy::start(&allowed).map_err(|error| {
        anyhow::Error::new(Unavailable {
            diagnostic: format!("{error:#}"),
        })
    })?;
    binds.read_write(proxy.socket_path().as_path(), Path::new(SOCKET_DEST));
    binds.read_only(&socat, &socat);
    // The Harness reaches its provider through the relay alone. OpenCode
    // calls its own local server over HTTP, so it keeps loopback out of the
    // proxy, as #1009's checks required.
    let relay = format!("http://127.0.0.1:{RELAY_PORT}");
    for name in ["HTTPS_PROXY", "HTTP_PROXY", "https_proxy", "http_proxy"] {
        environment.set_str(name, &relay);
    }
    for name in ["NO_PROXY", "no_proxy"] {
        environment.set_str(name, "127.0.0.1,localhost");
    }
    let program = harness_into(request, &mut binds, &mut environment, &binary);
    conceal_toolchain_credentials(request, &mut binds)?;
    let mut argv = base_argv();
    binds.argv(&mut argv);
    let env = environment.finish(&request.adapter_env);
    // bwrap applies `--clearenv` to the sandboxed command, not to itself.
    argv.push("--clearenv".into());
    for (name, value) in &env {
        argv.push("--setenv".into());
        argv.push(name.clone());
        argv.push(value.clone());
    }
    // Test the complete mount/namespace setup before a Harness owns the
    // process. Otherwise bwrap's stderr can look like an agent failure to
    // Harnesses whose session protocol does not include launcher errors.
    let probe = crate::process::output_until(
        Command::new(bwrap)
            .args(&argv)
            .args(["--", "/bin/true"])
            .current_dir(request.repository)
            .envs(env.iter().map(|(name, value)| (name, value))),
        None,
        crate::process::Control {
            name: "sandbox startup check",
            interruption: crate::process::Interruption::Ordinary,
            stop: &|child| crate::process::stop(child, &[libc::SIGTERM, libc::SIGKILL]),
        },
        std::time::Instant::now() + Duration::from_secs(5),
    )
    .map_err(|error| {
        anyhow::Error::new(Unavailable {
            diagnostic: format!("{error:#}"),
        })
    })?;
    if !probe.status.success() {
        return Err(anyhow::Error::new(Unavailable {
            diagnostic: String::from_utf8_lossy(&probe.stderr).into_owned(),
        }));
    }
    let prefix = vec![SHELL.into(), "-c".into(), relay_script(&socat).into()];
    Ok(Prepared {
        argv,
        env,
        program,
        prefix,
        stderr: request.fresh_home.join("launcher.stderr"),
        _grok,
        _proxy: proxy,
    })
}

/// The relay turning the sandbox's proxy port into the proxy's socket, then
/// the Harness as `$0` with its arguments as `$@`. The shell waits for the
/// relay's port before running the Harness: a relay that never answers fails
/// the session before the Harness starts, as a sandbox that could not start.
/// The relay's standard input comes from nowhere, so it never steals the
/// Harness's own; when the Harness ends, its process namespace ends the relay
/// with it.
fn relay_script(socat: &Path) -> String {
    let socat = socat.display();
    format!(
        "{socat} TCP-LISTEN:{RELAY_PORT},bind=127.0.0.1,reuseaddr,fork \
         UNIX-CONNECT:{SOCKET_DEST} </dev/null >/dev/null & relay=$!; \
         i=0; while [ \"$i\" -lt 50 ]; do \
         {socat} - TCP:127.0.0.1:{RELAY_PORT} </dev/null >/dev/null 2>&1 && break; \
         i=$((i + 1)); sleep 0.1; done; \
         if {socat} - TCP:127.0.0.1:{RELAY_PORT} </dev/null >/dev/null 2>&1; then \
         exec \"$0\" \"$@\"; else echo 'sandbox failed to start: its relay did not answer' >&2; \
         kill $relay 2>/dev/null; exit 1; fi"
    )
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

/// The session's whole environment. `--clearenv` drops everything the
/// operator's process inherited, including `RESEND_API_KEY` and GitHub
/// tokens, and these pairs are set back. The Harness login is one of them.
/// Secrets travel only as these values, never in the Harness's arguments:
/// the caller appends those after `--` itself.
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

    /// The session environment: the fresh home, a `PATH` of the Harness's
    /// directories and the base system, the sandbox's `/tmp`, and the login
    /// variables. `prepare` publishes these with `--setenv` after `--clearenv`.
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
/// only each other, its network holds only its own loopback, and the base
/// userland is read-only. Without the proxy there is no route out.
fn base_argv() -> Vec<OsString> {
    let mut argv = vec![
        "--unshare-pid",
        "--unshare-ipc",
        "--unshare-uts",
        "--unshare-net",
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
fn toolchains_into(home: &Path, binds: &mut Binds, environment: &mut Environment) {
    let roots = toolchains(home);
    for root in &roots {
        binds.read_only(root, root);
        // Toolchain bin directories carry their shims at the same paths.
        let bin = root.join("bin");
        if bin.is_dir() {
            environment.prepend_path(&bin);
        }
    }
    for bin in tool_bin_dirs(&roots) {
        environment.prepend_path(&bin);
    }
}

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

/// Hide Cargo registry credentials that a toolchain bind would otherwise
/// expose. The empty stand-in is read-only, so the session cannot recreate
/// the file inside the mount.
fn conceal_toolchain_credentials(request: &Request<'_>, binds: &mut Binds) -> Result<()> {
    let paths: Vec<PathBuf> = toolchains(request.home)
        .into_iter()
        .flat_map(|root| [root.join("credentials"), root.join("credentials.toml")])
        .filter(|path| path.is_file())
        .collect();
    if paths.is_empty() {
        return Ok(());
    }
    let blank = request.fresh_home.join(".toolchain-credential");
    fs::write(&blank, "").with_context(|| {
        format!(
            "sandbox failed to start: could not create {}",
            blank.display()
        )
    })?;
    for path in paths {
        binds.read_only(&blank, &path);
    }
    Ok(())
}

/// The `bin` directories of known tools that resolve under a bound toolchain
/// root: versioned layouts (`~/.rustup/toolchains/<name>/bin`,
/// `~/.nvm/versions/node/<version>/bin`) carry no top-level `bin` of their
/// own. Only directories under a bound root are named, so every named path is
/// visible inside at the same path.
fn tool_bin_dirs(roots: &[PathBuf]) -> Vec<PathBuf> {
    tool_bin_dirs_on(
        &std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect::<Vec<_>>(),
        roots,
    )
}

fn tool_bin_dirs_on(path_dirs: &[PathBuf], roots: &[PathBuf]) -> Vec<PathBuf> {
    const TOOLS: [&str; 6] = ["cargo", "rustc", "node", "npm", "python3", "php"];
    let mut bins = Vec::new();
    for tool in TOOLS {
        let Some(path) = path_dirs
            .iter()
            .map(|directory| directory.join(tool))
            .find(|path| crate::process::is_executable(path))
        else {
            continue;
        };
        let Ok(canonical) = path.canonicalize() else {
            continue;
        };
        if roots.iter().any(|root| canonical.starts_with(root))
            && let Some(parent) = canonical.parent()
        {
            let parent = parent.to_path_buf();
            if !bins.contains(&parent) {
                bins.push(parent);
            }
        }
    }
    bins
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

    // These Harnesses read the same production user instructions. Bind only
    // instruction and hook paths, leaving their remembered sessions behind.
    if matches!(
        request.harness,
        Harness::Claude | Harness::Grok | Harness::Muse
    ) {
        for path in [".claude/CLAUDE.md", ".claude/rules", ".claude/hooks"] {
            user_file(request, binds, path);
        }
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
            user_file(request, binds, ".claude/settings.json");
        }
        Harness::Codex => {
            // Copy the login into the fresh home. A refresh stays in that
            // copy; the operator's file is not mounted.
            login_into(
                request,
                environment,
                Some("CODEX_HOME"),
                ".codex",
                "auth.json",
            )?;
            user_file(request, binds, ".codex/config.toml");
            user_file(request, binds, ".codex/AGENTS.md");
        }
        Harness::Muse => {
            // Copy each login file. Muse writes its login in place (#1009);
            // that write stays in the copy.
            for login in ["muse/auth.json", "muse/auth.json.lock"] {
                login_into(
                    request,
                    environment,
                    Some("XDG_CONFIG_HOME"),
                    ".config",
                    login,
                )?;
            }
            user_file(request, binds, ".config/muse/settings.json");
        }
        Harness::Agy => {
            // Copy the token file, never the D-Bus session bus, which would
            // expose every user service on it.
            login_into(
                request,
                environment,
                None,
                ".gemini/antigravity-cli",
                "antigravity-oauth-token",
            )?;
            user_file(request, binds, ".gemini/GEMINI.md");
            user_file(request, binds, ".gemini/settings.json");
        }
        Harness::Grok => {
            guard = Some(grok_into(request)?);
            for path in [".grok/settings.json", ".grok/rules", ".grok/hooks"] {
                user_file(request, binds, path);
            }
        }
        Harness::OpenCode => {
            opencode_into(request, environment)?;
            user_file(request, binds, ".config/opencode/opencode.json");
            user_file(request, binds, ".config/opencode/AGENTS.md");
        }
    }
    Ok(guard)
}

/// Copy one login file into the fresh home: `override_env` names the
/// environment override when there is one (`CODEX_HOME`, `XDG_CONFIG_HOME`,
/// none for agy), `default_dir` its home-relative default, and `file` the
/// path under it. The copy is what the session reads and refreshes. The live
/// file stays outside the sandbox, so a Grader that follows packet contents
/// cannot replace the operator's login. A missing login is left for the
/// session: its login failure voids the Trial, as it does outside the
/// sandbox. An override always keeps its meaning inside, pointed at the
/// fresh home, so session state the Harness writes beside a missing login
/// still lands where its reader looks.
fn login_into(
    request: &Request<'_>,
    environment: &mut Environment,
    override_env: Option<&str>,
    default_dir: &str,
    file: &str,
) -> Result<()> {
    let (source_dir, override_env) =
        match override_env.and_then(|env| std::env::var_os(env).filter(|dir| !dir.is_empty())) {
            Some(dir) => (PathBuf::from(dir), override_env),
            None => (request.home.join(default_dir), None),
        };
    let source = source_dir.join(file);
    let dest = if override_env.is_some() {
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
        fs::copy(&source, &dest).with_context(|| {
            format!(
                "sandbox failed to start: could not copy the login {}",
                source.display()
            )
        })?;
    }
    if let Some(env) = override_env {
        // The override names the directory the file sits `depth` levels
        // under: one for `CODEX_HOME/auth.json`, two for
        // `XDG_CONFIG_HOME/muse/auth.json`.
        let depth = Path::new(file).components().count();
        let dir = dest.ancestors().nth(depth).unwrap_or(request.fresh_home);
        environment.set(env, dir);
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
/// `GROK_HOME=~/.thirdshift/trials/grok grok login`. A session gets a copy of
/// its login under the fresh home, not the live directory.
fn trials_home(home: &Path) -> PathBuf {
    home.join(".thirdshift/trials/grok")
}

/// Copy the Trials Grok login into the fresh home, holding the Trials home
/// against every other Grok session until this one ends. The live directory
/// stays outside the sandbox. Fails before the session when the home has no
/// login.
fn grok_into(request: &Request<'_>) -> Result<GrokGuard> {
    let home = trials_home(request.home);
    let guard = GrokGuard::acquire(request.home)?;
    if !home.join("auth.json").is_file() {
        return Err(anyhow::Error::new(NoLogin {
            diagnostic: format!(
                "Grok Trial is not logged in; sign the Trials home in once with \
                 GROK_HOME={} grok login",
                home.display()
            ),
        }));
    }
    let dest = request.fresh_home.join(GROK_HOME_NAME);
    fs::create_dir_all(&dest).with_context(|| {
        format!(
            "sandbox failed to start: could not create {}",
            dest.display()
        )
    })?;
    for name in GROK_KEPT {
        let source = home.join(name);
        if source.is_file() {
            fs::copy(&source, dest.join(name)).with_context(|| {
                format!(
                    "sandbox failed to start: could not copy the login {}",
                    source.display()
                )
            })?;
        }
    }
    Ok(guard)
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
/// `{env:…}` naming the variable that carries the key, with web search and
/// web fetch denied, since a Trial runs behind the allowlisting proxy.
fn provider_config(model: Option<&str>) -> Result<String> {
    let Some(provider) = proxy::opencode_provider(model) else {
        bail!(
            "OpenCode Trial login failed: the Model must name its provider as \
             <provider>/<model>, so the Trial's config can set that provider's apiKey"
        );
    };
    Ok(serde_json::json!({
        "provider": {
            provider: { "apiKey": format!("{{env:{OPENCODE_API_KEY_ENV}}}") }
        },
        "permission": {
            "webfetch": "deny",
            "websearch": "deny",
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
