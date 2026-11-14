//! A supervisor survives the fixture harness so it can observe cleanup after
//! SIGKILL, and reap exact recorded descendants rather than depending on PID 1.

use super::*;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::Child;
use std::thread;
use std::time::{Duration, Instant};

const BOUND: Duration = Duration::from_secs(5);

struct Fixture {
    harness: Child,
    root: TempDir,
    owned: Vec<libc::pid_t>,
}

impl Fixture {
    fn start(mode: &str) -> Self {
        // SAFETY: adoption is process-wide, but all waits name only this
        // fixture's recorded children, never another parallel test's children.
        assert_eq!(unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) }, 0);
        let root = TempDir::new().unwrap();
        let output = fs::File::create(root.path().join("output")).unwrap();
        let harness = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "lifetime::fixture_process",
                "--ignored",
                "--nocapture",
            ])
            .env("RELEASE_LIFETIME_MODE", mode)
            .env("RELEASE_LIFETIME_ROOT", root.path())
            .env("TMPDIR", root.path())
            .stdout(output.try_clone().unwrap())
            .stderr(output)
            .process_group(0)
            .spawn()
            .unwrap();
        let mut fixture = Self {
            harness,
            root,
            owned: Vec::new(),
        };
        let deadline = Instant::now() + support::WAIT_BOUND;
        while !fixture.root.path().join("ready").exists() {
            assert!(
                fixture.harness.try_wait().unwrap().is_none(),
                "{}",
                fixture.output()
            );
            assert!(
                Instant::now() < deadline,
                "fixture never started: {}",
                fixture.output()
            );
            thread::sleep(Duration::from_millis(10));
        }
        // The guardian reaps its descendants before exiting. Observe it
        // first so SIGKILL cleanup can finish before checking child PIDs.
        fixture.owned = ["guardian", "command", "descendant"]
            .map(|name| {
                fs::read_to_string(fixture.root.path().join(name))
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap()
            })
            .to_vec();
        fixture
    }

    fn output(&self) -> String {
        fs::read_to_string(self.root.path().join("output")).unwrap()
    }

    fn finish(&mut self, success: bool) {
        fs::write(self.root.path().join("continue"), "").unwrap();
        let deadline = Instant::now() + BOUND;
        loop {
            if let Some(status) = self.harness.try_wait().unwrap() {
                assert_eq!(status.success(), success, "{}", self.output());
                break;
            }
            assert!(
                Instant::now() < deadline,
                "fixture exceeded {BOUND:?}: {}",
                self.output()
            );
            thread::sleep(Duration::from_millis(10));
        }
        self.assert_owned_gone();
    }

    fn kill_harness(&mut self) {
        // Kill only the harness; descendants must be stopped by its fixture.
        self.harness.kill().unwrap();
        assert_eq!(self.harness.wait().unwrap().signal(), Some(libc::SIGKILL));
        self.assert_owned_gone();
    }

    fn assert_owned_gone(&mut self) {
        for &pid in &self.owned {
            let deadline = Instant::now() + BOUND;
            loop {
                let mut status = 0;
                // SAFETY: only reap the exact child recorded by this fixture.
                match unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) } {
                    found if found == pid => break,
                    -1 => {
                        let error = std::io::Error::last_os_error();
                        if error.kind() == std::io::ErrorKind::Interrupted {
                            continue;
                        }
                        assert_eq!(error.raw_os_error(), Some(libc::ECHILD));
                        assert_eq!(
                            unsafe { libc::kill(pid, 0) },
                            -1,
                            "owned process {pid} survived"
                        );
                        assert_eq!(
                            std::io::Error::last_os_error().raw_os_error(),
                            Some(libc::ESRCH)
                        );
                        break;
                    }
                    0 => {}
                    result => panic!("unexpected waitpid result: {result}"),
                }
                assert!(
                    Instant::now() < deadline,
                    "owned process {pid} survived: {}",
                    self.output()
                );
                thread::sleep(Duration::from_millis(10));
            }
        }
        self.owned.clear();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Emergency cleanup also bounds a regression in the lifetime helper.
        // The harness PID still belongs to our unreaped child when live.
        if self.harness.try_wait().ok().flatten().is_none() {
            unsafe {
                libc::kill(-(self.harness.id() as libc::pid_t), libc::SIGKILL);
            }
        }
        let _ = self.harness.kill();
        let _ = self.harness.wait();
        for &pid in &self.owned {
            let mut status = 0;
            if unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) } == 0 {
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                    libc::waitpid(pid, &mut status, 0);
                }
            }
        }
    }
}

#[test]
fn missing_workflows_fail_with_captured_output_within_five_seconds() {
    assert_timeout("missing-ci");
}

#[test]
fn pending_workflows_fail_with_captured_output_within_five_seconds() {
    assert_timeout("pending-ci");
}

#[test]
fn missing_release_workflows_fail_with_captured_output_within_five_seconds() {
    assert_timeout("missing-release");
}

#[test]
fn pending_release_workflows_fail_with_captured_output_within_five_seconds() {
    assert_timeout("pending-release");
}

#[test]
fn release_summary_harness_ends_after_timeout() {
    assert_timeout("summary-timeout");
}

#[test]
fn release_summary_harness_ends_after_fixture_sigkill() {
    Fixture::start("summary-sigkill").kill_harness();
}

fn assert_timeout(mode: &str) {
    let mut fixture = Fixture::start(mode);
    fixture.finish(false);
    let output = fixture.output();
    for expected in [
        "release fixture exceeded 500ms",
        "stdout:",
        "command stdout",
        "stderr:",
        "command stderr",
    ] {
        assert!(output.contains(expected), "{mode}: {output}");
    }
    if !mode.starts_with("summary-") {
        assert!(
            output.lines().any(|line| {
                stamped(line).is_some_and(|(_, message)| message.starts_with("waiting for"))
            }),
            "{output}"
        );
    }
}

#[test]
fn release_descendants_end_after_normal_completion() {
    Fixture::start("complete").finish(true);
}

#[test]
fn release_descendants_end_after_assertion_unwinding() {
    let mut fixture = Fixture::start("assertion");
    fixture.finish(false);
    assert!(fixture.output().contains("intentional fixture assertion"));
}

#[test]
fn release_descendants_end_after_harness_sigkill() {
    Fixture::start("sigkill").kill_harness();
}

#[test]
#[ignore = "subprocess entry point for release lifetime tests"]
fn fixture_process() {
    let root = PathBuf::from(std::env::var_os("RELEASE_LIFETIME_ROOT").unwrap());
    let mode = std::env::var("RELEASE_LIFETIME_MODE").unwrap();
    let release = Release::new();
    release.ci_reports(GREEN);
    match mode.as_str() {
        "missing-ci" => release.ci_reports("[]"),
        "pending-release" => release.release_workflow(json!({"pending_polls": 100000})),
        "missing-release" => release.release_workflow(json!({"hidden_polls": 100000})),
        _ => {
            release.ci_reports(r#"[{"name":"test","conclusion":"success","pending_polls":100000}]"#)
        }
    }
    if mode.starts_with("summary-") {
        // The real summary sandbox hides the fixture root. Signal readiness
        // from the host-side launcher, then let the sandboxed fake hang.
        let bwrap = "/usr/bin/bwrap";
        write_executable(
            &release.bin().join("bwrap"),
            &format!(
                r#"#!/bin/sh
case "$*" in *'-- /bin/true') exec {bwrap} "$@";; esac
echo $$ > "$RELEASE_LIFETIME_ROOT/descendant"
touch "$RELEASE_LIFETIME_ROOT/ready"
echo 'release-summary sandbox started' >&2
exec {bwrap} "$@"
"#
            ),
        );
        fs::remove_file(release.bin().join("claude")).unwrap();
        write_executable(
            &release.bin().join("claude"),
            "#!/bin/sh\nif [ \"$1 $2\" = 'auth status' ]; then exit 0; fi\nexec sleep 86400\n",
        );
    }
    let gh = release.bin().join("gh");
    let real_gh = fs::read_link(&gh).unwrap();
    fs::remove_file(&gh).unwrap();
    write_executable(
        &gh,
        r#"#!/bin/bash
case "$RELEASE_LIFETIME_MODE:$*" in
    *-release:*'run list '*'--workflow release.yml '*) ;;
    *-release:*) exec -a gh "$RELEASE_REAL_GH" "$@" ;;
    summary-*:*) exec -a gh "$RELEASE_REAL_GH" "$@" ;;
    *:api*check-runs*) ;;
    *) exec -a gh "$RELEASE_REAL_GH" "$@" ;;
esac
if mkdir "$RELEASE_LIFETIME_ROOT/started" 2>/dev/null; then
    sleep 86400 </dev/null >/dev/null 2>&1 &
    echo $! > "$RELEASE_LIFETIME_ROOT/descendant"
    touch "$RELEASE_LIFETIME_ROOT/ready"
fi
if [ "$RELEASE_LIFETIME_MODE" = complete ] && [ -e "$RELEASE_LIFETIME_ROOT/continue" ]; then
    head=$(git ls-remote origin refs/heads/release-0.2.0 | cut -f1)
    gh fake checks "$head" '[{"name":"test","conclusion":"success"}]'
fi
exec -a gh "$RELEASE_REAL_GH" "$@"
"#,
    );
    let prepared = native::command(&release, "");
    let mut command = release.command(Path::new("bash"), &release.maintainer());
    for (key, value) in prepared.get_envs() {
        match value {
            Some(value) => {
                command.env(key, value);
            }
            None => {
                command.env_remove(key);
            }
        }
    }
    command
        .arg("-c")
        .arg(
            r#"
echo $$ > "$RELEASE_LIFETIME_ROOT/command"
echo "$PPID" > "$RELEASE_LIFETIME_ROOT/guardian"
echo 'command stdout'
echo 'command stderr' >&2
exec "$1" release 0.2.0
"#,
        )
        .arg("bash")
        .arg(env!("CARGO_BIN_EXE_thirdshift"))
        .env("THIRDSHIFT_CI_GRACE_MS", "300000")
        .env("RELEASE_REAL_GH", real_gh);
    let child = support::child_lifetime::FixtureChild::spawn(&mut command, "");
    let deadline = Instant::now() + support::WAIT_BOUND;
    while !root.join("continue").exists() {
        assert!(
            Instant::now() < deadline,
            "supervisor never released fixture"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_ne!(mode, "assertion", "intentional fixture assertion");
    let output = child.wait(if mode == "complete" {
        support::WAIT_BOUND
    } else {
        Duration::from_millis(500)
    });
    assert!(output.status.success(), "{output:?}");
}
