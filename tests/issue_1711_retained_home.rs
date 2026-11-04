mod support;

use std::fs;
use support::Scenario;

// This stand-in exercises the exact environment handed to bwrap. It is not
// a filesystem-confinement test, and does not require working namespaces.
fn environment_forwarding_bwrap(scenario: &Scenario) {
    support::write_executable(
        &scenario.path("bin/bwrap"),
        r#"#!/bin/bash
set -eu
while [ "$1" != -- ]; do
    case "$1" in
        --setenv) export "$2=$3"; shift 3 ;;
        *) shift ;;
    esac
done
shift
exec "$@"
"#,
    );
}

fn retained_home_case(harness: &str, program: &str) {
    for read_machine in [false, true] {
        let scenario = Scenario::new();
        scenario.user_config_is("");
        environment_forwarding_bwrap(&scenario);
        let binary = scenario.path(&format!("bin/{harness}"));
        fs::remove_file(&binary).unwrap();
        support::write_executable(&binary, program);
        let report = scenario.path("report.txt");
        fs::write(&report, "Investigate the machine alert.").unwrap();
        let mut args = vec![
            "triage",
            "--recommend-only",
            "harness",
            harness,
            "--file",
            report.to_str().unwrap(),
        ];
        if read_machine {
            args.push("--read-machine");
        }
        let result = scenario.run(&args);
        assert_eq!(
            result.code,
            Some(0),
            "{harness}, read_machine={read_machine}: {}",
            result.stderr
        );
        let recommendation: serde_json::Value = serde_json::from_str(result.stdout.trim()).unwrap();
        assert_eq!(
            recommendation["investigation"],
            "native completion recovered"
        );
    }
}

#[test]
fn read_machine_muse_recovers_the_native_final_reply() {
    retained_home_case(
        "muse",
        r#"#!/bin/bash
set -eu
if [ "${1:-}" = --version ]; then echo 'Muse Code 1.0'; exit; fi
dir="${XDG_DATA_HOME:-$HOME/.local/share}/muse/sessions/2026/11/03/private-triage"
mkdir -p "$dir"
cat > "$dir/session.jsonl" <<'JSON'
{"stream":{"kind":"session","id":"private-triage"},"payload_type":"runtime.session","payload":{"kind":"run","event":{"kind":"assistant_message_committed","text":"Triage recommendation: {\"category\":\"bug\",\"state\":\"needs-triage\",\"issues\":[],\"investigation\":\"native completion recovered\"}"}}}
JSON
# Muse's terminal fallback concatenates replies when the individual output
# events are absent. Its retained final reply supplies the correct text.
cat <<'JSON'
{"stream":{"kind":"session","id":"private-triage"},"payload_type":"run.terminal.completed","payload":{"text":"earlier replyTriage recommendation: {\"category\":\"bug\",\"state\":\"needs-triage\",\"issues\":[],\"investigation\":\"native completion recovered\"}"}}
JSON
"#,
    );
}

#[test]
fn read_machine_opencode_recovers_the_native_export() {
    retained_home_case(
        "opencode",
        r#"#!/bin/bash
set -eu
if [ "${1:-}" = --version ]; then echo 'OpenCode 2.0'; exit; fi
dir="${XDG_DATA_HOME:-$HOME/.local/share}/opencode"
if [ "${1:-}" = session ] && [ "${2:-}" = export ]; then
    cat "$dir/private-triage-export.json"
    exit
fi
cat >/dev/null
mkdir -p "$dir"
cat > "$dir/private-triage-export.json" <<'JSON'
{"info":{"outcome":"succeeded"},"messages":[{"type":"assistant","content":[{"type":"text","text":"Triage recommendation: {\"category\":\"bug\",\"state\":\"needs-triage\",\"issues\":[],\"investigation\":\"native completion recovered\"}"}]}]}
JSON
# A completed standalone session whose text stream event was dropped.
echo '{"type":"step_finish","sessionID":"private-triage","part":{"reason":"stop"}}'
"#,
    );
}
