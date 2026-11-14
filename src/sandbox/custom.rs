//! One sandbox for a Custom runner, all its Harnesses and direct-SDK calls.

use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;

use super::*;
use crate::evals::Runner;

/// The attempt contains only this Trial's repository, frozen bundle, fresh
/// home, request and output. The rest of Experiment storage stays unbound.
pub(crate) fn prepare(
    runner: &Runner,
    attempt: &Path,
    home: &Path,
    environment: &[(String, String)],
    record_socket: &Path,
) -> Result<Prepared> {
    let fresh_home = attempt.join("home");
    let credentials = Credentials::read().map_err(|error| no_login(format!("{error:#}")))?;
    let mut binds = Binds::default();
    binds.read_write(attempt, attempt);
    binds.read_only(&attempt.join("runner"), &attempt.join("runner"));
    for path in [
        attempt.join("session"),
        attempt.join("record-command.py"),
        attempt.join("request.json"),
        record_socket.to_path_buf(),
    ] {
        binds.read_only(&path, &path);
    }
    let mut env = Environment::default();
    for (name, value) in environment {
        if name != "PATH" {
            env.set_str(name, value);
        }
    }
    toolchains_into(home, &mut binds, &mut env);
    // Host-prepared Cargo proxies are not under the ordinary toolchain roots.
    if let Some((_, cargo)) = environment.iter().find(|(name, _)| name == "CARGO_HOME") {
        let cargo = Path::new(cargo);
        binds.read_only(cargo, cargo);
        env.prepend_path(&cargo.join("session/bin"));
    }
    let wrappers = attempt.join("harness-wrappers");
    fs::create_dir_all(&wrappers)?;
    let socat = resolve_binary("socat")?;
    let mut allowed = BTreeSet::new();
    let mut grok = None;
    for (name, declared) in &runner.harnesses {
        let harness = Harness::named(name).context("unknown frozen Harness")?;
        let request = Request {
            harness,
            model: None,
            repository: attempt,
            skills_dir: &attempt.join("runner"),
            harness_binary: &declared.binary.path,
            home,
            fresh_home: &fresh_home,
            secrets: Secrets::from(&credentials),
            adapter_env: vec![],
        };
        match harness {
            Harness::Claude => {
                let token = credentials
                    .claude_setup_token()
                    .ok_or_else(|| no_login("declared claude has no Trials setup token"))?;
                env.set_str(CLAUDE_TOKEN_ENV, &token);
            }
            Harness::Codex => required_login(
                &request,
                &mut env,
                Some("CODEX_HOME"),
                ".codex",
                "auth.json",
            )?,
            Harness::Muse => required_login(
                &request,
                &mut env,
                Some("XDG_CONFIG_HOME"),
                ".config",
                "muse/auth.json",
            )?,
            Harness::Agy => required_login(
                &request,
                &mut env,
                None,
                ".gemini/antigravity-cli",
                "antigravity-oauth-token",
            )?,
            Harness::Grok => grok = Some(grok_into(&request)?),
            Harness::OpenCode => opencode(runner, &credentials, &mut env)?,
            Harness::Kimi => kimi(runner, &request, &mut env, &declared.hosts)?,
        }
        for (key, value) in harness.adapter().environment() {
            env.set_str(key, value);
        }
        allowed.extend(declared.hosts.iter().cloned());
        let binary = harness_into(&request, &mut binds, &mut env, &declared.binary.path);
        let wrapper = wrappers.join(name);
        write_without_following(
            &wrapper,
            wrapper_script(harness, &binary, &socat, Path::new(SOCKET_DEST)).as_bytes(),
        )?;
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755))?;
    }
    for provider in &runner.manifest.providers {
        let key = credentials.provider_key(*provider).ok_or_else(|| {
            no_login(format!("missing [providers].{} Trial key", provider.name()))
        })?;
        env.set_str(provider.environment(), key);
        allowed.extend(provider.hosts().iter().map(|host| host.to_string()));
    }
    binds.read_only(&wrappers, &wrappers);
    env.prepend_path(&wrappers);
    let request = Request {
        harness: Harness::Claude,
        model: None,
        repository: attempt,
        skills_dir: &attempt.join("runner"),
        harness_binary: Path::new(SHELL),
        home,
        fresh_home: &fresh_home,
        secrets: Secrets::default(),
        adapter_env: vec![],
    };
    finish(
        &request,
        binds,
        env,
        SHELL.into(),
        grok,
        &allowed.into_iter().collect::<Vec<_>>(),
    )
}

fn no_login(message: impl Into<String>) -> anyhow::Error {
    NoLogin {
        diagnostic: message.into(),
    }
    .into()
}

fn required_login(
    request: &Request<'_>,
    environment: &mut Environment,
    override_env: Option<&str>,
    directory: &str,
    file: &str,
) -> Result<()> {
    let source = override_env
        .and_then(std::env::var_os)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| request.home.join(directory))
        .join(file);
    ensure!(
        source.is_file(),
        no_login(format!(
            "declared {} has no login at {}",
            request.harness.name(),
            source.display()
        ))
    );
    login_into(request, environment, override_env, directory, file)
}

fn opencode(runner: &Runner, credentials: &Credentials, env: &mut Environment) -> Result<()> {
    let keys = credentials.opencode_keys();
    let mut providers = serde_json::Map::new();
    for model in &runner.manifest.harnesses["opencode"] {
        let provider = trial_provider(Some(model))?;
        if providers.contains_key(provider) {
            continue;
        }
        let key = keys
            .for_provider(provider)
            .ok_or_else(|| no_login(format!("missing OpenCode Trial key for {provider}")))?;
        let variable = format!("THIRDSHIFT_OPENCODE_KEY_{}", providers.len());
        env.set_str(&variable, key);
        providers.insert(
            provider.into(),
            serde_json::json!({"apiKey": format!("{{env:{variable}}}")}),
        );
    }
    env.set_str(
        OPENCODE_CONFIG_ENV,
        &serde_json::json!({
            "provider": providers,
            "permission": {"webfetch":"deny", "websearch":"deny"}
        })
        .to_string(),
    );
    Ok(())
}

/// Copy only the declared aliases and the providers they use, retaining a
/// declared provider's login while excluding all other Kimi credentials.
fn kimi(
    runner: &Runner,
    request: &Request<'_>,
    env: &mut Environment,
    allowed: &BTreeSet<String>,
) -> Result<()> {
    use crate::harness::kimi::{HOME_ENV, HOME_NAME};
    let source = crate::harness::kimi::home(None).context("missing Kimi home")?;
    let text = fs::read_to_string(source.join("config.toml"))
        .map_err(|_| no_login("declared kimi has no config.toml"))?;
    let config: toml::Table = toml::from_str(&text)
        .map_err(|_| no_login("invalid Kimi config.toml (contents hidden)"))?;
    let mut models = toml::Table::new();
    let mut providers = toml::Table::new();
    let mut managed = false;
    for alias in &runner.manifest.harnesses["kimi"] {
        let model = config
            .get("models")
            .and_then(|models| models.get(alias))
            .cloned()
            .ok_or_else(|| no_login(format!("missing Kimi alias {alias}")))?;
        let id = model
            .get("provider")
            .and_then(toml::Value::as_str)
            .context("Kimi alias has no provider")?;
        let provider = config
            .get("providers")
            .and_then(|providers| providers.get(id))
            .cloned()
            .ok_or_else(|| no_login(format!("missing Kimi provider {id}")))?;
        let base_url = model
            .get("base_url")
            .and_then(toml::Value::as_str)
            .or_else(|| kimi_setting(&provider, "base_url"))
            .unwrap_or("https://api.kimi.com/coding/v1");
        let hosts = proxy::kimi_hosts(base_url)?;
        ensure!(
            hosts.iter().all(|host| allowed.contains(*host)),
            no_login("Kimi provider hosts changed since Arm preparation")
        );
        if let Some(variable) = provider.get("api_key_env").and_then(toml::Value::as_str) {
            let key = std::env::var_os(variable)
                .filter(|key| !key.is_empty())
                .ok_or_else(|| no_login(format!("missing Kimi Trial login variable {variable}")))?;
            env.set(variable, Path::new(&key));
        } else if kimi_setting(&provider, "api_key").is_some() {
            // Direct and provider-env keys stay in this private config copy.
        } else {
            managed = true;
        }
        providers.insert(id.into(), provider);
        models.insert(alias.clone(), model);
    }
    if managed {
        required_login(
            request,
            env,
            Some(HOME_ENV),
            HOME_NAME,
            "credentials/kimi-code.json",
        )?;
    }
    let directory = request.fresh_home.join(HOME_NAME);
    reject_fresh_symlink(request.fresh_home, &directory)?;
    fs::create_dir_all(&directory)?;
    let config = toml::Table::from_iter([
        (
            "default_model".into(),
            toml::Value::String(runner.manifest.harnesses["kimi"][0].clone()),
        ),
        ("models".into(), toml::Value::Table(models)),
        ("providers".into(), toml::Value::Table(providers)),
    ]);
    write_without_following(
        &directory.join("config.toml"),
        toml::to_string(&config)?.as_bytes(),
    )?;
    env.set(HOME_ENV, &directory);
    Ok(())
}

/// Kimi's provider config accepts conventional names inside its env table.
/// Those values stay in the config rather than the process environment.
fn kimi_setting<'a>(provider: &'a toml::Value, field: &str) -> Option<&'a str> {
    provider
        .get(field)
        .and_then(toml::Value::as_str)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let variable = match (provider.get("type")?.as_str()?, field) {
                ("kimi", "api_key") => "KIMI_API_KEY",
                ("kimi", "base_url") => "KIMI_BASE_URL",
                ("anthropic", "api_key") => "ANTHROPIC_API_KEY",
                ("anthropic", "base_url") => "ANTHROPIC_BASE_URL",
                ("openai" | "openai_responses", "api_key") => "OPENAI_API_KEY",
                ("openai" | "openai_responses", "base_url") => "OPENAI_BASE_URL",
                ("google-genai", "api_key") => "GOOGLE_API_KEY",
                _ => return None,
            };
            provider
                .get("env")?
                .get(variable)?
                .as_str()
                .filter(|value| !value.is_empty())
        })
}

/// Give each real Harness execution a relay whose connections the host can
/// correlate, including concurrent invocations. The SYSTEM command's quotes
/// survive socat's address parsing before its shell expands the environment.
fn wrapper_script(harness: Harness, binary: &Path, socat: &Path, socket: &Path) -> String {
    let switches = crate::args::shell_command(
        harness
            .adapter()
            .trial_web_restrictions()
            .iter()
            .map(|arg| arg.to_string())
            .collect(),
    );
    let quoted = |path: &Path| {
        crate::args::shell_command(vec![path.as_os_str().to_string_lossy().into_owned()])
    };
    let binary = quoted(binary);
    let socat = quoted(socat);
    let socket = quoted(socket);
    let method = proxy::HARNESS_REGISTER_METHOD;
    let prefix = proxy::HARNESS_CONNECTION_PREFIX;
    let base = proxy::HARNESS_RELAY_BASE;
    let name = harness.name();
    format!(
        r#"#!/bin/sh
if [ "$#" -eq 1 ]; then
    case "$1" in
        --version|-V|version|--help|-h|help) exec {binary} {switches} "$@" ;;
    esac
fi
invocation=$(printf '{method} {name} HTTP/1.1\r\n\r\n' | {socat} - UNIX-CONNECT:{socket})
case "$invocation" in
    ''|*[!0-9]*) echo 'sandbox Harness accounting failed: registration unavailable' >&2; exit 1 ;;
esac
port=$(({base} + invocation))
THIRDSHIFT_PROXY_INVOCATION="$invocation" THIRDSHIFT_PROXY_SOCAT={socat} THIRDSHIFT_PROXY_SOCKET=UNIX-CONNECT:{socket} \
    {socat} "TCP-LISTEN:$port,bind=127.0.0.1,reuseaddr,fork" \
    'SYSTEM:(printf \"{prefix}%s\\n\" \"$THIRDSHIFT_PROXY_INVOCATION\"; /bin/cat) | exec \"$THIRDSHIFT_PROXY_SOCAT\" - \"$THIRDSHIFT_PROXY_SOCKET\"' \
    </dev/null >/dev/null 2>/dev/null & relay=$!
trap 'kill "$relay" 2>/dev/null; wait "$relay" 2>/dev/null' EXIT
i=0
while [ "$i" -lt 50 ]; do
    {socat} - "TCP:127.0.0.1:$port" </dev/null >/dev/null 2>&1 && break
    i=$((i + 1)); sleep 0.1
done
if ! {socat} - "TCP:127.0.0.1:$port" </dev/null >/dev/null 2>&1; then
    echo 'sandbox Harness accounting failed: relay unavailable' >&2; exit 1
fi
HTTPS_PROXY="http://127.0.0.1:$port" HTTP_PROXY="http://127.0.0.1:$port" \
https_proxy="http://127.0.0.1:$port" http_proxy="http://127.0.0.1:$port" \
    {binary} {switches} "$@"
exit "$?"
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    #[test]
    fn wrappers_count_non_probe_runs_and_preserve_the_harness_protocol() {
        let Ok(socat) = resolve_binary("socat") else {
            println!("SKIPPED (Custom runner wrapper): socat is not on PATH");
            return;
        };
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("harness with 'quotes'");
        let response = directory.path().join("proxy-response");
        fs::write(
            &binary,
            format!(
                r#"#!/bin/sh
case "$*" in *--version|*--help) printf 'probe'; exit 0 ;; esac
printf '%s\n' "$@"
cat
printf 'CONNECT example.com:443 HTTP/1.1\r\n\r\n' | '{}' - "TCP:127.0.0.1:${{HTTPS_PROXY##*:}}" > '{}'
printf 'harness stderr' >&2
exit 7
"#,
                socat.display(),
                response.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        let guard = proxy::start(&["api.anthropic.com".into()]).unwrap();
        let wrapper = directory.path().join("claude");
        fs::write(
            &wrapper,
            wrapper_script(Harness::Claude, &binary, &socat, &guard.socket_path()),
        )
        .unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
        for probe in ["--version", "--help"] {
            let result = Command::new(&wrapper).arg(probe).output().unwrap();
            assert!(result.status.success(), "{result:?}");
            assert_eq!(result.stdout, b"probe");
        }
        assert!(guard.accounting().harness_invocations.is_empty());
        let mut child = Command::new(&wrapper)
            .args(["-p", "test"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"stdin preserved")
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(7), "{output:?}");
        assert_eq!(
            output.stdout,
            b"--disallowedTools=WebSearch,WebFetch\n-p\ntest\nstdin preserved"
        );
        assert_eq!(output.stderr, b"harness stderr");
        let response = fs::read_to_string(response).unwrap();
        assert!(
            response.starts_with("HTTP/1.1 403 Forbidden\r\n"),
            "{response:?}"
        );
        let accounting = guard.accounting();
        assert_eq!(accounting.provider_connections, 0);
        assert_eq!(accounting.harness_invocations.len(), 1);
        assert_eq!(accounting.harness_invocations[0].harness, Harness::Claude);
        assert_eq!(accounting.harness_invocations[0].provider_connections, 0);
    }
}
