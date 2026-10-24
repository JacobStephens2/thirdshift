use super::*;

fn fixture_with_passthrough_trials() -> Fixture {
    let fixture = Fixture::new();
    for commit in [&fixture.baseline, &fixture.candidate] {
        let binary = fixture.scenario.path(&format!(
            "home/.thirdshift/evals/builds/{commit}/thirdshift"
        ));
        let real = binary.with_extension("real");
        fs::rename(&binary, &real).unwrap();
        fs::write(
            &binary,
            format!(
                "#!/bin/sh\nexport THIRDSHIFT_TRIAL_NO_SANDBOX=1\nexec {:?} \"$@\"\n",
                real
            ),
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fixture.scenario.agent_does(&format!(
        "{}\nprintf 'caught\\n' > \"$reports/spec.md\"\n",
        review_reports_prelude()
    ));
    fixture
}

fn set_check(fixture: &Fixture, command: &[&str]) {
    let task = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/spec-review/tasks/fixture/task.toml");
    let manifest = fs::read_to_string(&task).unwrap();
    let checks = format!(
        "checks = [{}]\n[inputs]",
        serde_json::to_string(command).unwrap()
    );
    fs::write(task, manifest.replace("[inputs]", &checks)).unwrap();
}

#[test]
fn code_graders_cannot_read_arm_labels_from_mountinfo() {
    if !std::process::Command::new("bwrap")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        println!("SKIPPED (Grader mount metadata): bwrap is not on PATH");
        return;
    }
    let fixture = fixture_with_passthrough_trials();
    set_check(
        &fixture,
        &[
            "sh",
            "-c",
            "! grep -Eq -- '--(baseline|candidate)--' /proc/self/mountinfo",
        ],
    );
    let _run = fixture.scenario.run(&["experiment", "candidate", "--json"]);
    let experiment = fixture.experiment();
    let results = experiment["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    for result in results {
        let check = result["score"]["checks"]
            .as_array()
            .unwrap()
            .last()
            .unwrap();
        assert_eq!(check["passed"], true, "{check}");
    }
}

#[test]
fn code_grader_exec_failure_voids_trial() {
    let fixture = fixture_with_passthrough_trials();
    set_check(&fixture, &["sh", "-c", "true"]);
    let bwrap = fixture.scenario.path("bin/bwrap");
    fs::write(&bwrap, "#!/thirdshift-missing-interpreter\n").unwrap();
    fs::set_permissions(&bwrap, fs::Permissions::from_mode(0o755)).unwrap();
    let _run = fixture.scenario.run(&["experiment", "candidate", "--json"]);
    let experiment = fixture.experiment();
    let results = experiment["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    for result in results {
        assert_eq!(result["outcome"], "void", "{result}");
        let attempts = result["attempts"].as_array().unwrap();
        assert_eq!(attempts.len(), 3, "{result}");
        for attempt in attempts {
            assert_eq!(attempt["void_cause"], "sandbox_unavailable", "{attempt}");
        }
    }
}

#[test]
fn code_graders_see_reports_and_repository_but_no_trial_logs_or_arm_paths() {
    if !std::process::Command::new("bwrap")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        println!("SKIPPED (sandboxed code Grader): bwrap is not on PATH");
        return;
    }
    let fixture = fixture_with_passthrough_trials();
    let check = format!(
        "set -eu; test ! -e '{host}'; test -z \"$THIRDSHIFT_TRIAL_LOG\"; \
         test \"$PWD\" = /sandbox/repository; \
         test \"$(git remote get-url origin)\" = /sandbox/repository/.git/eval-origin.git; \
         if grep -Eq -- '--(baseline|candidate)--' /sandbox/packet/repository/.git/config; then exit 42; fi; \
         test -f .thirdshift-issue.md; grep -q caught \"$THIRDSHIFT_TRIAL_SPEC_REPORT\"",
        host = fixture.scenario.launch_dir().display()
    );
    set_check(&fixture, &["sh", "-c", &check]);
    let _run = fixture.scenario.run(&["experiment", "candidate", "--json"]);
    // The ordinary fake Model Grader cannot access its host recording path;
    // inspect the already-published Code Scores at the Experiment seam.
    let experiment = fixture.experiment();
    let results = experiment["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    for result in results {
        let check = result["score"]["checks"]
            .as_array()
            .unwrap()
            .last()
            .unwrap();
        assert_eq!(check["passed"], true, "{check}");
    }
}
