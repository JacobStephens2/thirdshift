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
fn labelled_issue_reads_include_every_page_and_exclude_pull_requests() {
    with_recorded_signal(
        "github::execution_tests::labelled_issue_reads_include_every_page_and_exclude_pull_requests",
        |_| {
            let fixture = Fixture::new("");
            let label = Label::new("Architect plan & idea", "");
            let mut response = String::from(
                "{\"html_url\":\"https://github.com/acme/widgets/pull/42\",\"pull_request\":{}}\n",
            );
            for number in (1..=1001).rev() {
                response.push_str(
                    &serde_json::json!({
                        "html_url": format!("https://github.com/acme/widgets/issues/{number}"),
                        "title": format!("Plan {number}"),
                        "labels": [{"name": label.name()}],
                    })
                    .to_string(),
                );
                response.push('\n');
            }
            for state in ["open", "closed"] {
                fixture.script(&format!(
                    r#"
test "$*" = 'api --paginate --jq .[] repos/acme/widgets/issues?state={state}&labels=Architect+plan+%26+idea&sort=created&direction=desc&per_page=100' || exit 2
cat <<'ISSUES'
{response}ISSUES
"#,
                ));
                let github = GitHub::new();
                let listed = match state {
                    "open" => github.open_issues_labelled("acme/widgets", label),
                    "closed" => github.closed_issues_labelled("acme/widgets", label),
                    _ => unreachable!(),
                }
                .unwrap();
                assert_eq!(listed.len(), 1001, "{state}");
                assert_eq!(listed[0].issue.number, 1001);
                assert_eq!(listed[0].title, "Plan 1001");
                assert!(listed[0].labels.has(label));
                assert_eq!(listed[1000].issue.number, 1);
            }
        },
    );
}

#[test]
fn a_transient_private_security_issue_listing_recovers() {
    with_recorded_signal(
        "github::execution_tests::a_transient_private_security_issue_listing_recovers",
        |_| {
            let fixture = Fixture::new("");
            fixture.script(&format!(
                r#"
if [ "$2" = 'repos/acme/widgets' ]; then
  printf '%s' '{{"private":true}}'
  exit 0
fi
case "$*" in
  *security-advisories*)
    echo 'gh: Not Found (HTTP 404)' >&2
    exit 1
    ;;
esac
echo read >> '{reads}'
if [ ! -f '{failed}' ]; then
  touch '{failed}'
  echo 'HTTP 504: Gateway Timeout' >&2
  exit 1
fi
exit 0
"#,
                reads = fixture.path("reads").display(),
                failed = fixture.path("failed").display(),
            ));
            let result = GitHub::new().security_records("acme/widgets");
            assert!(
                result.is_ok(),
                "private issue listing did not recover: {:?}",
                result.err()
            );
            assert_eq!(
                fs::read_to_string(fixture.path("reads")).unwrap(),
                "read\nread\n"
            );
        },
    );
}

#[test]
fn private_record_reads_recover_without_repeating_the_following_write() {
    with_recorded_signal(
        "github::execution_tests::private_record_reads_recover_without_repeating_the_following_write",
        |_| {
            use crate::security::reproduction::{Outcome, Reproduction};
            let fixture = Fixture::new("");
            let value = serde_json::json!({
                "number": 42, "title": "Bound input", "state": "open",
                "html_url": "https://github.com/acme/widgets/issues/42",
                "labels": [{"name": "needs-triage"}],
                "body": "Fingerprint: `bounded`\nAudited commit: `0123456789abcdef0123456789abcdef01234567`\nPrivate evidence.\n",
            });
            let record = SecurityRecords::issues(Vec::new())
                .decode_created(value.clone())
                .unwrap();
            let reproduction = Reproduction {
                outcome: Outcome::NotReproduced,
                notes: "Private reproduction notes.".into(),
                test: "bounded_input()".into(),
            };
            for operation in ["link", "reproduction"] {
                fixture.script(&format!(
                    r#"
if [ "$2" = '--method' ]; then
  echo write >> '{writes}'
  cat > '{body}'
  echo 'HTTP 504: private mutation evidence' >&2
  exit 1
fi
echo read >> '{reads}'
if [ ! -f '{failed}' ]; then
  touch '{failed}'
  echo 'HTTP 504: private read evidence' >&2
  exit 1
fi
cat <<'JSON'
{value}
JSON
"#,
                    reads = fixture.path("reads").display(),
                    writes = fixture.path("writes").display(),
                    body = fixture.path("body").display(),
                    failed = fixture.path("failed").display(),
                ));
                let github = GitHub::new();
                let result = match operation {
                    "link" => github.link_security_fix("acme/widgets", &record, &issue()),
                    "reproduction" => {
                        github.update_security_record("acme/widgets", &record, &reproduction)
                    }
                    _ => unreachable!(),
                };
                let error = format!("{:#}", result.unwrap_err());
                assert!(
                    !error.contains("private read evidence")
                        && !error.contains("private mutation evidence")
                );
                assert_eq!(
                    fs::read_to_string(fixture.path("reads")).unwrap(),
                    "read\nread\n",
                    "{operation}: {error}"
                );
                assert_eq!(
                    fs::read_to_string(fixture.path("writes")).unwrap(),
                    "write\n"
                );
                let body = fs::read_to_string(fixture.path("body")).unwrap();
                assert!(body.contains(if operation == "link" {
                    "Fix Ticket:"
                } else {
                    "Private reproduction notes."
                }));
                for name in ["reads", "writes", "failed"] {
                    fs::remove_file(fixture.path(name)).unwrap();
                }
            }
        },
    );
}

#[test]
fn a_transient_issue_read_recovers_without_exposing_the_failure() {
    with_recorded_signal(
        "github::execution_tests::a_transient_issue_read_recovers_without_exposing_the_failure",
        |_| {
            let fixture = Fixture::new("");
            fixture.script(&format!(
                r#"
echo read >> '{reads}'
if [ ! -f '{failed}' ]; then
  touch '{failed}'
  echo 'HTTP 504: Gateway Timeout' >&2
  exit 1
fi
printf '%s' '{{"title":"Recovered"}}'
"#,
                reads = fixture.path("reads").display(),
                failed = fixture.path("failed").display(),
            ));
            assert_eq!(GitHub::new().issue_title(&issue()).unwrap(), "Recovered");
            assert_eq!(
                fs::read_to_string(fixture.path("reads")).unwrap(),
                "read\nread\n"
            );
        },
    );
}

#[test]
fn permanent_and_quota_read_failures_are_not_retried() {
    with_recorded_signal(
        "github::execution_tests::permanent_and_quota_read_failures_are_not_retried",
        |_| {
            let fixture = Fixture::new("");
            for stderr in [
                "HTTP 401: Bad credentials: unexpected EOF",
                "HTTP 403: Resource not accessible",
                "HTTP 404: Not Found",
                "HTTP 422: Validation Failed",
                "To get started with GitHub CLI, please run: gh auth login",
                "GraphQL: API rate limit exceeded for user ID 123. (HTTP 429)",
                "GraphQL: API rate limit already exceeded for user ID 123.",
                "GraphQL: RATE_LIMIT_EXCEEDED (HTTP 429)",
                "API rate limit exceeded for user ID 123. (HTTP 429)",
            ] {
                fixture.script(&format!(
                    "echo read >> '{}'\ncat >&2 <<'ERROR'\n{stderr}\nERROR\nexit 1",
                    fixture.path("reads").display()
                ));
                let error = GitHub::new().issue_title(&issue()).unwrap_err();
                assert!(format!("{error:#}").contains(stderr));
                assert_eq!(
                    fs::read_to_string(fixture.path("reads")).unwrap(),
                    "read\n",
                    "{stderr}"
                );
                fs::remove_file(fixture.path("reads")).unwrap();
            }
        },
    );
}

#[test]
fn persistent_read_failures_stop_after_three_attempts_with_the_last_cause() {
    with_recorded_signal(
        "github::execution_tests::persistent_read_failures_stop_after_three_attempts_with_the_last_cause",
        |_| {
            let fixture = Fixture::new("");
            // macOS wc pads the count; format the attempt as an integer.
            fixture.script(&format!(
                "echo read >> '{reads}'\nn=$(wc -l < '{reads}')\necho \"HTTP 504: failure $((n))\" >&2\nexit 1",
                reads = fixture.path("reads").display(),
            ));
            let started = Instant::now();
            let error = GitHub::new().issue_title(&issue()).unwrap_err();
            assert_eq!(
                error.to_string(),
                "gh issue view failed: HTTP 504: failure 3"
            );
            assert_eq!(
                fs::read_to_string(fixture.path("reads")).unwrap(),
                "read\nread\nread\n"
            );
            assert!(started.elapsed() >= Duration::from_secs(3));
            assert!(started.elapsed() < Duration::from_secs(10));
        },
    );
}

#[test]
fn interruption_during_read_backoff_starts_no_further_request() {
    with_recorded_signal(
        "github::execution_tests::interruption_during_read_backoff_starts_no_further_request",
        |signal| {
            let fixture = Fixture::new("");
            let reads = fixture.path("reads");
            fixture.script(&format!(
                "echo read >> '{}'\necho 'HTTP 504: Gateway Timeout' >&2\nexit 1",
                reads.display()
            ));
            let marker = reads.clone();
            let sender = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !marker.exists() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                assert!(marker.exists(), "gh never started");
                std::thread::sleep(Duration::from_millis(250));
                signal_hook::low_level::raise(signal).unwrap();
            });
            let started = Instant::now();
            let result = GitHub::new().issue_title(&issue());
            sender.join().unwrap();
            assert_eq!(result.unwrap_err().to_string(), "interrupted");
            assert_eq!(fs::read_to_string(reads).unwrap(), "read\n");
            assert!(started.elapsed() < Duration::from_secs(2));
        },
    );
}

#[test]
fn transient_transport_failures_recover_across_read_only_operations() {
    with_recorded_signal(
        "github::execution_tests::transient_transport_failures_recover_across_read_only_operations",
        |_| {
            let fixture = Fixture::new("");
            let github = GitHub::new();
            for (operation, failure, json) in [
                (
                    "issues",
                    "echo 'HTTP 429: Too Many Requests' >&2; exit 1",
                    "",
                ),
                (
                    "pr",
                    "echo 'read tcp: connection reset by peer' >&2; exit 1",
                    r#"{"body":"Recovered"}"#,
                ),
                ("labels", "echo 'EOF' >&2; exit 1", "[]"),
                (
                    "tickets",
                    "echo 'HTTP 503: Service Unavailable' >&2; exit 1",
                    r#"{"data":{"repository":{"issue":{"subIssues":{"nodes":[]}}}}}"#,
                ),
                ("profile", "printf '{'", r#"{"email":"me@example.com"}"#),
            ] {
                fixture.script(&format!(
                    r#"
echo read >> '{reads}'
if [ ! -f '{failed}' ]; then
  touch '{failed}'
  {failure}
else
  printf '%s' '{json}'
fi
"#,
                    reads = fixture.path("reads").display(),
                    failed = fixture.path("failed").display(),
                ));
                match operation {
                    "issues" => assert!(
                        github
                            .open_issues_labelled("acme/widgets", crate::labels::READY_FOR_AGENT)
                            .unwrap()
                            .is_empty()
                    ),
                    "pr" => assert_eq!(github.pr_body(&issue(), 12).unwrap(), "Recovered"),
                    "labels" => github.ensure_labels("acme/widgets", &[]).unwrap(),
                    "tickets" => assert!(github.tickets(&issue()).unwrap().is_empty()),
                    "profile" => assert_eq!(
                        github.profile_email().unwrap().as_deref(),
                        Some("me@example.com")
                    ),
                    _ => unreachable!(),
                }
                assert_eq!(
                    fs::read_to_string(fixture.path("reads")).unwrap(),
                    "read\nread\n",
                    "{operation}"
                );
                fs::remove_file(fixture.path("reads")).unwrap();
                fs::remove_file(fixture.path("failed")).unwrap();
            }
        },
    );
}

#[test]
fn transient_mutation_failures_are_never_retried() {
    with_recorded_signal(
        "github::execution_tests::transient_mutation_failures_are_never_retried",
        |_| {
            let fixture = Fixture::new("");
            fixture.script(&format!(
                r#"
if [ "$1 $2" = 'label list' ]; then
  printf '[]'
  exit 0
fi
echo "$1 $2" >> '{}'
echo 'HTTP 504: Gateway Timeout' >&2
exit 1
"#,
                fixture.path("mutations").display()
            ));
            let github = GitHub::new();
            for result in [
                github.close_issue(&issue(), "Done"),
                github.set_labels(&issue(), &Labels::default()),
                github.ensure_labels("acme/widgets", &[crate::labels::NEEDS_TRIAGE]),
                github
                    .create_issue(&issue(), "Title", "Body", &[])
                    .map(drop),
                github
                    .create_draft_pr(&issue(), "issue-7", "main", "Title", "Body")
                    .map(drop),
                github.mark_ready(&issue(), "12"),
                github.set_pr_body(&issue(), 12, "Body"),
                github.merge(&issue(), 12, "head"),
                github.rerun_failed_jobs(&issue(), 123),
            ] {
                assert!(format!("{:#}", result.unwrap_err()).contains("HTTP 504"));
            }
            assert_eq!(
                fs::read_to_string(fixture.path("mutations")).unwrap(),
                "issue close\napi --method\nlabel create\nissue create\npr create\npr ready\napi --method\npr merge\nrun rerun\n"
            );
        },
    );
}

#[test]
fn completion_read_retries_without_clearing_interruption() {
    with_recorded_signal(
        "github::execution_tests::completion_read_retries_without_clearing_interruption",
        |signal| {
            let fixture = Fixture::new("");
            fixture.script(&format!(
                r#"
if [ ! -f '{failed}' ]; then
  touch '{failed}'
  echo 'HTTP 504: Gateway Timeout' >&2
  exit 1
fi
printf '%s' '{{"title":"Recovered"}}'
"#,
                failed = fixture.path("failed").display(),
            ));
            let github = GitHub::new();
            signal_hook::low_level::raise(signal).unwrap();
            assert_eq!(
                github.completion().issue_title(&issue()).unwrap(),
                "Recovered"
            );
            assert!(interrupt::requested());
            assert_eq!(
                github.issue_title(&issue()).unwrap_err().to_string(),
                "interrupted"
            );
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
                .map(|check| (check.name.as_str(), check.conclusion.clone()))
                .collect();
            assert_eq!(
                answers,
                [
                    ("build", Conclusion::Success),
                    ("lint", Conclusion::Failure),
                    ("external", Conclusion::Pending)
                ]
            );
        },
    );
}

#[test]
fn a_completed_newer_workflow_without_observed_jobs_cannot_establish_green_ci() {
    with_recorded_signal(
        "github::execution_tests::a_completed_newer_workflow_without_observed_jobs_cannot_establish_green_ci",
        |_| {
            let _fixture = Fixture::new(
                r#"
case "$5" in
  */check-runs*) printf '%s\n' '{"id":1,"name":"test","status":"completed","conclusion":"cancelled","details_url":"https://github.com/acme/widgets/actions/runs/900/job/1"}' ;;
  */actions/runs*) printf '%s\n' \
    '{"id":900,"workflow_id":10,"created_at":"2026-10-19T12:00:00Z","status":"completed","conclusion":"cancelled"}' \
    '{"id":901,"workflow_id":10,"created_at":"2026-10-19T12:01:00Z","status":"completed","conclusion":"failure","name":"CI","html_url":"https://github.com/acme/widgets/actions/runs/901"}' ;;
  */status*) printf '%s\n' '{"context":"deploy","state":"success"}' ;;
  *) exit 1 ;;
esac
"#,
            );
            if let Ok(checks) = GitHub::new().checks_on(&issue(), "abc") {
                assert!(
                    checks.iter().any(|check| !matches!(
                        check.conclusion,
                        Conclusion::Success | Conclusion::Neutral | Conclusion::Skipped
                    )),
                    "the newest workflow is failed but its jobs were not observed; this observation must not establish green CI"
                );
            }
        },
    );
}

#[test]
fn checks_on_ignores_older_runs_of_each_workflow_but_keeps_other_checks() {
    with_recorded_signal(
        "github::execution_tests::checks_on_ignores_older_runs_of_each_workflow_but_keeps_other_checks",
        |_| {
            let _fixture = Fixture::new(
                r#"
case "$5" in
  */check-runs*) printf '%s\n' \
    '{"id":1,"name":"test","status":"completed","conclusion":"cancelled","details_url":"https://github.com/acme/widgets/actions/runs/900/job/1"}' \
    '{"id":2,"name":"old-only","status":"completed","conclusion":"failure","details_url":"https://github.com/acme/widgets/actions/runs/900/job/2"}' \
    '{"id":3,"name":"test","status":"in_progress","details_url":"https://github.com/acme/widgets/actions/runs/901/job/3"}' \
    '{"id":4,"name":"test","status":"completed","conclusion":"failure","details_url":"https://github.com/acme/widgets/actions/runs/902/job/4"}' \
    '{"id":5,"name":"external","status":"completed","conclusion":"cancelled","details_url":"https://ci.example/test"}' ;;
  */actions/runs*) printf '%s\n' \
    '{"id":900,"workflow_id":10,"created_at":"2026-10-19T12:00:00Z"}' \
    '{"id":902,"workflow_id":20,"created_at":"2026-10-19T11:00:00Z"}' \
    '{"id":901,"workflow_id":10,"created_at":"2026-10-19T12:01:00Z"}' ;;
  */status*) printf '%s\n' '{"context":"deploy","state":"error"}' ;;
  *) exit 1 ;;
esac
"#,
            );
            let checks = GitHub::new().checks_on(&issue(), "abc").unwrap();
            let answers: Vec<_> = checks
                .iter()
                .map(|check| (check.name.as_str(), check.conclusion.clone()))
                .collect();
            assert_eq!(
                answers,
                [
                    ("test", Conclusion::Unfinished),
                    ("test", Conclusion::Failure),
                    ("external", Conclusion::Cancelled),
                    ("deploy", Conclusion::Error),
                ]
            );
        },
    );
}

#[test]
fn a_newer_pending_workflow_run_without_jobs_cannot_establish_green_ci() {
    with_recorded_signal(
        "github::execution_tests::a_newer_pending_workflow_run_without_jobs_cannot_establish_green_ci",
        |_| {
            let _fixture = Fixture::new(
                r#"
case "$5" in
  */check-runs*) printf '%s\n' '{"id":1,"name":"test","status":"completed","conclusion":"cancelled","details_url":"https://github.com/acme/widgets/actions/runs/900/job/1"}' ;;
  */actions/runs*) printf '%s\n' \
    '{"id":900,"workflow_id":10,"created_at":"2026-10-19T12:00:00Z","status":"completed"}' \
    '{"id":901,"workflow_id":10,"created_at":"2026-10-19T12:01:00Z","status":"queued","name":"CI","html_url":"https://github.com/acme/widgets/actions/runs/901"}' ;;
  */status*) printf '%s\n' '{"context":"deploy","state":"success"}' ;;
  *) exit 1 ;;
esac
"#,
            );
            let checks = GitHub::new().checks_on(&issue(), "abc").unwrap();
            assert!(checks.iter().any(|check| check.workflow_run
                == Some(WorkflowRun {
                    id: 901,
                    workflow_id: 10
                })));
            assert!(
                checks
                    .iter()
                    .any(|check| check.conclusion == Conclusion::Unfinished)
            );
            assert!(
                !checks
                    .iter()
                    .any(|check| check.conclusion == Conclusion::Cancelled)
            );
        },
    );
}

#[test]
fn an_in_progress_newer_run_with_only_a_finished_job_cannot_establish_green_ci() {
    with_recorded_signal(
        "github::execution_tests::an_in_progress_newer_run_with_only_a_finished_job_cannot_establish_green_ci",
        |_| {
            // A later job that `needs` this one has no check run yet. The
            // superseded run's failure must not remain, and the open run must
            // not look finished: CI watch treats a listing with no pending
            // or failed check as passed and a Merge run can land it.
            let _fixture = Fixture::new(
                r#"
case "$5" in
  */check-runs*) printf '%s\n' \
    '{"id":1,"name":"test","status":"completed","conclusion":"failure","details_url":"https://github.com/acme/widgets/actions/runs/900/job/1"}' \
    '{"id":2,"name":"setup","status":"completed","conclusion":"success","details_url":"https://github.com/acme/widgets/actions/runs/901/job/2"}' ;;
  */actions/runs*) printf '%s\n' \
    '{"id":900,"workflow_id":10,"created_at":"2026-10-19T12:00:00Z","status":"completed","conclusion":"cancelled","name":"CI"}' \
    '{"id":901,"workflow_id":10,"created_at":"2026-10-19T12:01:00Z","status":"in_progress","name":"CI","html_url":"https://github.com/acme/widgets/actions/runs/901"}' ;;
  */status*) ;;
  *) exit 1 ;;
esac
"#,
            );
            let checks = GitHub::new().checks_on(&issue(), "abc").unwrap();
            let observed: Vec<_> = checks
                .iter()
                .map(|check| format!("{}:{:?}", check.name, check.conclusion))
                .collect();
            assert!(
                checks.iter().all(|check| check.name != "test"),
                "superseded run stayed visible: {observed:?}"
            );
            assert!(
                checks
                    .iter()
                    .any(|check| check.conclusion == Conclusion::Unfinished),
                "open run looked finished: {observed:?}"
            );
            assert!(
                checks.iter().any(|check| {
                    check.workflow_run
                        == Some(WorkflowRun {
                            id: 901,
                            workflow_id: 10,
                        })
                }),
                "newest run lost its identity: {observed:?}"
            );
        },
    );
}

#[test]
fn github_reports_each_check_as_github_named_it() {
    with_recorded_signal(
        "github::execution_tests::github_reports_each_check_as_github_named_it",
        |_| {
            let _fixture = Fixture::new(
                r#"
case "$1 $2" in
  'api --paginate')
    if test "$4" = '.check_runs[]'; then
      printf '%s\n' \
        '{"name":"build","status":"completed","conclusion":"cancelled"}' \
        '{"name":"test","status":"completed","conclusion":"timed_out"}' \
        '{"name":"lint","status":"in_progress","conclusion":null}' \
        '{"name":"docs","status":"completed","conclusion":null}' \
        '{"name":"audit","status":"completed","conclusion":"someday"}'
    else
      printf '%s\n' '{"context":"external","state":"error"}'
    fi ;;
  *) exit 1 ;;
esac
"#,
            );
            let checks = GitHub::new().checks_on(&issue(), "abc").unwrap();
            let answers: Vec<_> = checks
                .iter()
                .map(|check| (check.name.as_str(), check.conclusion.clone()))
                .collect();
            assert_eq!(
                answers,
                [
                    ("build", Conclusion::Cancelled),
                    ("test", Conclusion::TimedOut),
                    ("lint", Conclusion::Unfinished),
                    ("docs", Conclusion::Unrecognised(String::new())),
                    ("audit", Conclusion::Unrecognised("someday".into())),
                    ("external", Conclusion::Error),
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
                ("GraphQL: secondary rate limit exceeded (HTTP 429)", true),
                ("gh: Bad Gateway (HTTP 502)", true),
                ("gh: service unavailable (HTTP 503)", true),
                ("gh: Gateway Timeout (HTTP 504)", true),
                ("GraphQL: API rate limit exceeded (HTTP 429)", false),
                ("GraphQL: API rate limit already exceeded", false),
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
            let error = crate::ci::Watch::new(&issue())
                .assess("abc", None, &mut || Ok(()), &mut |_| Ok(false))
                .err()
                .unwrap();

            assert_eq!(error.to_string(), "interrupted");
            assert_eq!(fs::read_to_string(fixture.path("reads")).unwrap(), "read\n");
            assert!(started.elapsed() < Duration::from_secs(2));
            assert!(!exists(fixture.pid().unwrap()));
        },
    );
}
