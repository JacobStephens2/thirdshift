use super::*;

#[test]
fn trials_and_graders_keep_running_after_the_install_path_is_replaced() {
    survives_install_change(false, false);
}

#[test]
fn trials_and_graders_keep_running_after_the_install_path_is_removed() {
    survives_install_change(true, false);
}

#[test]
fn a_scope_prepared_before_replacement_executes_the_original_wrapper() {
    survives_install_change(false, true);
}

fn survives_install_change(remove: bool, prepared: bool) {
    let fixture = Fixture::with_session("spec-review", 3);
    let scenario = &fixture.scenario;
    let wait = scenario.waits_to_be_replaced();
    if prepared {
        // Hold the first launch at the external systemd seam, after the
        // Experiment has prepared its wrapper path and before it is exec'd.
        support::write_executable(
            &scenario.path("bin/systemd-run"),
            &format!(
                "#!/bin/sh\n{wait}\nwhile [ \"$#\" -gt 0 ] && [ \"$1\" != -- ]; do shift; done\nshift\nexec \"$@\"\n"
            ),
        );
    }
    scenario.agent_does(&format!("{}{wait}", review_reports_prelude()));
    let replacement_ran = scenario.path("replacement-ran");
    let result = scenario.run_copy_replaced_midway_with_env(
        &Fixture::experiment_args(&[]),
        &NO_SANDBOX,
        |installed| {
            if remove {
                fs::remove_file(installed).unwrap();
            } else {
                let replacement = installed.with_extension("new");
                support::write_executable(
                    &replacement,
                    &format!("#!/bin/sh\ntouch {}\nexit 1\n", replacement_ran.display()),
                );
                fs::rename(replacement, installed).unwrap();
            }
        },
    );

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(!replacement_ran.exists(), "the replacement wrapper ran");
    let experiment: Value = serde_json::from_str(&result.stdout).unwrap();
    let results = experiment["results"].as_array().unwrap();
    assert_eq!(results.len(), 6, "later Trials must still dispatch");
    for result in results {
        assert_eq!(result["outcome"], "succeeded", "{result}");
        assert_eq!(result["score"]["passed"], true, "{result}");
    }
    assert_eq!(fixture.trial_calls().len(), 6);
    assert!(!fixture.grader_calls().is_empty(), "Graders must run too");
    for call in fixture.grader_calls() {
        let directory = PathBuf::from(call["cwd"].as_str().unwrap());
        assert_eq!(
            fs::read_to_string(directory.join("scores.json.adj"))
                .unwrap()
                .trim(),
            "1000",
            "the original OOM wrapper must run"
        );
    }
}
