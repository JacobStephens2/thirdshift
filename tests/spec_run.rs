//! Spec runs: `thirdshift <Issue URL>` on a Spec, an issue with sub-issues,
//! works through its Tickets in dependency order, each a Merge run into the
//! Spec branch, then opens the Spec PR from the Spec branch into the Base
//! branch as a draft, has the Spec review review the Spec branch, and marks
//! the Spec PR ready for review. The Spec PR then goes through the Repair
//! loop, and with `merge`, the Self-merge.

mod support;

use support::Scenario;
use support::resend::ResendStandIn;

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

/// The newest pull request from `head`, if there is one.
fn pr_from(scenario: &Scenario, head: &str) -> Option<serde_json::Value> {
    scenario.gh_state()["prs"]
        .as_array()
        .into_iter()
        .flatten()
        .rfind(|pr| pr["head"] == head)
        .cloned()
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

#[test]
fn closed_tickets_are_not_run() {
    let scenario = linear_spec();

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert!(!sessions_by_issue(&scenario).contains(&"23".to_string()));
    assert!(!result.stderr.contains("#23"), "stderr: {}", result.stderr);
}

#[test]
fn the_spec_branch_is_pushed_before_the_first_ticket_starts() {
    let scenario = linear_spec();
    scenario.agent_does_for(
        21,
        &format!(
            "git ls-remote --exit-code origin refs/heads/issue-20 >/dev/null\n{}",
            agent_lands(21, "first.txt")
        ),
    );

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let pushed = result.stderr.find("thirdshift: pushing issue-20").unwrap();
    let started = result.stderr.find("thirdshift: starting #21").unwrap();
    assert!(pushed < started, "stderr: {}", result.stderr);
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
fn a_merge_ask_from_the_user_config_self_merges_the_spec_pr() {
    let scenario = linear_spec();
    scenario.user_config_is("[merge]\nalways = true\n");

    let result = scenario.run(&[&spec_url(&scenario)]);

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
    assert_contains(&prompts[1], "/thirdshift:resolving-merge-conflicts");
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
fn a_failed_ticket_ends_the_spec_run_before_the_tickets_it_blocks() {
    let scenario = linear_spec();
    scenario.agent_does_for(21, "exit 1");

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert_eq!(sessions_by_issue(&scenario), ["21"]);
    assert_contains(&result.stderr, "thirdshift: #21 failed");
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
fn once_the_last_ticket_lands_the_spec_review_reviews_the_spec_branch_against_the_base_branch() {
    let scenario = linear_spec();

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let review = spec_review_call(&scenario);
    assert_eq!(review["branch"], "issue-20");
    let prompt = review["prompt"].as_str().unwrap();
    for part in [
        "/thirdshift:code-review main, with the Spec https://github.com/acme/widgets/issues/20 as the spec\n",
        &spec_url(&scenario),
        "using main as the fixed point",
        "/thirdshift:tdd",
        "Update PR https://github.com/acme/widgets/pull/2 using /thirdshift:pr",
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

#[test]
fn the_spec_review_writes_the_spec_pr_body_and_the_checklist_is_put_back_after_it() {
    let scenario = linear_spec();
    scenario.agent_does_for(
        SPEC,
        r#"gh fake pr issue-20 body '"The whole Spec.\n\nUnaddressed findings: none\n\nCloses #20"'"#,
    );

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let spec = spec_pr(&scenario);
    assert_eq!(spec["isDraft"], false);
    assert_eq!(
        spec["body"],
        format!(
            "The whole Spec.\n\nUnaddressed findings: none\n\nCloses #20\n\n{DONE_CHECKLIST}\n"
        )
    );
}

/// The linear Spec's Tickets checklist once every Ticket is done.
const DONE_CHECKLIST: &str = "<!-- thirdshift:tickets -->
## Tickets

- [x] #21 landed with https://github.com/acme/widgets/pull/1
- [x] #22 landed with https://github.com/acme/widgets/pull/3
- [x] #23 done
<!-- /thirdshift:tickets -->";

#[test]
fn a_spec_review_that_keeps_the_markers_has_the_checklist_replaced_between_them() {
    let scenario = linear_spec();
    scenario.agent_does_for(
        SPEC,
        r#"gh fake pr issue-20 body '"Before.\n\n<!-- thirdshift:tickets -->\nstale\n<!-- /thirdshift:tickets -->\n\nAfter. Closes #20"'"#,
    );

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(
        spec_pr(&scenario)["body"],
        format!("Before.\n\n{DONE_CHECKLIST}\n\nAfter. Closes #20")
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
    assert_contains(logged, "/acme-widgets-issue-20-");
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
fn an_issue_with_no_sub_issues_is_an_ordinary_run() {
    let scenario = Scenario::new();
    scenario.agent_does(
        r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#,
    );

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    assert_eq!(sessions_by_issue(&scenario), ["7"]);
    assert!(!result.stderr.contains("#7:"), "stderr: {}", result.stderr);
    assert_eq!(scenario.gh_state()["prs"][0]["base"], "main");
}

#[test]
fn help_does_not_mention_the_ticket_runs_hidden_argument() {
    let scenario = Scenario::new();

    let result = scenario.run(&["help"]);

    assert_eq!(result.code, Some(0));
    assert!(!result.stdout.contains("spec-branch"), "{}", result.stdout);
}

/// Bash that touches `started-<ticket>` in the scenario root, then waits up
/// to ten seconds for `started-<other>` there, failing if it never appears:
/// the session for `ticket` only ends once `other`'s has started too.
fn waits_for_other_session(scenario: &Scenario, ticket: u32, other: u32) -> String {
    let root = scenario.path("");
    format!(
        r#"
touch {root}/started-{ticket}
for _ in $(seq 200); do test -f {root}/started-{other} && break; sleep 0.05; done
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
    for args in [["parallel", "1"], ["--parallel", "1"]] {
        let scenario = diamond_spec();
        second_ticket_needs_the_first_landed(&scenario);
        let url = spec_url(&scenario);

        let result = scenario.run(&[args[0], args[1], &url]);

        assert_eq!(result.code, Some(0), "{args:?} stderr: {}", result.stderr);
        assert_eq!(sessions_by_issue(&scenario), ["21", "22", "23", "20"]);
        let landed = result.stderr.find("#21 landed\n").unwrap();
        let started = result.stderr.find("starting #22\n").unwrap();
        assert!(landed < started, "stderr: {}", result.stderr);
    }
}

#[test]
fn spec_parallel_in_the_user_config_sets_how_many_tickets_run_at_once() {
    let scenario = diamond_spec();
    second_ticket_needs_the_first_landed(&scenario);
    scenario.user_config_is("[spec]\nparallel = 1\n");

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(sessions_by_issue(&scenario), ["21", "22", "23", "20"]);
}

#[test]
fn the_parallel_flag_wins_over_spec_parallel() {
    let scenario = diamond_spec();
    independent_tickets_wait_for_each_other(&scenario);
    scenario.user_config_is("[spec]\nparallel = 1\n");

    let result = scenario.run(&[&spec_url(&scenario), "parallel", "2"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
}

#[test]
fn without_a_limit_at_most_three_tickets_run_at_once() {
    let scenario = Scenario::new();
    scenario.issue_titled(SPEC, SPEC_TITLE);
    let tickets = [21, 22, 23, 24, 25];
    scenario.spec_has_tickets(SPEC, &tickets.map(|ticket| (ticket, &[][..])));
    let root = scenario.path("").display().to_string();
    for ticket in tickets {
        // Each session waits, up to ten seconds, until three are running or
        // have been, then for a moment more, and records how many it saw at
        // once.
        scenario.agent_does_for(
            ticket,
            &format!(
                r#"
mkdir -p {root}/running
touch {root}/running/{ticket}
for _ in $(seq 200); do
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
fn parallel_on_an_issue_with_no_sub_issues_stops_before_any_work_naming_the_flag() {
    let scenario = Scenario::new();

    let result = scenario.run(&["parallel", "2", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    scenario.assert_rejected_before_any_work(&result, "parallel");
    assert!(
        scenario.origin_log("issue-7").is_none(),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn a_needs_info_ticket_and_what_it_blocks_are_not_run_while_an_independent_ticket_lands() {
    let scenario = spec_of(&[(21, &[]), (22, &[21]), (23, &[])]);
    scenario.issue_labelled(21, &["needs-info"]);

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_failed_spec_run(
        &scenario,
        &result,
        "- [ ] #21 unready: labelled needs-info\n\
         - [ ] #22 blocked by #21\n\
         - [x] #23 landed with https://github.com/acme/widgets/pull/1\n",
    );
    assert_eq!(sessions_by_issue(&scenario), ["23"]);
    assert_eq!(scenario.gh_state()["issues"]["23"], "CLOSED");
    for line in [
        "thirdshift: #21 unready: labelled needs-info\n",
        "thirdshift: #22 blocked by #21\n",
        "thirdshift: #23 landed with https://github.com/acme/widgets/pull/1\n",
    ] {
        assert_contains(&result.stderr, line);
    }
}

#[test]
fn each_unready_label_keeps_a_ticket_from_running() {
    for label in ["ready-for-human", "wontfix", "needs-triage"] {
        let scenario = spec_of(&[(21, &[]), (22, &[])]);
        scenario.issue_labelled(21, &[label]);
        scenario.issue_labelled(22, &["ready-for-agent"]);

        let result = scenario.run(&[&spec_url(&scenario)]);

        assert_failed_spec_run(
            &scenario,
            &result,
            &format!(
                "- [ ] #21 unready: labelled {label}\n\
                 - [x] #22 landed with https://github.com/acme/widgets/pull/1\n"
            ),
        );
        assert_eq!(sessions_by_issue(&scenario), ["22"], "{label}");
        assert_contains(
            &result.stderr,
            &format!("thirdshift: #21 unready: labelled {label}\n"),
        );
    }
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
fn an_open_outside_blocker_holds_a_ticket_back_and_a_closed_one_does_not() {
    let scenario = spec_of(&[(21, &[99]), (22, &[98])]);
    scenario.issue_is(98, "CLOSED");

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_failed_spec_run(
        &scenario,
        &result,
        "- [ ] #21 blocked by #99 (outside the Spec)\n\
         - [x] #22 landed with https://github.com/acme/widgets/pull/1\n",
    );
    assert_eq!(sessions_by_issue(&scenario), ["22"]);
    assert_contains(
        &result.stderr,
        "thirdshift: #21 blocked by #99 (outside the Spec)\n",
    );
}

#[test]
fn tickets_in_a_cycle_are_not_run_and_the_summary_names_the_cycle() {
    let scenario = spec_of(&[(21, &[22]), (22, &[21]), (23, &[21]), (24, &[])]);

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_failed_spec_run(
        &scenario,
        &result,
        "- [ ] #21 in a cycle: #21 blocked by #22 blocked by #21\n\
         - [ ] #22 in a cycle: #22 blocked by #21 blocked by #22\n\
         - [ ] #23 blocked by #21\n\
         - [x] #24 landed with https://github.com/acme/widgets/pull/1\n",
    );
    assert_eq!(sessions_by_issue(&scenario), ["24"]);
    for line in [
        "thirdshift: #21 in a cycle: #21 blocked by #22 blocked by #21\n",
        "thirdshift: #22 in a cycle: #22 blocked by #21 blocked by #22\n",
        "thirdshift: #23 blocked by #21\n",
    ] {
        assert_contains(&result.stderr, line);
    }
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
fn removing_a_needs_info_label_while_another_ticket_runs_lets_it_run() {
    let scenario = spec_of(&[(21, &[]), (22, &[])]);
    scenario.issue_labelled(21, &["needs-info"]);
    scenario.agent_does_for(
        22,
        &format!("gh fake labels 21 '[]'\n{}", agent_lands(22, "22.txt")),
    );

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(sessions_by_issue(&scenario), ["22", "21", "20"]);
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
fn sigterm_while_a_ticket_runs_ends_it_through_its_failed_run_and_the_spec_run_as_interrupted() {
    assert_interrupt_fails_the_spec_run("TERM");
}

#[test]
fn sighup_while_a_ticket_runs_ends_it_through_its_failed_run_and_the_spec_run_as_interrupted() {
    assert_interrupt_fails_the_spec_run("HUP");
}

#[test]
fn sigint_to_the_spec_run_alone_is_passed_on_to_the_ticket_run() {
    assert_interrupt_fails_the_spec_run("INT");
}

/// Send `signal` to the Spec run's process alone while #21's agent is at
/// work, leaving it half done: #21's Run pushes that work as a failed run,
/// #22 never starts, and the Spec run ends only after, as interrupted.
fn assert_interrupt_fails_the_spec_run(signal: &str) {
    let scenario = linear_spec();
    let started = scenario.path("agent-started");
    scenario.agent_does_for(
        21,
        &format!(
            "echo 'half done' > wip.txt\ntouch {}\nsleep 30\n",
            started.display()
        ),
    );

    let began = std::time::Instant::now();
    let result = scenario.run_and_signal(&[&spec_url(&scenario)], "agent-started", signal);

    assert!(
        began.elapsed() < std::time::Duration::from_secs(20),
        "the Ticket's session was not stopped"
    );
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
        result.stderr.lines().last(),
        Some("thirdshift: interrupted"),
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
fn a_ticket_that_fails_once_the_spec_pr_is_open_is_shown_failed_in_its_checklist() {
    let scenario = spec_of(&[(21, &[]), (22, &[])]);
    scenario.agent_does_for(22, "exit 1");

    // One at a time, so the Spec PR is open when #22 starts.
    let result = scenario.run(&["parallel", "1", &spec_url(&scenario)]);

    assert_failed_spec_run(
        &scenario,
        &result,
        "- [x] #21 landed with https://github.com/acme/widgets/pull/1\n\
         - [ ] #22 failed: claude exited 1\n",
    );
    assert_contains(
        &result.stderr,
        "thirdshift: updating the Spec PR's Tickets checklist\n",
    );
}

#[test]
fn a_ticket_that_lands_just_before_github_stops_answering_still_gets_the_draft_spec_pr() {
    let scenario = spec_of(&[(21, &[]), (22, &[21])]);
    scenario.agent_does_for(
        21,
        &format!("gh fake fails 'api graphql'\n{}", agent_lands(21, "21.txt")),
    );

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_failed_spec_run(
        &scenario,
        &result,
        "- [ ] #21 landed with https://github.com/acme/widgets/pull/1, but is still open\n\
         - [ ] #22 blocked by #21\n",
    );
    assert_eq!(sessions_by_issue(&scenario), ["21"]);
}

#[test]
fn a_failed_spec_review_that_rewrote_the_body_has_the_checklist_put_back_in_the_draft() {
    let scenario = linear_spec();
    scenario.agent_does_for(
        SPEC,
        "gh fake pr issue-20 body '\"Half a description. Closes #20\"'\nexit 1\n",
    );

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    let spec = spec_pr(&scenario);
    assert_eq!(spec["isDraft"], true);
    assert_eq!(
        spec["body"],
        format!("Half a description. Closes #20\n\n{DONE_CHECKLIST}\n")
    );
}

#[test]
fn a_checklist_update_github_refuses_is_only_a_warning() {
    let scenario = spec_of(&[(21, &[]), (22, &[])]);
    scenario.agent_does_for(
        21,
        &format!(
            "gh fake fails 'api --method'\n{}",
            agent_lands(21, "21.txt")
        ),
    );

    // One at a time, so the Spec PR is open when #22 starts.
    let result = scenario.run(&["parallel", "1", &spec_url(&scenario)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(sessions_by_issue(&scenario), ["21", "22", "20"]);
    assert_contains(
        &result.stderr,
        "thirdshift: could not update the Spec PR's Tickets checklist: ",
    );
    assert_contains(&result.stderr, "thirdshift: #22 landed\n");
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
    assert_contains(&text, "#21 interrupted\n");
    assert_contains(&text, "#22 blocked by #21\n");
}

#[test]
fn a_ready_ticket_an_interrupt_kept_from_starting_is_not_started_in_the_notification() {
    let scenario = spec_of(&[(21, &[]), (22, &[])]);
    scenario.agent_does_for(
        21,
        &format!(
            "touch {}\nsleep 30\n",
            scenario.path("agent-started").display()
        ),
    );
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);

    let result = scenario.run_and_signal_with_env(
        &[
            "--email",
            "me@example.com",
            "parallel",
            "1",
            &spec_url(&scenario),
        ],
        &[
            ("THIRDSHIFT_RESEND_URL", resend.url()),
            ("RESEND_API_KEY", "re_test_123"),
        ],
        "agent-started",
        "TERM",
    );

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    let (_, text) = the_one_notification(&resend);
    assert_contains(&text, "#21 interrupted\n");
    assert_contains(&text, "#22 not started\n");
}

#[test]
fn no_email_keeps_email_always_from_sending_for_a_spec_run() {
    let scenario = linear_spec();
    scenario.user_config_is("[email]\nalways = true\nto = \"me@example.com\"\n");
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);

    let result = run_emailing(&scenario, &resend, &["--no-email", &spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(resend.requests().len(), 0);
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

/// The PRs on the fake GitHub from `head`.
fn prs_from(scenario: &Scenario, head: &str) -> Vec<serde_json::Value> {
    scenario.gh_state()["prs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|pr| pr["head"] == head)
        .cloned()
        .collect()
}

#[test]
fn rerunning_a_failed_spec_run_continues_the_spec_branch_and_the_failed_tickets_issue_branch() {
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
        calls[3]["prompt"].as_str().unwrap(),
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
fn with_every_ticket_closed_and_a_spec_branch_the_spec_run_goes_straight_to_the_spec_review() {
    let scenario = spec_of(&[(21, &[]), (22, &[21])]);
    scenario.issue_is(21, "CLOSED");
    scenario.issue_is(22, "CLOSED");
    scenario.origin_has_branch("issue-20", "main", &["Tickets 21 and 22"]);

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(sessions_by_issue(&scenario), ["20"]);
    assert!(
        !result.stderr.contains("starting #"),
        "stderr: {}",
        result.stderr
    );
    let spec = spec_pr(&scenario);
    assert_eq!(
        result.stdout,
        format!("{}\n", spec["url"].as_str().unwrap())
    );
    assert_eq!(spec["base"], "main");
    assert_eq!(spec["isDraft"], false);
    assert_contains(spec["body"].as_str().unwrap(), "Closes #20");
    assert_eq!(prs_from(&scenario, "issue-20").len(), 1);
}

#[test]
fn with_every_ticket_closed_and_no_spec_branch_there_is_nothing_to_do() {
    let scenario = spec_of(&[(21, &[]), (22, &[21])]);
    scenario.issue_is(21, "CLOSED");
    scenario.issue_is(22, "CLOSED");

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    scenario.assert_rejected_before_any_work(
        &result,
        "thirdshift: every Ticket is closed and there is no Spec branch; nothing to do\n",
    );
    assert_eq!(scenario.origin_log("issue-20"), None);
    assert_eq!(scenario.launch_git(&["branch", "--list", "issue-20"]), "");
    let gh = scenario.gh_state();
    assert_eq!(gh["prs"], serde_json::json!([]));
    assert_eq!(gh["issues"]["20"], "OPEN");
}
