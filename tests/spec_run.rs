//! Spec runs: `thirdshift <Issue URL>` on a Spec, an issue with sub-issues,
//! works through its Tickets in dependency order, each a Merge run into the
//! Spec branch, then opens the Spec PR from the Spec branch into the Base
//! branch as a draft, has the Spec review review the Spec branch, and marks
//! the Spec PR ready for review. The Spec PR then goes through the Repair
//! loop, and with `merge`, the Self-merge.

mod support;

use support::resend::ResendStandIn;
use support::{Scenario, WAIT_BOUND, leaves_running};

const SPEC: u32 = 20;
const SPEC_TITLE: &str = "Widgets, all of them";

/// The agent for Ticket `ticket` commits `file` and opens its PR, closing
/// the Ticket, into the Spec branch.
fn agent_lands(ticket: u32, file: &str) -> String {
    format!(
        r#"
echo "{ticket}" > {file}
git add {file}
git commit -q -m "Ticket {ticket}"
gh pr create --base issue-{SPEC} --head issue-{ticket} --title "Ticket {ticket}" --body "Closes #{ticket}"
"#
    )
}

/// A Spec #20 with a linear chain of Tickets: #21, then #22, blocked by #21,
/// and #23, already closed. Each open Ticket's agent lands its own file, and
/// #22's first checks that #21's work is already on its branch.
fn linear_spec() -> Scenario {
    let scenario = Scenario::new();
    scenario.issue_titled(SPEC, SPEC_TITLE);
    scenario.spec_has_tickets(SPEC, &[(21, &[]), (22, &[21]), (23, &[])]);
    scenario.issue_is(23, "CLOSED");
    scenario.agent_does_for(21, &agent_lands(21, "first.txt"));
    scenario.agent_does_for(
        22,
        &format!("test -f first.txt\n{}", agent_lands(22, "second.txt")),
    );
    scenario
}

fn spec_url(scenario: &Scenario) -> String {
    scenario.issue_url(SPEC)
}

/// The issue number each agent session was for, the first Issue URL its
/// prompt names, in order.
fn sessions_by_issue(scenario: &Scenario) -> Vec<String> {
    scenario
        .claude_calls()
        .iter()
        .map(|call| {
            let prompt = call["prompt"].as_str().unwrap();
            let (_, after) = prompt.split_once("/issues/").unwrap();
            after.chars().take_while(char::is_ascii_digit).collect()
        })
        .collect()
}

/// The Spec review's session: the only one for the Spec.
fn spec_review_call(scenario: &Scenario) -> serde_json::Value {
    let calls = scenario.claude_calls();
    let sessions = sessions_by_issue(scenario);
    let reviews: Vec<_> = sessions
        .iter()
        .enumerate()
        .filter(|(_, issue)| *issue == &SPEC.to_string())
        .map(|(i, _)| calls[i].clone())
        .collect();
    assert_eq!(reviews.len(), 1, "sessions: {sessions:?}");
    reviews[0].clone()
}

/// The pull requests from `head`, oldest first.
fn prs_from(scenario: &Scenario, head: &str) -> Vec<serde_json::Value> {
    scenario.gh_state()["prs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|pr| pr["head"] == head)
        .cloned()
        .collect()
}

/// The newest pull request from `head`, if there is one.
fn pr_from(scenario: &Scenario, head: &str) -> Option<serde_json::Value> {
    prs_from(scenario, head).pop()
}

/// The Spec PR.
fn spec_pr(scenario: &Scenario) -> serde_json::Value {
    pr_from(scenario, "issue-20").expect("no Spec PR")
}

/// The Tickets checklist in `body`, markers included.
fn checklist_in(body: &str) -> &str {
    let start = body
        .find("<!-- thirdshift:tickets -->")
        .unwrap_or_else(|| panic!("no Tickets checklist in: {body}"));
    let end =
        body.find("<!-- /thirdshift:tickets -->").unwrap() + "<!-- /thirdshift:tickets -->".len();
    &body[start..end]
}

fn assert_contains(text: &str, part: &str) {
    assert!(text.contains(part), "expected {part:?} in: {text}");
}

/// A complete Security review with no old findings writes its empty private
/// report as well as the public introduced-finding outcome.
fn security_review_script(introduced: &[&str]) -> String {
    format!(
        r#"
report=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/^Private report file: `\(.*\)`\.$/\1/p')
test -n "$report"
printf '%s\n' '[]' > "$report"
printf '%s\n' 'Security review: {outcome}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
        outcome = serde_json::json!({
            "unaddressed_count": introduced.len(),
            "findings": introduced,
            "pre_existing_count": 0
        })
    )
}

#[test]
fn a_linear_spec_lands_each_ticket_in_order_then_opens_a_ready_spec_pr() {
    let scenario = linear_spec();

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let spec_pr_url = "https://github.com/acme/widgets/pull/2";
    assert_eq!(result.stdout, format!("{spec_pr_url}\n"));
    assert_eq!(
        result.stderr.lines().last(),
        Some(format!("thirdshift: PR {spec_pr_url} is ready for review").as_str()),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(sessions_by_issue(&scenario), ["21", "22", "20"]);

    let gh = scenario.gh_state();
    for ticket in [21, 22] {
        let pr = pr_from(&scenario, &format!("issue-{ticket}")).unwrap();
        assert_eq!(pr["base"], "issue-20");
        assert_eq!(pr["state"], "MERGED");
        assert_eq!(gh["issues"][ticket.to_string()], "CLOSED");
    }
    let spec = spec_pr(&scenario);
    assert_eq!(spec["url"], spec_pr_url);
    assert_eq!(spec["head"], "issue-20");
    assert_eq!(spec["base"], "main");
    assert_eq!(spec["state"], "OPEN");
    assert_eq!(spec["isDraft"], false);
    assert_eq!(spec["title"], SPEC_TITLE);
    assert_contains(spec["body"].as_str().unwrap(), "Closes #20");
    assert_eq!(gh["issues"]["20"], "OPEN");

    assert_eq!(
        scenario.origin_file("issue-20", "second.txt").as_deref(),
        Some("22\n")
    );
    assert_eq!(scenario.origin_file("main", "first.txt"), None);
    scenario.assert_cleaned_up("issue-20");
}

#[test]
fn security_review_runs_once_after_spec_review_over_the_whole_spec_branch() {
    let scenario = linear_spec();
    let base_commit = scenario.launch_git(&["rev-parse", "main"]);
    scenario.agent_does_for_in_session(
        SPEC,
        1,
        r#"
echo reviewed > spec-reviewed.txt
git add spec-reviewed.txt
git commit -q -m 'Complete Spec review'
"#,
    );
    scenario.agent_does_for_in_session(
        SPEC,
        2,
        &format!(
            r#"
test -f first.txt
test -f second.txt
test -f spec-reviewed.txt
gh pr view issue-20 --json isDraft | grep -q '"isDraft": true'
! git cat-file -e origin/issue-20:spec-reviewed.txt
merge_base=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/^Merge base commit: `\(.*\)`\.$/\1/p')
test "$merge_base" = "$(git rev-parse refs/remotes/origin/main)"
git diff "$merge_base"...HEAD --name-only > changed.txt
grep -qx first.txt changed.txt
grep -qx second.txt changed.txt
grep -qx spec-reviewed.txt changed.txt
rm changed.txt
echo fixed > security-fixed.txt
git add security-fixed.txt
git commit -q -m 'Fix reproduced Security finding'
{review}
"#,
            review = security_review_script(&[])
        ),
    );

    let result = scenario.run(&[&spec_url(&scenario), "security-review", "merge"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(sessions_by_issue(&scenario), ["21", "22", "20", "20"]);
    let calls = scenario.claude_calls();
    assert_contains(
        calls[2]["prompt"].as_str().unwrap(),
        "/thirdshift-code-review main",
    );
    let prompt = calls[3]["prompt"].as_str().unwrap();
    assert_contains(prompt, "guidance mode");
    assert_contains(prompt, "Do not delegate auditors");
    assert_contains(prompt, "https://github.com/acme/widgets/issues/20");
    assert_contains(
        prompt,
        &format!(
            "branch issue-20 against Base branch main with `git diff {}...HEAD`",
            base_commit.trim()
        ),
    );
    assert_eq!(calls[3]["branch"], "issue-20");
    assert_eq!(spec_pr(&scenario)["state"], "MERGED");
    for file in [
        "first.txt",
        "second.txt",
        "spec-reviewed.txt",
        "security-fixed.txt",
    ] {
        assert!(
            scenario.origin_file("main", file).is_some(),
            "missing {file}"
        );
    }
    let reviewed = result.stderr.find("spec-review: session ended").unwrap();
    let security = result
        .stderr
        .find("security-review: session started")
        .unwrap();
    assert!(reviewed < security, "{}", result.stderr);
}

#[test]
fn spec_review_leaves_the_push_to_delivery_after_the_optional_security_review() {
    let scenario = linear_spec();
    let result = scenario.run(&[&spec_url(&scenario)]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let call = spec_review_call(&scenario);
    assert_contains(
        call["prompt"].as_str().unwrap(),
        "Do not push: thirdshift pushes branch issue-20 after the optional Security review.",
    );
}

#[test]
fn security_review_on_a_spec_obeys_config_and_command_overrides() {
    for (config, word, enabled) in [
        ("", None, false),
        ("[security]\nreview = true\n", None, true),
        (
            "[security]\nreview = true\n",
            Some("no-security-review"),
            false,
        ),
        (
            "[security]\nreview = false\n",
            Some("security-review"),
            true,
        ),
    ] {
        let scenario = linear_spec();
        scenario.user_config_is(config);
        scenario.agent_does_for_in_session(SPEC, 2, &security_review_script(&[]));
        let url = spec_url(&scenario);
        let mut args = vec![url.as_str(), "merge"];
        args.extend(word);

        let result = scenario.run(&args);

        assert_eq!(result.code, Some(0), "{config} {word:?}: {}", result.stderr);
        assert_eq!(
            sessions_by_issue(&scenario),
            if enabled {
                vec!["21", "22", "20", "20"]
            } else {
                vec!["21", "22", "20"]
            }
        );
        assert_eq!(spec_pr(&scenario)["state"], "MERGED");
    }
}

#[test]
fn security_findings_refusals_and_incomplete_reviews_hold_only_the_spec_self_merge() {
    let introduced = security_review_script(&["Cross-tenant read"]);
    for (script, cause) in [
        (
            introduced.as_str(),
            "Security review left 1 unaddressed introduced finding(s): Cross-tenant read",
        ),
        (
            r#"printf '%s\n' 'API Error: [cyber] Private refusal evidence' > "$FAKE_CLAUDE_FINAL_MESSAGE""#,
            "Security review refused: Claude Code's [cyber] safeguard refusal",
        ),
        (
            r#"printf '%s\n' 'Review incomplete' > "$FAKE_CLAUDE_FINAL_MESSAGE""#,
            "Security review incomplete",
        ),
        ("exit 1\n", "Security review incomplete"),
    ] {
        for merge in [true, false] {
            let scenario = linear_spec();
            scenario.agent_does_for_in_session(SPEC, 2, script);

            let result = scenario.run(&[
                &spec_url(&scenario),
                "security-review",
                if merge { "merge" } else { "no-merge" },
            ]);

            assert_eq!(
                result.code,
                Some(i32::from(merge)),
                "{cause}: {}",
                result.stderr
            );
            assert_contains(&result.stderr, cause);
            assert_eq!(sessions_by_issue(&scenario), ["21", "22", "20", "20"]);
            let pr = spec_pr(&scenario);
            assert_eq!(pr["state"], "OPEN");
            assert_eq!(pr["isDraft"], false);
            let body = pr["body"].as_str().unwrap();
            assert_contains(body, "### Security");
            // Introduced finding titles and sanitized refusal/incomplete causes.
            assert_contains(
                body,
                if cause.contains("Cross-tenant read") {
                    "Cross-tenant read"
                } else {
                    cause
                },
            );
            assert!(!body.contains("Private refusal evidence"));
            assert_contains(body, "#21 landed");
            assert_contains(body, "#22 landed");
            assert_eq!(scenario.gh_state()["issues"]["20"], "OPEN");
            assert_eq!(scenario.origin_file("main", "first.txt"), None);
            for ticket in ["issue-21", "issue-22"] {
                assert_eq!(pr_from(&scenario, ticket).unwrap()["state"], "MERGED");
            }
        }
    }
}

#[test]
fn a_spec_security_hold_still_repairs_ci_without_repeating_the_review() {
    let scenario = linear_spec();
    scenario.agent_does_for_in_session(SPEC, 1, &checks_on_head(RED));
    scenario.agent_does_for_in_session(SPEC, 2, &security_review_script(&["Cross-tenant read"]));
    scenario.agent_does_for_in_session(
        SPEC,
        3,
        &format!("{}{}", commits_fix(1), checks_on_head(GREEN)),
    );

    let result = scenario.run(&[&spec_url(&scenario), "security-review", "merge"]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert_contains(&result.stderr, "Cross-tenant read");
    let prompts = spec_prompts(&scenario);
    assert_eq!(prompts.len(), 3);
    assert_contains(&prompts[0], "/thirdshift-code-review main");
    assert_contains(&prompts[1], "guidance mode");
    assert_contains(&prompts[2], "CI");
    assert_eq!(
        scenario.origin_file("issue-20", "fix-1.txt").as_deref(),
        Some("fix\n")
    );
    assert_eq!(spec_pr(&scenario)["state"], "OPEN");
    assert_eq!(spec_pr(&scenario)["isDraft"], false);
}

#[test]
fn a_spec_run_claims_the_spec_and_its_tickets_runs_change_no_label() {
    let scenario = linear_spec();
    scenario.issue_labelled(SPEC, &["ready-for-agent", "enhancement"]);
    for ticket in [21, 22] {
        scenario.issue_labelled(ticket, &["ready-for-agent"]);
    }

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(scenario.issue_labels(SPEC), ["enhancement", "in-progress"]);
    for ticket in [21, 22] {
        assert_eq!(scenario.issue_labels(ticket), ["ready-for-agent"]);
    }
    assert_contains(
        &result.stderr,
        "thirdshift: labelling #20 in-progress, in place of ready-for-agent\n",
    );
    assert_eq!(result.stderr.matches(" in-progress").count(), 1);
}

#[test]
fn each_ticket_branches_off_the_spec_branch_and_its_prompts_name_it_as_the_base() {
    let scenario = linear_spec();

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    for ticket in [21, 22] {
        assert_contains(
            &result.stderr,
            &format!("thirdshift: #{ticket}: creating worktree"),
        );
        assert_contains(
            &result.stderr,
            &format!("on issue-{ticket} from origin/issue-20\n"),
        );
    }
    for call in &scenario.claude_calls()[..2] {
        assert_contains(
            call["prompt"].as_str().unwrap(),
            "The base branch is issue-20.",
        );
    }
}

/// The file in the scenario root that says the stand-in for a newer
/// thirdshift was run.
#[cfg(target_os = "linux")]
const REPLACEMENT_RAN: &str = "replacement-ran";

/// Rename a stand-in for a newer thirdshift, a script that only records that
/// it was run, over `installed`, as the release installer, `thirdshift
/// update` and `cargo install` each put a new binary in place.
#[cfg(target_os = "linux")]
fn rename_a_replacement_over(scenario: &Scenario, installed: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    let script = format!(
        "#!/bin/sh\ntouch {}\nexit 1\n",
        scenario.path(REPLACEMENT_RAN).display()
    );
    let new = installed.with_extension("new");
    std::fs::write(&new, script).unwrap();
    std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::rename(&new, installed).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn a_tickets_run_started_after_thirdshift_was_replaced_runs_the_spec_runs_own_binary() {
    let scenario = linear_spec();
    scenario.agent_does_for(
        21,
        &format!(
            "{}{}",
            agent_lands(21, "first.txt"),
            scenario.waits_to_be_replaced()
        ),
    );
    // The session's script is run by the agent, which #22's Run started.
    let name_of_run = scenario.path("name-of-run-22");
    scenario.agent_does_for(
        22,
        &format!(
            "cat /proc/$(ps -o ppid= -p $PPID | tr -d ' ')/comm > {}\n{}",
            name_of_run.display(),
            agent_lands(22, "second.txt")
        ),
    );

    let result = scenario.run_copy_replaced_midway(&[&spec_url(&scenario)], |installed| {
        rename_a_replacement_over(&scenario, installed)
    });

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_contains(&result.stderr, "thirdshift: #22 landed\n");
    assert_eq!(pr_from(&scenario, "issue-22").unwrap()["state"], "MERGED");
    assert_eq!(
        scenario.origin_file("issue-20", "second.txt").as_deref(),
        Some("22\n")
    );
    assert!(
        !scenario.path(REPLACEMENT_RAN).exists(),
        "#22 ran the replacement"
    );
    // As `pgrep thirdshift` and `top` see #22's Run.
    assert_eq!(
        std::fs::read_to_string(name_of_run).unwrap(),
        "thirdshift\n"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_tickets_run_started_after_thirdshift_was_replaced_can_still_start_its_base_fix() {
    let scenario = spec_of(&[(21, &[]), (22, &[21])]);
    scenario.agent_does_for(
        21,
        &format!(
            "{}{}",
            agent_lands(21, "21.txt"),
            scenario.waits_to_be_replaced()
        ),
    );
    scenario.agent_does_for(22, &agent_lands_with_an_inherited_failure(22, "22.txt"));
    base_fix_lands_on_the_spec_branch(&scenario, 23);

    let result = scenario
        .run_copy_replaced_midway(&[&spec_url(&scenario), "base-fix"], |installed| {
            rename_a_replacement_over(&scenario, installed)
        });

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_contains(
        &result.stderr,
        "#22: starting Base fix #23 into issue-20: https://github.com/acme/widgets/issues/23\n",
    );
    for head in ["issue-22", "issue-23"] {
        assert_eq!(pr_from(&scenario, head).unwrap()["state"], "MERGED");
    }
    assert!(scenario.origin_file("issue-20", "ci-fix.txt").is_some());
    assert!(
        !scenario.path(REPLACEMENT_RAN).exists(),
        "a Run ran the replacement"
    );
}

#[test]
fn ticket_lines_are_relayed_with_the_ticket_number_and_the_spec_run_says_what_it_does() {
    let scenario = linear_spec();

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    for line in [
        "thirdshift: starting #21\n",
        "thirdshift: #21: implement: session started\n",
        "thirdshift: #21: PR https://github.com/acme/widgets/pull/1 is merged\n",
        "thirdshift: #21 landed\n",
        "thirdshift: starting #22\n",
        "thirdshift: #22 landed\n",
    ] {
        assert_contains(&result.stderr, line);
    }
    let first_landed = result.stderr.find("#21 landed").unwrap();
    let second_started = result.stderr.find("starting #22").unwrap();
    assert!(first_landed < second_started, "stderr: {}", result.stderr);
    // A Ticket's own stderr never reaches the terminal unprefixed.
    assert!(
        !result.stderr.contains("thirdshift: thirdshift:"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn each_relayed_ticket_line_has_exactly_one_time() {
    let scenario = linear_spec();

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let relayed: Vec<&str> = result
        .stamped_stderr
        .lines()
        .filter(|line| line.contains(" #21: "))
        .collect();
    assert!(!relayed.is_empty(), "stderr: {}", result.stamped_stderr);
    for line in relayed {
        let unprefixed = line.strip_prefix("thirdshift: ").unwrap();
        let (_, message) =
            support::split_stamp(unprefixed).unwrap_or_else(|| panic!("unstamped: {line}"));
        let message = message.strip_prefix("#21: ").unwrap();
        assert!(
            support::split_stamp(message).is_none(),
            "stamped twice: {line}"
        );
    }
}

#[test]
fn ticket_runs_send_no_run_notification() {
    let scenario = linear_spec();
    scenario.user_config_is("[email]\nalways = true\nto = \"me@example.com\"\n");
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);

    let result = scenario.run_with_env(
        &[&spec_url(&scenario)],
        &[
            ("THIRDSHIFT_RESEND_URL", resend.url()),
            ("RESEND_API_KEY", "re_test_123"),
        ],
    );

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_contains(
        requests[0].body["subject"].as_str().unwrap(),
        "acme/widgets#20",
    );
}

#[test]
fn ticket_runs_leave_the_launch_directory_to_the_spec_run() {
    let scenario = linear_spec();
    scenario.origin_has_commit("main", "README.md", "widgets, updated\n", "Upstream work");
    scenario.user_config_is("[launch]\npull = true\n");

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(
        result
            .stderr
            .matches("in the Launch directory from")
            .count(),
        1,
        "stderr: {}",
        result.stderr
    );
    assert_contains(
        &result.stderr,
        "thirdshift: updating main in the Launch directory from origin/main\n",
    );
}

/// Assert the Spec PR was merged into main with a merge commit, the Spec
/// branch deleted on origin and the Spec closed.
fn assert_spec_pr_merged(scenario: &Scenario, result: &support::RunResult) {
    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let spec_pr_url = "https://github.com/acme/widgets/pull/2";
    assert_eq!(result.stdout, format!("{spec_pr_url}\n"));
    assert_contains(
        &result.stderr,
        &format!("thirdshift: PR {spec_pr_url} is merged\n"),
    );
    assert_eq!(spec_pr(scenario)["state"], "MERGED");
    let gh = scenario.gh_state();
    assert_eq!(gh["issues"]["20"], "CLOSED");
    let parents = scenario.origin_git(&["log", "-1", "--format=%P", "refs/heads/main"]);
    assert_eq!(
        parents.split_whitespace().count(),
        2,
        "main's tip is not a merge commit"
    );
    assert_eq!(
        scenario.origin_file("main", "second.txt").as_deref(),
        Some("22\n")
    );
    assert_eq!(scenario.origin_log("issue-20"), None);
    scenario.assert_cleaned_up("issue-20");
}

#[test]
fn merge_on_a_spec_self_merges_the_spec_pr_into_the_base_branch() {
    let scenario = linear_spec();

    let result = scenario.run(&["merge", &spec_url(&scenario)]);

    assert_spec_pr_merged(&scenario, &result);
}

#[test]
fn no_merge_on_a_spec_leaves_the_spec_pr_ready_while_its_tickets_still_merge() {
    let scenario = linear_spec();
    scenario.user_config_is("[merge]\nalways = true\n");

    let result = scenario.run(&["--no-merge", &spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    for ticket in ["issue-21", "issue-22"] {
        assert_eq!(pr_from(&scenario, ticket).unwrap()["state"], "MERGED");
    }
    let spec = spec_pr(&scenario);
    let gh = scenario.gh_state();
    assert_eq!(spec["state"], "OPEN");
    assert_eq!(spec["isDraft"], false);
    assert_eq!(gh["issues"]["20"], "OPEN");
    assert_eq!(scenario.origin_file("main", "first.txt"), None);
}

/// Bash that has someone else push `file` with `content` to main.
fn base_moves_on(file: &str, content: &str) -> String {
    format!(
        r#"
other="$(mktemp -d)"
git clone -q https://github.com/acme/widgets.git "$other"
echo "{content}" > "$other/{file}"
git -C "$other" add -A
git -C "$other" commit -q -m "Base moves on"
git -C "$other" push -q origin main
rm -rf "$other"
"#
    )
}

/// Bash that sets the check runs on the worktree's HEAD to `checks`.
fn checks_on_head(checks: &str) -> String {
    format!("gh fake checks \"$(git rev-parse HEAD)\" '{checks}'\n")
}

/// Bash that commits a fix, `fix-<n>.txt`, as a CI-fix Repair would.
fn commits_fix(n: usize) -> String {
    format!("echo fix > fix-{n}.txt\ngit add fix-{n}.txt\ngit commit -q -m \"Fix CI {n}\"\n")
}

const GREEN: &str = r#"[{"name": "test", "conclusion": "success"}]"#;
const RED: &str = r#"[{"name": "test", "conclusion": "failure"}]"#;

/// The prompts of the sessions for the Spec, in order.
fn spec_prompts(scenario: &Scenario) -> Vec<String> {
    let calls = scenario.claude_calls();
    sessions_by_issue(scenario)
        .iter()
        .enumerate()
        .filter(|(_, issue)| *issue == &SPEC.to_string())
        .map(|(i, _)| calls[i]["prompt"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_base_branch_that_moved_into_a_conflict_gets_a_conflict_repair_and_the_spec_pr_ends_ready() {
    let scenario = linear_spec();
    scenario.agent_does_for_in_session(SPEC, 1, &base_moves_on("first.txt", "base first"));
    scenario.agent_does_for_in_session(
        SPEC,
        2,
        "printf '21\\nbase first\\n' > first.txt\ngit add first.txt\ngit commit -q --no-edit\n",
    );

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let prompts = spec_prompts(&scenario);
    assert_eq!(prompts.len(), 2, "{prompts:?}");
    assert_contains(&prompts[1], "/thirdshift-resolving-merge-conflicts");
    assert_contains(&prompts[1], "A merge of origin/main into issue-20");
    assert_contains(&prompts[1], &spec_url(&scenario));
    let spec = spec_pr(&scenario);
    assert_eq!(spec["state"], "OPEN");
    assert_eq!(spec["isDraft"], false);
    assert_eq!(
        scenario.origin_file("issue-20", "first.txt").as_deref(),
        Some("21\nbase first\n")
    );
}

#[test]
fn red_ci_on_the_spec_prs_head_gets_a_ci_fix_repair() {
    let scenario = linear_spec();
    scenario.agent_does_for_in_session(SPEC, 1, &checks_on_head(RED));
    scenario.agent_does_for_in_session(
        SPEC,
        2,
        &format!("{}{}", commits_fix(1), checks_on_head(GREEN)),
    );

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let prompts = spec_prompts(&scenario);
    assert_eq!(prompts.len(), 2, "{prompts:?}");
    assert!(
        prompts[1].starts_with("CI failed on pull request https://github.com/acme/widgets/pull/2"),
        "prompt: {}",
        prompts[1]
    );
    assert_contains(&prompts[1], &spec_url(&scenario));
    assert_eq!(spec_pr(&scenario)["isDraft"], false);
    assert_eq!(
        scenario.origin_file("issue-20", "fix-1.txt").as_deref(),
        Some("fix\n")
    );
}

#[test]
fn a_ci_fix_repair_on_the_spec_pr_that_makes_no_commit_gets_a_check_re_run() {
    let scenario = linear_spec();
    scenario.agent_does_for_in_session(
        SPEC,
        1,
        &checks_on_head(
            r#"[{"name": "test", "conclusion": "failure",
                 "url": "https://github.com/acme/widgets/actions/runs/900/job/1",
                 "rerun": {"conclusion": "success"}}]"#,
        ),
    );
    scenario.agent_does_for_in_session(SPEC, 2, "true\n");

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(
        spec_prompts(&scenario).len(),
        2,
        "the Spec review and 1 Repair"
    );
    assert_eq!(
        scenario.gh_calls_of("run", "rerun"),
        vec![vec![
            "run",
            "rerun",
            "900",
            "--failed",
            "--repo",
            "acme/widgets"
        ]]
    );
    assert_contains(&result.stderr, "re-running the failed checks on ");
    assert_eq!(spec_pr(&scenario)["isDraft"], false);
}

/// Bash that sets the check runs on `origin/<branch>`'s tip, as the worktree
/// last fetched it, to `checks`.
fn checks_on_origin(branch: &str, checks: &str) -> String {
    format!("gh fake checks \"$(git rev-parse origin/{branch})\" '{checks}'\n")
}

/// The cause of a Failed run whose red check `test` is an Inherited failure
/// from `base`, as its tip on origin is now.
fn inherited_failure(scenario: &Scenario, base: &str) -> String {
    let base_commit = scenario.origin_git(&["rev-parse", base]);
    format!(
        "CI red on test, which also fails on {base} at {}; fix {base} first",
        &base_commit[..7]
    )
}

#[test]
fn a_spec_pr_whose_red_check_also_fails_on_the_base_branch_commit_gets_no_repair() {
    let scenario = linear_spec();
    scenario.agent_does_for(
        SPEC,
        &format!("{}{}", checks_on_head(RED), checks_on_origin("main", RED)),
    );

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_contains(&result.stderr, &inherited_failure(&scenario, "main"));
    assert_eq!(spec_prompts(&scenario).len(), 1, "only the Spec review");
    let spec = spec_pr(&scenario);
    assert_eq!(spec["state"], "OPEN");
    assert_eq!(spec["isDraft"], true);
}

#[test]
fn with_base_fix_the_spec_prs_inherited_failure_gets_a_base_fix_into_the_base_branch() {
    let scenario = linear_spec();
    scenario.agent_does_for(
        SPEC,
        &format!("{}{}", checks_on_head(RED), checks_on_origin("main", RED)),
    );
    // The Base fix issue is the next after the Tickets.
    scenario.agent_does_for(
        24,
        &format!(
            r#"
echo "fixed" > ci-fix.txt
git add ci-fix.txt
git commit -q -m "Fix CI on main"
gh pr create --base main --head issue-24 --title "Fix CI on main" --body "Closes #24"
{}"#,
            checks_on_head(GREEN)
        ),
    );

    let result = scenario.run(&[&spec_url(&scenario), "base-fix"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_contains(
        &result.stderr,
        "thirdshift: starting Base fix #24 into main: https://github.com/acme/widgets/issues/24\n",
    );
    let gh = scenario.gh_state();
    assert_eq!(gh["titles"]["24"], "CI red on main: test");
    let fix = pr_from(&scenario, "issue-24").expect("the Base fix's PR");
    assert_eq!(fix["base"], "main");
    assert_eq!(fix["state"], "MERGED");
    let spec = spec_pr(&scenario);
    assert_eq!(spec["state"], "OPEN");
    assert_eq!(spec["isDraft"], false);
    assert!(scenario.origin_file("issue-20", "ci-fix.txt").is_some());
}

/// The agent for Ticket `ticket` lands `file` with `test` red on its head
/// and on the Spec branch: an Inherited failure.
fn agent_lands_with_an_inherited_failure(ticket: u32, file: &str) -> String {
    format!(
        "{}{}{}",
        agent_lands(ticket, file),
        checks_on_head(RED),
        checks_on_origin("issue-20", RED)
    )
}

/// A Spec #20 whose one Ticket, #21, lands with an Inherited failure.
fn spec_whose_ticket_inherits_a_failure() -> Scenario {
    let scenario = spec_of(&[(21, &[])]);
    scenario.agent_does_for(21, &agent_lands_with_an_inherited_failure(21, "first.txt"));
    scenario
}

/// The agent, on the Base fix issue `fix`, the next after the Tickets, commits
/// a fix and opens its PR into the Spec branch, with `test` green on its head.
fn base_fix_lands_on_the_spec_branch(scenario: &Scenario, fix: u32) {
    scenario.agent_does_for(
        fix,
        &format!(
            r#"
echo "fixed" > ci-fix.txt
git add ci-fix.txt
git commit -q -m "Fix CI on the Spec branch"
gh pr create --base issue-20 --head issue-{fix} --title "Fix CI" --body "Closes #{fix}"
{}"#,
            checks_on_head(GREEN)
        ),
    );
}

#[test]
fn with_base_fix_a_tickets_inherited_failure_gets_a_base_fix_into_the_spec_branch() {
    let scenario = spec_whose_ticket_inherits_a_failure();
    base_fix_lands_on_the_spec_branch(&scenario, 22);

    let result = scenario.run(&[&spec_url(&scenario), "base-fix"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_contains(
        &result.stderr,
        "#21: starting Base fix #22 into issue-20: https://github.com/acme/widgets/issues/22\n",
    );
    assert_eq!(
        scenario.gh_state()["titles"]["22"],
        "CI red on issue-20: test"
    );
    let fix = pr_from(&scenario, "issue-22").expect("the Base fix's PR");
    assert_eq!(fix["base"], "issue-20");
    assert_eq!(fix["state"], "MERGED");
    assert_eq!(pr_from(&scenario, "issue-21").unwrap()["state"], "MERGED");
    for file in ["first.txt", "ci-fix.txt"] {
        assert!(scenario.origin_file("issue-20", file).is_some(), "{file}");
    }
    assert_eq!(spec_pr(&scenario)["isDraft"], false);
}

#[test]
fn a_tickets_run_offers_the_spec_runs_command_with_a_base_fix_and_the_spec_run_repeats_none_of_it()
{
    let scenario = spec_whose_ticket_inherits_a_failure();
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
    let spec_url = spec_url(&scenario);

    let result = run_emailing(
        &scenario,
        &resend,
        &["--email", "me@example.com", "parallel", "1", &spec_url],
    );

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    let cause = inherited_failure(&scenario, "issue-20");
    // The Ticket's Run says it on its own stderr, between its cause and its
    // session log.
    assert_contains(
        &result.stderr,
        &format!(
            "thirdshift: #21: {cause}\n\
             thirdshift: #21: Base check: test\n\
             thirdshift: #21: Retry with: thirdshift {spec_url} --email me@example.com parallel 1 base-fix\n\
             thirdshift: #21: Or set: base.fix = true in ~/.thirdshift/config.toml, \
             to allow a Base fix for every Run on this machine\n\
             thirdshift: #21: session log: "
        ),
    );
    // What the Spec run reads back is the cause and the session log, as ever.
    let failed = format!("#21 failed: {cause} (session log: ");
    assert_contains(&result.stderr, &format!("thirdshift: {failed}"));
    let (_, text) = the_one_notification(&resend);
    assert_contains(&text, &failed);
    for label in ["Base check:", "Retry with:", "Or set:"] {
        assert!(!text.contains(label), "{label} in: {text}");
        assert_eq!(result.stderr.matches(label).count(), 1, "{label}");
    }
}

#[test]
fn with_no_base_fix_on_a_spec_a_tickets_run_links_the_checks_and_offers_no_base_fix() {
    let scenario = spec_whose_ticket_inherits_a_failure();

    let result = scenario.run(&[&spec_url(&scenario), "no-base-fix"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_contains(&result.stderr, "thirdshift: #21: Base check: test\n");
    assert!(
        !result.stderr.contains("Retry with:"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn a_ticket_that_landed_after_a_base_fix_says_so_in_the_notification_and_the_checklist() {
    let scenario = spec_whose_ticket_inherits_a_failure();
    base_fix_lands_on_the_spec_branch(&scenario, 22);
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);

    let result = run_emailing(
        &scenario,
        &resend,
        &[
            "--email",
            "me@example.com",
            "base-fix",
            &spec_url(&scenario),
        ],
    );

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let landed = "#21 landed with https://github.com/acme/widgets/pull/1, \
                  after Base fix https://github.com/acme/widgets/issues/22 merged\n";
    let (_, text) = the_one_notification(&resend);
    assert_contains(&text, landed);
    // The Spec PR's own Repair loop took no Base fix.
    assert!(!text.contains("Base fix:"), "{text}");
    let body = spec_pr(&scenario)["body"].as_str().unwrap().to_string();
    assert_contains(&body, &format!("- [x] {landed}"));
}

/// A Spec #20 whose Tickets #21 and #22, neither blocked, each land a file
/// and meet `test` red on their head and on the Spec branch.
fn two_tickets_with_the_same_inherited_failure() -> Scenario {
    let scenario = spec_of(&[(21, &[]), (22, &[])]);
    for ticket in [21, 22] {
        scenario.agent_does_for(
            ticket,
            &format!(
                "{}{}{}",
                agent_lands(ticket, &format!("{ticket}.txt")),
                checks_on_head(RED),
                checks_on_origin("issue-20", RED)
            ),
        );
    }
    scenario
}

/// Bash that waits until both Tickets have looked for an open Base fix
/// issue, so neither meets a Spec branch already fixed.
const BOTH_TICKETS_HAVE_LOOKED: &str = r#"
for _ in $(seq 200); do
    looked="$(grep -A1 '"issue",' "$FAKE_GH_RECORD" | grep -c '"list",' || true)"
    [ "$looked" -ge 2 ] && break
    sleep 0.05
done
[ "$looked" -ge 2 ]
"#;

const BASE_FIX_23: &str = "https://github.com/acme/widgets/issues/23";

#[test]
fn two_tickets_meeting_the_same_inherited_failure_share_one_base_fix() {
    let scenario = two_tickets_with_the_same_inherited_failure();
    // The Base fix issue is the next after the Tickets.
    scenario.agent_does_for(
        23,
        &format!(
            r#"{BOTH_TICKETS_HAVE_LOOKED}
echo "fixed" > ci-fix.txt
git add ci-fix.txt
git commit -q -m "Fix CI on the Spec branch"
gh pr create --base issue-20 --head issue-23 --title "Fix CI" --body "Closes #23"
{}"#,
            checks_on_head(GREEN)
        ),
    );

    let result = scenario.run(&[&spec_url(&scenario), "base-fix", "parallel", "2"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    for line in [
        format!(": starting Base fix #23 into issue-20: {BASE_FIX_23}\n"),
        format!(": waiting on Base fix #23, already open: {BASE_FIX_23}\n"),
        ": Base fix #23 closed; merging issue-20 in again\n".to_string(),
    ] {
        assert_eq!(
            result.stderr.matches(&line).count(),
            1,
            "{line:?} in stderr: {}",
            result.stderr
        );
    }
    assert_eq!(scenario.gh_calls_of("issue", "create").len(), 1);
    let sessions = sessions_by_issue(&scenario);
    assert_eq!(
        sessions.iter().filter(|issue| *issue == "23").count(),
        1,
        "one Base fix Run: {sessions:?}"
    );
    let fix = pr_from(&scenario, "issue-23").expect("the Base fix's PR");
    assert_eq!(fix["base"], "issue-20");
    assert_eq!(fix["state"], "MERGED");
    for ticket in [21, 22] {
        let pr = pr_from(&scenario, &format!("issue-{ticket}")).unwrap();
        assert_eq!(pr["state"], "MERGED", "#{ticket}");
    }
    for file in ["21.txt", "22.txt", "ci-fix.txt"] {
        assert!(scenario.origin_file("issue-20", file).is_some(), "{file}");
    }
    assert_eq!(spec_pr(&scenario)["isDraft"], false);
}

#[test]
fn a_shared_base_fix_that_fails_fails_the_ticket_that_started_it_and_the_one_waiting_on_it() {
    let scenario = two_tickets_with_the_same_inherited_failure();
    scenario.agent_does_for(23, &format!("{BOTH_TICKETS_HAVE_LOOKED}exit 3\n"));

    let result = scenario.run(&[&spec_url(&scenario), "base-fix", "parallel", "2"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(scenario.gh_calls_of("issue", "create").len(), 1);
    assert_eq!(scenario.gh_state()["issues"]["23"], "OPEN");
    let failed = |cause: &str| {
        let line = format!(" failed: Base fix {BASE_FIX_23} {cause}");
        let lines = result.stderr.lines();
        lines
            .filter(|at| at.starts_with("thirdshift: #2") && at.contains(&line))
            .count()
    };
    assert_eq!(failed("failed: "), 1, "stderr: {}", result.stderr);
    assert_eq!(
        failed("ended with its issue still open"),
        1,
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn a_policy_refusal_on_the_spec_pr_leaves_it_ready_and_exits_1() {
    let scenario = linear_spec();
    scenario.agent_does_for(
        SPEC,
        "gh fake refuse-merges 1 'Merge commits are not allowed on this repository.'\n",
    );

    let result = scenario.run(&["merge", &spec_url(&scenario)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/2\n");
    assert_contains(
        &result.stderr,
        "Merge commits are not allowed on this repository.",
    );
    let gh = scenario.gh_state();
    let spec = spec_pr(&scenario);
    assert_eq!(spec["state"], "OPEN");
    assert_eq!(spec["isDraft"], false);
    assert_eq!(gh["issues"]["20"], "OPEN");
    assert_eq!(scenario.origin_file("main", "first.txt"), None);
}

#[test]
fn repairs_exhausted_on_the_spec_pr_send_it_back_to_draft_and_exit_1() {
    // Five real Repairs, as the cap runs out only after them. The Run's unit
    // tests take the counting.
    let scenario = linear_spec();
    scenario.agent_does_for_in_session(SPEC, 1, &checks_on_head(RED));
    for session in 2..=6 {
        scenario.agent_does_for_in_session(
            SPEC,
            session,
            &format!("{}{}", commits_fix(session - 1), checks_on_head(RED)),
        );
    }

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_contains(&result.stderr, "repairs exhausted: CI red");
    assert_eq!(spec_prompts(&scenario).len(), 6);
    let spec = spec_pr(&scenario);
    assert_eq!(spec["state"], "OPEN");
    assert_eq!(spec["isDraft"], true);
}

#[test]
fn once_the_last_ticket_lands_the_spec_review_reviews_the_spec_branch_against_the_base_branch() {
    let scenario = linear_spec();

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let review = spec_review_call(&scenario);
    assert_eq!(review["branch"], "issue-20");
    let prompt = review["prompt"].as_str().unwrap();
    for part in [
        "/thirdshift-code-review main, with the Spec https://github.com/acme/widgets/issues/20 as the spec\n",
        &spec_url(&scenario),
        "using main as the fixed point",
        "the `thirdshift-tdd` skill",
        "Update PR https://github.com/acme/widgets/pull/2 using the `thirdshift-pr` skill",
        "\"Unaddressed findings\"",
        "Include \"Closes #20\"",
        "You run headless",
    ] {
        assert_contains(prompt, part);
    }
    let last_landed = result.stderr.find("thirdshift: #22 landed").unwrap();
    let reviewing = result.stderr.find("spec-review: session started").unwrap();
    assert!(last_landed < reviewing, "stderr: {}", result.stderr);
}

#[test]
fn the_spec_reviews_commits_reach_the_spec_branch_on_origin() {
    let scenario = linear_spec();
    scenario.agent_does_for(
        SPEC,
        r#"
echo "reviewed" > review.txt
git add review.txt
git commit -q -m "Fix a Spec finding"
git push -q origin issue-20
"#,
    );

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(
        scenario.origin_file("issue-20", "review.txt").as_deref(),
        Some("reviewed\n")
    );
}

#[test]
fn a_spec_review_that_commits_without_pushing_still_reaches_origin() {
    let scenario = linear_spec();
    scenario.agent_does_for(
        SPEC,
        r#"
echo "reviewed" > review.txt
git add review.txt
git commit -q -m "Fix a Spec finding"
"#,
    );

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(
        scenario.origin_file("issue-20", "review.txt").as_deref(),
        Some("reviewed\n")
    );
}

#[test]
fn the_spec_pr_is_a_draft_during_the_spec_review_and_marked_ready_after_it() {
    let scenario = linear_spec();
    // Fails the session unless the Spec PR is a draft while it runs.
    scenario.agent_does_for(
        SPEC,
        "gh pr view issue-20 --repo acme/widgets --json isDraft | grep -q '\"isDraft\": true'\n",
    );

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(spec_pr(&scenario)["isDraft"], false);
    let reviewed = result.stderr.find("spec-review: session ended").unwrap();
    let ready = result
        .stderr
        .find("PR https://github.com/acme/widgets/pull/2 is ready")
        .unwrap();
    assert!(reviewed < ready, "stderr: {}", result.stderr);
}

/// The line thirdshift writes in the Spec PR's body once the Spec review has
/// written it, when nothing chose the Harness, Model or Effort.
const BUILT_WITH: &str =
    "Built with claude · default model · default effort <!-- thirdshift:built-with -->";

/// The linear Spec's Tickets checklist once every Ticket is done.
const DONE_CHECKLIST: &str = "<!-- thirdshift:tickets -->
## Tickets

- [x] #21 landed with https://github.com/acme/widgets/pull/1
- [x] #22 landed with https://github.com/acme/widgets/pull/3
- [x] #23 done
<!-- /thirdshift:tickets -->";

#[test]
fn a_spec_review_whose_resume_ends_with_killed_background_work_still_delivers_the_spec_pr() {
    let scenario = linear_spec();
    // The Spec review rewrites the body without the checklist, and ends with
    // a hung test run it could not stop. So does its Resume, the fourth
    // session, after #21's, #22's and the Spec review.
    let review = format!(
        "gh fake pr issue-20 body '\"Reviewed.\\n\\nCloses #20\"'\n{}",
        leaves_running("Run each browser test file")
    );
    scenario.agent_does_for(SPEC, &review);
    scenario.agent_does_in_session(4, &review);

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_contains(
        &result.stderr,
        "thirdshift: spec-review-resume: session started",
    );
    assert_contains(
        &result.stderr,
        "thirdshift: spec-review: the Resume ended with a background task still running \
         (Run each browser test file), which was killed; carrying on",
    );
    let spec = spec_pr(&scenario);
    assert_eq!(spec["isDraft"], false);
    assert_eq!(
        spec["body"],
        format!("Reviewed.\n\nCloses #20\n\n{BUILT_WITH}\n\n{DONE_CHECKLIST}\n")
    );
}

#[test]
fn the_spec_pr_opens_as_a_draft_once_the_first_ticket_lands_with_the_checklist() {
    let scenario = linear_spec();
    // #21's session: no Spec PR yet. #22's: a draft one, #21 ticked.
    scenario.agent_does_for(
        21,
        &format!(
            "if gh pr view issue-20 --repo acme/widgets --json url; then exit 1; fi\n{}",
            agent_lands(21, "first.txt")
        ),
    );
    scenario.agent_does_for(
        22,
        &format!(
            "gh pr view issue-20 --repo acme/widgets --json isDraft,body > {}\n{}",
            scenario.path("seen-by-22").display(),
            agent_lands(22, "second.txt")
        ),
    );

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let seen: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(scenario.path("seen-by-22")).unwrap())
            .unwrap();
    assert_eq!(seen["isDraft"], true);
    let body = seen["body"].as_str().unwrap();
    assert_contains(body, "Closes #20");
    assert_eq!(
        checklist_in(body),
        "<!-- thirdshift:tickets -->
## Tickets

- [x] #21 landed with https://github.com/acme/widgets/pull/1
- [ ] #22 running
- [x] #23 done
<!-- /thirdshift:tickets -->"
    );
    let spec = spec_pr(&scenario);
    assert_eq!(spec["title"], SPEC_TITLE);
    assert_eq!(spec["base"], "main");
    let landed = result.stderr.find("thirdshift: #21 landed").unwrap();
    let opened = result
        .stderr
        .find("thirdshift: opening the Spec PR into main as a draft")
        .unwrap();
    let started = result.stderr.find("thirdshift: starting #22").unwrap();
    assert!(
        landed < opened && opened < started,
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn the_spec_review_is_logged_under_the_specs_name() {
    let scenario = linear_spec();

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let logged = result
        .stderr
        .lines()
        .find(|line| line.ends_with("-spec-review.jsonl"))
        .unwrap_or_else(|| panic!("no Spec review log in: {}", result.stderr));
    assert_contains(logged, "/20-");
}

#[test]
fn a_failed_spec_review_ends_the_spec_run_without_marking_the_spec_pr_ready() {
    let scenario = linear_spec();
    scenario.agent_does_for(SPEC, "exit 1");

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/2\n");
    assert_eq!(sessions_by_issue(&scenario), ["21", "22", "20"]);
    let spec = spec_pr(&scenario);
    assert_eq!(spec["state"], "OPEN");
    assert_eq!(spec["isDraft"], true);
    assert!(
        !result.stderr.contains("is ready for review"),
        "stderr: {}",
        result.stderr
    );
    assert_contains(&result.stderr, "thirdshift: session log: ");
}

#[test]
fn a_spec_pr_closed_during_the_spec_review_fails_the_spec_run() {
    let scenario = linear_spec();
    scenario.agent_does_for(SPEC, r#"gh fake pr issue-20 state '"CLOSED"'"#);

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_contains(&result.stderr, "is closed, not open");
    assert_eq!(spec_pr(&scenario)["isDraft"], true);
}

#[test]
fn a_failed_ticket_that_retargets_the_spec_pr_leaves_its_body_untouched() {
    let scenario = linear_spec();
    scenario.origin_has_branch("develop", "main", &[]);
    scenario.agent_does_for(
        22,
        r#"gh fake pr issue-20 base '"develop"'
gh fake pr issue-20 body '"untouched"'
exit 1
"#,
    );

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert_eq!(spec_pr(&scenario)["body"], "untouched");
    assert_contains(&result.stderr, "PR targets develop, not main");
    assert_contains(&result.stderr, "#22 failed");
    assert!(!sessions_by_issue(&scenario).contains(&SPEC.to_string()));
}

#[test]
fn successful_ticket_completion_with_an_invalid_spec_pr_starts_no_review() {
    for replacement in [false, true] {
        let scenario = linear_spec();
        scenario.origin_has_branch("develop", "main", &[]);
        let change = if replacement {
            r#"gh fake pr issue-20 state '"CLOSED"'
gh pr create --base main --head issue-20 --title Replacement --body untouched"#
        } else {
            r#"gh fake pr issue-20 base '"develop"'
gh fake pr issue-20 body '"untouched"'"#
        };
        scenario.agent_does_for(
            22,
            &format!("{}\n{change}\n", agent_lands(22, "second.txt")),
        );

        let result = scenario.run(&[&spec_url(&scenario)]);

        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(!sessions_by_issue(&scenario).contains(&SPEC.to_string()));
        assert_eq!(spec_pr(&scenario)["body"], "untouched");
        assert_contains(
            &result.stderr,
            if replacement {
                "is closed, not open"
            } else {
                "PR targets develop, not main"
            },
        );
        assert_contains(&result.stderr, "#22 landed");
        assert_eq!(scenario.gh_state()["issues"]["22"], "CLOSED");
        if replacement {
            assert_eq!(spec_pr(&scenario)["isDraft"], false);
        }
    }
}

#[test]
fn a_spec_pr_replaced_during_review_gets_its_checklist_on_the_delivered_identity() {
    for failing_review in [false, true] {
        let scenario = linear_spec();
        scenario.agent_does_for(
            SPEC,
            &format!(
                r#"
gh fake pr issue-20 state '"CLOSED"'
gh pr create --base main --head issue-20 --title Replacement --body 'Closes #20'
{}
"#,
                if failing_review { "exit 1" } else { "true" }
            ),
        );

        let result = scenario.run(&[&spec_url(&scenario)]);

        assert_eq!(
            result.code,
            Some(i32::from(failing_review)),
            "{}",
            result.stderr
        );
        let prs = prs_from(&scenario, "issue-20");
        assert_eq!(prs[0]["state"], "CLOSED");
        assert_eq!(
            result.stdout,
            format!("{}\n", prs[1]["url"].as_str().unwrap())
        );
        assert_eq!(prs[1]["isDraft"], failing_review);
        let body = prs[1]["body"].as_str().unwrap();
        assert_contains(checklist_in(body), "- [x] #21");
        assert_contains(checklist_in(body), "- [x] #22");
        if !failing_review {
            assert_contains(body, "Built with claude");
        }
    }
}

#[test]
fn spec_delivery_rejects_late_retargeting_and_keeps_the_original_identity() {
    for merge in [false, true] {
        for replacement in [false, true] {
            let scenario = linear_spec();
            scenario.origin_has_branch("develop", "main", &[]);
            let change = if replacement {
                r#"gh fake pr issue-20 state '"CLOSED"'
gh pr create --base main --head issue-20 --title Replacement --body untouched"#
            } else {
                r#"gh fake pr issue-20 base '"develop"'"#
            };
            scenario.agent_does_for(
                SPEC,
                &format!("gh fake on-ci-read 1 '{}'\n", change.replace('\'', r"'\''")),
            );
            let issue = spec_url(&scenario);
            let args = if merge {
                vec!["merge", &issue]
            } else {
                vec![&*issue]
            };

            let result = scenario.run(&args);

            assert_eq!(result.code, Some(1), "{}", result.stderr);
            let prs = prs_from(&scenario, "issue-20");
            if replacement {
                assert_contains(&result.stderr, "is closed, not open");
                assert_eq!(prs[0]["state"], "CLOSED");
                assert_eq!(prs[1]["isDraft"], false);
                assert_eq!(prs[1]["body"], "untouched");
            } else {
                assert_contains(&result.stderr, "PR targets develop, not main");
                assert_eq!(prs[0]["isDraft"], true);
            }
            let number = prs[0]["number"].as_u64().unwrap().to_string();
            assert!(
                scenario
                    .gh_calls_of("pr", "merge")
                    .iter()
                    .all(|call| call[2] != number)
            );
            assert_eq!(scenario.gh_state()["issues"][SPEC.to_string()], "OPEN");
            assert!(scenario.origin_log("issue-20").is_some());
            assert_eq!(scenario.origin_log("main").unwrap(), vec!["Initial commit"]);
            assert_eq!(
                scenario.origin_log("develop").unwrap(),
                vec!["Initial commit"]
            );
        }
    }
}

#[test]
fn help_does_not_mention_the_ticket_runs_hidden_argument() {
    let scenario = Scenario::new();

    let result = scenario.run(&["help"]);

    assert_eq!(result.code, Some(0));
    assert!(!result.stdout.contains("spec-branch"), "{}", result.stdout);
}

/// How many times a session's script that waits on other sessions looks for
/// them, 0.05 seconds apart: the harness's [`WAIT_BOUND`] of looking.
const LOOKS: u128 = WAIT_BOUND.as_millis() / 50;

/// Bash that touches `started-<ticket>` in the scenario root, then waits up
/// to [`WAIT_BOUND`] for `started-<other>` there, failing if it never appears:
/// the session for `ticket` only ends once `other`'s has started too.
fn waits_for_other_session(scenario: &Scenario, ticket: u32, other: u32) -> String {
    let root = scenario.path("");
    format!(
        r#"
touch {root}/started-{ticket}
for _ in $(seq {LOOKS}); do test -f {root}/started-{other} && break; sleep 0.05; done
test -f {root}/started-{other}
"#,
        root = root.display()
    )
}

/// A Spec #20 with two independent Tickets, #21 and #22, and #23, blocked by
/// both, whose agent checks that both have landed on its branch.
fn diamond_spec() -> Scenario {
    let scenario = Scenario::new();
    scenario.issue_titled(SPEC, SPEC_TITLE);
    scenario.spec_has_tickets(SPEC, &[(21, &[]), (22, &[]), (23, &[21, 22])]);
    scenario.agent_does_for(21, &agent_lands(21, "first.txt"));
    scenario.agent_does_for(22, &agent_lands(22, "second.txt"));
    scenario.agent_does_for(
        23,
        &format!(
            "test -f first.txt\ntest -f second.txt\n{}",
            agent_lands(23, "third.txt")
        ),
    );
    scenario
}

/// Make #21's and #22's sessions each wait for the other's to start, so the
/// Spec run only succeeds if both run at once.
fn independent_tickets_wait_for_each_other(scenario: &Scenario) {
    for (ticket, other, file) in [(21, 22, "first.txt"), (22, 21, "second.txt")] {
        scenario.agent_does_for(
            ticket,
            &format!(
                "{}{}",
                waits_for_other_session(scenario, ticket, other),
                agent_lands(ticket, file)
            ),
        );
    }
}

/// Make #22's session check that #21 already landed on the Spec branch it
/// branched off, as it has only if #21's Run ended before #22's started.
fn second_ticket_needs_the_first_landed(scenario: &Scenario) {
    scenario.agent_does_for(
        22,
        &format!("test -f first.txt\n{}", agent_lands(22, "second.txt")),
    );
}

/// A Spec #20 whose Tickets are `tickets`, each with the issues it is
/// blocked by, and whose open Tickets' agents each land their own file.
fn spec_of(tickets: &[(u32, &[u32])]) -> Scenario {
    let scenario = Scenario::new();
    scenario.issue_titled(SPEC, SPEC_TITLE);
    scenario.spec_has_tickets(SPEC, tickets);
    for (ticket, _) in tickets {
        scenario.agent_does_for(*ticket, &agent_lands(*ticket, &format!("{ticket}.txt")));
    }
    scenario
}

/// Assert the Spec run ended as a Failed spec run: exit 1, no Spec review,
/// and a draft Spec PR, its URL on stdout, with `checklist` as its Tickets
/// checklist.
fn assert_failed_spec_run(scenario: &Scenario, result: &support::RunResult, checklist: &str) {
    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert!(
        !sessions_by_issue(scenario).contains(&SPEC.to_string()),
        "a Spec review was started"
    );
    let spec = spec_pr(scenario);
    assert_eq!(
        result.stdout,
        format!("{}\n", spec["url"].as_str().unwrap())
    );
    assert_eq!(spec["state"], "OPEN");
    assert_eq!(spec["isDraft"], true);
    assert_eq!(
        checklist_in(spec["body"].as_str().unwrap()),
        format!(
            "<!-- thirdshift:tickets -->\n## Tickets\n\n{checklist}<!-- /thirdshift:tickets -->"
        )
    );
}

#[test]
fn independent_tickets_run_at_once_and_the_ticket_they_block_waits_for_both() {
    let scenario = diamond_spec();
    independent_tickets_wait_for_each_other(&scenario);

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let mut sessions = sessions_by_issue(&scenario);
    assert_eq!(sessions.pop().as_deref(), Some("20"));
    assert_eq!(sessions.pop().as_deref(), Some("23"));
    sessions.sort();
    assert_eq!(sessions, ["21", "22"]);
    let started = |ticket| {
        result
            .stderr
            .find(&format!("starting #{ticket}\n"))
            .unwrap()
    };
    let landed = |ticket| result.stderr.find(&format!("#{ticket} landed\n")).unwrap();
    assert!(started(22) < landed(21), "stderr: {}", result.stderr);
    assert!(started(21) < landed(22), "stderr: {}", result.stderr);
    assert!(landed(21) < started(23), "stderr: {}", result.stderr);
    assert!(landed(22) < started(23), "stderr: {}", result.stderr);
    assert_eq!(
        scenario.origin_file("issue-20", "third.txt").as_deref(),
        Some("23\n")
    );
    scenario.assert_cleaned_up("issue-20");
}

#[test]
fn two_tickets_starting_together_in_one_launch_directory_both_get_their_worktrees() {
    let scenario = diamond_spec();
    independent_tickets_wait_for_each_other(&scenario);

    let result = scenario.run(&[&spec_url(&scenario), "--parallel", "2"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    for ticket in [21, 22] {
        assert_contains(
            &result.stderr,
            &format!("on issue-{ticket} from origin/issue-20\n"),
        );
    }
    let cwds: Vec<String> = scenario
        .claude_calls()
        .iter()
        .map(|call| call["cwd"].as_str().unwrap().to_string())
        .collect();
    for ticket in [21, 22] {
        let worktree = scenario.path(&format!("work/widgets-issue-{ticket}"));
        assert!(cwds.contains(&worktree.display().to_string()), "{cwds:?}");
    }
}

#[test]
fn parallel_1_runs_the_tickets_one_at_a_time() {
    // One spelling end to end; the argument parsing's unit tests take both.
    let scenario = diamond_spec();
    second_ticket_needs_the_first_landed(&scenario);
    let url = spec_url(&scenario);

    let result = scenario.run(&["--parallel", "1", &url]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(sessions_by_issue(&scenario), ["21", "22", "23", "20"]);
    let landed = result.stderr.find("#21 landed\n").unwrap();
    let started = result.stderr.find("starting #22\n").unwrap();
    assert!(landed < started, "stderr: {}", result.stderr);
}

#[test]
fn without_a_limit_at_most_three_tickets_run_at_once() {
    let scenario = Scenario::new();
    scenario.issue_titled(SPEC, SPEC_TITLE);
    let tickets = [21, 22, 23, 24, 25];
    scenario.spec_has_tickets(SPEC, &tickets.map(|ticket| (ticket, &[][..])));
    let root = scenario.path("").display().to_string();
    for ticket in tickets {
        // Each session waits, up to `WAIT_BOUND`, until three are running or
        // have been, then for a moment more, and records how many it saw at
        // once.
        scenario.agent_does_for(
            ticket,
            &format!(
                r#"
mkdir -p {root}/running
touch {root}/running/{ticket}
for _ in $(seq {LOOKS}); do
  test -f {root}/three && break
  test "$(ls {root}/running | wc -l)" -ge 3 && touch {root}/three && break
  sleep 0.05
done
sleep 0.5
ls {root}/running | wc -l >> {root}/seen
rm {root}/running/{ticket}
{}"#,
                agent_lands(ticket, &format!("{ticket}.txt"))
            ),
        );
    }

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let seen = std::fs::read_to_string(scenario.path("seen")).unwrap();
    let most = seen
        .split_whitespace()
        .map(|n| n.parse::<u32>().unwrap())
        .max();
    assert_eq!(most, Some(3), "{seen}");
}

#[test]
fn an_unready_label_keeps_a_ticket_from_running() {
    // One label end to end; the Spec run's unit tests take each of them.
    let scenario = spec_of(&[(21, &[]), (22, &[])]);
    scenario.issue_labelled(21, &["wontfix"]);
    scenario.issue_labelled(22, &["ready-for-agent"]);

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_failed_spec_run(
        &scenario,
        &result,
        "- [ ] #21 unready: labelled wontfix\n\
         - [x] #22 landed with https://github.com/acme/widgets/pull/1\n",
    );
    assert_eq!(sessions_by_issue(&scenario), ["22"]);
    assert_contains(
        &result.stderr,
        "thirdshift: #21 unready: labelled wontfix\n",
    );
}

#[test]
fn a_failed_ticket_stops_only_its_dependents_and_is_not_started_again() {
    let scenario = spec_of(&[(21, &[]), (22, &[21]), (23, &[]), (24, &[])]);
    scenario.agent_does_for(21, "exit 1");

    // One at a time, so the pull requests are numbered in Ticket order.
    let result = scenario.run(&["parallel", "1", &spec_url(&scenario)]);

    assert_failed_spec_run(
        &scenario,
        &result,
        "- [ ] #21 failed: claude exited 1\n\
         - [ ] #22 blocked by #21\n\
         - [x] #23 landed with https://github.com/acme/widgets/pull/1\n\
         - [x] #24 landed with https://github.com/acme/widgets/pull/3\n",
    );
    assert_eq!(sessions_by_issue(&scenario), ["21", "23", "24"]);
    let gh = scenario.gh_state();
    assert_eq!(gh["issues"]["23"], "CLOSED");
    assert_eq!(gh["issues"]["24"], "CLOSED");
    let failed = result
        .stderr
        .lines()
        .rfind(|line| line.starts_with("thirdshift: #21 failed: "))
        .unwrap_or_else(|| panic!("no failed line for #21 in: {}", result.stderr));
    assert!(
        failed.starts_with("thirdshift: #21 failed: claude exited 1 (session log: "),
        "{failed}"
    );
    let log = failed.rsplit("session log: ").next().unwrap();
    assert!(
        std::path::Path::new(log.trim_end_matches(')')).exists(),
        "{failed}"
    );
    assert_contains(&result.stderr, "thirdshift: #22 blocked by #21\n");
}

#[test]
fn a_ticket_with_its_own_sub_issues_is_unready() {
    let scenario = spec_of(&[(21, &[]), (22, &[])]);
    scenario.spec_has_tickets(21, &[(30, &[])]);

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_failed_spec_run(
        &scenario,
        &result,
        "- [ ] #21 unready: has sub-issues\n\
         - [x] #22 landed with https://github.com/acme/widgets/pull/1\n",
    );
    assert_eq!(sessions_by_issue(&scenario), ["22"]);
    assert_contains(&result.stderr, "thirdshift: #21 unready: has sub-issues\n");
}

#[test]
fn help_explains_unready_tickets_and_that_a_spec_run_takes_every_ticket_it_can_reach() {
    let scenario = Scenario::new();

    let result = scenario.run(&["help"]);

    assert_eq!(result.code, Some(0));
    for part in [
        "Spec run",
        "every Ticket",
        "can reach",
        "Unready Ticket",
        "ready-for-human, needs-info, wontfix or\nneeds-triage",
        "sub-issues",
        "cycle",
        "merge on a Spec merges the Spec PR",
        "Tickets always merge into the Spec branch",
    ] {
        assert_contains(&result.stdout, part);
    }
}

#[test]
fn a_signal_to_the_spec_run_alone_fails_the_ticket_run_then_ends_the_spec_run_as_interrupted() {
    // SIGTERM to the Spec run's process alone while #21's agent is at work,
    // leaving it half done: #21's Run pushes that work as a failed run, #22
    // never starts, and the Spec run ends only after, as interrupted. One
    // signal end to end: the interrupt's unit tests show SIGINT, SIGTERM and
    // SIGHUP are each recorded alike, and the Spec run passes any of them on
    // to the Ticket's Run as SIGTERM.
    let scenario = linear_spec();
    let started = scenario.path("agent-started");
    let outlived = scenario.path("agent-outlived-its-sleep");
    scenario.agent_does_for(
        21,
        &format!(
            "echo 'half done' > wip.txt\ntouch {}\nsleep 30\ntouch {}\n",
            started.display(),
            outlived.display()
        ),
    );

    let result = scenario.run_and_signal(&[&spec_url(&scenario)], "agent-started", "TERM");

    assert!(!outlived.exists(), "the Ticket's session was not stopped");
    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert_eq!(sessions_by_issue(&scenario), ["21"]);
    assert_eq!(
        scenario.origin_log("issue-21").unwrap()[0],
        "thirdshift: failed run (interrupted)"
    );
    assert_eq!(
        scenario.origin_file("issue-21", "wip.txt").as_deref(),
        Some("half done\n")
    );
    assert_contains(&result.stderr, "thirdshift: #21: interrupted\n");
    assert_contains(&result.stderr, "thirdshift: #21 interrupted\n");
    assert!(!result.stderr.contains("#22"), "stderr: {}", result.stderr);
    assert_eq!(
        support::before_command_log(&result.stderr).last(),
        Some(&"thirdshift: interrupted"),
        "stderr: {}",
        result.stderr
    );
    let gh = scenario.gh_state();
    assert!(
        gh["prs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|pr| pr["head"] != "issue-20"),
        "a Spec PR was opened: {}",
        gh["prs"]
    );
}

#[test]
fn the_first_landing_opens_its_spec_pr_when_accounting_is_interrupted() {
    let scenario = linear_spec();
    scenario.agent_does_for(
        21,
        &format!(
            "{}gh fake on-issue-view 20 'touch {}; sleep 1'\n",
            agent_lands(21, "first.txt"),
            scenario.path("reading-spec-title").display()
        ),
    );

    // The Ticket has landed; interruption arrives while accounting reads
    // the title, before opening the Spec PR. No subsequent work may start.
    let result = scenario.run_and_signal(&[&spec_url(&scenario)], "reading-spec-title", "TERM");

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert_eq!(sessions_by_issue(&scenario), ["21"]);
    let pr = spec_pr(&scenario);
    let pr_url = pr["url"].as_str().unwrap();
    assert_eq!(result.stdout, format!("{pr_url}\n"));
    assert_eq!(pr["isDraft"], true);
    let checklist = checklist_in(pr["body"].as_str().unwrap());
    assert_contains(
        checklist,
        "- [x] #21 landed with https://github.com/acme/widgets/pull/1\n",
    );
    assert_contains(checklist, "- [ ] #22 not started\n");
    assert_contains(&result.stderr, "thirdshift: #21 landed\n");
    assert_eq!(
        support::before_command_log(&result.stderr).last(),
        Some(&"thirdshift: interrupted")
    );
    scenario.assert_cleaned_up("issue-20");
}

#[test]
fn an_existing_spec_pr_accounts_for_every_drained_ticket_after_interruption() {
    let scenario = diamond_spec();
    scenario.origin_has_branch("issue-20", "main", &[]);
    let pr_url = scenario.github_has_pr("issue-20", "main", "OPEN");
    for (ticket, other) in [(21, 22), (22, 21)] {
        scenario.agent_does_for(
            ticket,
            &format!(
                "echo 'half done {ticket}' > wip.txt\n{}touch {root}/both-started\nsleep 30\n",
                waits_for_other_session(&scenario, ticket, other),
                root = scenario.path("").display()
            ),
        );
    }

    let result = scenario.run_and_signal(
        &[&spec_url(&scenario), "parallel", "2"],
        "both-started",
        "TERM",
    );

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert_eq!(result.stdout, format!("{pr_url}\n"));
    let mut sessions = sessions_by_issue(&scenario);
    sessions.sort();
    assert_eq!(
        sessions,
        ["21", "22"],
        "no new Ticket or Spec review may start"
    );
    let pr = spec_pr(&scenario);
    assert_eq!(pr["url"], pr_url);
    assert_eq!(pr["isDraft"], true);
    assert_eq!(
        checklist_in(pr["body"].as_str().unwrap()),
        "<!-- thirdshift:tickets -->\n## Tickets\n\n- [ ] #21 interrupted\n- [ ] #22 interrupted\n- [ ] #23 blocked by #21, #22\n<!-- /thirdshift:tickets -->"
    );
    for ticket in [21, 22] {
        assert_contains(
            &result.stderr,
            &format!("thirdshift: #{ticket} interrupted\n"),
        );
        assert_eq!(
            scenario.origin_log(&format!("issue-{ticket}")).unwrap()[0],
            "thirdshift: failed run (interrupted)"
        );
        scenario.assert_cleaned_up(&format!("issue-{ticket}"));
    }
    assert_eq!(
        support::before_command_log(&result.stderr).last(),
        Some(&"thirdshift: interrupted")
    );
    assert!(
        !result.stderr.contains("could not update the Spec PR"),
        "{}",
        result.stderr
    );
    scenario.assert_cleaned_up("issue-20");
}

#[test]
fn parent_only_interruption_reaches_all_active_tickets_before_waiting_and_saves_their_work() {
    let scenario = diamond_spec();
    let root = scenario.path("");
    // Each Failed run's push must observe its sibling session interrupted
    // before it can finish. Salvage pushes may hold the repository locks one
    // at a time; waiting on one Ticket before signalling the other cannot pass.
    scenario.repo_has_hook(
        &scenario.origin_dir(),
        "pre-receive",
        &format!(
            r#"#!/bin/sh
while read old new ref; do
  case "$ref" in
    refs/heads/issue-21) other=22 ;;
    refs/heads/issue-22) other=21 ;;
    *) continue ;;
  esac
  for attempt in $(seq 80); do
    test -f {root}/interrupted-$other && break
    sleep 0.05
  done
  test -f {root}/interrupted-$other || exit 1
done
"#,
            root = root.display()
        ),
    );
    for (ticket, other) in [(21, 22), (22, 21)] {
        scenario.agent_does_for(ticket, &format!(
            "trap ': > {root}/interrupted-{ticket}; exit 143' TERM\n\
             echo 'half done {ticket}' > wip.txt\n{}touch {root}/both-started\nsleep 30\ntouch {root}/outlived-{ticket}\n",
            waits_for_other_session(&scenario, ticket, other), root = root.display()
        ));
    }

    let result = scenario.run_and_signal(
        &[&spec_url(&scenario), "parallel", "2"],
        "both-started",
        "TERM",
    );

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    let mut sessions = sessions_by_issue(&scenario);
    sessions.sort();
    assert_eq!(sessions, ["21", "22"]);
    for ticket in [21, 22] {
        assert!(scenario.path(&format!("interrupted-{ticket}")).exists());
        assert!(!scenario.path(&format!("outlived-{ticket}")).exists());
        assert_eq!(
            scenario.origin_file(&format!("issue-{ticket}"), "wip.txt"),
            Some(format!("half done {ticket}\n"))
        );
        assert_eq!(
            scenario.origin_log(&format!("issue-{ticket}")).unwrap()[0],
            "thirdshift: failed run (interrupted)"
        );
        assert_contains(
            &result.stderr,
            &format!("thirdshift: #{ticket} interrupted\n"),
        );
        scenario.assert_cleaned_up(&format!("issue-{ticket}"));
    }
    assert!(!result.stderr.contains("starting #23"), "{}", result.stderr);
    assert_eq!(
        support::before_command_log(&result.stderr).last(),
        Some(&"thirdshift: interrupted")
    );
}

#[test]
fn a_ticket_whose_red_check_also_fails_on_the_spec_branch_shows_the_cause_in_the_checklist_and_the_notification()
 {
    let scenario = spec_of(&[(21, &[]), (22, &[])]);
    scenario.agent_does_for(
        22,
        &format!(
            "{}{}{}",
            agent_lands(22, "22.txt"),
            checks_on_head(RED),
            checks_on_origin("issue-20", RED)
        ),
    );

    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);

    // One at a time, so #22 branches off a Spec branch #21 has landed on.
    let result = run_emailing(
        &scenario,
        &resend,
        &[
            "--email",
            "me@example.com",
            "parallel",
            "1",
            &spec_url(&scenario),
        ],
    );

    let cause = inherited_failure(&scenario, "issue-20");
    assert_failed_spec_run(
        &scenario,
        &result,
        &format!(
            "- [x] #21 landed with https://github.com/acme/widgets/pull/1\n\
             - [ ] #22 failed: {cause}\n"
        ),
    );
    assert_eq!(sessions_by_issue(&scenario), ["21", "22"], "no Repair");
    assert_contains(&result.stderr, &format!("thirdshift: #22 failed: {cause}"));
    let (_, text) = the_one_notification(&resend);
    assert_contains(&text, &format!("#22 failed: {cause}"));
    assert_eq!(pr_from(&scenario, "issue-22").unwrap()["state"], "OPEN");
    assert_eq!(scenario.origin_file("issue-20", "22.txt"), None);
}

/// Run `args` against `resend`, with `RESEND_API_KEY` set.
fn run_emailing(scenario: &Scenario, resend: &ResendStandIn, args: &[&str]) -> support::RunResult {
    scenario.run_with_env(
        args,
        &[
            ("THIRDSHIFT_RESEND_URL", resend.url()),
            ("RESEND_API_KEY", "re_test_123"),
        ],
    )
}

/// The subject and text of the one notification `resend` received.
fn the_one_notification(resend: &ResendStandIn) -> (String, String) {
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    let body = &requests[0].body;
    (
        body["subject"].as_str().unwrap().to_string(),
        body["text"].as_str().unwrap().to_string(),
    )
}

#[test]
fn a_ready_spec_run_sends_one_notification_with_a_line_per_ticket() {
    let scenario = linear_spec();
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);

    let result = run_emailing(
        &scenario,
        &resend,
        &["--email", "me@example.com", &spec_url(&scenario)],
    );

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        format!("[thirdshift] acme/widgets#20 {SPEC_TITLE}: ready for review")
    );
    assert!(
        text.starts_with("Result:       ready for review\n"),
        "{text}"
    );
    for part in [
        "Pull request: https://github.com/acme/widgets/pull/2\n",
        "Took:",
        "#21 landed with https://github.com/acme/widgets/pull/1\n",
        "#22 landed with https://github.com/acme/widgets/pull/3\n",
    ] {
        assert_contains(&text, part);
    }
    assert!(!text.contains("#23"), "{text}");
}

#[test]
fn a_failed_spec_run_sends_one_notification_with_each_tickets_outcome() {
    let scenario = spec_of(&[(21, &[]), (22, &[21]), (23, &[])]);
    scenario.agent_does_for(21, "exit 1");
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);

    let result = run_emailing(
        &scenario,
        &resend,
        &["--email", "me@example.com", &spec_url(&scenario)],
    );

    assert_failed_spec_run(
        &scenario,
        &result,
        "- [ ] #21 failed: claude exited 1\n\
         - [ ] #22 blocked by #21\n\
         - [x] #23 landed with https://github.com/acme/widgets/pull/1\n",
    );
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        format!("[thirdshift] acme/widgets#20 {SPEC_TITLE}: failed")
    );
    assert!(text.starts_with("Result:       failed\n"), "{text}");
    for part in [
        "Cause:        Tickets not done: #21, #22\n",
        "#21 failed: claude exited 1 (session log: ",
        "#22 blocked by #21\n",
        "#23 landed with https://github.com/acme/widgets/pull/1\n",
    ] {
        assert_contains(&text, part);
    }
}

#[test]
fn an_interrupted_spec_run_sends_one_notification_with_each_tickets_outcome() {
    let scenario = linear_spec();
    scenario.agent_does_for(
        21,
        &format!(
            "touch {}\nsleep 30\n",
            scenario.path("agent-started").display()
        ),
    );
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);

    let result = scenario.run_and_signal_with_env(
        &["--email", "me@example.com", &spec_url(&scenario)],
        &[
            ("THIRDSHIFT_RESEND_URL", resend.url()),
            ("RESEND_API_KEY", "re_test_123"),
        ],
        "agent-started",
        "TERM",
    );

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        format!("[thirdshift] acme/widgets#20 {SPEC_TITLE}: interrupted")
    );
    assert!(text.starts_with("Result:       interrupted\n"), "{text}");
    assert_contains(&text, "#21 interrupted\n");
    assert_contains(&text, "#22 blocked by #21\n");
}

#[test]
fn a_missing_resend_api_key_stops_the_spec_run_before_any_ticket_starts() {
    let scenario = linear_spec();
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);

    let result = scenario.run_with_env(
        &["--email", "me@example.com", &spec_url(&scenario)],
        &[("THIRDSHIFT_RESEND_URL", resend.url())],
    );

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_contains(&result.stderr, "RESEND_API_KEY");
    assert!(
        !result.stderr.contains("starting #"),
        "stderr: {}",
        result.stderr
    );
    assert!(scenario.claude_calls().is_empty());
    assert_eq!(resend.requests().len(), 0);
}

#[test]
fn without_resend_api_key_a_spec_run_sends_with_the_key_in_the_credentials() {
    let scenario = linear_spec();
    scenario.credentials_are("[resend]\nkey = \"re_file_456\"\n");
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);

    let result = scenario.run_with_env(
        &["--email", "me@example.com", &spec_url(&scenario)],
        &[("THIRDSHIFT_RESEND_URL", resend.url())],
    );

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer re_file_456")
    );
}

#[test]
fn broken_credentials_stop_the_spec_run_before_any_ticket_starts() {
    let scenario = linear_spec();
    let path = scenario.credentials_are("[resend]\nkye = \"re_file_456\"\n");
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);

    let result = scenario.run_with_env(
        &["--email", "me@example.com", &spec_url(&scenario)],
        &[("THIRDSHIFT_RESEND_URL", resend.url())],
    );

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_contains(&result.stderr, "resend.kye");
    assert_contains(&result.stderr, &path.display().to_string());
    assert!(scenario.claude_calls().is_empty());
    assert_eq!(resend.requests().len(), 0);
}

#[test]
fn a_missing_address_stops_the_spec_run_before_any_ticket_starts() {
    let scenario = linear_spec();
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);

    let result = run_emailing(&scenario, &resend, &["--email", &spec_url(&scenario)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_contains(&result.stderr, "no email address");
    assert!(
        !result.stderr.contains("starting #"),
        "stderr: {}",
        result.stderr
    );
    assert!(scenario.claude_calls().is_empty());
    assert_eq!(resend.requests().len(), 0);
}

#[test]
fn a_resend_error_leaves_the_spec_runs_outcome_alone_with_a_warning() {
    let scenario = linear_spec();
    let resend = ResendStandIn::replying(401, r#"{"message":"API key is invalid"}"#);

    let result = run_emailing(
        &scenario,
        &resend,
        &["--email", "me@example.com", &spec_url(&scenario)],
    );

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/2\n");
    assert_eq!(resend.requests().len(), 1);
    assert_eq!(
        result.stderr.lines().last(),
        Some(
            "thirdshift: warning: could not send the Run notification: \
             Resend refused the email (401 Unauthorized): API key is invalid; \
             the key came from RESEND_API_KEY"
        ),
        "stderr: {}",
        result.stderr
    );
}

/// Bash for Ticket #22's first session: it starts its work and opens its
/// PR into the Spec branch, then fails. A Continuation's session updates
/// that PR rather than opening another.
const TICKET_22_STARTS_THEN_FAILS: &str = r#"
echo started > started.txt
git add started.txt
git commit -q -m "Start on 22"
gh pr create --base issue-20 --head issue-22 --title "Ticket 22" --body "Closes #22"
exit 1
"#;

#[test]
fn rerunning_a_failed_spec_run_continues_the_spec_branch_and_the_failed_tickets_issue_branch() {
    // Two Spec runs, as the rerun is the behavior.
    let scenario = spec_of(&[(21, &[]), (22, &[21])]);
    scenario.agent_does_for_in_session(22, 1, TICKET_22_STARTS_THEN_FAILS);
    scenario.agent_does_for_in_session(
        22,
        2,
        "test -f started.txt\necho 22 > 22.txt\ngit add 22.txt\ngit commit -q -m 'Ticket 22'\n",
    );
    let first = scenario.run(&[&spec_url(&scenario)]);
    assert_eq!(first.code, Some(1), "stderr: {}", first.stderr);
    let spec_pr_url = "https://github.com/acme/widgets/pull/2";
    assert_eq!(first.stdout, format!("{spec_pr_url}\n"));

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{spec_pr_url}\n"));
    assert_eq!(sessions_by_issue(&scenario), ["21", "22", "22", "20"]);
    assert!(
        !result.stderr.contains("starting #21"),
        "stderr: {}",
        result.stderr
    );
    let calls = scenario.claude_calls();
    assert_contains(
        calls[2]["prompt"].as_str().unwrap(),
        "You are continuing work on branch issue-22",
    );
    assert_contains(
        calls[2]["prompt"].as_str().unwrap(),
        "The base branch is issue-20.",
    );
    assert_contains(
        spec_review_call(&scenario)["prompt"].as_str().unwrap(),
        &format!("Update PR {spec_pr_url}"),
    );

    let spec_prs = prs_from(&scenario, "issue-20");
    assert_eq!(spec_prs.len(), 1, "Spec PRs: {spec_prs:?}");
    assert_eq!(spec_prs[0]["url"], spec_pr_url);
    assert_eq!(spec_prs[0]["isDraft"], false);
    let ticket_prs = prs_from(&scenario, "issue-22");
    assert_eq!(ticket_prs.len(), 1, "#22's PRs: {ticket_prs:?}");
    assert_eq!(ticket_prs[0]["base"], "issue-20");
    assert_eq!(ticket_prs[0]["state"], "MERGED");
    assert_eq!(scenario.gh_state()["issues"]["22"], "CLOSED");
    for branch in ["issue-20-branch-2", "issue-22-branch-2"] {
        assert_eq!(scenario.origin_log(branch), None, "{branch} was started");
    }
    for file in ["21.txt", "started.txt", "22.txt"] {
        assert!(
            scenario.origin_file("issue-20", file).is_some(),
            "{file} is not on the Spec branch"
        );
    }
}

#[test]
fn with_every_ticket_closed_and_a_spec_branch_the_spec_run_goes_straight_to_the_spec_review_of_its_spec_pr()
 {
    let scenario = spec_of(&[(21, &[]), (22, &[21])]);
    scenario.issue_is(21, "CLOSED");
    scenario.issue_is(22, "CLOSED");
    scenario.origin_has_branch("issue-20", "main", &["Tickets 21 and 22"]);
    let spec_pr_url = scenario.github_has_pr("issue-20", "main", "OPEN");

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(sessions_by_issue(&scenario), ["20"]);
    assert!(
        !result.stderr.contains("starting #"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(result.stdout, format!("{spec_pr_url}\n"));
    let spec = spec_pr(&scenario);
    assert_eq!(spec["url"], spec_pr_url);
    assert_eq!(spec["base"], "main");
    assert_eq!(spec["isDraft"], false);
    assert_eq!(
        checklist_in(spec["body"].as_str().unwrap()),
        "<!-- thirdshift:tickets -->\n## Tickets\n\n- [x] #21 done\n- [x] #22 done\n<!-- /thirdshift:tickets -->"
    );
    assert_eq!(prs_from(&scenario, "issue-20").len(), 1);
}

#[test]
fn a_spec_run_that_left_no_spec_branch_on_origin_and_no_spec_pr_releases_the_specs_claim() {
    let scenario = linear_spec();
    scenario.issue_labelled(SPEC, &["ready-for-agent", "enhancement"]);
    scenario.repo_has_hook(
        &scenario.origin_dir(),
        "pre-receive",
        "#!/bin/sh\necho \"origin says no\"\nexit 1\n",
    );

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_contains(&result.stderr, "origin says no");
    assert_eq!(scenario.origin_log("issue-20"), None);
    assert!(pr_from(&scenario, "issue-20").is_none());
    assert_eq!(
        scenario.issue_labels(SPEC),
        ["enhancement", "ready-for-agent"]
    );
    assert_contains(
        &result.stderr,
        "thirdshift: releasing the Claim on #20: labelling it ready-for-agent, in place of in-progress\n",
    );
}

#[test]
fn a_merge_run_of_a_spec_ends_with_the_spec_closed_and_its_claim_removed() {
    let scenario = linear_spec();
    scenario.issue_labelled(SPEC, &["ready-for-agent", "enhancement"]);
    for ticket in [21, 22] {
        scenario.issue_labelled(ticket, &["ready-for-agent"]);
    }

    let result = scenario.run(&["merge", &spec_url(&scenario)]);

    assert_spec_pr_merged(&scenario, &result);
    assert_eq!(scenario.issue_labels(SPEC), ["enhancement"]);
    assert_contains(
        &result.stderr,
        "thirdshift: removing in-progress from issue #20\n",
    );
    for ticket in [21, 22] {
        assert_eq!(scenario.issue_labels(ticket), ["ready-for-agent"]);
    }
}

/// The arguments of each call to `claude` that was an agent session, which
/// streams JSON, and of each that was the test call that checks a Model.
fn sessions_and_test_calls(scenario: &Scenario) -> (Vec<Vec<String>>, Vec<Vec<String>>) {
    scenario
        .claude_calls()
        .iter()
        .map(|call| {
            let args = call["argv"].as_array().unwrap().iter();
            args.map(|arg| arg.as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        })
        .partition(|args| args.iter().any(|arg| arg == "--output-format"))
}

#[test]
fn a_spec_run_checks_its_model_once_and_every_ticket_and_the_spec_review_runs_on_it() {
    let scenario = linear_spec();

    let result = scenario.run(&[&spec_url(&scenario), "model", "opus", "effort", "high"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let (sessions, test_calls) = sessions_and_test_calls(&scenario);
    assert_eq!(test_calls.len(), 1, "{test_calls:?}");
    assert_eq!(
        sessions.len(),
        3,
        "#21, #22 and the Spec review: {sessions:?}"
    );
    for args in sessions {
        assert!(
            args.windows(4)
                .any(|window| window == ["--model", "opus", "--effort", "high"]),
            "{args:?}"
        );
    }
}
