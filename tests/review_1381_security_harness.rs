//! A Step's Harness must not move the Security review of a run it dispatches.
//!
//! ADR-0021: `security.harness` wins over the command's Harness, Model and
//! Effort. Resolving the review against the default choice, then replacing
//! that choice with the Step's, drops the review onto the Step when the two
//! defaults matched.

mod support;

use std::fs;

use support::Scenario;

const PUBLISH_ON_CODEX: &str = r#"
version = 1
description = "Publish, then run on Codex"

[[steps]]
name = "publish"
skill = "to-spec"

[[steps]]
name = "run"
catalogue = "run-published-issue"
harness = "codex"
model = "gpt-6.1-sol"
effort = "high"
"#;

#[test]
fn a_dispatched_run_keeps_its_security_review_on_the_security_harness() {
    let scenario = Scenario::new();
    // The review Harness is the same choice as the default, so a resolution
    // that only keeps a difference from the default stores nothing. The Step
    // then runs the dispatched Ticket on Codex.
    scenario.user_config_is(
        r#"
[security]
harness = "claude"
review = true
"#,
    );
    let dir = scenario.path("home/.thirdshift/processes");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("publish.toml"), PUBLISH_ON_CODEX).unwrap();
    scenario.agent_does_in_session(
        1,
        "\
url=$(gh issue create --title Ticket --body 'Do the thing')
printf '%s\\n' \"$url\" > \"$FAKE_CLAUDE_FINAL_MESSAGE\"
",
    );
    scenario.agent_does_for(8, &review_or_land());

    let result = scenario.run(&["process", "publish"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result.stderr.contains("security-review: on claude")
            && result.stderr.contains("chosen by security.harness"),
        "the Security review left security.harness: {}",
        result.stderr
    );
    assert!(
        !result.stderr.contains("security-review: on codex"),
        "the Step's Harness ran the Security review: {}",
        result.stderr
    );
    let review_on_codex = scenario.codex_calls().iter().any(|call| {
        call["prompt"]
            .as_str()
            .is_some_and(|prompt| prompt.contains("guidance mode"))
    });
    assert!(
        !review_on_codex,
        "codex ran the Security review: {:?}",
        scenario.codex_calls()
    );
}

#[test]
fn a_command_harness_does_not_move_the_dispatched_security_review() {
    let scenario = Scenario::new();
    scenario.user_config_is(
        r#"
[security]
harness = "claude"
review = true
"#,
    );
    let dir = scenario.path("home/.thirdshift/processes");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("publish.toml"),
        r#"
version = 1
description = "Publish, then run"

[[steps]]
name = "publish"
skill = "to-spec"

[[steps]]
name = "run"
catalogue = "run-published-issue"
"#,
    )
    .unwrap();
    scenario.agent_does_in_session(
        1,
        "\
url=$(gh issue create --title Ticket --body 'Do the thing')
printf '%s\\n' \"$url\" > \"$FAKE_CLAUDE_FINAL_MESSAGE\"
",
    );
    scenario.agent_does_for(8, &review_or_land());

    let result = scenario.run(&[
        "process",
        "publish",
        "harness",
        "codex",
        "model",
        "gpt-6.1-sol",
        "effort",
        "high",
    ]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result.stderr.contains("security-review: on claude")
            && result.stderr.contains("chosen by security.harness"),
        "the command's Harness ran the Security review: {}",
        result.stderr
    );
}

#[test]
fn a_step_model_does_not_replace_the_security_harness_model() {
    let scenario = Scenario::new();
    scenario.user_config_is(
        r#"
[harness.claude]
model = "claude-opus-5-5"
effort = "high"

[security]
harness = "claude"
review = true
"#,
    );
    let dir = scenario.path("home/.thirdshift/processes");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("publish.toml"),
        r#"
version = 1
description = "Publish, then run on another Model"

[[steps]]
name = "publish"
skill = "to-spec"

[[steps]]
name = "run"
catalogue = "run-published-issue"
model = "claude-haiku-4-5"
effort = "low"
"#,
    )
    .unwrap();
    // Model checks are text calls and do not set the prompt variable. The
    // publishing session is the one that names its Step.
    scenario.agent_does(
        r#"
if [ -z "${FAKE_CLAUDE_PROMPT:-}" ]; then
  exit 0
fi
if printf '%s' "$FAKE_CLAUDE_PROMPT" | grep -q 'guidance mode'; then
  printf '%s' "$FAKE_CLAUDE_PROMPT" > review-prompt.txt
  report=$(sed -n 's/^Private report file: `\(.*\)`\.$/\1/p' review-prompt.txt)
  test -n "$report"
  printf '%s\n' '[]' > "$report"
  printf '%s\n' 'Security review: {"unaddressed_count":0,"findings":[],"pre_existing_count":0}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
elif printf '%s' "$FAKE_CLAUDE_PROMPT" | grep -q 'This is Step publish'; then
  url=$(gh issue create --title Ticket --body 'Do the thing')
  printf '%s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
else
  echo 8 > issue-8.txt
  git add issue-8.txt
  git commit -q -m 'Ticket 8'
  gh pr create --base main --head issue-8 --title 'Ticket 8' --body 'Closes #8'
fi
"#,
    );

    let result = scenario.run(&["process", "publish"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let review = scenario
        .claude_calls()
        .into_iter()
        .find(|call| {
            call["prompt"]
                .as_str()
                .is_some_and(|prompt| prompt.contains("guidance mode"))
        })
        .expect("no Security review session");
    let args: Vec<_> = review["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap())
        .collect();
    let value = |flag: &str| {
        args.iter()
            .position(|arg| *arg == flag)
            .map(|at| args[at + 1])
    };
    assert_eq!(value("--model"), Some("claude-opus-5-5"), "{args:?}");
    assert_eq!(value("--effort"), Some("high"), "{args:?}");
}

fn review_or_land() -> String {
    r#"
if printf '%s' "$FAKE_CLAUDE_PROMPT" | grep -q 'guidance mode'; then
  printf '%s' "$FAKE_CLAUDE_PROMPT" > review-prompt.txt
  report=$(sed -n 's/^Private report file: `\(.*\)`\.$/\1/p' review-prompt.txt)
  test -n "$report"
  printf '%s\n' '[]' > "$report"
  printf '%s\n' 'Security review: {"unaddressed_count":0,"findings":[],"pre_existing_count":0}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
else
  echo 8 > issue-8.txt
  git add issue-8.txt
  git commit -q -m 'Ticket 8'
  gh pr create --base main --head issue-8 --title 'Ticket 8' --body 'Closes #8'
fi
"#
    .to_string()
}
