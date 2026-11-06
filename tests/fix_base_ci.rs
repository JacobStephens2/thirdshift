//! Base branch maintenance through the public Fix Pass.
mod support;

use serde_json::json;
use support::Scenario;

fn failed_base(scenario: &Scenario, attempt: u64) -> String {
    let head = scenario
        .origin_git(&["rev-parse", "main"])
        .trim()
        .to_string();
    let mut state = scenario.gh_state();
    state["checks"][&head] = json!([{
        "id":901, "name":"test", "conclusion":"failure", "url":"https://github.com/acme/widgets/actions/runs/900/job/901"
    }]);
    state["runs"] = json!([{
        "id":900, "head_branch":"main", "event":"push", "head_sha":head,
        "html_url":"https://github.com/acme/widgets/actions/runs/900",
        "status":"completed", "conclusion":"failure", "updated_at":"2020-01-01T00:00:00Z",
        "run_attempt":attempt, "jobs":[{"id":901,"name":"test","conclusion":"failure"}]
    }]);
    scenario.write_gh_state(&state);
    head
}

#[test]
fn first_failed_base_attempt_is_rerun_and_the_pass_skips_before_pr_selection() {
    let scenario = Scenario::new();
    failed_base(&scenario, 1);
    let result = scenario.run(&["fix", "no-email"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        scenario.gh_calls_of("run", "rerun").len(),
        1,
        "{}",
        result.stderr
    );
    assert!(result.stderr.contains("re-running"), "{}", result.stderr);
    assert!(scenario.gh_calls_of("pr", "list").is_empty());
    assert!(scenario.claude_calls().is_empty());
    let again = scenario.run(&["fix", "no-email"]);
    assert_eq!(again.code, Some(0), "{}", again.stderr);
    assert_eq!(scenario.gh_calls_of("run", "rerun").len(), 1);
}

const FIX: &str = r#"
echo fixed > ci-fix.txt
git add ci-fix.txt
git commit -qm 'Fix base CI'
"#;

#[test]
fn persistent_base_failure_opens_a_ready_pr_at_the_origin_tip() {
    let scenario = Scenario::new();
    let head = failed_base(&scenario, 2);
    scenario.agent_does(FIX);
    let result = scenario.run(&["fix", "no-merge", "no-email"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    let prs = state["prs"].as_array().unwrap();
    assert_eq!(prs.len(), 1, "{}", result.stderr);
    let branch = format!("fix-main-{}", &head[..7]);
    assert_eq!(prs[0]["head"], branch);
    assert_eq!(prs[0]["base"], "main");
    assert_eq!(prs[0]["title"], "Fix CI on main: test");
    assert_eq!(prs[0]["state"], "OPEN");
    assert_eq!(prs[0]["isDraft"], false);
    assert_eq!(
        scenario
            .origin_git(&["rev-parse", &format!("{branch}~1")])
            .trim(),
        head
    );
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 1);
    let prompt = calls[0]["prompt"].as_str().unwrap();
    assert!(
        prompt.contains("https://github.com/acme/widgets/actions/runs/900/job/901"),
        "{prompt}"
    );
    assert!(prompt.contains("every red check"), "{prompt}");
    let (_, notification) = support::resend::saved_notification(&scenario, "fix");
    assert!(notification.contains("Fix run"), "{notification}");
    let again = scenario.run(&["fix", "no-email"]);
    assert_eq!(again.code, Some(0), "{}", again.stderr);
    assert_eq!(scenario.claude_calls().len(), 1);
    assert_eq!(scenario.gh_state()["prs"].as_array().unwrap().len(), 1);
}

#[test]
fn delivery_repairs_checks_also_red_on_base_and_self_merges_with_issue_cleanup() {
    let scenario = Scenario::new();
    failed_base(&scenario, 2);
    let mut state = scenario.gh_state();
    state["titles"]["7"] = "CI red on main".into();
    state["issues"]["8"] = "OPEN".into();
    state["titles"]["8"] = "CI red on main: test".into();
    state["labels"]["8"] = json!(["base-fix"]);
    state["issues"]["9"] = "OPEN".into();
    state["titles"]["9"] = "CI red on mainline".into();
    scenario.write_gh_state(&state);
    scenario.user_config_is("[merge]\nalways = true\n");
    scenario.agent_does_in_session(1, &format!("{FIX}\ngh fake checks \"$(git rev-parse HEAD)\" '[{{\"name\":\"test\",\"conclusion\":\"failure\",\"url\":\"https://ci.example/test\"}}]'"));
    scenario.agent_does_in_session(2, "echo repaired >> ci-fix.txt\ngit add ci-fix.txt\ngit commit -qm 'Repair inherited-named check'");
    let result = scenario.run(&["fix", "no-email"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    assert_eq!(state["prs"][0]["state"], "MERGED", "{}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 2, "{}", result.stderr);
    assert_eq!(state["issues"]["7"], "CLOSED", "{}", result.stderr);
    assert_eq!(state["issues"]["8"], "CLOSED", "{}", result.stderr);
    assert_eq!(state["issues"]["9"], "OPEN");
    assert!(
        scenario.claude_calls()[1]["prompt"]
            .as_str()
            .unwrap()
            .contains("every red check")
    );
}

#[test]
fn existing_repair_prs_in_all_states_suppress_another_session() {
    for (status, draft, labels) in [
        ("OPEN", false, json!([])),
        ("OPEN", true, json!([])),
        ("OPEN", false, json!(["in-progress"])),
        ("CLOSED", false, json!([])),
        ("MERGED", false, json!([])),
    ] {
        let scenario = Scenario::new();
        let head = failed_base(&scenario, 2);
        let branch = format!("fix-main-{}", &head[..7]);
        scenario.origin_has_branch(&branch, "main", &["Attempted CI fix"]);
        scenario.github_has_pr(&branch, "main", status);
        let mut state = scenario.gh_state();
        state["prs"][0]["isDraft"] = draft.into();
        state["prs"][0]["labels"] = labels;
        scenario.write_gh_state(&state);
        let result = scenario.run(&["fix", "no-email"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert!(
            result.stderr.contains("already exists"),
            "{}",
            result.stderr
        );
        assert!(scenario.claude_calls().is_empty());
        assert_eq!(scenario.gh_state()["prs"].as_array().unwrap().len(), 1);
    }
}

#[test]
fn author_scoped_pass_leaves_base_ci_alone() {
    let scenario = Scenario::new();
    failed_base(&scenario, 1);
    let result = scenario.run(&["fix", "--author", "acme", "no-email"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(scenario.gh_calls_of("run", "rerun").is_empty());
    assert!(scenario.claude_calls().is_empty());
}

#[test]
fn non_rerunnable_tip_checks_ignore_an_old_failed_push_attempt() {
    let scenario = Scenario::new();
    failed_base(&scenario, 1);
    let mut state = scenario.gh_state();
    state["runs"][0]["head_sha"] = "old-commit".into();
    let tip = scenario.origin_git(&["rev-parse", "main"]);
    state["checks"][tip.trim()] =
        json!([{"name":"external", "conclusion":"failure", "url":"https://ci.example/failed"}]);
    scenario.write_gh_state(&state);
    scenario.agent_does(FIX);
    let result = scenario.run(&["fix", "no-email"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(scenario.gh_calls_of("run", "rerun").is_empty());
    assert_eq!(
        scenario.gh_state()["prs"][0]["title"],
        "Fix CI on main: external"
    );
}

#[test]
fn flags_override_merge_and_email_config_for_base_repairs() {
    use support::resend::ResendStandIn;
    for (flags, config, merged, emails) in [
        (
            vec!["no-merge", "email", "operator@example.com"],
            "[merge]\nalways=true\n",
            false,
            1,
        ),
        (
            vec!["merge", "email-unless-merged", "operator@example.com"],
            "",
            true,
            0,
        ),
        (
            vec!["no-merge", "no-email"],
            "[merge]\nalways=true\n[email]\nalways=true\nto='operator@example.com'\n",
            false,
            0,
        ),
        (
            vec!["no-merge"],
            "[email]\nunless_merged=true\nto='operator@example.com'\n",
            false,
            1,
        ),
    ] {
        let scenario = Scenario::new();
        failed_base(&scenario, 2);
        scenario.agent_does(FIX);
        scenario.user_config_is(config);
        let resend = ResendStandIn::replying(200, r#"{"id":"sent"}"#);
        let mut args = vec!["fix"];
        args.extend(flags);
        let result = scenario.run_with_env(
            &args,
            &[
                ("THIRDSHIFT_RESEND_URL", resend.url()),
                ("RESEND_API_KEY", "re_test_123"),
            ],
        );
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_eq!(
            scenario.gh_state()["prs"][0]["state"],
            if merged { "MERGED" } else { "OPEN" }
        );
        let requests = resend.requests();
        assert_eq!(requests.len(), emails, "{}", result.stderr);
        if let Some(request) = requests.first() {
            assert!(
                request.body["text"]
                    .as_str()
                    .unwrap()
                    .contains("https://github.com/acme/widgets/pull/1")
            );
        }
    }
}

#[test]
fn named_base_with_no_rerunnable_jobs_repairs_its_origin_tip() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("release", "main", &["Release tip"]);
    failed_base(&scenario, 1);
    let tip = scenario.origin_git(&["rev-parse", "release"]);
    let mut state = scenario.gh_state();
    state["checks"] = json!({});
    state["runs"][0]["head_branch"] = "release".into();
    state["runs"][0]["head_sha"] = tip.trim().into();
    state["runs"][0]["jobs"] = json!([]);
    scenario.write_gh_state(&state);
    scenario.agent_does(FIX);
    let result = scenario.run(&["fix", "base", "release", "no-email"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    assert_eq!(state["prs"][0]["base"], "release");
    assert_eq!(state["prs"][0]["title"], "Fix CI on release: push workflow");
    assert_eq!(
        state["prs"][0]["head"],
        format!("fix-release-{}", &tip[..7])
    );
    assert!(scenario.gh_calls_of("run", "rerun").is_empty());
}

/// A commit status or external check can be posted by a principal who cannot
/// close issues. Merging a pull request into the default branch closes issues
/// named by `fixes #15` or `resolves acme/widgets#8` in the description
/// (any case, optional colon). The repair description must not carry that
/// reference through from the check name or URL.
#[test]
fn external_check_text_cannot_close_issues_from_the_repair_pr_body() {
    let scenario = Scenario::new();
    failed_base(&scenario, 1);
    let mut state = scenario.gh_state();
    state["runs"][0]["head_sha"] = "old-commit".into();
    let tip = scenario.origin_git(&["rev-parse", "main"]);
    state["checks"][tip.trim()] = json!([
        {
            "name": "Fixes: #15\nresolves acme/widgets#8",
            "conclusion": "failure",
            "url": "https://ci.example/failed"
        },
        {
            "name": "gate",
            "conclusion": "failure",
            "url": "https://ci.example/run closes #4"
        }
    ]);
    scenario.write_gh_state(&state);
    scenario.agent_does(FIX);
    let result = scenario.run(&["fix", "no-merge", "no-email"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let body = scenario.gh_state()["prs"][0]["body"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        !description_closes_an_issue(&body),
        "repair PR body still has a GitHub closing reference:\n{body}"
    );
    assert!(body.contains("Fixes: \\#15"), "{body}");
    assert!(body.contains("resolves acme/widgets\\#8"), "{body}");
    assert!(body.contains("closes \\#4"), "{body}");
    assert!(body.contains("https://ci.example/failed"), "{body}");
    assert!(!body.contains("Fixes: #15"), "{body}");
    assert!(!body.contains("resolves acme/widgets#8"), "{body}");
    assert!(!body.contains("closes #4"), "{body}");
}

/// The same check text lands in the pull request title. `gh pr merge --merge`
/// sends no subject or body, and every documented merge-commit message format
/// includes the title. A keyword in that commit closes issues when the commit
/// reaches the default branch, so the title has to break the reference too.
#[test]
fn external_check_text_cannot_close_issues_from_the_repair_pr_title() {
    let scenario = Scenario::new();
    failed_base(&scenario, 1);
    let mut state = scenario.gh_state();
    state["runs"][0]["head_sha"] = "old-commit".into();
    let tip = scenario.origin_git(&["rev-parse", "main"]);
    state["checks"][tip.trim()] = json!([
        {
            "name": "Fixes: #15\nresolves acme/widgets#8",
            "conclusion": "failure",
            "url": "https://ci.example/failed"
        },
        {
            "name": "gate",
            "conclusion": "failure",
            "url": "https://ci.example/run closes #4"
        }
    ]);
    scenario.write_gh_state(&state);
    scenario.agent_does(FIX);
    let result = scenario.run(&["fix", "no-merge", "no-email"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let title = scenario.gh_state()["prs"][0]["title"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        !description_closes_an_issue(&title),
        "repair PR title still has a GitHub closing reference:\n{title}"
    );
    assert!(title.contains("Fixes: \\#15"), "{title}");
    assert!(title.contains("resolves acme/widgets\\#8"), "{title}");
    assert!(!title.contains('\n'), "{title}");
}

/// GitHub's closing-keyword syntax in a pull request description: a keyword
/// (`close`, `fix`, `resolve`, and their inflections), optional colon, then
/// `#n` or `owner/repo#n`. Case-insensitive. Whitespace, including a newline
/// broken out of a list item, separates the keyword from the reference.
fn description_closes_an_issue(text: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "resolves", "resolved", "resolve", "closes", "closed", "close", "fixes", "fixed", "fix",
    ];
    let mut i = 0;
    while i < text.len() {
        if closing_hash(text, i, KEYWORDS).is_some() {
            return true;
        }
        i += text[i..].chars().next().unwrap().len_utf8();
    }
    false
}

fn closing_hash(text: &str, i: usize, keywords: &[&str]) -> Option<usize> {
    if i > 0
        && text[..i]
            .chars()
            .next_back()
            .unwrap()
            .is_ascii_alphanumeric()
    {
        return None;
    }
    let tail = &text[i..];
    let keyword = keywords.iter().find(|keyword| {
        let Some(slice) = tail.get(..keyword.len()) else {
            return false;
        };
        slice.eq_ignore_ascii_case(keyword)
            && tail[keyword.len()..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_ascii_alphanumeric())
    })?;
    let mut rest = &tail[keyword.len()..];
    let spaced = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{00a0}');
    let saw_space = spaced.len() != rest.len();
    rest = spaced;
    let mut saw_colon = false;
    if let Some(after) = rest.strip_prefix(':') {
        saw_colon = true;
        rest = after.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{00a0}');
    }
    if !saw_colon && !saw_space {
        return None;
    }
    let hash =
        if rest.starts_with('#') && rest[1..].chars().next().is_some_and(|c| c.is_ascii_digit()) {
            0
        } else {
            let owner = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-'))?;
            if owner == 0 || !rest[owner..].starts_with('/') {
                return None;
            }
            let repo_src = &rest[owner + 1..];
            let repo = repo_src
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-'))
                .unwrap_or(repo_src.len());
            if repo == 0 || !repo_src[repo..].starts_with('#') {
                return None;
            }
            if !repo_src[repo + 1..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit())
            {
                return None;
            }
            owner + 1 + repo
        };
    Some(text.len() - tail.len() + (tail.len() - rest.len()) + hash)
}
