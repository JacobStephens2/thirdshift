//! Recovery advice is printed by the CLI; real Git retains its refusal checks.
mod support;

use support::Scenario;

fn recovery_command(stderr: &str, operation: &str) -> String {
    stderr
        .lines()
        .find(|line| line.trim_start().starts_with("git -C ") && line.contains(operation))
        .unwrap_or_else(|| panic!("missing {operation}: {stderr}"))
        .trim()
        .to_string()
}

fn run_advice(scenario: &Scenario, command: &str) -> std::process::Output {
    std::process::Command::new("sh")
        .args(["-c", command])
        .current_dir(scenario.path(""))
        .env("HOME", scenario.path("home"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap()
}

#[test]
fn local_branch_advice_preserves_work_and_offers_commands_that_git_can_refuse() {
    for on_origin in [false, true] {
        let scenario = Scenario::new();
        if on_origin {
            scenario.origin_has_branch("issue-7", "main", &["Remote work"]);
        }
        scenario.launch_git(&["checkout", "-q", "-b", "issue-7"]);
        scenario.commit_locally("local.txt", "keep me\n", "Local work");
        let head = scenario.launch_git(&["rev-parse", "HEAD"]);
        scenario.launch_checks_out("main");
        let result = scenario.run(&[&scenario.issue_url(7)]);
        scenario.assert_rejected_before_any_work(&result, "the local branch issue-7");
        assert_eq!(scenario.launch_git(&["rev-parse", "issue-7"]), head);
        assert_eq!(
            scenario.origin_log("issue-7").map(|log| log.len()),
            on_origin.then_some(2)
        );
        assert!(result.stderr.contains("Alternatives"), "{}", result.stderr);
        let delete = recovery_command(&result.stderr, " branch -d ");
        assert!(
            !run_advice(&scenario, &delete).status.success(),
            "unmerged work must refuse deletion"
        );
        assert_eq!(scenario.launch_git(&["rev-parse", "issue-7"]), head);
        let push = recovery_command(&result.stderr, " push ");
        assert_eq!(run_advice(&scenario, &push).status.success(), !on_origin);
        let rename = recovery_command(&result.stderr, " branch -m ");
        assert!(run_advice(&scenario, &rename).status.success());
        assert_eq!(
            scenario.launch_git(&["rev-parse", "issue-7-preserved"]),
            head
        );
    }
}

#[test]
fn occupied_checkout_advice_names_the_actual_path_and_quotes_removal_safely() {
    for relative in [
        "work/widgets-issue-7",
        "work/manual ' $(touch PWNED); & checkout",
    ] {
        let scenario = Scenario::new();
        scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
        scenario.launch_git(&["fetch", "-q", "origin"]);
        let occupied = scenario.path(relative);
        scenario.launch_git(&[
            "worktree",
            "add",
            "-b",
            "issue-7",
            occupied.to_str().unwrap(),
            "origin/issue-7",
        ]);
        std::fs::write(occupied.join("local.txt"), "keep this checkout\n").unwrap();
        let before = scenario.launch_git(&["worktree", "list", "--porcelain", "-z"]);
        let head = scenario.launch_git(&["rev-parse", "issue-7"]);

        let result = scenario.run(&[&scenario.issue_url(7)]);

        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert_eq!(result.stdout, "");
        assert!(scenario.claude_calls().is_empty());
        assert_eq!(
            scenario.launch_git(&["worktree", "list", "--porcelain", "-z"]),
            before
        );
        assert_eq!(scenario.launch_git(&["rev-parse", "issue-7"]), head);
        assert!(result.stderr.contains("Continuation"), "{}", result.stderr);
        assert!(
            result.stderr.contains(occupied.to_str().unwrap()),
            "{}",
            result.stderr
        );
        let status = recovery_command(&result.stderr, " status ");
        let inspected = run_advice(&scenario, &status);
        assert!(inspected.status.success(), "{inspected:?}");
        assert!(String::from_utf8_lossy(&inspected.stdout).contains("local.txt"));
        let remove = recovery_command(&result.stderr, " worktree remove ");
        assert!(!run_advice(&scenario, &remove).status.success());
        assert_eq!(
            std::fs::read_to_string(occupied.join("local.txt")).unwrap(),
            "keep this checkout\n"
        );
        assert!(!scenario.path("PWNED").exists());
        assert_eq!(scenario.origin_log("issue-7").unwrap().len(), 2);
        std::fs::remove_file(occupied.join("local.txt")).unwrap();
        assert!(run_advice(&scenario, &remove).status.success());
        assert!(!occupied.exists());
        assert_eq!(scenario.launch_git(&["rev-parse", "issue-7"]), head);
    }
}

#[test]
fn recovery_commands_quote_the_launch_directory_and_identify_its_checked_out_branch() {
    let scenario = Scenario::new();
    scenario.launch_git(&["checkout", "-q", "-b", "issue-7"]);
    scenario.commit_locally("local.txt", "preserved work\n", "Local work");
    std::fs::write(
        scenario.launch_dir().join("uncommitted.txt"),
        "unfinished\n",
    )
    .unwrap();
    let head = scenario.launch_git(&["rev-parse", "HEAD"]);
    let launch = scenario.path("work/launch ' $(touch PWNED); & repo");
    std::fs::rename(scenario.launch_dir(), &launch).unwrap();

    let result = scenario.run_from(&launch, &[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(scenario.claude_calls().is_empty());
    assert!(
        result.stderr.contains("This is the Launch directory"),
        "{}",
        result.stderr
    );
    assert!(
        result.stderr.contains("Before renaming or deleting"),
        "{}",
        result.stderr
    );
    assert!(
        !result.stderr.contains(" worktree remove "),
        "{}",
        result.stderr
    );
    assert_eq!(
        std::fs::read_to_string(launch.join("uncommitted.txt")).unwrap(),
        "unfinished\n"
    );
    assert!(scenario.origin_log("issue-7").is_none());
    for operation in [" status ", " push ", " branch -m "] {
        let command = recovery_command(&result.stderr, operation);
        let output = run_advice(&scenario, &command);
        assert!(output.status.success(), "{command}: {output:?}");
    }
    assert!(!scenario.path("PWNED").exists());
    assert!(!launch.join("PWNED").exists());
    assert_eq!(scenario.origin_git(&["rev-parse", "issue-7"]), head);
    assert_eq!(
        std::fs::read_to_string(launch.join("uncommitted.txt")).unwrap(),
        "unfinished\n"
    );
}

#[test]
fn an_issue_branch_checked_out_in_launch_can_continue_after_switching_to_base() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
    scenario.github_has_pr("issue-7", "main", "OPEN");
    scenario.launch_checks_out("issue-7");
    let head = scenario.launch_git(&["rev-parse", "HEAD"]);
    let result = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("This is the Launch directory"),
        "{}",
        result.stderr
    );
    assert!(!result.stderr.contains(" worktree remove "));
    assert!(scenario.claude_calls().is_empty());
    assert_eq!(scenario.launch_git(&["rev-parse", "HEAD"]), head);

    scenario.launch_checks_out("main");
    scenario.agent_does(
        "echo continued > continued.txt\ngit add continued.txt\ngit commit -q -m Continued",
    );
    let continued = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(continued.code, Some(0), "{}", continued.stderr);
    assert_eq!(scenario.origin_log("issue-7").unwrap()[0], "Continued");
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn branch_switch_advice_is_only_offered_when_it_frees_acquisition() {
    for relative in ["work/widgets-issue-7", "work/preserved-checkout"] {
        let scenario = Scenario::new();
        scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
        scenario.github_has_pr("issue-7", "main", "OPEN");
        scenario.launch_git(&["fetch", "-q", "origin"]);
        let occupied = scenario.path(relative);
        scenario.launch_git(&[
            "worktree",
            "add",
            "-b",
            "issue-7",
            occupied.to_str().unwrap(),
            "origin/issue-7",
        ]);
        let refused = scenario.run(&[&scenario.issue_url(7)]);
        assert_eq!(refused.code, Some(1), "{}", refused.stderr);
        let can_keep_checkout = relative == "work/preserved-checkout";
        assert_eq!(refused.stderr.contains("Alternatively, leave this checkout on another branch and retry, keeping the preserved checkout"), can_keep_checkout, "{}", refused.stderr);

        scenario.launch_git(&[
            "-C",
            occupied.to_str().unwrap(),
            "switch",
            "-c",
            "preserved-checkout",
        ]);
        scenario.agent_does(
            "echo continued > continued.txt\ngit add continued.txt\ngit commit -q -m Continued",
        );
        let retried = scenario.run(&[&scenario.issue_url(7)]);
        assert_eq!(
            retried.code,
            Some(if can_keep_checkout { 0 } else { 1 }),
            "{}",
            retried.stderr
        );
        if can_keep_checkout {
            assert_eq!(scenario.origin_log("issue-7").unwrap()[0], "Continued");
        } else {
            assert!(
                !retried
                    .stderr
                    .contains("Alternatively, leave this checkout on another branch and retry"),
                "{}",
                retried.stderr
            );
            assert!(scenario.claude_calls().is_empty());
        }
        assert!(occupied.join("README.md").exists());
    }
}

#[test]
fn continuation_from_a_linked_launch_does_not_offer_to_remove_the_main_checkout() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
    scenario.github_has_pr("issue-7", "main", "OPEN");
    let linked = scenario.path("work/linked-launch");
    scenario.launch_git(&[
        "worktree",
        "add",
        "--detach",
        linked.to_str().unwrap(),
        "main",
    ]);
    scenario.launch_checks_out("issue-7");
    std::fs::write(
        scenario.launch_dir().join("untracked.txt"),
        "keep this work\n",
    )
    .unwrap();
    let before = scenario.launch_git(&["worktree", "list", "--porcelain", "-z"]);

    let result = scenario.run_from(&linked, &[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("This is the main checkout"),
        "{}",
        result.stderr
    );
    assert!(
        !result.stderr.contains(" worktree remove "),
        "{}",
        result.stderr
    );
    assert!(result.stderr.contains("Continuation"), "{}", result.stderr);
    assert_eq!(
        scenario.launch_git(&["worktree", "list", "--porcelain", "-z"]),
        before
    );
    assert_eq!(
        std::fs::read_to_string(scenario.launch_dir().join("untracked.txt")).unwrap(),
        "keep this work\n"
    );
    assert!(scenario.claude_calls().is_empty());
}

use std::path::Path;

fn git_at(scenario: &Scenario, directory: &Path, arguments: &[&str]) {
    let output = std::process::Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .env("HOME", scenario.path("home"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {arguments:?}: {output:?}");
}

#[test]
fn main_checkout_at_target_does_not_offer_a_branch_switch_as_sufficient_recovery() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
    scenario.github_has_pr("issue-7", "main", "OPEN");
    let linked = scenario.path("work/linked-launch");
    scenario.launch_git(&[
        "worktree",
        "add",
        "--detach",
        linked.to_str().unwrap(),
        "main",
    ]);
    scenario.launch_checks_out("issue-7");
    let main = scenario.path("work/widgets-issue-7");
    std::fs::rename(scenario.launch_dir(), &main).unwrap();
    git_at(&scenario, &main, &["worktree", "repair"]);

    let refused = scenario.run_from(&linked, &[&scenario.issue_url(7)]);
    assert_eq!(refused.code, Some(1), "{}", refused.stderr);
    assert!(
        refused.stderr.contains("This is the main checkout"),
        "{}",
        refused.stderr
    );
    git_at(&scenario, &main, &["switch", "main"]);
    let retried = scenario.run_from(&linked, &[&scenario.issue_url(7)]);
    assert_eq!(retried.code, Some(1), "{}", retried.stderr);
    assert!(scenario.claude_calls().is_empty());
    assert!(main.join("README.md").exists());
    assert!(
        !refused
            .stderr
            .contains("switch it to another branch, then rerun"),
        "Switching leaves the target occupied and the retry still fails:\nFIRST:\n{}\nRETRY:\n{}",
        refused.stderr,
        retried.stderr
    );
    assert!(
        refused.stderr.contains("different parent directory"),
        "{}",
        refused.stderr
    );
    continue_from_another_parent(&scenario, &main);
}

#[test]
fn launch_at_target_does_not_offer_a_branch_switch_as_sufficient_recovery() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
    scenario.github_has_pr("issue-7", "main", "OPEN");
    scenario.launch_checks_out("issue-7");
    let launch = scenario.path("work/widgets-issue-7");
    std::fs::rename(scenario.launch_dir(), &launch).unwrap();

    let refused = scenario.run_from(&launch, &[&scenario.issue_url(7)]);
    assert_eq!(refused.code, Some(1), "{}", refused.stderr);
    assert!(
        refused.stderr.contains("This is the Launch directory"),
        "{}",
        refused.stderr
    );
    git_at(&scenario, &launch, &["switch", "main"]);
    let retried = scenario.run_from(&launch, &[&scenario.issue_url(7)]);
    assert_eq!(retried.code, Some(1), "{}", retried.stderr);
    assert!(scenario.claude_calls().is_empty());
    assert!(launch.join("README.md").exists());
    assert!(
        !refused
            .stderr
            .contains("switch it to the Base branch, then rerun"),
        "Switching leaves the target occupied and the retry still fails:\nFIRST:\n{}\nRETRY:\n{}",
        refused.stderr,
        retried.stderr
    );
    assert!(
        refused.stderr.contains("different parent directory"),
        "{}",
        refused.stderr
    );
    continue_from_another_parent(&scenario, &launch);
}

fn continue_from_another_parent(scenario: &Scenario, repository: &Path) {
    let parent = scenario.path("other-parent");
    std::fs::create_dir(&parent).unwrap();
    let launch = parent.join("linked-launch");
    git_at(
        scenario,
        repository,
        &[
            "worktree",
            "add",
            "--detach",
            launch.to_str().unwrap(),
            "main",
        ],
    );
    scenario.agent_does(
        "echo continued > continued.txt\ngit add continued.txt\ngit commit -q -m Continued",
    );
    let result = scenario.run_from(&launch, &[&scenario.issue_url(7)]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.origin_log("issue-7").unwrap()[0], "Continued");
    assert!(repository.join("README.md").exists());
}
