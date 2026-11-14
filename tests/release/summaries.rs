//! Real summary sandboxes with self-contained fake Harness executables.
use super::*;

fn harness(release: &Release, name: &str, reply: &str) {
    let binary = release.bin().join(name);
    let original = if name == "claude" {
        fs::remove_file(&binary).unwrap();
        release.bin().join("claude-session")
    } else {
        let original = release.bin().join(format!("{name}-session"));
        fs::rename(&binary, &original).unwrap();
        original
    };
    fs::create_dir_all(release.root().join("home/.thirdshift/trials/grok")).unwrap();
    fs::write(
        release
            .root()
            .join("home/.thirdshift/trials/grok/auth.json"),
        "test login",
    )
    .unwrap();
    fs::write(
        release.root().join("home/.thirdshift/credentials.toml"),
        "[opencode]\napi_key = 'test-provider-key'\n",
    )
    .unwrap();
    write_executable(
        &binary,
        &format!(
            r#"#!/usr/bin/python3
import json, os, pathlib, sys
name, reply = {name}, {reply}
if os.environ.get('HOME') == {home}:
    os.execv({original}, [name, *sys.argv[1:]])
args = sys.argv[1:]
assert not pathlib.Path({home}).exists(), 'host home mounted'
assert not os.environ.get('HOST_RELEASE_SECRET'), 'ambient secret leaked'
if '--prompt-file' in args:
    prompt = pathlib.Path(args[args.index('--prompt-file') + 1]).read_text()
elif '--agent-file' in args:
    agent = pathlib.Path(args[args.index('--agent-file') + 1]).read_text()
    assert 'tools: []' in agent
    prompt = agent.split('---', 2)[2].strip()
else:
    prompt = sys.stdin.read()
    if name == 'agy': prompt = json.loads(prompt)['message']['content']
assert 'RELEASE-LARGE-INPUT-END' in prompt, 'release input truncated'
assert 'Old news.' not in prompt, 'already shipped PR included'
assert '0.2.0' in prompt and 'version_diff' in prompt
for forbidden in ['--dangerously-bypass-approvals-and-sandbox','--dangerously-skip-permissions','--always-approve','--yolo','--auto']:
    assert forbidden not in args, (name, forbidden, args)
required = {{'claude':['--tools','--strict-mcp-config'],
    'codex':['--sandbox','read-only','shell_tool','unified_exec','apps','plugins','browser_use','computer_use','hooks'],
    'agy':['--sandbox','--disable-slash-commands'],
    'grok':['--tools','todo_write','--disable-web-search'],
    'muse':['--disable-shell','--disable-write','--disable-web-tools'],
    'opencode':[], 'kimi':['--agent-file']}}
assert all(flag in args for flag in required[name]), args
if name == 'claude': assert args[args.index('--tools')+1] == ''
if name == 'opencode': assert json.loads(os.environ['OPENCODE_CONFIG_CONTENT']) == {{'permission':{{'*':'deny'}}}}
if name == 'codex':
    assert args[args.index('-m')+1] == 'gpt-6-luna', args
    assert 'model_reasoning_effort="high"' in args, args
def emit(value): print(json.dumps(value), flush=True)
emit({{'release_test_input':{{'argv':args,'prompt':prompt,'cwd':os.getcwd()}}}})
if name in ['claude','grok']:
    emit({{'type':'system','subtype':'init','session_id':'sandbox-summary'}})
    emit({{'type':'result','subtype':'success','is_error':False,'result':reply}})
elif name == 'codex':
    emit({{'type':'thread.started','thread_id':'sandbox-summary'}})
    emit({{'type':'item.completed','item':{{'type':'agent_message','text':reply}}}})
    emit({{'type':'turn.completed'}})
elif name == 'agy': emit({{'event':'result','result':{{'status':'SUCCESS','response':reply}}}})
elif name == 'muse':
    emit({{'id':'final','payload_type':'run.output.delta','payload':{{'text':reply}}}})
    emit({{'id':'ended','payload_type':'run.terminal.completed','payload':{{'text':reply}}}})
elif name == 'opencode':
    emit({{'type':'step_start','sessionID':'sandbox-summary','part':{{}}}})
    emit({{'type':'text','sessionID':'sandbox-summary','part':{{'text':reply}}}})
    emit({{'type':'step_finish','sessionID':'sandbox-summary','part':{{'reason':'stop'}}}})
elif name == 'kimi':
    emit({{'role':'assistant','content':reply}})
    emit({{'role':'meta','type':'session.resume_hint','session_id':'sandbox-summary'}})
"#,
            name = json!(name),
            reply = json!(reply),
            home = json!(release.root().join("home").display().to_string()),
            original = json!(original.display().to_string())
        ),
    );
}

#[test]
fn every_configured_harness_gets_complete_release_evidence_in_a_toolless_sandbox() {
    for name in ["claude", "codex", "agy", "grok", "muse", "opencode", "kimi"] {
        let release = Release::new();
        // PR input exceeds Linux's per-argument limit, so it must travel on
        // stdin or through a sandboxed file, including for Kimi's agent file.
        let mut state = release.gh_state();
        state["prs"][1]["body"] = json!(format!("{}RELEASE-LARGE-INPUT-END", "x".repeat(150_000)));
        fs::write(release.root().join("gh-state.json"), state.to_string()).unwrap();
        let mut config = format!("[harness]\ndefault = '{name}'\n");
        if name == "codex" {
            config.push_str("[harness.codex]\nmodel = 'GPT-6-Luna'\neffort = 'High'\n");
        } else if name == "opencode" {
            config.push_str("[harness.opencode]\nmodel = 'anthropic/claude'\n");
        } else if name == "kimi" {
            config.push_str("[harness.kimi]\nmodel = 'kimi-code/kimi-for-coding'\n");
        }
        fs::create_dir_all(release.root().join("home/.thirdshift")).unwrap();
        fs::write(release.root().join("home/.thirdshift/config.toml"), config).unwrap();
        harness(&release, name, AGENT_SUMMARY);
        release.ci_reports(GREEN);
        let output = release.run_with(&["0.2.0"], "", &[("HOST_RELEASE_SECRET", "outside")]);
        assert!(output.status.success(), "{name}: {}", stderr(&output));
        assert_eq!(
            summary_and_rest(&release.pr_body("release-0.2.0")).0.trim(),
            AGENT_SUMMARY,
            "{name}: {}",
            stderr(&output)
        );
        assert_eq!(release.agent_calls().len(), 1, "{name}");
    }
}
