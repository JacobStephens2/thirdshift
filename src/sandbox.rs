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
use crate::resend_key::Credentials;
use crate::{config, interrupt};

pub(crate) mod code_grader;

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

/// Prepare a Trial's sandbox: the Harness's fixed binary, the Factory skills
/// folder, the production home, a fresh home under its output directory, and
/// the Trial secrets the Credentials hold, if they hold any. A broken
/// Credentials file leaves the secrets empty rather than blocking a Trial
/// whose Harness needs none; an OpenCode Trial without its key still fails
/// before its session starts.
///
/// When `THIRDSHIFT_TRIAL_NO_SANDBOX` is set and not empty, there is no
/// sandbox: Trials and Graders use the pass-through launcher. The Experiment
/// suite sets it so its fake Harnesses stay observable at fixed paths, and
/// it is the emergency escape when bwrap breaks on a machine.
pub(crate) fn for_trial(
    choice: &crate::harness::Choice,
    repository: &Path,
    output_dir: &Path,
) -> Result<Option<Prepared>> {
    if disabled() {
        return Ok(None);
    }
    for_session(
        choice,
        &resolve_binary(choice.harness.name())?,
        repository,
        output_dir,
    )
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
    let secrets = Credentials::read()
        .map(|credentials| Secrets::from(&credentials))
        .unwrap_or_default();
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
    /// What a session runner holds to launch inside the sandbox. The guards
    /// stay with the `Prepared` they came from, which outlives the sessions.
    pub(crate) fn launch(&self) -> Launch {
        Launch {
            argv: self.argv.clone(),
            env: self.env.clone(),
            program: self.program.clone(),
            prefix: self.prefix.clone(),
            stderr: Some(self.stderr.clone()),
        }
    }

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

/// How one session is launched inside a prepared sandbox.
#[derive(Clone)]
pub(crate) struct Launch {
    pub argv: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    pub program: PathBuf,
    prefix: Vec<OsString>,
    stderr: Option<PathBuf>,
}

impl Launch {
    /// Publish a caller-supplied session path after clearing the ambient environment.
    pub(crate) fn set_env(&mut self, name: &str, value: &Path) {
        self.argv.extend([
            OsString::from("--setenv"),
            name.into(),
            value.as_os_str().to_owned(),
        ]);
        self.env.push((name.into(), value.as_os_str().to_owned()));
    }

    /// The bwrap command running the Harness's `args` inside the sandbox.
    /// The caller sets the working directory, which must be the bound
    /// repository. `argv` clears the environment and sets only the sandbox's
    /// own variables, so a Grader cannot read the operator's ambient secrets.
    /// The Harness runs behind the relay: `prefix` starts the relay, then
    /// runs the program as `$0` with `args` as `$@`, so the relay dies with
    /// the session.
    pub fn command(&self, args: &[String]) -> Result<Command> {
        let mut command = Command::new(find_bwrap().unwrap_or_else(|| PathBuf::from("bwrap")));
        self.configure_command(&mut command, args)?;
        Ok(command)
    }

    /// A Grader's scope must enclose bwrap and all processes it starts.
    pub(crate) fn scoped_command(
        &self,
        args: &[String],
        directory: &Path,
        caps: &crate::scope::Caps,
    ) -> Result<crate::scope::Launch> {
        let program = find_bwrap().unwrap_or_else(|| PathBuf::from("bwrap"));
        let mut launched = crate::scope::grader_command(&program, &[], directory, caps)?;
        self.configure_command(launched.command(), args)?;
        Ok(launched)
    }

    fn configure_command(&self, command: &mut Command, args: &[String]) -> Result<()> {
        command.args(&self.argv);
        command.arg("--");
        command.args(&self.prefix);
        command.arg(&self.program);
        command.args(args);
        command.envs(self.env.iter().map(|(name, value)| (name, value)));
        if let Some(stderr) = &self.stderr {
            command.stderr(fs::File::create(stderr)?);
        }
        Ok(())
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

/// Prepare a Trial sandbox as `request` says: copy logins into a fresh home, bind
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
        &|| Ok(()),
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
            .unwrap()
            .current_dir(repository)
            .output()
            .expect("sandboxed session did not run")
    }

    #[test]
    fn a_trial_keeps_production_user_rules() {
        if !bwrap_or_skip("production user rules") {
            return;
        }
        for harness in [
            Harness::Claude,
            Harness::Grok,
            Harness::Muse,
            Harness::OpenCode,
        ] {
            let layout = Layout::new();
            let mut paths = if harness == Harness::OpenCode {
                vec![".config/opencode/AGENTS.md"]
            } else {
                vec![
                    ".claude/CLAUDE.md",
                    ".claude/rules/fixture.md",
                    ".claude/hooks/fixture.sh",
                ]
            };
            if harness == Harness::Grok {
                paths.push(".grok/rules/fixture.md");
                let login = trials_home(&layout.home);
                fs::create_dir_all(&login).unwrap();
                fs::write(login.join("auth.json"), "{}").unwrap();
            }
            for path in &paths {
                let source = layout.home.join(path);
                fs::create_dir_all(source.parent().unwrap()).unwrap();
                fs::write(source, "production-rule\n").unwrap();
            }
            let script = paths
                .iter()
                .map(|path| format!("cat \"$HOME/{path}\""))
                .collect::<Vec<_>>()
                .join(" && ");
            let secrets = Secrets {
                opencode_api_key: Some("fixture-key".into()),
                ..Secrets::default()
            };
            let model = (harness == Harness::OpenCode).then_some("anthropic/claude");
            let request = layout.request(harness, &script, secrets, model, &[]);
            let prepared = prepare(&request).unwrap();
            let output = run_script(&prepared, &layout.repo);
            assert!(
                output.status.success(),
                "the {harness:?} Trial lost production rules: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                "production-rule\n".repeat(paths.len())
            );
        }
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

    /// Whether the internet is reachable from here: without it, the tests
    /// that dial out report themselves skipped rather than failing on the
    /// dial. Opening a TCP connection sends no request.
    fn internet_or_skip(reason: &str) -> bool {
        use std::net::ToSocketAddrs;
        let reachable = ("api.anthropic.com", 443)
            .to_socket_addrs()
            .ok()
            .and_then(|mut resolved| resolved.next())
            .is_some_and(|resolved| {
                std::net::TcpStream::connect_timeout(
                    &std::net::SocketAddr::new(resolved.ip(), 443),
                    Duration::from_secs(8),
                )
                .is_ok()
            });
        if !reachable {
            println!("SKIPPED ({reason}): no route to the internet");
        }
        reachable
    }

    /// This machine's non-loopback address, if it has one: connecting a UDP
    /// socket chooses the address without sending anything.
    fn local_ip() -> Option<std::net::IpAddr> {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
        socket.connect("8.8.8.8:80").ok()?;
        socket
            .local_addr()
            .ok()
            .map(|address| address.ip())
            .filter(|address| !address.is_loopback())
    }

    /// A raw CONNECT through the sandbox's relay: prints the proxy's status
    /// line for `target`.
    fn connect_script(target: &str) -> String {
        format!(
            "printf 'CONNECT {target} HTTP/1.1\\r\\nHost: {target}\\r\\n\\r\\n' \
             | socat - TCP:127.0.0.1:{RELAY_PORT} | head -n 1"
        )
    }

    #[test]
    fn a_sandboxed_process_reaches_an_allowed_host_through_the_proxy() {
        if !bwrap_or_skip("proxy allow") || !internet_or_skip("proxy allow") {
            return;
        }
        let layout = Layout::new();
        let script = format!(
            "test \"$HTTPS_PROXY\" = \"http://127.0.0.1:{RELAY_PORT}\" || exit 41; \
             curl -sS -o /dev/null --max-time 20 https://api.anthropic.com"
        );
        let request = layout.request(Harness::Claude, &script, Secrets::default(), None, &[]);
        let prepared = prepare(&request).unwrap();

        let output = run_script(&prepared, &layout.repo);

        assert!(
            output.status.success(),
            "allowed host unreachable through the proxy: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn a_disallowed_host_is_refused_through_the_proxy() {
        if !bwrap_or_skip("proxy refuse") {
            return;
        }
        let layout = Layout::new();
        let script = connect_script("github.com:443");
        let request = layout.request(Harness::Claude, &script, Secrets::default(), None, &[]);
        let prepared = prepare(&request).unwrap();

        let output = run_script(&prepared, &layout.repo);

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "HTTP/1.1 403 Forbidden"
        );
    }

    /// `localhost` is refused before it resolves here: it is not on the
    /// Harness's allowlist, so the proxy never resolves it. The resolution
    /// guard itself is covered in `proxy.rs`, where `localhost` is allowed
    /// and refused for resolving to loopback.
    #[test]
    fn a_name_that_resolves_to_loopback_is_refused_through_the_proxy() {
        if !bwrap_or_skip("proxy loopback") {
            return;
        }
        let layout = Layout::new();
        let script = connect_script("localhost:443");
        let request = layout.request(Harness::Claude, &script, Secrets::default(), None, &[]);
        let prepared = prepare(&request).unwrap();

        let output = run_script(&prepared, &layout.repo);

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "HTTP/1.1 403 Forbidden"
        );
    }

    #[test]
    fn this_machine_is_unreachable_through_the_proxy() {
        if !bwrap_or_skip("proxy host address") {
            return;
        }
        let Some(address) = local_ip() else {
            println!("SKIPPED (proxy host address): no non-loopback address");
            return;
        };
        let layout = Layout::new();
        let script = connect_script(&format!("{address}:443"));
        let request = layout.request(Harness::Claude, &script, Secrets::default(), None, &[]);
        let prepared = prepare(&request).unwrap();

        let output = run_script(&prepared, &layout.repo);

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "HTTP/1.1 403 Forbidden"
        );
    }

    #[test]
    fn without_the_proxy_a_sandboxed_process_has_no_route() {
        if !bwrap_or_skip("no route") {
            return;
        }
        let layout = Layout::new();
        let script = "if env -u HTTPS_PROXY -u HTTP_PROXY -u https_proxy -u http_proxy \
            curl -sS -o /dev/null --max-time 15 https://api.anthropic.com 2>/dev/null; \
            then exit 51; fi";
        let request = layout.request(Harness::Claude, script, Secrets::default(), None, &[]);
        let prepared = prepare(&request).unwrap();

        let output = run_script(&prepared, &layout.repo);

        assert!(
            output.status.success(),
            "reached the internet without the proxy: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(output.stdout.is_empty());
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
    fn a_codex_login_refresh_stays_in_the_fresh_home() {
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
            "{\"refresh\": \"original\"}\n"
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "{\"refresh\": \"renewed\"}\n"
        );
    }

    #[test]
    fn a_grok_login_renewed_by_rename_stays_in_the_fresh_home() {
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
            "trials login v1\n"
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
        assert_eq!(kept, ["auth.json", "auth.json.lock", "sessions"]);
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
        if !bwrap_or_skip("grok lock") {
            return;
        }
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
        let request = layout.request(
            Harness::OpenCode,
            script,
            secrets,
            Some("anthropic/claude"),
            &[],
        );
        let prepared = prepare(&request).unwrap();

        let output = run_script(&prepared, &layout.repo);

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let config: serde_json::Value = serde_json::from_slice(output.stdout.trim_ascii()).unwrap();
        assert_eq!(
            config["provider"]["anthropic"]["apiKey"],
            format!("{{env:{OPENCODE_API_KEY_ENV}}}")
        );
        assert_eq!(config["permission"]["webfetch"], "deny");
        assert_eq!(config["permission"]["websearch"], "deny");
    }

    #[test]
    fn an_opencode_trial_naming_an_unknown_provider_fails_before_its_session_starts() {
        if !bwrap_or_skip("opencode provider") {
            return;
        }
        let layout = Layout::new();
        let secrets = Secrets {
            claude_setup_token: None,
            opencode_api_key: Some("opencode-provider-key".into()),
        };
        let request = layout.request(
            Harness::OpenCode,
            "exit 99",
            secrets,
            Some("acme/model"),
            &[],
        );

        let error = format!("{:#}", prepare(&request).err().unwrap());

        assert!(error.contains("login failed"), "{error}");
        assert!(error.contains("unknown provider acme"), "{error}");
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
    fn a_bound_toolchain_is_reachable_on_the_session_path() {
        if !bwrap_or_skip("toolchain PATH") {
            return;
        }
        let layout = Layout::new();
        let bin = layout.home.join(".cargo/bin");
        fs::create_dir_all(&bin).unwrap();
        let tool = bin.join("fake-tool");
        fs::write(&tool, "#!/bin/sh\necho toolchain-tool\n").unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
        let request = layout.request(Harness::Codex, "fake-tool", Secrets::default(), None, &[]);
        let prepared = prepare(&request).unwrap();

        let output = run_script(&prepared, &layout.repo);

        assert!(
            output.status.success(),
            "bound toolchain not on PATH: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "toolchain-tool"
        );
    }

    #[test]
    fn a_muse_login_under_an_overridden_config_home_is_found_in_place() {
        if !bwrap_or_skip("muse XDG_CONFIG_HOME") {
            return;
        }
        let layout = Layout::new();
        let config_home = layout.home.join("xdg-config");
        fs::create_dir_all(config_home.join("muse")).unwrap();
        fs::write(config_home.join("muse/auth.json"), "{\"login\": \"xdg\"}\n").unwrap();
        // NOTE: mutates process env; run serially with other env-touching tests.
        let saved = std::env::var_os("XDG_CONFIG_HOME");
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &config_home) };
        let prepared = prepare(&layout.request(
            Harness::Muse,
            "cat \"$XDG_CONFIG_HOME/muse/auth.json\"",
            Secrets::default(),
            None,
            &[],
        ));
        match saved {
            Some(value) => unsafe { std::env::set_var("XDG_CONFIG_HOME", value) },
            None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
        }
        let prepared = prepared.unwrap();

        let output = run_script(&prepared, &layout.repo);

        assert!(
            output.status.success(),
            "muse login missed under overridden XDG_CONFIG_HOME: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "{\"login\": \"xdg\"}"
        );
    }

    #[test]
    fn tool_bin_dirs_names_only_bound_roots() {
        let root = tempfile::TempDir::new().unwrap();
        // Tools are matched by canonical path; macOS temp dirs sit behind
        // the /var -> /private/var symlink.
        let toolchain = root.path().canonicalize().unwrap().join("toolchain");
        let deep = toolchain.join("toolchains/stable/bin");
        fs::create_dir_all(&deep).unwrap();
        let tool = deep.join("cargo");
        fs::write(&tool, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
        let path_dirs = vec![deep.clone(), PathBuf::from("/usr/bin")];

        assert_eq!(
            tool_bin_dirs_on(&path_dirs, std::slice::from_ref(&toolchain)),
            vec![deep]
        );
        assert!(tool_bin_dirs_on(&path_dirs, &[root.path().join("elsewhere")]).is_empty());
        assert!(tool_bin_dirs_on(&[], &[toolchain]).is_empty());
    }

    #[test]
    fn a_signal_through_bwrap_ends_the_session_and_its_orphans() {
        if !bwrap_or_skip("signals") {
            return;
        }
        let layout = Layout::new();
        let script = "sleep 60 & echo $! > signal.pid; wait";
        let request = layout.request(Harness::Codex, script, Secrets::default(), None, &[]);
        let prepared = prepare(&request).unwrap();
        let mut child = prepared
            .launch()
            .command(&[])
            .unwrap()
            .current_dir(&layout.repo)
            .spawn()
            .expect("sandboxed session did not start");
        let mut pid = None;
        for _ in 0..100 {
            if let Ok(text) = fs::read_to_string(layout.repo.join("signal.pid"))
                && let Ok(found) = text.trim().parse::<i32>()
            {
                pid = Some(found);
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let pid = pid.expect("sandboxed session never started its orphan");
        // SAFETY: SIGINT asks the sandbox to end its session, as an
        // interrupted Trial's Harness stop does.
        let terminated = unsafe { libc::kill(child.id() as i32, libc::SIGINT) == 0 };
        assert!(terminated);
        let status = child.wait().expect("sandboxed session did not end");
        assert!(!status.success());
        // SAFETY: kill with signal 0 sends nothing; it only reports an error.
        let alive = unsafe { libc::kill(pid, 0) == 0 };
        assert!(!alive, "orphaned sleep {pid} outlived its sandbox");
        fs::remove_file(layout.repo.join("signal.pid")).unwrap();
    }

    #[test]
    fn a_grok_home_copies_only_the_login_and_its_lock() {
        let layout = Layout::new();
        let home = trials_home(&layout.home);
        fs::create_dir_all(home.join("sessions/stale")).unwrap();
        fs::write(home.join("auth.json"), "login\n").unwrap();
        fs::write(home.join("auth.json.lock"), "lock\n").unwrap();
        fs::write(home.join("stray.txt"), "stray\n").unwrap();

        let request = layout.request(Harness::Grok, "", Secrets::default(), None, &[]);
        let _guard = grok_into(&request).unwrap();

        let mut kept: Vec<_> = fs::read_dir(layout.fresh.join(GROK_HOME_NAME))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        kept.sort();
        assert_eq!(kept, ["auth.json", "auth.json.lock"]);
        assert!(home.join("sessions/stale").is_dir());
        assert!(home.join("stray.txt").is_file());
    }
}
