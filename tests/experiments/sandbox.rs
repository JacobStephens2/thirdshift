use super::*;

#[test]
fn a_check_diagnostic_cannot_claim_sandbox_failure() {
    if !std::process::Command::new("bwrap")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        println!("SKIPPED (Grader startup status): bwrap is not on PATH");
        return;
    }
    let fixture = fixture_with_passthrough_trials();
    set_check(
        &fixture,
        &[
            "sh",
            "-c",
            "printf 'bwrap: fixture check failed\\n' >&2; exit 1",
        ],
    );
    let _run = fixture.scenario.run(&["experiment", "candidate", "--json"]);
    let experiment = fixture.experiment();
    let results = experiment["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    for result in results {
        assert_eq!(result["outcome"], "succeeded", "{result}");
        assert_eq!(result["attempts"].as_array().unwrap().len(), 1, "{result}");
        assert_eq!(result["score"]["passed"], false, "{result}");
    }
}

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
    super::code_graders::set_checks(fixture, &[command]);
}

#[test]
fn code_grader_git_rewrite_stays_inside_its_snapshot() {
    if !std::process::Command::new("bwrap")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        println!("SKIPPED (Grader git confinement): bwrap is not on PATH");
        return;
    }
    for symlinked in ["directory", "config"] {
        let fixture = fixture_with_passthrough_trials();
        let outside = fixture.scenario.path("outside.git");
        fs::create_dir_all(outside.join("eval-origin.git")).unwrap();
        let config = outside.join("config");
        let original = "[remote \"origin\"]\n\turl = /experiments/private-arm\n";
        fs::write(&config, original).unwrap();
        let change = if symlinked == "directory" {
            format!("rm -rf .git; ln -s {outside:?} .git")
        } else {
            format!("rm .git/config; ln -s {config:?} .git/config")
        };
        fixture
            .scenario
            .agent_does(&format!("{}\n{change}\nexit 1\n", review_reports_prelude(),));
        set_check(&fixture, &["true"]);
        let _run = fixture.scenario.run(&["experiment", "candidate", "--json"]);
        let experiment = fixture.experiment();
        let results = experiment["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        for result in results {
            assert_eq!(result["outcome"], "failed", "{result}");
            assert_eq!(result["score"]["checks"][2]["passed"], true, "{result}");
        }
        assert_eq!(
            fs::read_to_string(&config).unwrap(),
            original,
            "symlinked {symlinked} redirected the Grader rewrite outside its snapshot",
        );
    }
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
         test \"$(cat /proc/self/oom_score_adj)\" = 1000; \
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

#[test]
fn sandboxed_code_graders_keep_their_memory_caps_and_startup_evidence() {
    if !scope_available()
        || !std::process::Command::new("bwrap")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
    {
        println!("SKIPPED (sandboxed Grader memory cap): no systemd user manager or bwrap");
        return;
    }
    let (Ok(bus), Ok(runtime)) = (
        std::env::var("DBUS_SESSION_BUS_ADDRESS"),
        std::env::var("XDG_RUNTIME_DIR"),
    ) else {
        println!("SKIPPED (sandboxed Grader memory cap): no user bus in test env");
        return;
    };
    for over_cap in [false, true] {
        let fixture = fixture_with_passthrough_trials();
        if over_cap {
            set_check(
                &fixture,
                &[
                    "python3",
                    "-c",
                    "a=[bytearray(1000000) for _ in range(2000)]",
                ],
            );
        } else {
            set_check(
                &fixture,
                &[
                    "sh",
                    "-c",
                    "test \"$(cat /proc/self/oom_score_adj)\" = 1000 && \
                     test -z \"${DBUS_SESSION_BUS_ADDRESS+x}\" && \
                     test -z \"${XDG_RUNTIME_DIR+x}\" && \
                     grep -q caught \"$THIRDSHIFT_TRIAL_SPEC_REPORT\"",
                ],
            );
        }
        let run = fixture.scenario.run_with_env(
            &[
                "experiment",
                "candidate",
                "--json",
                "--min-discriminating-tasks",
                "1",
            ],
            &[
                ("THIRDSHIFT_TRIAL_MEMORY_HIGH", "300M"),
                ("THIRDSHIFT_TRIAL_MEMORY_MAX", "300M"),
                ("THIRDSHIFT_TRIAL_MEMORY_SWAP_MAX", "0"),
                ("DBUS_SESSION_BUS_ADDRESS", bus.as_str()),
                ("XDG_RUNTIME_DIR", runtime.as_str()),
            ],
        );
        if over_cap {
            assert_eq!(run.code, Some(1), "{}", run.stderr);
            assert!(
                run.stderr.contains("code Grader hit its memory cap"),
                "{}",
                run.stderr
            );
        } else {
            // The fixture's Model Grader cannot record through its sandbox;
            // Code Scores were already saved, proving bwrap's startup status
            // survived the real systemd scope and its OOM-score wrapper.
            let experiment = fixture.experiment();
            let results = experiment["results"].as_array().unwrap();
            assert_eq!(results.len(), 2, "{}", run.stderr);
            for result in results {
                assert_eq!(result["outcome"], "succeeded", "{result}");
                assert_eq!(result["score"]["checks"][2]["passed"], true, "{result}");
            }
        }
    }
}
