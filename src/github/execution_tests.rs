use super::*;
use crate::interrupt;
use crate::test_support::with_recorded_signal;
use std::fs;
use std::os::unix::fs::PermissionsExt;
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
        fs::write(
            &gh,
            format!(
                "#!/bin/sh\necho $$ > '{}'\n{script}\n",
                self.path("pid").display()
            ),
        )
        .unwrap();
        fs::set_permissions(gh, fs::Permissions::from_mode(0o755)).unwrap();
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
            assert!(
                github
                    .pull_request_for(&issue(), "issue-7")
                    .unwrap()
                    .is_none()
            );
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
            assert!(github.pull_request_for(&issue(), "issue-7").is_err());
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
                .pull_request_for(&issue(), "issue-7")
                .err()
                .expect("cancellation became absence");
            assert_eq!(error.to_string(), "interrupted");
            assert!(!exists(fixture.pid().unwrap()));
        },
    );
}

fn cancelled_merge(name: &str, state: &str, head: &str, confirmed: bool) {
    with_recorded_signal(name, |signal| {
        let fixture = Fixture::new(&format!(
            r#"
case "$1 $2" in
  'pr merge') kill -{signal} {parent}; exec sleep 3 ;;
  'pr view') printf '%s' '{{"state":"{state}","headRefOid":"{head}"}}' ;;
  *) exit 1 ;;
esac
"#,
            parent = std::process::id()
        ));
        let github = GitHub::new();
        let result = github.merge(&issue(), "issue-7", "requested-head");
        if confirmed {
            result.unwrap();
        } else {
            assert_eq!(result.unwrap_err().to_string(), "interrupted");
        }
        assert!(interrupt::requested());
        assert_eq!(
            github
                .mark_ready(&issue(), "issue-7")
                .unwrap_err()
                .to_string(),
            "interrupted"
        );
        assert!(!exists(fixture.pid().unwrap()));
    });
}

#[test]
fn a_cancelled_merge_is_confirmed_only_at_the_requested_head() {
    cancelled_merge(
        "github::execution_tests::a_cancelled_merge_is_confirmed_only_at_the_requested_head",
        "MERGED",
        "requested-head",
        true,
    );
}

#[test]
fn a_cancelled_merge_at_another_head_remains_a_failure() {
    cancelled_merge(
        "github::execution_tests::a_cancelled_merge_at_another_head_remains_a_failure",
        "MERGED",
        "foreign-head",
        false,
    );
}

#[test]
fn an_unconfirmed_cancelled_merge_remains_a_failure() {
    cancelled_merge(
        "github::execution_tests::an_unconfirmed_cancelled_merge_remains_a_failure",
        "OPEN",
        "requested-head",
        false,
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
