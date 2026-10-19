use super::*;
use crate::interrupt;
use crate::test_support::with_recorded_signal;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// PATH changes happen only in the isolated process. Cleanup uses the
/// executable's recorded PID, including after a failed assertion.
struct Fixture {
    temp: tempfile::TempDir,
}

impl Fixture {
    fn new(script: &str) -> Self {
        let fixture = Self {
            temp: tempfile::tempdir().unwrap(),
        };
        fixture.script(script);
        let path = format!(
            "{}:{}",
            fixture.temp.path().display(),
            std::env::var("PATH").unwrap()
        );
        // SAFETY: this isolated process runs one test; no environment readers
        // or execution workers have started.
        unsafe { std::env::set_var("PATH", path) };
        fixture
    }

    fn path(&self, name: &str) -> PathBuf {
        self.temp.path().join(name)
    }

    fn script(&self, script: &str) {
        let gh = self.path("gh");
        crate::test_support::write_executable(
            &gh,
            &format!(
                "#!/bin/sh\necho $$ > '{}'\n{script}\n",
                self.path("pid").display()
            ),
        );
    }

    /// Script consecutive external failures followed by a successful response.
    fn responses(&self, failures: &[&str], stdout: &str) {
        let _ = fs::remove_file(self.path("calls"));
        let mut script = format!(
            "echo call >> '{calls}'\nattempt=$(wc -l < '{calls}')\ncase $attempt in\n",
            calls = self.path("calls").display()
        );
        for (index, stderr) in failures.iter().enumerate() {
            script.push_str(&format!(
                "{}) cat >&2 <<'ERROR'\n{stderr}\nERROR\nexit 1 ;;\n",
                index + 1
            ));
        }
        script.push_str(&format!("esac\ncat <<'JSON'\n{stdout}\nJSON"));
        self.script(&script);
    }

    fn calls(&self) -> usize {
        fs::read_to_string(self.path("calls"))
            .unwrap()
            .lines()
            .count()
    }

    fn pid(&self) -> Option<libc::pid_t> {
        fs::read_to_string(self.path("pid"))
            .ok()?
            .trim()
            .parse()
            .ok()
    }
}

fn exists(pid: libc::pid_t) -> bool {
    // SAFETY: signal 0 only probes the fixture's recorded PID.
    unsafe { libc::kill(pid, 0) == 0 }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(pid) = self.pid()
            && exists(pid)
        {
            // SAFETY: this PID belongs to the fixture executable.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
}

fn issue() -> IssueUrl {
    IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap()
}

#[test]
fn a_transient_issue_read_recovers() {
    with_recorded_signal(
        "github::execution_tests::a_transient_issue_read_recovers",
        |_| {
            let fixture = Fixture::new("");
            fixture.script(&format!(
                r#"
if test ! -f '{attempt}'; then
  touch '{attempt}'
  echo 'HTTP 504: Gateway Timeout (https://api.github.com/graphql)' >&2
  exit 1
fi
printf '%s' '{{"title":"Recovered"}}'
"#,
                attempt = fixture.path("attempt").display()
            ));
            assert_eq!(GitHub::new().issue_title(&issue()).unwrap(), "Recovered");
        },
    );
}

#[test]
fn transient_reads_exhaust_three_attempts_with_the_last_cause() {
    with_recorded_signal(
        "github::execution_tests::transient_reads_exhaust_three_attempts_with_the_last_cause",
        |_| {
            let fixture = Fixture::new("");
            fixture.responses(
                &["HTTP 504: first", "HTTP 504: second", "HTTP 504: last"],
                r#"{"title":"Must not reach a fourth attempt"}"#,
            );
            let started = Instant::now();
            let error = GitHub::new().issue_title(&issue()).unwrap_err();
            assert_eq!(error.to_string(), "gh issue view failed: HTTP 504: last");
            assert_eq!(fixture.calls(), 3);
            assert!(started.elapsed() >= Duration::from_secs(3));
            assert!(started.elapsed() < Duration::from_secs(15));
        },
    );
}

#[test]
fn permanent_and_quota_errors_fail_on_the_first_read() {
    with_recorded_signal(
        "github::execution_tests::permanent_and_quota_errors_fail_on_the_first_read",
        |_| {
            let fixture = Fixture::new("");
            for stderr in [
                "HTTP 401: Bad credentials",
                "HTTP 404: Not Found",
                "HTTP 422: Validation failed: unexpected EOF",
                "GraphQL: API rate limit already exceeded for user ID 123.",
                "GraphQL: API rate limit exceeded (HTTP 429)",
                "GraphQL: RATE_LIMIT_EXCEEDED (HTTP 429)",
                "To get started with GitHub CLI, please run: gh auth login",
                "unsupported response",
            ] {
                fixture.responses(&[stderr], r#"{"title":"Must not retry"}"#);
                let error = GitHub::new().issue_title(&issue()).unwrap_err();
                assert_eq!(error.to_string(), format!("gh issue view failed: {stderr}"));
                assert_eq!(fixture.calls(), 1, "{stderr}");
            }
        },
    );
}

#[test]
fn issue_pr_and_label_lists_recover_from_transient_failures() {
    with_recorded_signal(
        "github::execution_tests::issue_pr_and_label_lists_recover_from_transient_failures",
        |_| {
            let fixture = Fixture::new("");
            let github = GitHub::new();
            fixture.responses(&["HTTP 429: secondary rate limit"], "[]");
            assert!(
                github
                    .open_issues_labelled("acme/widgets", crate::labels::READY_FOR_AGENT)
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(fixture.calls(), 2);

            fixture.responses(&["read tcp: connection reset by peer"], "[]");
            assert!(
                github
                    .pull_requests_with_head_prefix(&issue(), "issue-")
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(fixture.calls(), 2);

            fixture.responses(&["EOF"], "[]");
            github.ensure_labels("acme/widgets", &[]).unwrap();
            assert_eq!(fixture.calls(), 2);

            fixture.responses(
                &["HTTP 503: unavailable", "HTTP 502: Bad Gateway"],
                r#"{"body":"Recovered body"}"#,
            );
            assert_eq!(github.pr_body(&issue(), 12).unwrap(), "Recovered body");
            assert_eq!(fixture.calls(), 3);
        },
    );
}

#[test]
fn transient_mutation_failures_never_repeat_the_request() {
    with_recorded_signal(
        "github::execution_tests::transient_mutation_failures_never_repeat_the_request",
        |_| {
            let fixture = Fixture::new("");
            let github = GitHub::new();
            for mutation in [
                "labels", "comment", "create", "body", "ready", "merge", "rerun",
            ] {
                fixture.responses(&["HTTP 504: response lost after mutation"], "{}");
                let result = match mutation {
                    "labels" => github.remove_label(&issue(), "in-progress"),
                    "comment" => github.close_issue(&issue(), "Finished"),
                    "create" => github
                        .create_draft_pr(&issue(), "issue-7", "main", "Work", "Closes #7")
                        .map(drop),
                    "body" => github.set_pr_body(&issue(), 12, "Updated"),
                    "ready" => github.mark_ready(&issue(), "12"),
                    "merge" => github.merge(&issue(), 12, "abc"),
                    "rerun" => github.rerun_failed_jobs(&issue(), 123),
                    _ => unreachable!(),
                };
                assert!(
                    result.unwrap_err().to_string().contains("HTTP 504"),
                    "{mutation}"
                );
                assert_eq!(fixture.calls(), 1, "{mutation}");
            }
        },
    );
}

#[test]
fn interruption_cancels_the_read_retry_wait() {
    with_recorded_signal(
        "github::execution_tests::interruption_cancels_the_read_retry_wait",
        |signal| {
            let fixture = Fixture::new("");
            fixture.responses(
                &["HTTP 504: Gateway Timeout"],
                r#"{"title":"Must not retry"}"#,
            );
            let calls = fixture.path("calls");
            let sender = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !calls.exists() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                std::thread::sleep(Duration::from_millis(200));
                signal_hook::low_level::raise(signal).unwrap();
            });
            let started = Instant::now();
            let result = GitHub::new().issue_title(&issue());
            sender.join().unwrap();
            assert_eq!(result.unwrap_err().to_string(), "interrupted");
            assert!(started.elapsed() < Duration::from_secs(1));
            assert_eq!(fixture.calls(), 1);
            assert!(interrupt::requested());
        },
    );
}

#[test]
fn completion_read_recovery_preserves_recorded_interruption() {
    with_recorded_signal(
        "github::execution_tests::completion_read_recovery_preserves_recorded_interruption",
        |signal| {
            let fixture = Fixture::new("");
            fixture.responses(
                &["HTTP 504: Gateway Timeout"],
                r#"{"title":"Recovered completion"}"#,
            );
            let github = GitHub::new();
            signal_hook::low_level::raise(signal).unwrap();
            assert_eq!(
                github.completion().issue_title(&issue()).unwrap(),
                "Recovered completion"
            );
            assert_eq!(fixture.calls(), 2);
            assert!(interrupt::requested());
            assert_eq!(
                github.issue_title(&issue()).unwrap_err().to_string(),
                "interrupted"
            );
            assert_eq!(fixture.calls(), 2);
        },
    );
}

#[test]
fn parent_only_interruption_stops_active_github() {
    with_recorded_signal(
        "github::execution_tests::parent_only_interruption_stops_active_github",
        |signal| {
            let fixture = Fixture::new(&format!(
                "kill -{signal} {}\nexec sleep 3",
                std::process::id()
            ));
            let started = Instant::now();
            assert_eq!(
                GitHub::new().issue_title(&issue()).unwrap_err().to_string(),
                "interrupted"
            );
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "GitHub waited for natural expiry"
            );
            assert!(
                !exists(fixture.pid().expect("the executable never started")),
                "GitHub left its executable alive"
            );
        },
    );
}

#[test]
fn ordinary_github_refuses_reads_and_writes_before_spawn() {
    with_recorded_signal(
        "github::execution_tests::ordinary_github_refuses_reads_and_writes_before_spawn",
        |signal| {
            let fixture = Fixture::new("echo 'no pull requests found' >&2; exit 1");
            let github = GitHub::new();
            assert!(github.pr_snapshot(&issue(), "issue-7").unwrap().is_none());
            fs::remove_file(fixture.path("pid")).unwrap();
            signal_hook::low_level::raise(signal).unwrap();
            assert_eq!(
                github.issue_title(&issue()).unwrap_err().to_string(),
                "interrupted"
            );
            assert_eq!(
                github
                    .mark_ready(&issue(), "issue-7")
                    .unwrap_err()
                    .to_string(),
                "interrupted"
            );
            assert_eq!(
                github.checks_on(&issue(), "abc").err().unwrap().to_string(),
                "interrupted"
            );
            assert!(github.pr_snapshot(&issue(), "issue-7").is_err());
            assert!(
                !fixture.path("pid").exists(),
                "an ordinary operation spawned after interruption"
            );
        },
    );
}

fn finish_issue_creation(name: &str, already_interrupted: bool) {
    with_recorded_signal(name, |signal| {
        let github = GitHub::new();
        let completion = github.completion();
        let fixture = Fixture::new("");
        fixture.script(&format!(
            r#"
case "$1 $2" in
  'label list')
    kill -{signal} {parent}
    sleep 0.1
    printf '[]'
    ;;
  'label create') touch '{label}' ;;
  'issue create') printf '  https://github.com/acme/widgets/issues/8\n\n' ;;
  *) exit 1 ;;
esac
"#,
            parent = std::process::id(),
            label = fixture.path("label-created").display()
        ));
        if already_interrupted {
            signal_hook::low_level::raise(signal).unwrap();
        }
        let created = completion
            .create_issue(
                &issue(),
                "Finish",
                "Work",
                &[crate::labels::READY_FOR_AGENT],
            )
            .unwrap();
        assert_eq!(created.url, "https://github.com/acme/widgets/issues/8");
        assert!(fixture.path("label-created").exists());
        assert!(interrupt::requested());
        assert_eq!(
            github.issue_title(&issue()).unwrap_err().to_string(),
            "interrupted"
        );
        assert_eq!(
            GitHub::new().issue_title(&issue()).unwrap_err().to_string(),
            "interrupted"
        );
    });
}

#[test]
fn completion_finishes_nested_label_reads_and_writes_without_permitting_ordinary_views() {
    finish_issue_creation(
        "github::execution_tests::completion_finishes_nested_label_reads_and_writes_without_permitting_ordinary_views",
        true,
    );
}

#[test]
fn completion_finishes_with_a_newly_arriving_interrupt() {
    finish_issue_creation(
        "github::execution_tests::completion_finishes_with_a_newly_arriving_interrupt",
        false,
    );
}

#[test]
fn an_optional_pr_probe_propagates_active_cancellation_instead_of_absence() {
    with_recorded_signal(
        "github::execution_tests::an_optional_pr_probe_propagates_active_cancellation_instead_of_absence",
        |signal| {
            let fixture = Fixture::new(&format!(
                "echo 'no pull requests found' >&2\nkill -{signal} {}\nexec sleep 3",
                std::process::id()
            ));
            let error = GitHub::new()
                .pr_snapshot(&issue(), "issue-7")
                .expect_err("cancellation became absence");
            assert_eq!(error.to_string(), "interrupted");
            assert!(!exists(fixture.pid().unwrap()));
        },
    );
}

#[test]
fn cancelled_numbered_merge_still_allows_an_independent_completion_snapshot() {
    with_recorded_signal(
        "github::execution_tests::cancelled_numbered_merge_still_allows_an_independent_completion_snapshot",
        |signal| {
            let fixture = Fixture::new(&format!(
                r#"
case "$1 $2 $3" in
  'pr merge 12') kill -{signal} {parent}; exec sleep 3 ;;
  'pr view 12') printf '%s' '{{"number":12,"url":"https://github.com/acme/widgets/pull/12","state":"MERGED","headRefName":"issue-7","baseRefName":"main","isDraft":false,"mergeable":"UNKNOWN","headRefOid":"requested-head","isCrossRepository":false}}' ;;
  *) exit 1 ;;
esac
"#,
                parent = std::process::id()
            ));
            let github = GitHub::new();
            assert_eq!(
                github
                    .merge(&issue(), 12, "requested-head")
                    .unwrap_err()
                    .to_string(),
                "interrupted"
            );
            let snapshot = github
                .completion()
                .pr_snapshot(&issue(), "12")
                .unwrap()
                .unwrap();
            assert_eq!(snapshot.pr.number, 12);
            assert_eq!(snapshot.pr.state, PrState::Merged);
            assert_eq!(snapshot.pr.head, "issue-7");
            assert_eq!(snapshot.pr.base, "main");
            assert_eq!(snapshot.head_commit, "requested-head");
            assert_eq!(snapshot.mergeable, Mergeable::Unknown);
            assert!(!snapshot.from_fork);
            assert!(interrupt::requested());
            assert!(github.pr_snapshot(&issue(), "12").is_err());
            assert!(github.mark_ready(&issue(), "12").is_err());
            assert!(!exists(fixture.pid().unwrap()));
        },
    );
}

#[test]
fn numbered_pr_transitions_use_the_repository_and_guarded_merge_flags() {
    with_recorded_signal(
        "github::execution_tests::numbered_pr_transitions_use_the_repository_and_guarded_merge_flags",
        |_| {
            let _fixture = Fixture::new(
                r#"
case "$*" in
  'pr ready 12 --repo acme/widgets' | 'pr ready 12 --undo --repo acme/widgets' | 'pr merge 12 --repo acme/widgets --merge --match-head-commit abc') ;;
  *) echo "unexpected args: $*" >&2; exit 1 ;;
esac
"#,
            );
            let github = GitHub::new();
            github.mark_ready(&issue(), "12").unwrap();
            github
                .completion()
                .convert_to_draft(&issue(), "12")
                .unwrap();
            github.merge(&issue(), 12, "abc").unwrap();
        },
    );
}

#[test]
fn github_preserves_trimmed_text_json_and_paginated_items() {
    with_recorded_signal(
        "github::execution_tests::github_preserves_trimmed_text_json_and_paginated_items",
        |_| {
            let _fixture = Fixture::new(
                r#"
case "$1 $2" in
  'issue view') printf ' {"title":"Widgets"}\n' ;;
  'pr create') printf '  https://github.com/acme/widgets/pull/1\n\n' ;;
  'api --paginate')
    if test "$4" = '.check_runs[]'; then
      printf '%s\n' '{"name":"build","status":"completed","conclusion":"success"}' '{"name":"lint","status":"completed","conclusion":"failure"}'
    else
      printf '%s\n' '{"context":"external","state":"pending"}'
    fi ;;
  *) exit 1 ;;
esac
"#,
            );
            let github = GitHub::new();
            assert_eq!(github.issue_title(&issue()).unwrap(), "Widgets");
            assert_eq!(
                github
                    .create_draft_pr(&issue(), "issue-7", "main", "Widgets", "Work")
                    .unwrap(),
                "https://github.com/acme/widgets/pull/1"
            );
            let checks = github.checks_on(&issue(), "abc").unwrap();
            let answers: Vec<_> = checks
                .iter()
                .map(|check| (check.name.as_str(), check.state))
                .collect();
            assert_eq!(
                answers,
                [
                    ("build", CheckState::Passed),
                    ("lint", CheckState::Failed),
                    ("external", CheckState::Pending)
                ]
            );
        },
    );
}

#[test]
fn github_keeps_its_exit_and_parse_diagnostics() {
    with_recorded_signal(
        "github::execution_tests::github_keeps_its_exit_and_parse_diagnostics",
        |_| {
            let fixture = Fixture::new("printf ' refused \\n' >&2; exit 1");
            let github = GitHub::new();
            assert_eq!(
                github.issue_title(&issue()).unwrap_err().to_string(),
                "gh issue view failed: refused"
            );
            assert_eq!(
                github
                    .mark_ready(&issue(), "issue-7")
                    .unwrap_err()
                    .to_string(),
                "gh pr ready issue-7 --repo acme/widgets failed: refused"
            );
            assert_eq!(
                github.checks_on(&issue(), "abc").err().unwrap().to_string(),
                "gh api repos/acme/widgets/commits/abc/check-runs?per_page=100 failed: refused"
            );
            fixture.script("printf 'invalid'");
            assert_eq!(
                github.issue_title(&issue()).unwrap_err().to_string(),
                "gh issue view returned invalid JSON"
            );
            assert_eq!(
                github.checks_on(&issue(), "abc").err().unwrap().to_string(),
                "gh api repos/acme/widgets/commits/abc/check-runs?per_page=100 returned invalid JSON"
            );
        },
    );
}

#[test]
fn spec_creation_observes_the_returned_number_with_completion_after_interruption() {
    with_recorded_signal(
        "github::execution_tests::spec_creation_observes_the_returned_number_with_completion_after_interruption",
        |signal| {
            let fixture = Fixture::new("");
            fixture.script(&format!(
                r#"
printf '%s\n' "$1 $2 $3" >> '{calls}'
case "$1 $2 $3" in
  'pr view issue-7')
    if test -f '{discovered}'; then
      printf '%s' '{{"number":13,"url":"https://github.com/acme/widgets/pull/13","state":"OPEN","headRefName":"issue-7","baseRefName":"main","isDraft":true,"mergeable":"MERGEABLE","headRefOid":"head","isCrossRepository":false}}'
    else
      touch '{discovered}'
      echo 'no pull requests found' >&2
      exit 1
    fi ;;
  'issue view 7') printf '%s' '{{"title":"Widgets"}}' ;;
  'pr create --repo') printf '  https://github.com/acme/widgets/pull/12\n\n' ;;
  'pr view 12') printf '%s' '{{"number":12,"url":"https://github.com/acme/widgets/pull/12","state":"OPEN","headRefName":"issue-7","baseRefName":"main","isDraft":true,"mergeable":"MERGEABLE","headRefOid":"head","isCrossRepository":false}}' ;;
  *) exit 1 ;;
esac
"#,
                calls = fixture.path("calls").display(),
                discovered = fixture.path("discovered").display(),
            ));
            let mut pr =
                crate::pull_request::PullRequest::new(&issue(), "issue-7", "main", GitHub::new());
            pr.resume_spec().unwrap();
            signal_hook::low_level::raise(signal).unwrap();

            assert_eq!(
                pr.land_spec("- [x] #8 landed\n").unwrap(),
                "https://github.com/acme/widgets/pull/12"
            );
            assert_eq!(
                fs::read_to_string(fixture.path("calls")).unwrap(),
                "pr view issue-7\nissue view 7\npr create --repo\npr view 12\n"
            );
            assert!(interrupt::requested());
            pr.begin_delivery();
            assert_eq!(pr.mark_ready(None).unwrap_err().to_string(), "interrupted");
        },
    );
}

#[test]
fn tickets_keep_the_urls_of_sub_issues_and_their_open_and_closed_blockers() {
    with_recorded_signal(
        "github::execution_tests::tickets_keep_the_urls_of_sub_issues_and_their_open_and_closed_blockers",
        |_| {
            let _fixture = Fixture::new(
                r#"
case "$*" in
  *'nodes { url state }'*)
    cat <<'JSON'
{"data":{"repository":{"issue":{"subIssues":{"nodes":[{"url":"https://github.com/other/widgets/issues/21","state":"OPEN","labels":{"nodes":[]},"subIssues":{"totalCount":0},"blockedBy":{"nodes":[{"url":"https://github.com/acme/widgets/issues/21","state":"OPEN"},{"url":"https://github.com/another/widgets/issues/21","state":"CLOSED"}]}}]}}}}}
JSON
    ;;
  *) echo '{}' ;;
esac
"#,
            );
            let tickets = GitHub::new().tickets(&issue()).unwrap();
            assert_eq!(tickets.len(), 1);
            assert_eq!(
                tickets[0].issue.url,
                "https://github.com/other/widgets/issues/21"
            );
            assert_eq!(
                tickets[0]
                    .blockers
                    .iter()
                    .map(|issue| issue.url.as_str())
                    .collect::<Vec<_>>(),
                [
                    "https://github.com/acme/widgets/issues/21",
                    "https://github.com/another/widgets/issues/21"
                ]
            );
            assert_eq!(
                tickets[0]
                    .open_blockers
                    .iter()
                    .map(|issue| issue.url.as_str())
                    .collect::<Vec<_>>(),
                ["https://github.com/acme/widgets/issues/21"]
            );
        },
    );
}

#[test]
fn a_ticket_or_blocker_without_a_valid_url_fails_the_graph_read() {
    with_recorded_signal(
        "github::execution_tests::a_ticket_or_blocker_without_a_valid_url_fails_the_graph_read",
        |_| {
            let fixture = Fixture::new("");
            for (ticket_url, blocker_url) in [
                (
                    serde_json::Value::Null,
                    serde_json::json!("https://github.com/acme/widgets/issues/21"),
                ),
                (
                    serde_json::json!("https://github.com/acme/widgets/issues/22"),
                    serde_json::Value::Null,
                ),
                (
                    serde_json::json!("https://github.com/acme/widgets/issues/22"),
                    serde_json::json!("https://example.invalid/issues/21"),
                ),
            ] {
                let response = serde_json::json!({"data":{"repository":{"issue":{"subIssues":{"nodes":[{"url":ticket_url,"state":"OPEN","labels":{"nodes":[]},"subIssues":{"totalCount":0},"blockedBy":{"nodes":[{"url":blocker_url,"state":"CLOSED"}]}}]}}}}});
                fixture.script(&format!("cat <<'JSON'\n{response}\nJSON"));
                assert!(
                    GitHub::new().tickets(&issue()).is_err(),
                    "accepted {response}"
                );
            }
        },
    );
}

#[test]
fn check_reads_classify_transport_and_api_failures_without_hiding_diagnostics() {
    with_recorded_signal(
        "github::execution_tests::check_reads_classify_transport_and_api_failures_without_hiding_diagnostics",
        |_| {
            let fixture = Fixture::new("");
            for (stderr, transient) in [
                ("unexpected end of JSON input", true),
                ("Get https://api.github.com: unexpected EOF", true),
                ("Get \"https://api.github.com\": EOF", true),
                ("EOF", true),
                ("error connecting to api.github.com", true),
                ("dial tcp: lookup api.github.com: no such host", true),
                ("read tcp: connection reset by peer", true),
                ("net/http: TLS handshake timeout", true),
                ("gh: too many requests (HTTP 429)", true),
                (
                    "GraphQL: API rate limit already exceeded for user ID 123 (HTTP 429)",
                    false,
                ),
                ("gh: Bad Gateway (HTTP 502)", true),
                ("gh: service unavailable (HTTP 503)", true),
                ("gh: Bad credentials (HTTP 401)", false),
                ("gh: Resource not accessible (HTTP 403)", false),
                ("gh: Not Found (HTTP 404)", false),
                ("gh: Validation Failed (HTTP 422)", false),
                ("gh: Bad credentials (HTTP 401): unexpected EOF", false),
                ("gh: Bad credentials (HTTP 401): EOF", false),
                (
                    "To get started with GitHub CLI, please run: gh auth login",
                    false,
                ),
                ("unknown flag: --paginate", false),
                ("unsupported response format", false),
            ] {
                fixture.script(&format!("cat >&2 <<'ERROR'\n{stderr}\nERROR\nexit 1"));
                let error = GitHub::new().checks_on(&issue(), "abc").err().unwrap();
                assert_eq!(check_read_is_transient(&error), transient, "{error:#}");
                assert!(format!("{error:#}").contains(stderr));
            }
            for (stdout, transient) in [("{\"name\":", true), ("invalid", false), ("{}", false)] {
                fixture.script(&format!("cat <<'JSON'\n{stdout}\nJSON"));
                let error = GitHub::new().checks_on(&issue(), "abc").err().unwrap();
                assert_eq!(check_read_is_transient(&error), transient, "{error:#}");
            }
        },
    );
}

#[test]
fn cancellation_during_a_ci_check_read_starts_no_further_request() {
    with_recorded_signal(
        "github::execution_tests::cancellation_during_a_ci_check_read_starts_no_further_request",
        |signal| {
            let fixture = Fixture::new("");
            fixture.script(&format!(
                "echo read >> '{}'\nkill -{signal} {}\nexec sleep 3",
                fixture.path("reads").display(),
                std::process::id()
            ));
            let started = Instant::now();
            let error = crate::ci::watch(&issue(), "abc", None, &mut || Ok(()))
                .err()
                .unwrap();

            assert_eq!(error.to_string(), "interrupted");
            assert_eq!(fs::read_to_string(fixture.path("reads")).unwrap(), "read\n");
            assert!(started.elapsed() < Duration::from_secs(2));
            assert!(!exists(fixture.pid().unwrap()));
        },
    );
}
