use super::*;

fn declarations(fixture: &Fixture, text: &str) {
    let path = fixture
        .scenario
        .path("home/.thirdshift/runners/fixture.toml");
    let manifest = fs::read_to_string(&path).unwrap();
    fs::write(path, format!("{text}\n{manifest}")).unwrap();
}

fn harness_script(fixture: &Fixture, name: &str, body: &str) {
    let path = fixture.scenario.path(&format!("bin/{name}"));
    let _ = fs::remove_file(&path);
    fs::write(&path, format!("#!/bin/sh\nif [ \"$1\" = --version ]; then echo '{name} fixture 1'; exit 0; fi\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn custom_runner_preparation_pins_declared_harnesses_and_provider_hosts() {
    let fixture = process::fixture();
    runner(&fixture, DELIVER);
    harness_script(&fixture, "claude", "exit 0");
    harness_script(&fixture, "opencode", "exit 0");
    declarations(
        &fixture,
        "providers = [\"openai\", \"google\", \"kimi\"]\nharnesses = { claude = [], opencode = [\"anthropic/claude\", \"baseten/model\"] }",
    );
    let record = fixture.scenario.path("arms.json");
    let result = fixture.scenario.run(&[
        "evals",
        "prepare-arms",
        ".",
        record.to_str().unwrap(),
        "--baseline-runner",
        "fixture",
        "--candidate-runner",
        "fixture",
        "--suite",
        "process",
    ]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let record: Value = serde_json::from_slice(&fs::read(record).unwrap()).unwrap();
    for arm in ["baseline", "candidate"] {
        let fixed = &record[arm]["runner"]["harnesses"]["claude"]["binary"];
        assert_eq!(fixed["name"], "claude");
        assert!(PathBuf::from(fixed["path"].as_str().unwrap()).is_absolute());
        assert!(!fixed["version"].as_str().unwrap().is_empty());
        let hosts = record[arm]["runner"]["harnesses"]["opencode"]["hosts"]
            .as_array()
            .unwrap();
        assert!(hosts.contains(&json!("api.anthropic.com")), "{hosts:?}");
        assert!(hosts.contains(&json!("inference.baseten.co")), "{hosts:?}");
        assert!(!hosts.contains(&json!("api.openai.com")), "{hosts:?}");
    }
}

#[test]
fn custom_runner_preparation_refuses_unresolved_provider_declarations() {
    for (declaration, evidence) in [
        ("providers = [\"unknown\"]", "unknown variant"),
        (
            "harnesses = { unknown = [] }",
            "unknown Custom runner Harness",
        ),
        ("harnesses = { opencode = [] }", "requires explicit Models"),
        ("harnesses = { kimi = [] }", "requires explicit Models"),
        (
            "harnesses = { opencode = [\"unknown/model\"] }",
            "unknown provider",
        ),
        (
            "harnesses = { opencode = [\"anthropic/\"] }",
            "<provider>/<model>",
        ),
        (
            "harnesses = { kimi = [\"unknown\"] }",
            "not in Kimi Code's config",
        ),
    ] {
        let fixture = process::fixture();
        runner(&fixture, DELIVER);
        harness_script(&fixture, "opencode", "exit 0");
        harness_script(
            &fixture,
            "kimi",
            &format!(
                "cat <<'JSON'\n{}\nJSON",
                include_str!("../fixtures/kimi-providers.json")
            ),
        );
        declarations(&fixture, declaration);
        let record = fixture.scenario.path("arms.json");
        let result = fixture.scenario.run(&[
            "evals",
            "prepare-arms",
            ".",
            record.to_str().unwrap(),
            "--baseline-runner",
            "fixture",
            "--candidate-runner",
            "fixture",
            "--suite",
            "process",
        ]);
        assert_ne!(result.code, Some(0), "{declaration} must be refused");
        assert!(result.stderr.contains(evidence), "{}", result.stderr);
        assert!(!record.exists());
    }
}

#[test]
fn custom_runner_sandbox_confines_children_credentials_network_and_harness_io() {
    if !std::process::Command::new("bwrap")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        println!("SKIPPED (Custom runner sandbox): bwrap is not on PATH");
        return;
    }
    let fixture = process::fixture();
    harness_script(
        &fixture,
        "claude",
        "printf '%s\\n' \"$@\"; cat; printf 'harness stderr' >&2; exit 7",
    );
    harness_script(&fixture, "opencode", "exit 0");
    fixture.scenario.credentials_are("[claude]\nsetup_token = \"trial-login\"\n[providers]\nopenai = \"trial-sdk-key\"\nxai = \"undeclared-key\"\n[opencode.keys]\nanthropic = \"anthropic-key\"\nbaseten = \"baseten-key\"\nxai = \"undeclared-opencode-key\"\n");
    let hidden = fixture.scenario.path("operator-private");
    fs::write(&hidden, "operator secret").unwrap();
    fs::create_dir_all(fixture.scenario.path("home/.codex")).unwrap();
    fs::write(
        fixture.scenario.path("home/.codex/auth.json"),
        "undeclared login",
    )
    .unwrap();
    runner(
        &fixture,
        &(format!(
            r#"
assert not pathlib.Path({hidden:?}).exists(), 'unbound host file exposed'
assert 'THIRDSHIFT_OPERATOR_TOKEN' not in os.environ
assert os.environ['OPENAI_API_KEY'] == 'trial-sdk-key'
assert 'XAI_API_KEY' not in os.environ
assert os.environ['CLAUDE_CODE_OAUTH_TOKEN'] == 'trial-login'
assert not (pathlib.Path(os.environ['HOME']) / '.codex/auth.json').exists()
assert not (pathlib.Path(os.environ['HOME']) / '.thirdshift/credentials.toml').exists()
config = json.loads(os.environ['OPENCODE_CONFIG_CONTENT'])
assert sorted(config['provider']) == ['anthropic', 'baseten'], config
assert config['permission'] == {{'webfetch':'deny', 'websearch':'deny'}}, config
assert 'undeclared' not in str(config)
child = subprocess.run(['claude', '-p', 'test'], input='stdin preserved', text=True, capture_output=True)
assert child.returncode == 7, child
assert child.stdout == '--disallowedTools=WebSearch,WebFetch\n-p\ntest\nstdin preserved', child
assert child.stderr == 'harness stderr', child
import socket
proxy = os.environ['HTTPS_PROXY'].split('://')[1].split(':')
for host in ['github.com', 'example.com', 'api.x.ai']:
    with socket.create_connection((proxy[0], int(proxy[1])), timeout=5) as sock:
        sock.sendall(('CONNECT '+host+':443 HTTP/1.1\r\nHost: '+host+'\r\n\r\n').encode())
        assert b'403' in sock.recv(4096), host
"#
        ) + DELIVER),
    );
    declarations(
        &fixture,
        "providers = [\"openai\"]\nharnesses = { claude = [], opencode = [\"anthropic/claude\", \"baseten/model\"] }",
    );
    let result = fixture.scenario.run_with_env(
        &runner_args(),
        &[("THIRDSHIFT_OPERATOR_TOKEN", "operator-token")],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let view: Value = serde_json::from_str(&result.stdout).unwrap();
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "delivered", "{result}");
        assert_eq!(result["score"]["passed"], true, "{result}");
    }
}

fn runner_args() -> Vec<&'static str> {
    vec![
        "experiment",
        "--baseline-runner",
        "fixture",
        "--candidate-runner",
        "fixture",
        "--suite",
        "process",
        "--json",
        "--yes",
        "--draws",
        "100",
        "--min-discriminating-tasks",
        "1",
    ]
}

#[test]
fn custom_runner_families_constrain_the_panel_before_trials_start() {
    for declaration in [
        "providers = [\"openai\"]",
        "harnesses = { codex = [] }",
        "harnesses = { opencode = [\"openai/model\"] }",
    ] {
        let fixture = process::fixture();
        pairs::pair_fixture(&fixture, pairs::CANDIDATE_TEST);
        runner(&fixture, DELIVER);
        harness_script(&fixture, "claude", "exit 0");
        harness_script(&fixture, "codex", "exit 0");
        harness_script(&fixture, "opencode", "exit 0");
        declarations(&fixture, declaration);
        let result = run(
            &fixture,
            &[
                "--panel-seat",
                "harness=claude,model=judge-one",
                "--panel-seat",
                "harness=codex,model=judge-two",
                "--panel",
                "counted",
                "--people",
                "counted",
                "--people-reader",
                "ann@example.com",
            ],
        );
        assert_ne!(
            result.code,
            Some(0),
            "stand-in must be refused: {}",
            result.stdout
        );
        assert!(
            result
                .stderr
                .contains("the voting Grader panel spans 1 family (anthropic)"),
            "{}",
            result.stderr
        );
        let trials = fixture.scenario.path("home/.thirdshift/evals/experiments");
        if trials.is_dir() {
            for experiment in fs::read_dir(trials).unwrap() {
                assert!(!experiment.unwrap().path().join("started").exists());
            }
        }
    }
}

#[test]
fn missing_declared_credentials_void_custom_trials_without_using_ambient_keys() {
    for declaration in [
        "providers = [\"openai\"]",
        "harnesses = { claude = [] }",
        "harnesses = { codex = [] }",
        "harnesses = { muse = [] }",
        "harnesses = { agy = [] }",
        "harnesses = { grok = [] }",
        "harnesses = { opencode = [\"anthropic/model\"] }",
    ] {
        let fixture = process::fixture();
        runner(
            &fixture,
            "raise Exception('runner must not start without declared credentials')",
        );
        for name in ["claude", "codex", "muse", "agy", "grok", "opencode"] {
            harness_script(&fixture, name, "exit 0");
        }
        declarations(&fixture, declaration);
        let result = fixture
            .scenario
            .run_with_env(&runner_args(), &[("OPENAI_API_KEY", "ambient-key")]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let view: Value = serde_json::from_str(&result.stdout).unwrap();
        let results = view["results"].as_array().unwrap();
        assert_eq!(results.len(), 2, "{view}");
        for result in results {
            assert_eq!(result["outcome"], "void", "{result}");
            for attempt in result["attempts"].as_array().unwrap() {
                assert_eq!(attempt["void_cause"], "session_unavailable", "{attempt}");
                assert!(
                    !attempt.to_string().contains("runner must not start"),
                    "{attempt}"
                );
            }
        }
    }
}

#[test]
fn declared_harness_wrappers_apply_web_restrictions_and_preserve_process_io() {
    if !std::process::Command::new("bwrap")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        println!("SKIPPED (Custom runner wrappers): bwrap is not on PATH");
        return;
    }
    let fixture = process::fixture();
    for name in ["claude", "codex", "grok", "muse"] {
        harness_script(
            &fixture,
            name,
            "printf '%s\\n' \"$@\"; cat; printf 'stderr' >&2; exit 7",
        );
    }
    fixture
        .scenario
        .credentials_are("[claude]\nsetup_token = \"trial-login\"\n");
    for relative in [
        ".codex/auth.json",
        ".config/muse/auth.json",
        ".thirdshift/trials/grok/auth.json",
    ] {
        let login = fixture.scenario.path(&format!("home/{relative}"));
        fs::create_dir_all(login.parent().unwrap()).unwrap();
        fs::write(login, "fixture-login").unwrap();
    }
    runner(
        &fixture,
        &(String::from(
            r#"
for name, switches in [
    ('claude', ['--disallowedTools=WebSearch,WebFetch']),
    ('codex', ['-c', 'web_search="disabled"']),
    ('grok', ['--disable-web-search']),
    ('muse', ['--disable-web-tools']),
]:
    child = subprocess.run([name, '-p', 'prompt'], input='stdin', text=True, capture_output=True)
    assert child.returncode == 7, child
    assert child.stdout == '\n'.join(switches + ['-p', 'prompt', 'stdin']), child
    assert child.stderr == 'stderr', child
assert (pathlib.Path(os.environ['HOME']) / '.grok/auth.json').read_text() == 'fixture-login'
"#,
        ) + DELIVER),
    );
    declarations(
        &fixture,
        "harnesses = { claude = [], codex = [], grok = [], muse = [] }",
    );
    let result = fixture.scenario.run(&runner_args());
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let view: Value = serde_json::from_str(&result.stdout).unwrap();
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "delivered", "{result}");
        assert_eq!(result["score"]["passed"], true, "{result}");
    }
}

#[test]
fn custom_kimi_trials_keep_only_declared_alias_logins_and_provider_families() {
    if !std::process::Command::new("bwrap")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        println!("SKIPPED (Custom Kimi runner): bwrap is not on PATH");
        return;
    }
    let fixture = process::fixture();
    harness_script(
        &fixture,
        "kimi",
        &format!(
            "cat <<'JSON'\n{}\nJSON",
            include_str!("../fixtures/kimi-providers.json")
        ),
    );
    let home = fixture.scenario.path("home/.kimi-code");
    fs::create_dir_all(home.join("credentials")).unwrap();
    fs::write(home.join("credentials/kimi-code.json"), "managed-login").unwrap();
    fs::write(
        home.join("config.toml"),
        r#"
default_model = "undeclared"
[providers.fireworks]
type = "openai"
base_url = "https://api.fireworks.ai/inference/v1"
api_key_env = "FIREWORKS_API_KEY"
[providers."managed:kimi-code"]
type = "kimi"
base_url = "https://api.kimi.com/coding/v1"
[providers.undeclared]
type = "openai"
api_key = "undeclared-key"
[models."kimi-k3.1-pro"]
provider = "fireworks"
model = "accounts/fireworks/models/kimi-k3p1-pro"
max_context_size = 262144
[models."kimi-code/kimi-for-coding"]
provider = "managed:kimi-code"
model = "kimi-for-coding"
max_context_size = 262144
[models.undeclared]
provider = "undeclared"
model = "undeclared"
"#,
    )
    .unwrap();
    runner(
        &fixture,
        &(String::from(
            r#"
import tomllib
home = pathlib.Path(os.environ['KIMI_CODE_HOME'])
config = tomllib.loads((home / 'config.toml').read_text())
assert sorted(config['models']) == ['kimi-code/kimi-for-coding', 'kimi-k3.1-pro'], config
assert sorted(config['providers']) == ['fireworks', 'managed:kimi-code'], config
assert 'undeclared-key' not in str(config), config
assert os.environ['FIREWORKS_API_KEY'] == 'declared-login'
assert (home / 'credentials/kimi-code.json').read_text() == 'managed-login'
"#,
        ) + DELIVER),
    );
    declarations(
        &fixture,
        "harnesses = { kimi = [\"kimi-k3.1-pro\", \"kimi-code/kimi-for-coding\"] }",
    );
    let result = fixture
        .scenario
        .run_with_env(&runner_args(), &[("FIREWORKS_API_KEY", "declared-login")]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let view: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(
        view["prepared"]["baseline"]["runner"]["harnesses"]["kimi"]["families"],
        json!(["fireworks-ai", "moonshot"])
    );
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "delivered", "{result}");
        assert_eq!(result["score"]["passed"], true, "{result}");
    }
}

#[test]
fn custom_runner_does_not_inherit_the_operator_environment() {
    let fixture = process::fixture();
    runner(
        &fixture,
        &(String::from(
            r#"leaked = os.environ.get("THIRDSHIFT_OPERATOR_TOKEN")
assert leaked is None, leaked
assert os.environ["HOME"]
assert os.environ["PATH"]
"#,
        ) + DELIVER),
    );
    let result = fixture.scenario.run_with_env(
        &[
            "experiment",
            "--baseline-runner",
            "fixture",
            "--candidate-runner",
            "fixture",
            "--suite",
            "process",
            "--json",
            "--yes",
            "--draws",
            "100",
            "--min-discriminating-tasks",
            "1",
        ],
        &[
            NO_SANDBOX[0],
            ("THIRDSHIFT_OPERATOR_TOKEN", "dummy-operator-token"),
        ],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        !result.stdout.contains("dummy-operator-token"),
        "{}",
        result.stdout
    );
    let view: Value = serde_json::from_str(&result.stdout).unwrap();
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "delivered", "{result}");
        assert_eq!(result["score"]["passed"], true, "{result}");
        assert!(
            !result.to_string().contains("dummy-operator-token"),
            "{result}"
        );
    }
}

#[test]
fn custom_runner_reports_ignored_uncommitted_scratch_in_trial_diagnostics() {
    let fixture = process::fixture();
    runner(
        &fixture,
        &DELIVER.replace(
            "pathlib.Path('feature.txt').write_text('uncommitted regression\\n')",
            "",
        ),
    );
    let result = run(&fixture, &[]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let view: Value = serde_json::from_str(&result.stdout).unwrap();
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "delivered", "{result}");
        assert_eq!(result["score"]["passed"], true, "{result}");
        let diagnostic = result["attempts"][0]["cause"].as_str().unwrap_or("");
        assert!(diagnostic.contains(".scratch/private.txt"), "{result}");
    }
}

#[test]
fn custom_runner_can_use_the_host_prepared_rust_toolchain() {
    let fixture = process::fixture();
    let task = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/process/tasks/fixture/task.toml");
    fs::write(
        task.parent().unwrap().join("rust-pin"),
        "[toolchain]\nchannel = \"1.99.0\"\n",
    )
    .unwrap();
    let text = fs::read_to_string(&task).unwrap();
    fs::write(
        &task,
        text.replace(
            "[inputs]\n",
            "[inputs]\n\"rust-toolchain.toml\" = \"rust-pin\"\n",
        ),
    )
    .unwrap();
    fs::create_dir_all(fixture.scenario.path("home/.rustup")).unwrap();
    let rustup = r#"#!/usr/bin/env python3
import os, pathlib, sys
home = pathlib.Path(os.environ.get('RUSTUP_HOME', str(pathlib.Path(os.environ['HOME']) / '.rustup')))
if sys.argv[1:3] == ['toolchain', 'install']:
    (home / 'installed').write_text('1.99.0')
elif (home / 'installed').is_file():
    print('cargo 1.99.0 (fixture)')
else:
    print('prepared Rust toolchain is not available in this HOME', file=sys.stderr)
    sys.exit(1)
"#;
    for name in ["rustup", "cargo"] {
        let path = fixture.scenario.path(&format!("bin/{name}"));
        fs::write(&path, rustup).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    runner(
        &fixture,
        &(String::from("subprocess.run(['cargo', '--version'], check=True)\n") + DELIVER),
    );
    let result = run(&fixture, &[]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let view: Value = serde_json::from_str(&result.stdout).unwrap();
    assert!(fixture.scenario.path("home/.rustup/installed").is_file());
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "delivered", "{result}");
        assert_eq!(result["score"]["passed"], true, "{result}");
    }
}

#[test]
fn a_self_reported_void_cannot_hide_committed_excluded_paths() {
    let fixture = process::fixture();
    let body = DELIVER.replace(
        "subprocess.run(['git', 'commit'",
        "subprocess.run(['git', 'add', '-f', '.scratch/private.txt'], check=True)\nsubprocess.run(['git', 'commit'",
    ) + "\njson.dump({'version':1,'outcome':'void','void_cause':'session_unavailable'}, open(output / 'result.json','w'))\n";
    runner(&fixture, &body);
    let result = run(&fixture, &[]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let view: Value = serde_json::from_str(&result.stdout).unwrap();
    let results = view["results"].as_array().unwrap();
    assert_eq!(results.len(), 2, "{view}");
    for result in results {
        assert_eq!(result["outcome"], "failed", "{result}");
        assert_eq!(result["attempts"].as_array().unwrap().len(), 1, "{result}");
        assert!(
            result["attempts"][0]["cause"]
                .as_str()
                .unwrap()
                .contains("git_exclude path on HEAD"),
            "{result}"
        );
    }
}

#[test]
fn nonzero_exit_without_outer_void_evidence_fails_despite_a_runner_void_claim() {
    let fixture = process::fixture();
    runner(
        &fixture,
        "json.dump({'version':1,'outcome':'void','void_cause':'session_unavailable'}, open(output / 'result.json','w'))\nprint('runner exited seven', file=sys.stderr)\nsys.exit(7)",
    );
    let result = run(&fixture, &[]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let view: Value = serde_json::from_str(&result.stdout).unwrap();
    let results = view["results"].as_array().unwrap();
    assert_eq!(results.len(), 2, "{view}");
    for result in results {
        assert_eq!(result["outcome"], "failed", "{result}");
        assert_eq!(result["attempts"].as_array().unwrap().len(), 1, "{result}");
        let cause = result["attempts"][0]["cause"].as_str().unwrap();
        assert!(cause.contains("exit status: 7"), "{cause}");
        assert!(cause.contains("runner exited seven"), "{cause}");
    }
}

fn runner(fixture: &Fixture, body: &str) -> String {
    let directory = fixture.scenario.path("home/.thirdshift/runners");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("runner.py"),
        format!(
            r#"import json, os, pathlib, subprocess, sys
request = json.load(open(sys.argv[1]))
assert request['version'] == 1
assert pathlib.Path(request['issue_file']).is_file()
assert pathlib.Path(request['repository']).resolve() == pathlib.Path.cwd()
assert sys.stdin.read() == ''
output = pathlib.Path(request['output_dir'])
assert pathlib.Path(os.environ['HOME']).is_dir()
assert 'record_command' in request
{body}
"#
        ),
    )
    .unwrap();
    fs::write(
        directory.join("fixture.toml"),
        r#"
command = ["python3", "{runner_dir}/runner.py"]
git_exclude = [".scratch/"]
[files]
"runner.py" = "runner.py"
"#,
    )
    .unwrap();
    "fixture".into()
}

const DELIVER: &str = r#"
subprocess.run(['git', 'config', 'user.name', 'Eval'], check=True)
subprocess.run(['git', 'config', 'user.email', 'eval@example.invalid'], check=True)
pathlib.Path('feature.txt').write_text('implemented\n')
pathlib.Path('.scratch').mkdir()
pathlib.Path('.scratch/private.txt').write_text('runner scratch')
subprocess.run(['git', 'add', '.'], check=True)
subprocess.run(['git', 'commit', '-qm', 'Custom runner change'], check=True)
pathlib.Path('feature.txt').write_text('uncommitted regression\n')
json.dump({'version':1, 'outcome':'delivered'}, open(output / 'result.json', 'w'))
"#;

#[test]
fn custom_runner_preserves_empty_mapped_folders_in_bundle_and_home() {
    let fixture = process::fixture();
    runner(
        &fixture,
        &(String::from(
            "assert pathlib.Path(request['runner_dir'], 'mapped-empty/nested').is_dir()\nassert pathlib.Path(os.environ['HOME'], '.agents/skills/empty/nested').is_dir()\n",
        ) + DELIVER),
    );
    let source = fixture.scenario.path("home/.thirdshift/runners");
    fs::create_dir_all(source.join("empty/nested")).unwrap();
    let manifest = source.join("fixture.toml");
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(manifest, text + "\n\"empty\" = \"mapped-empty\"\n\"./empty\" = { home = \".agents/skills/empty\" }\n").unwrap();
    let result = run(&fixture, &[]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let view: Value = serde_json::from_str(&result.stdout).unwrap();
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "delivered", "{result}");
        assert_eq!(result["score"]["passed"], true, "{result}");
    }
    let files = &view["prepared"]["baseline"]["runner"]["files"];
    assert_eq!(
        files["home/.agents/skills/empty/nested/"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    assert_eq!(
        files["bundle/mapped-empty/nested/"].as_str().unwrap().len(),
        64
    );
}

fn run(fixture: &Fixture, extra: &[&str]) -> support::RunResult {
    let mut args = vec![
        "experiment",
        "--baseline-runner",
        "fixture",
        "--candidate-runner",
        "fixture",
        "--suite",
        "process",
        "--json",
        "--yes",
        "--draws",
        "100",
        "--min-discriminating-tasks",
        "1",
    ];
    args.extend_from_slice(extra);
    fixture.scenario.run_with_env(&args, &NO_SANDBOX)
}

#[test]
fn custom_runners_grade_committed_head_and_ignore_working_tree_scratch() {
    let fixture = process::fixture();
    runner(&fixture, DELIVER);
    let result = run(&fixture, &[]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let view: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(view["format"], 13);
    assert!(view["baseline"]["commit"].is_null());
    assert!(view["candidate"]["harness"].is_null());
    assert_eq!(view["baseline"]["runner"], "fixture");
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "delivered", "{result}");
        assert_eq!(result["score"]["passed"], true, "{result}");
        assert!(
            result["attempts"][0]["cause"]
                .as_str()
                .unwrap()
                .contains("uncommitted"),
            "{result}"
        );
    }
    result_format::assert_documented("Experiment", &view);
}

#[test]
fn broken_custom_runner_protocols_fail_trials_without_aborting_the_experiment() {
    for (body, evidence) in [
        ("pass", "result.json"),
        (
            "(output / 'result.json').write_text('unreadable json')",
            "invalid Custom runner",
        ),
        (
            "print('runner crashed', file=sys.stderr)\nsys.exit(7)",
            "exit status: 7",
        ),
        (
            "json.dump({'version':1,'outcome':'delivered'}, open(output / 'result.json','w'))\nsys.exit(9)",
            "nonzero",
        ),
        (
            "json.dump({'version':1,'outcome':'void','void_cause':'memory_cap'}, open(output / 'result.json','w'))",
            "outer void_cause",
        ),
        (
            "json.dump({'version':2,'outcome':'delivered'}, open(output / 'result.json','w'))",
            "unsupported",
        ),
    ] {
        let fixture = process::fixture();
        runner(&fixture, body);
        let run = run(&fixture, &[]);
        assert_eq!(run.code, Some(0), "{evidence}: {}", run.stderr);
        let view: Value = serde_json::from_str(&run.stdout).unwrap();
        assert_eq!(view["state"], "completed");
        for result in view["results"].as_array().unwrap() {
            assert_eq!(result["outcome"], "failed", "{result}");
            assert_eq!(result["score"]["passed"], false, "{result}");
            let cause = result["attempts"][0]["cause"].as_str().unwrap();
            assert!(
                cause.contains(evidence)
                    && cause.contains("exit status:")
                    && cause.contains("stderr:"),
                "{cause}"
            );
        }
    }
}

#[test]
fn force_committed_excluded_paths_fail_even_when_the_runner_changes_its_index() {
    let fixture = process::fixture();
    let body = DELIVER.replace("subprocess.run(['git', 'commit'", "subprocess.run(['git', 'add', '-f', '.scratch/private.txt'], check=True)\nsubprocess.run(['git', 'commit'")
        + "\nsubprocess.run(['git', 'rm', '--cached', '.scratch/private.txt'], check=True)\n";
    runner(&fixture, &body);
    let run = run(&fixture, &[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "failed", "{result}");
        assert!(
            result["attempts"][0]["cause"]
                .as_str()
                .unwrap()
                .contains("git_exclude path on HEAD"),
            "{result}"
        );
    }
}

#[test]
fn custom_runner_stopped_to_ask_is_a_failed_score_and_tallied() {
    let fixture = process::fixture();
    runner(
        &fixture,
        "json.dump({'version':1,'outcome':'stopped_to_ask','cause':'Which implementation?'}, open(output / 'result.json','w'))",
    );
    let run = run(&fixture, &[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "stopped_to_ask");
        assert_eq!(result["score"]["passed"], false);
    }
    assert_eq!(view["outcomes"]["baseline"]["stopped_to_ask"], 1);
    assert_eq!(view["outcomes"]["candidate"]["stopped_to_ask"], 1);
}

#[test]
fn mixed_arms_work_in_both_directions_and_candidate_runner_defaults_baseline_to_head() {
    for custom_baseline in [false, true] {
        let fixture = process::fixture();
        runner(&fixture, DELIVER);
        let args = if custom_baseline {
            vec![
                "experiment",
                "candidate",
                "--baseline-runner",
                "fixture",
                "--suite",
                "process",
                "--json",
                "--yes",
                "--draws",
                "100",
            ]
        } else {
            vec![
                "experiment",
                "--candidate-runner",
                "fixture",
                "--suite",
                "process",
                "--json",
                "--yes",
                "--draws",
                "100",
            ]
        };
        let run = fixture.scenario.run_with_env(&args, &NO_SANDBOX);
        assert_eq!(run.code, Some(0), "{}", run.stderr);
        let view: Value = serde_json::from_str(&run.stdout).unwrap();
        let native = if custom_baseline {
            "candidate"
        } else {
            "baseline"
        };
        assert_eq!(view[native]["commit"], fixture.candidate);
        assert_eq!(view[native]["process"], "ticket-run");
        assert!(view["results"].as_array().unwrap().iter().all(|result| result["outcome"] == "delivered" && result["score"]["passed"] == true), "{view}");
    }
}

#[test]
fn custom_runner_selection_refuses_conflicts_and_review_suites_before_dispatch() {
    let fixture = process::fixture();
    runner(&fixture, DELIVER);
    for args in [
        vec!["experiment", "--candidate-runner", "fixture"],
        vec![
            "experiment",
            "candidate",
            "--candidate-runner",
            "fixture",
            "--suite",
            "process",
        ],
        vec![
            "experiment",
            "--candidate-runner",
            "fixture",
            "--suite",
            "process",
            "--release",
        ],
        vec![
            "experiment",
            "--candidate-runner",
            "fixture",
            "--suite",
            "code-review",
        ],
        vec![
            "experiment",
            "--candidate-runner",
            "fixture",
            "--suite",
            "spec-review",
        ],
        vec![
            "experiment",
            "candidate",
            "--baseline",
            "HEAD",
            "--baseline-runner",
            "fixture",
            "--suite",
            "process",
        ],
    ] {
        let run = fixture.scenario.run_with_env(&args, &NO_SANDBOX);
        assert_ne!(run.code, Some(0), "{args:?}: {}", run.stderr);
        assert!(
            run.stderr.contains("Custom runners") || run.stderr.contains("conflicts"),
            "{}",
            run.stderr
        );
    }
    for arm in ["baseline", "candidate"] {
        for field in ["process", "harness", "model", "effort"] {
            let runner_flag = format!("--{arm}-runner");
            let conflict = format!("--{arm}-{field}");
            let args = vec![
                "experiment",
                "--baseline-runner",
                "fixture",
                "--candidate-runner",
                "fixture",
                "--suite",
                "process",
                &conflict,
                "claude",
            ];
            assert!(args.contains(&runner_flag.as_str()));
            let run = fixture.scenario.run_with_env(&args, &NO_SANDBOX);
            assert_ne!(run.code, Some(0));
            assert!(run.stderr.contains("conflicts"), "{}", run.stderr);
        }
    }
    assert!(
        !fixture
            .scenario
            .path("home/.thirdshift/evals/experiments")
            .exists()
    );
}

#[test]
fn custom_runner_digest_preserves_resume_across_unrelated_commits_and_changes_with_inputs() {
    let fixture = process::fixture();
    runner(&fixture, DELIVER);
    let first = run(&fixture, &[]);
    assert_eq!(first.code, Some(0), "{}", first.stderr);
    let first: Value = serde_json::from_str(&first.stdout).unwrap();
    result_format::assert_documented("Experiment", &first);
    fixture
        .scenario
        .commit_locally("unrelated.txt", "unrelated", "Unrelated commit");
    let resumed = run(&fixture, &[]);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    let resumed: Value = serde_json::from_str(&resumed.stdout).unwrap();
    assert_eq!(first["id"], resumed["id"]);
    assert_eq!(
        first["baseline"]["runner_digest"],
        resumed["baseline"]["runner_digest"]
    );
    let path = fixture.scenario.path("home/.thirdshift/runners/runner.py");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(path, text + "\n# Changed runner input\n").unwrap();
    let changed = run(&fixture, &[]);
    assert_eq!(changed.code, Some(0), "{}", changed.stderr);
    let changed: Value = serde_json::from_str(&changed.stdout).unwrap();
    assert_ne!(changed["id"], first["id"]);
    assert_ne!(
        changed["baseline"]["runner_digest"],
        first["baseline"]["runner_digest"]
    );
}

#[test]
fn a_frozen_home_skill_is_verified_before_a_resumed_trial() {
    let fixture = process::fixture();
    runner(&fixture, DELIVER);
    let source = fixture.scenario.path("home/.thirdshift/runners");
    fs::create_dir_all(source.join("skill/scripts")).unwrap();
    fs::write(source.join("skill/scripts/check"), "frozen helper").unwrap();
    let manifest = source.join("fixture.toml");
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(
        manifest,
        text + "\n\"skill\" = { home = \".agents/skills/frozen\" }\n",
    )
    .unwrap();
    let first = run(&fixture, &[]);
    assert_eq!(first.code, Some(0), "{}", first.stderr);
    let first: Value = serde_json::from_str(&first.stdout).unwrap();
    result_format::assert_documented("Experiment", &first);
    let experiment = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/experiments/{}/experiment.json",
        first["id"].as_str().unwrap()
    ));
    let mut saved: Value = serde_json::from_slice(&fs::read(&experiment).unwrap()).unwrap();
    saved["state"] = json!("paused");
    fs::write(&experiment, serde_json::to_vec(&saved).unwrap()).unwrap();
    // Force dispatch of an already completed Trial through the resume path.
    let results = experiment.parent().unwrap().join("results");
    let result = fs::read_dir(results)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let trial = experiment
        .parent()
        .unwrap()
        .join("trials")
        .join(result.file_stem().unwrap());
    fs::remove_dir_all(trial).unwrap();
    fs::remove_file(result).unwrap();
    let frozen = PathBuf::from(
        saved["prepared"]["candidate"]["runner"]["directory"]
            .as_str()
            .unwrap(),
    );
    fs::write(
        frozen.join("home/.agents/skills/frozen/scripts/check"),
        "tampered",
    )
    .unwrap();
    let resumed = run(&fixture, &[]);
    assert_ne!(resumed.code, Some(0), "{}", resumed.stderr);
    assert!(
        resumed.stderr.contains("frozen Custom runner file changed"),
        "{}",
        resumed.stderr
    );
}

#[test]
fn custom_runner_pairs_use_committed_new_tests_on_base_and_other_head() {
    let fixture = process::fixture();
    pairs::pair_fixture(&fixture, pairs::CANDIDATE_TEST);
    let body = DELIVER.replace("subprocess.run(['git', 'add', '.']", r#"pathlib.Path('test_new_runner.py').write_text("import unittest\nfrom pathlib import Path\nclass Runner(unittest.TestCase):\n def test_feature(self):\n  self.assertEqual(Path('feature.txt').read_text(), 'implemented\\n')\n")
subprocess.run(['git', 'add', '.']"#);
    runner(&fixture, &body);
    let run = run(&fixture, &[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(view["pairs"].as_array().unwrap().len(), 1, "{view}");
    for arm in ["baseline", "candidate"] {
        let side = &view["pairs"][0][arm];
        assert_eq!(side["fails_on_base"], true, "{side}");
        assert_eq!(side["passes_on_other"], true, "{side}");
        assert_eq!(side["test_files"], json!(["test_new_runner.py"]), "{side}");
        assert!(
            side["diff"]
                .as_array()
                .unwrap()
                .iter()
                .all(|file| !file["path"].as_str().unwrap().starts_with(".scratch")),
            "{side}"
        );
    }
}

#[test]
fn two_custom_runners_execute_outside_a_thirdshift_checkout_without_builds_or_harnesses() {
    let fixture = process::fixture();
    runner(&fixture, DELIVER);
    fs::remove_dir_all(fixture.scenario.path("home/.thirdshift/evals/builds")).unwrap();
    let outside = fixture.scenario.path("outside");
    fs::create_dir_all(&outside).unwrap();
    for name in [
        "cargo", "claude", "codex", "agy", "grok", "muse", "opencode", "kimi",
    ] {
        let path = fixture.scenario.path(&format!("bin/{name}"));
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }
        fs::write(
            &path,
            "#!/bin/sh\necho 'unexpected build or Harness lookup' >&2\nexit 77\n",
        )
        .unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let run = fixture.scenario.run_from_with_env(
        &outside,
        &[
            "experiment",
            "--baseline-runner",
            "fixture",
            "--candidate-runner",
            "fixture",
            "--suite",
            "process",
            "--json",
            "--yes",
            "--draws",
            "100",
        ],
        &NO_SANDBOX,
    );
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    assert!(
        view["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|result| result["score"]["passed"] == true),
        "{view}"
    );
    assert!(
        !fixture
            .scenario
            .path("home/.thirdshift/evals/builds")
            .exists()
    );
}
