//! Exercise real launch and supervision against a manager that misses collection.

use super::*;
use crate::process::{self, Control, Interruption, Invocation};

const FIXTURE: &str = "THIRDSHIFT_SCOPE_LIFECYCLE_FIXTURE";

// No cgroup processes remain, but the manager retains the loaded unit until
// explicitly stopped. No systemd bus or host cgroup mutation is involved.
const MANAGER: &str = r#"#!/usr/bin/python3
import os, pathlib, subprocess, sys
root = pathlib.Path(os.environ['THIRDSHIFT_SCOPE_LIFECYCLE_FIXTURE'])
args = sys.argv[1:]
program = pathlib.Path(sys.argv[0]).name
if program == 'systemd-run':
    assert '--collect' in args
    unit = next(a.removeprefix('--unit=') for a in args if a.startswith('--unit='))
    state = root / unit
    assert not state.exists(), 'unit already loaded'
    state.touch()
    command = args[args.index('--') + 1:]
    if command[1:2] == ['--oom-exec']:
        assert '--property=MemoryHigh=1G' in args
        assert '--property=MemoryMax=2G' in args
        assert '--property=MemorySwapMax=1G' in args
        assert '--property=OOMPolicy=kill' in args
        command = command[2:]
    result = subprocess.run(command)
    if (root / 'collect-automatically').exists():
        state.unlink()
    sys.exit(result.returncode)
if program == 'systemctl':
    if 'SystemState' in args:
        print('running')
        sys.exit(0)
    unit = next(a for a in args if a.endswith('.scope'))
    state = root / unit
    if 'show' in args:
        print('LoadState=' + ('loaded' if state.exists() else 'not-found'))
        print('ControlGroup=')
        sys.exit(0 if state.exists() else 1)
    if 'stop' in args:
        if (root / 'collect-during-stop').exists():
            state.unlink(missing_ok=True)
        if not state.exists():
            print('Unit not loaded', file=sys.stderr)
            sys.exit(5)
        state.unlink()
        sys.exit(0)
    raise AssertionError(args)
if program == 'journalctl':
    if (root / 'oom-killed').exists():
        print("Failed with result 'oom-kill'.")
    sys.exit(0)
raise AssertionError(program)
"#;

fn isolated(exercise: impl FnOnce(&Path)) {
    if let Some(root) = std::env::var_os(FIXTURE) {
        exercise(Path::new(&root));
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    for program in ["systemd-run", "systemctl", "journalctl"] {
        crate::test_support::write_executable(&bin.join(program), MANAGER);
    }
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    crate::test_support::assert_test_without_warnings(
        std::thread::current().name().unwrap(),
        |command| {
            command
                .env(FIXTURE, root.path())
                .env("PATH", std::env::join_paths(paths).unwrap());
        },
    );
}

fn execute(invocation: &mut dyn Invocation) -> std::process::Output {
    process::output(
        invocation,
        None,
        Control {
            name: "scope lifecycle fixture",
            interruption: Interruption::Ordinary,
            stop: &|child| process::stop(child, &[libc::SIGTERM, libc::SIGKILL]),
        },
    )
    .unwrap()
}

#[test]
fn finished_scopes_are_unloaded() {
    isolated(|root| {
        for kind in ["Session", "Trial", "Grader"] {
            let mut invocation: Box<dyn Invocation> = match kind {
                "Session" => session_command("/bin/true"),
                "Trial" => Box::new(
                    trial_command(Path::new("/bin/true"), &[], "fixture", 1, &Caps::default())
                        .unwrap(),
                ),
                "Grader" => Box::new(
                    grader_command(Path::new("/bin/true"), &[], root, &Caps::default()).unwrap(),
                ),
                _ => unreachable!(),
            };
            let unit = invocation.parts().1.unwrap().unit().to_owned();
            assert!(execute(invocation.as_mut()).status.success());
            assert!(
                root.join(&unit).exists(),
                "the fake manager retains the unit"
            );
            drop(invocation);
            assert!(
                !root.join(&unit).exists(),
                "finished {kind} scope remains loaded: {unit}"
            );
        }
    });
}

#[test]
fn reused_pid_and_counter_do_not_reuse_unit_names() {
    isolated(|_| {
        // The isolated test process has no concurrent name builders. Reset
        // the counter to simulate a later process holding this same PID.
        UNIT_COUNTER.store(0, Ordering::SeqCst);
        let mut first = session_command("unused");
        let first = first.parts().1.unwrap().unit().to_owned();
        UNIT_COUNTER.store(0, Ordering::SeqCst);
        let mut second = session_command("unused");
        let second = second.parts().1.unwrap().unit().to_owned();
        assert_ne!(
            first, second,
            "PID reuse must not collide with a retained unit"
        );
    });
}

#[test]
fn an_already_collected_scope_is_silent_even_when_collection_races_stop() {
    isolated(|root| {
        for marker in ["collect-automatically", "collect-during-stop"] {
            let marker = root.join(marker);
            std::fs::write(&marker, "").unwrap();
            let mut invocation = session_command("/bin/true");
            let unit = invocation.parts().1.unwrap().unit().to_owned();
            assert!(execute(invocation.as_mut()).status.success());
            drop(invocation);
            assert!(!root.join(unit).exists());
            std::fs::remove_file(marker).unwrap();
        }
        // The isolated helper also asserts that stderr contains no warning.
    });
}

#[test]
fn a_failed_trial_keeps_oom_evidence_before_unloading() {
    isolated(|root| {
        std::fs::write(root.join("oom-killed"), "").unwrap();
        let mut invocation = trial_command(
            Path::new("/bin/sh"),
            &["-c", "exit 137"],
            "oom-fixture",
            1,
            &Caps::default(),
        )
        .unwrap();
        let unit = invocation.unit().to_owned();
        let output = execute(&mut invocation);
        assert_eq!(output.status.code(), Some(137));
        assert!(invocation.oom_killed());
        drop(invocation);
        assert!(!root.join(unit).exists());
    });
}
