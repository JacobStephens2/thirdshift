//! Base-tests and cross-tests: each Arm's new tests on the base and on the
//! other Arm's final commit, kept per pair as evidence beside the Scores.
use super::*;

/// Both Arms implement the fix. Each commits its own new test file; the
/// Candidate's also asserts an extra file that only the Candidate writes.
fn pair_fixture(fixture: &Fixture, new_test: &str) {
    let task = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/process/tasks/fixture/task.toml");
    let text = fs::read_to_string(&task).unwrap();
    fs::write(
        &task,
        text + r#"
[process_grading.new_tests]
paths = ["test_new_*.py"]
command = ["python3", "-m", "unittest", "{path}"]
"#,
    )
    .unwrap();
    let script = fs::read_to_string(fixture.scenario.path("claude-script.sh")).unwrap();
    fixture.scenario.agent_does(&script.replace(
        "git add feature.txt",
        &format!(
            r#"case "$PATH" in
*/harnesses/baseline*)
    cat > test_new_baseline.py <<'PY'
import unittest
from pathlib import Path
class Baseline(unittest.TestCase):
    def test_feature(self):
        self.assertEqual(Path('feature.txt').read_text(), 'implemented\n')
PY
    git add test_new_baseline.py ;;
*)
    echo extra > extra.txt
    cat > test_new_candidate.py <<'PY'
{new_test}
PY
    git add extra.txt test_new_candidate.py ;;
esac
git add feature.txt"#
        ),
    ));
}

const CANDIDATE_TEST: &str = r#"import unittest
from pathlib import Path
class Candidate(unittest.TestCase):
    def test_feature(self):
        self.assertEqual(Path('feature.txt').read_text(), 'implemented\n')
    def test_extra(self):
        self.assertTrue(Path('extra.txt').exists())"#;

fn pair(view: &Value) -> &Value {
    let pairs = view["pairs"].as_array().unwrap();
    assert_eq!(pairs.len(), 1, "{view}");
    &pairs[0]
}

#[test]
fn each_arms_new_tests_run_on_the_base_and_the_other_arms_commit() {
    let fixture = super::process::fixture();
    pair_fixture(&fixture, CANDIDATE_TEST);
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    let pair = pair(&view);
    assert_eq!(pair["suite"], "process", "{pair}");
    assert_eq!(pair["task"], "fixture", "{pair}");
    assert_eq!(pair["trial"], 0, "{pair}");

    let baseline = &pair["baseline"];
    assert_eq!(baseline["test_files"], json!(["test_new_baseline.py"]));
    assert_eq!(baseline["fails_on_base"], true, "{baseline}");
    assert_eq!(baseline["passes_on_other"], true, "{baseline}");
    assert_eq!(baseline["disagreement"], false, "{baseline}");
    assert_eq!(
        baseline["base"][0]["command"],
        json!(["python3", "-m", "unittest", "test_new_baseline.py"]),
        "{baseline}"
    );

    // The Candidate's extra assertion fails on the Baseline's commit: a
    // disagreement to look into, which changes neither Arm's Score.
    let candidate = &pair["candidate"];
    assert_eq!(candidate["test_files"], json!(["test_new_candidate.py"]));
    assert_eq!(candidate["fails_on_base"], true, "{candidate}");
    assert_eq!(candidate["passes_on_other"], false, "{candidate}");
    assert_eq!(candidate["disagreement"], true, "{candidate}");
    assert!(
        candidate["cross"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("test_extra"),
        "{candidate}"
    );
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "delivered", "{result}");
        assert_eq!(result["score"]["passed"], true, "{result}");
        assert!(
            result["score"]["checks"]
                .as_array()
                .unwrap()
                .iter()
                .all(|check| check["passed"] == true),
            "{result}"
        );
    }
    assert!(
        run.stderr.contains("Pair evidence"),
        "progress names the evidence stage: {}",
        run.stderr
    );
    super::result_format::assert_documented("Experiment", &view);

    // Resuming the completed Experiment reads the same retained evidence.
    let shown = fixture.scenario.run_with_env(
        &["experiments", view["id"].as_str().unwrap(), "--json"],
        &NO_SANDBOX,
    );
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    assert_eq!(shown["pairs"], view["pairs"]);
    let text = fixture
        .scenario
        .run_with_env(&["experiments", view["id"].as_str().unwrap()], &NO_SANDBOX);
    assert!(
        text.stdout.contains(
            "Pair process/fixture #1 · Candidate tests fail on the Baseline's commit (disagreement)"
        ),
        "{}",
        text.stdout
    );
}

#[test]
fn tasks_without_new_tests_have_no_pair_evidence() {
    let fixture = super::process::fixture();
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(view["pairs"], json!([]), "{view}");
}

#[test]
fn an_arm_without_new_tests_has_nothing_to_run() {
    let fixture = super::process::fixture();
    pair_fixture(&fixture, CANDIDATE_TEST);
    let script = fs::read_to_string(fixture.scenario.path("claude-script.sh")).unwrap();
    fixture
        .scenario
        .agent_does(&script.replace("git add test_new_baseline.py", "rm test_new_baseline.py"));
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    let baseline = &pair(&view)["baseline"];
    assert_eq!(baseline["test_files"], json!([]), "{baseline}");
    assert_eq!(baseline["fails_on_base"], Value::Null, "{baseline}");
    assert_eq!(baseline["passes_on_other"], Value::Null, "{baseline}");
    assert_eq!(baseline["disagreement"], false, "{baseline}");
    assert_eq!(pair(&view)["candidate"]["fails_on_base"], true);
}

#[test]
fn new_tests_run_offline() {
    if !std::process::Command::new("bwrap")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        println!("SKIPPED (sandboxed pair evidence): bwrap is not on PATH");
        return;
    }
    let fixture = super::process::fixture();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    pair_fixture(
        &fixture,
        &format!(
            r#"import socket, unittest
class Candidate(unittest.TestCase):
    def test_offline(self):
        try:
            socket.create_connection(('127.0.0.1', {port}), timeout=1)
        except OSError:
            print('offline confirmed', flush=True)
        else:
            raise AssertionError('host network is reachable')"#
        ),
    );
    super::sandbox::sandbox_graders(&fixture);
    let run = fixture.scenario.run(&Fixture::experiment_args(&[]));
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    let candidate = &pair(&view)["candidate"];
    for run in ["base", "cross"] {
        let check = &candidate[run][0];
        assert_eq!(check["passed"], true, "{candidate}");
        assert!(
            check["reason"]
                .as_str()
                .unwrap()
                .contains("offline confirmed"),
            "{candidate}"
        );
    }
    assert_eq!(candidate["fails_on_base"], false, "{candidate}");
}

#[test]
fn new_tests_share_the_bar_timeout() {
    let fixture = super::process::fixture();
    pair_fixture(
        &fixture,
        r#"import time, unittest
class Candidate(unittest.TestCase):
    def test_slow(self):
        time.sleep(60)"#,
    );
    let run = fixture.run(&["--timeout", SHORT_TIMEOUT]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    let candidate = &pair(&view)["candidate"];
    for run in ["base", "cross"] {
        let check = &candidate[run][0];
        assert_eq!(check["passed"], false, "{candidate}");
        assert!(
            check["reason"].as_str().unwrap().contains("timed out"),
            "{candidate}"
        );
    }
    assert_eq!(candidate["passes_on_other"], false, "{candidate}");
    assert_eq!(candidate["disagreement"], true, "{candidate}");
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["score"]["passed"], true, "{result}");
    }
}
