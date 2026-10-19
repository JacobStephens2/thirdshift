mod support;
use std::os::unix::fs::PermissionsExt;
use support::Scenario;

#[test]
fn merging_before_the_push_comparison_must_not_recreate_the_branch() {
    let scenario = Scenario::new();
    let wrapper = scenario.path("bin/git");
    std::fs::write(
        &wrapper,
        r#"#!/bin/bash
if [[ "$1" == ls-remote && "$*" == *refs/heads/issue-7* && -e "$FAKE_CLAUDE_SCRIPT.armed" ]]; then
  unlink "$FAKE_CLAUDE_SCRIPT.armed"
  gh fake pr issue-7 state '"MERGED"'
  /usr/bin/git push -q origin --delete issue-7
fi
exec /usr/bin/git "$@"
"#,
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    scenario.agent_does(
        r#"
echo feature > feature.txt
git add feature.txt
git commit -q -m 'Add feature'
git push -q origin issue-7
gh pr create --base main --head issue-7 --title Feature --body 'Closes #7'
touch "$FAKE_CLAUDE_SCRIPT.armed"
"#,
    );
    let result = scenario.run(&["merge", &scenario.issue_url(7)]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result.stderr.contains("is merged\n"),
        "{}",
        result.stderr
    );
    assert!(
        scenario.origin_log("issue-7").is_none(),
        "obsolete push recreated the merged PR's deleted branch: {}",
        result.stderr
    );
}

#[test]
fn merging_at_publication_does_not_recreate_the_branch_even_during_failure_preservation() {
    for failed_session in [false, true] {
        let scenario = Scenario::new();
        let wrapper = scenario.path("bin/git");
        std::fs::write(
            &wrapper,
            r#"#!/bin/bash
if [[ "$1" == push && "$2" == --no-verify && -e "$FAKE_CLAUDE_SCRIPT.armed" ]]; then
  unlink "$FAKE_CLAUDE_SCRIPT.armed"
  gh fake pr issue-7 state '"MERGED"'
  /usr/bin/git push -q origin --delete issue-7
fi
exec /usr/bin/git "$@"
"#,
        )
        .unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let finish = if failed_session {
            "exit 17"
        } else {
            "git add unpublished.txt\ngit commit -q -m 'Unpublished work'"
        };
        scenario.agent_does(&format!(
            r#"
echo feature > feature.txt
git add feature.txt
git commit -q -m 'Add feature'
git push -q origin issue-7
gh pr create --base main --head issue-7 --title Feature --body 'Closes #7'
echo unpublished > unpublished.txt
touch "$FAKE_CLAUDE_SCRIPT.armed"
{finish}
"#
        ));
        let result = scenario.run(&["merge", &scenario.issue_url(7)]);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(
            result.stderr.contains("is merged, not open"),
            "{}",
            result.stderr
        );
        assert!(
            scenario.origin_log("issue-7").is_none(),
            "{}",
            result.stderr
        );
        let calls = scenario.claude_calls();
        let cwd = std::path::Path::new(calls[0]["cwd"].as_str().unwrap());
        assert_eq!(
            std::fs::read_to_string(cwd.join("unpublished.txt")).unwrap(),
            "unpublished\n"
        );
        if failed_session {
            assert!(
                result.stderr.contains("claude exited 17"),
                "{}",
                result.stderr
            );
        }
        assert_eq!(scenario.gh_state()["prs"][0]["body"], "Closes #7");
    }
}

#[test]
fn conditional_publication_never_overwrites_divergent_history() {
    let scenario = Scenario::new();
    scenario.agent_does(
        r#"
echo feature > feature.txt
git add feature.txt
git commit -q -m 'Add feature'
git push -q origin issue-7
gh pr create --base main --head issue-7 --title Feature --body 'Closes #7'
echo unpublished > unpublished.txt
git add unpublished.txt
git commit -q -m 'Unpublished work'
other=$(mktemp -d)
git clone -q --branch issue-7 "$(git remote get-url origin)" "$other"
git -C "$other" config user.name Other
git -C "$other" config user.email other@example.test
echo foreign > "$other/foreign.txt"
git -C "$other" add foreign.txt
git -C "$other" commit -q -m 'Foreign commit'
git -C "$other" push -q origin issue-7
rm -r "$other"
"#,
    );
    let result = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("non-fast-forward"),
        "{}",
        result.stderr
    );
    assert_eq!(
        scenario.origin_file("issue-7", "foreign.txt"),
        Some("foreign\n".to_owned())
    );
    assert_eq!(scenario.origin_file("issue-7", "unpublished.txt"), None);
    let calls = scenario.claude_calls();
    let cwd = std::path::Path::new(calls[0]["cwd"].as_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(cwd.join("unpublished.txt")).unwrap(),
        "unpublished\n"
    );
}
