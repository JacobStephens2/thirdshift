use super::*;

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
    let fixture = Fixture::new();
    // Only the Arm subprocesses use the pass-through launcher. The current
    // Experiment must sandbox its Graders independently of those builds.
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
    let task = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/spec-review/tasks/fixture/task.toml");
    let manifest = fs::read_to_string(&task).unwrap();
    let check = format!(
        "set -eu; test ! -e '{host}'; test -z \"$THIRDSHIFT_TRIAL_LOG\"; \
         test \"$PWD\" = /sandbox/repository; \
         test \"$(git remote get-url origin)\" = /sandbox/repository/.git/eval-origin.git; \
         test -f .thirdshift-issue.md; grep -q caught \"$THIRDSHIFT_TRIAL_SPEC_REPORT\"",
        host = fixture.scenario.launch_dir().display()
    );
    let checks = format!(
        "checks = [[\"sh\", \"-c\", {}]]\n[inputs]",
        serde_json::to_string(&check).unwrap()
    );
    fs::write(task, manifest.replace("[inputs]", &checks)).unwrap();
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
