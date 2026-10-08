use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, FixedOffset};

use super::*;
use crate::harness::{ChosenBy, Harness};
use crate::issue::IssueUrl;
use crate::logs::effects::Tail;

const STAMP: &str = "20261003T120000-0400";
const COMMAND: &str = "/logs/acme/widgets/commands/issue/7-20261003T120000-0400.log";
const ACTIVITY: &str = "/logs/acme/widgets/activity.log";

#[derive(Default)]
struct Memory {
    now: Option<DateTime<FixedOffset>>,
    terminal: Vec<(Stream, String)>,
    files: BTreeMap<PathBuf, String>,
    command_open_fails: bool,
    command_bytes_left: Option<usize>,
    activity_read_fails: bool,
    activity_write_fails: bool,
    terminal_fails: bool,
}

#[derive(Clone, Default)]
struct Scripted(Arc<Mutex<Memory>>);

impl Scripted {
    fn terminal(&self) -> String {
        self.0
            .lock()
            .unwrap()
            .terminal
            .iter()
            .map(|(_, line)| line.as_str())
            .collect()
    }

    fn file(&self, path: impl AsRef<Path>) -> String {
        self.0
            .lock()
            .unwrap()
            .files
            .get(path.as_ref())
            .cloned()
            .unwrap_or_default()
    }
}

struct Writer {
    memory: Scripted,
    path: PathBuf,
}

impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut memory = self.memory.0.lock().unwrap();
        let length = match &mut memory.command_bytes_left {
            Some(0) => return Err(io::Error::other("Command write failed")),
            Some(left) => {
                let length = bytes.len().min(*left);
                *left -= length;
                length
            }
            None => bytes.len(),
        };
        memory
            .files
            .get_mut(&self.path)
            .unwrap()
            .push_str(std::str::from_utf8(&bytes[..length]).unwrap());
        Ok(length)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Outside for Scripted {
    type CommandLog = Writer;

    fn now(&mut self) -> DateTime<FixedOffset> {
        self.0
            .lock()
            .unwrap()
            .now
            .unwrap_or_else(|| DateTime::parse_from_rfc3339("2026-10-03T12:00:00-04:00").unwrap())
    }

    fn emit(&mut self, stream: Stream, line: &str) -> io::Result<()> {
        let mut memory = self.0.lock().unwrap();
        if memory.terminal_fails {
            return Err(io::Error::other("terminal closed"));
        }
        memory.terminal.push((stream, format!("{line}\n")));
        Ok(())
    }

    fn open_command_log(&mut self, path: &Path) -> Result<Writer> {
        let mut memory = self.0.lock().unwrap();
        if memory.command_open_fails {
            anyhow::bail!("Command open failed");
        }
        memory.files.insert(path.to_path_buf(), String::new());
        Ok(Writer {
            memory: self.clone(),
            path: path.to_path_buf(),
        })
    }

    fn command_log_exists(&mut self, path: &Path) -> bool {
        self.0.lock().unwrap().files.contains_key(path)
    }

    fn activity_tail(&mut self, path: &Path, limit: u64) -> Result<Tail> {
        let mut memory = self.0.lock().unwrap();
        if memory.activity_read_fails {
            anyhow::bail!("Activity read failed");
        }
        let file = memory.files.entry(path.to_path_buf()).or_default();
        let from = file.len().saturating_sub(limit as usize);
        Ok(Tail {
            from: from as u64,
            bytes: file.as_bytes()[from..].to_vec(),
        })
    }

    fn append_activity(&mut self, path: &Path, line: &str) -> Result<()> {
        let mut memory = self.0.lock().unwrap();
        if memory.activity_write_fails {
            anyhow::bail!("Activity write failed");
        }
        memory
            .files
            .entry(path.to_path_buf())
            .or_default()
            .push_str(line);
        Ok(())
    }
}

fn issue() -> IssueUrl {
    IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap()
}

fn config(quiet: bool) -> UserConfig {
    UserConfig::parse(
        &format!("[logs]\ndir = '/logs'\n[activity]\nquiet_skips = {quiet}\n"),
        Path::new("/home/config.toml"),
        Path::new("/home"),
    )
    .unwrap()
}

fn harness() -> Choice {
    Choice {
        harness: Harness::Claude,
        model: Some("opus".into()),
        effort: Some("high".into()),
        chosen_by: ChosenBy::Command,
    }
}

#[test]
fn a_run_records_its_complete_output_and_activity_under_its_repository() {
    let memory = Scripted::default();
    let mut record = Record::new(memory.clone());
    let issue = issue();
    record.begin(Begin::Run(&issue));
    record.eprint("checking Origin match");
    record.configured(&config(false));
    assert_eq!(record.root(&issue.repo()), Path::new("/logs/acme/widgets"));
    record.started(Work::Run(&issue), &harness());
    record.print("https://github.com/acme/widgets/pull/8");
    record.ended("ready for review");

    assert_eq!(record.stamp(), STAMP);
    assert_eq!(record.command_log_path(), Some(PathBuf::from(COMMAND)));
    let output = "thirdshift: 12:00:00 starting on https://github.com/acme/widgets/issues/7, 2026-10-03 -0400\nchecking Origin match\nthirdshift: 12:00:00 logging this command to /logs/acme/widgets/commands/issue/7-20261003T120000-0400.log\nthirdshift: 12:00:00 sessions run on claude · opus · high\nhttps://github.com/acme/widgets/pull/8\n";
    assert_eq!(memory.terminal(), output);
    assert_eq!(memory.file(COMMAND), output);
    assert_eq!(
        memory.file(ACTIVITY),
        "2026-10-03 12:00:00 Run #7 started: commands/issue/7-20261003T120000-0400.log, on claude · opus · high\n2026-10-03 12:00:00 Run #7 ended: ready for review\n"
    );
}

#[test]
fn each_work_kind_records_only_its_first_start_and_keeps_its_own_command_folder() {
    let issue = issue();
    let repo = issue.repo();
    let dispatched = IssueUrl::parse("https://github.com/other/repo/issues/8").unwrap();
    for (begin, work, beginning, subject, relative) in [
        (
            Begin::Run(&issue),
            Work::Run(&issue),
            "starting on https://github.com/acme/widgets/issues/7",
            "Run #7",
            "commands/issue/7-20261003T120000-0400.log",
        ),
        (
            Begin::Run(&issue),
            Work::SpecRun(&issue),
            "starting on https://github.com/acme/widgets/issues/7",
            "Spec run #7",
            "commands/issue/7-20261003T120000-0400.log",
        ),
        (
            Begin::PickupRun,
            Work::PickupRun(&issue),
            "Pickup run starting",
            "Pickup run #7",
            "commands/pickup/7-20261003T120000-0400.log",
        ),
        (
            Begin::ArchitectRun,
            Work::ArchitectRun(&repo),
            "Architect run starting",
            "Architect run",
            "commands/architect/20261003T120000-0400.log",
        ),
    ] {
        let memory = Scripted::default();
        let mut record = Record::new(memory.clone());
        record.begin(begin);
        record.configured(&config(true));
        record.started(work, &harness());
        record.started(Work::Run(&dispatched), &Choice::default());
        record.started(Work::SpecRun(&dispatched), &Choice::default());
        record.ended("ready");
        let path = Path::new("/logs/acme/widgets").join(relative);
        assert_eq!(record.command_log_path(), Some(path.clone()));
        let output = format!(
            "thirdshift: 12:00:00 {beginning}, 2026-10-03 -0400\nthirdshift: 12:00:00 logging this command to {}\nthirdshift: 12:00:00 sessions run on claude · opus · high\n",
            path.display()
        );
        assert_eq!(memory.terminal(), output);
        assert_eq!(memory.file(path), output);
        assert_eq!(
            memory.file(ACTIVITY),
            format!(
                "2026-10-03 12:00:00 {subject} started: {relative}, on claude · opus · high\n2026-10-03 12:00:00 {subject} ended: ready\n"
            )
        );
        assert_eq!(memory.0.lock().unwrap().files.len(), 2);
    }
}

#[test]
fn held_output_keeps_streams_and_order_through_configuration_release_and_start() {
    let issue = issue();
    for quiet in [false, true] {
        let memory = Scripted::default();
        let mut record = Record::new(memory.clone());
        record.begin(Begin::PickupRun);
        record.print("before configuration\nsecond line");
        record.eprint("diagnostic");
        assert_eq!(memory.terminal(), "");
        assert_eq!(record.command_log_path(), None);
        record.configured(&config(quiet));
        let held = "thirdshift: 12:00:00 Pickup run starting, 2026-10-03 -0400\nbefore configuration\nsecond line\ndiagnostic\n";
        assert_eq!(memory.terminal(), if quiet { "" } else { held });
        record.show_held();
        record.show_held();
        assert_eq!(memory.terminal(), held);
        record.print("");
        record.eprint("thirdshift: 09:08:07 #21: relayed unchanged");
        record.started(Work::PickupRun(&issue), &harness());
        record.show_held();
        record.print("after start");
        record.ended("done");
        let path = "/logs/acme/widgets/commands/pickup/7-20261003T120000-0400.log";
        let expected = format!(
            "{held}\nthirdshift: 09:08:07 #21: relayed unchanged\nthirdshift: 12:00:00 logging this command to {path}\nthirdshift: 12:00:00 sessions run on claude · opus · high\nafter start\n"
        );
        assert_eq!(memory.terminal(), expected);
        assert_eq!(memory.file(path), expected);
        let streams: Vec<_> = memory
            .0
            .lock()
            .unwrap()
            .terminal
            .iter()
            .map(|(stream, _)| *stream)
            .collect();
        assert_eq!(
            streams,
            [
                Stream::Stderr,
                Stream::Stdout,
                Stream::Stderr,
                Stream::Stdout,
                Stream::Stderr,
                Stream::Stderr,
                Stream::Stderr,
                Stream::Stdout
            ]
        );
    }
}

#[test]
fn endings_without_work_reveal_held_output_once_and_create_no_logs() {
    let issue = issue();
    for begin in [
        None,
        Some(Begin::Run(&issue)),
        Some(Begin::ArchitectRun),
        Some(Begin::PickupRun),
    ] {
        let memory = Scripted::default();
        let mut record = Record::new(memory.clone());
        if let Some(begin) = begin {
            record.begin(begin);
        }
        record.configured(&config(true));
        record.eprint("failed before work");
        record.print("detail");
        record.ended("failed");
        let expected = memory.terminal();
        assert!(expected.ends_with("failed before work\ndetail\n"));
        record.ended("failed again");
        record.show_held();
        assert_eq!(memory.terminal(), expected);
        assert_eq!(record.command_log_path(), None);
        assert!(memory.0.lock().unwrap().files.is_empty());
    }
}

#[test]
fn a_child_inherits_its_parent_stamp_and_relays_output_without_a_beginning_or_logs() {
    let memory = Scripted::default();
    let mut record = Record::new(memory.clone());
    let issue = issue();
    let repo = issue.repo();
    record.begin(Begin::ChildRun("parent-stamp"));
    assert_eq!(record.stamp(), "parent-stamp");
    // Child starts are ignored even before configuration.
    for work in [
        Work::Run(&issue),
        Work::SpecRun(&issue),
        Work::PickupRun(&issue),
        Work::ArchitectRun(&repo),
    ] {
        record.started(work, &harness());
    }
    record.configured(&config(true));
    record.eprint("thirdshift: 09:08:07 implement: session started");
    record.print("child result");
    record.show_held();
    record.ended("ready");
    assert_eq!(
        memory.terminal(),
        "thirdshift: 09:08:07 implement: session started\nchild result\n"
    );
    assert_eq!(record.command_log_path(), None);
    assert!(memory.0.lock().unwrap().files.is_empty());
}

#[test]
fn lazy_stamps_are_owned_stable_and_independent_of_other_recordings() {
    let memory = Scripted::default();
    let mut first = Record::new(memory.clone());
    let mut stamp = first.stamp();
    assert_eq!(stamp, STAMP);
    stamp.clear();
    memory.0.lock().unwrap().now =
        Some(DateTime::parse_from_rfc3339("2026-10-04T13:01:02+05:30").unwrap());
    first.begin(Begin::Run(&issue()));
    first.configured(&config(false));
    first.started(Work::Run(&issue()), &harness());
    assert_eq!(first.stamp(), STAMP);
    assert_eq!(first.command_log_path(), Some(PathBuf::from(COMMAND)));
    assert!(memory.terminal().starts_with("thirdshift: 13:01:02 starting on https://github.com/acme/widgets/issues/7, 2026-10-04 +0530\n"));
    assert!(
        memory
            .file(ACTIVITY)
            .starts_with("2026-10-04 13:01:02 Run #7 started:")
    );

    let mut second = Record::new(memory.clone());
    assert_eq!(second.stamp(), "20261004T130102+0530");
    second.begin(Begin::ChildRun("ignored-after-lazy-query"));
    assert_eq!(second.stamp(), "20261004T130102+0530");
    assert_eq!(second.command_log_path(), None);
    let mut other_config = config(true);
    other_config.logs_dir = PathBuf::from("/other");
    second.configured(&other_config);
    assert_eq!(
        second.root(&issue().repo()),
        Path::new("/other/acme/widgets")
    );
    assert_eq!(first.root(&issue().repo()), Path::new("/logs/acme/widgets"));
}

#[test]
#[should_panic(
    expected = "the root of a repository's logs is asked for before the User config is loaded"
)]
fn a_root_query_before_configuration_still_refuses() {
    Record::new(Scripted::default()).root(&issue().repo());
}

#[test]
fn output_and_start_without_a_factory_beginning_keep_no_command_log() {
    let memory = Scripted::default();
    let mut record = Record::new(memory.clone());
    record.print("setup result");
    record.eprint("diagnostic");
    record.configured(&config(false));
    record.started(Work::Run(&issue()), &harness());
    record.ended("done");
    record.ended("done again");
    assert_eq!(
        memory.terminal(),
        "setup result\ndiagnostic\nthirdshift: 12:00:00 sessions run on claude · opus · high\n"
    );
    assert_eq!(record.command_log_path(), None);
    assert_eq!(record.stamp(), STAMP);
    assert_eq!(
        memory.file(ACTIVITY),
        "2026-10-03 12:00:00 Run #7 started: no Command log, on claude · opus · high\n2026-10-03 12:00:00 Run #7 ended: done\n2026-10-03 12:00:00 Run #7 ended: done again\n"
    );
    assert_eq!(memory.0.lock().unwrap().files.len(), 1);
}

fn skip(memory: &Scripted, pass: Pass, reason: &str) {
    let mut record = Record::new(memory.clone());
    record.begin(match pass {
        Pass::Pickup => Begin::PickupRun,
        Pass::Architect => Begin::ArchitectRun,
        Pass::Security => Begin::SecurityRun,
    });
    record.configured(&config(true));
    record.eprint(reason);
    let before = memory.terminal();
    record.skipped(pass, &issue().repo(), reason);
    assert_eq!(memory.terminal(), before);
    assert_eq!(record.command_log_path(), None);
}

#[test]
fn separate_recordings_suppress_skips_from_shared_history_until_their_kind_changes() {
    let memory = Scripted::default();
    skip(&memory, Pass::Pickup, "idle");
    skip(&memory, Pass::Architect, "a plan is open");
    skip(&memory, Pass::Pickup, "idle");
    // A Run's lines do not reset the Pickup run's skip.
    let mut run = Record::new(memory.clone());
    run.begin(Begin::Run(&issue()));
    run.configured(&config(false));
    run.started(Work::Run(&issue()), &harness());
    run.ended("ready");
    skip(&memory, Pass::Pickup, "idle");
    skip(&memory, Pass::Pickup, "at the Claim limit");
    skip(&memory, Pass::Pickup, "at the Claim limit");
    skip(&memory, Pass::Pickup, "idle");
    let mut pickup = Record::new(memory.clone());
    pickup.begin(Begin::PickupRun);
    pickup.configured(&config(true));
    pickup.started(Work::PickupRun(&issue()), &harness());
    // Even just an intervening start permits the same skip again.
    skip(&memory, Pass::Pickup, "idle");
    pickup.ended("ready");
    skip(&memory, Pass::Pickup, "idle");
    skip(&memory, Pass::Architect, "a plan is open");
    assert_eq!(
        memory.file(ACTIVITY),
        "2026-10-03 12:00:00 Pickup run skipped: idle\n2026-10-03 12:00:00 Architect run skipped: a plan is open\n2026-10-03 12:00:00 Run #7 started: commands/issue/7-20261003T120000-0400.log, on claude · opus · high\n2026-10-03 12:00:00 Run #7 ended: ready\n2026-10-03 12:00:00 Pickup run skipped: at the Claim limit\n2026-10-03 12:00:00 Pickup run skipped: idle\n2026-10-03 12:00:00 Pickup run #7 started: commands/pickup/7-20261003T120000-0400.log, on claude · opus · high\n2026-10-03 12:00:00 Pickup run skipped: idle\n2026-10-03 12:00:00 Pickup run #7 ended: ready\n2026-10-03 12:00:00 Pickup run skipped: idle\n"
    );
}

#[test]
fn skip_history_uses_the_last_64_kib_and_discards_a_cut_first_line() {
    let line = "2026-10-03 12:00:00 Pickup run skipped: idle\n";
    // At exactly 64 KiB the first line is whole. With one extra byte it
    // looks valid in the tail but belongs to a line cut at the beginning.
    for prefix in ["", "x"] {
        let memory = Scripted::default();
        let seed = format!("{prefix}{line}{}\n", "x".repeat(64 * 1024 - line.len() - 1));
        memory
            .0
            .lock()
            .unwrap()
            .files
            .insert(PathBuf::from(ACTIVITY), seed.clone());
        skip(&memory, Pass::Pickup, "idle");
        let expected = if prefix.is_empty() {
            seed
        } else {
            format!("{seed}{line}")
        };
        assert_eq!(memory.file(ACTIVITY), expected);
        skip(&memory, Pass::Pickup, "idle");
        assert_eq!(memory.file(ACTIVITY), expected);
    }
    let memory = Scripted::default();
    let seed = "Pickup run skipped: idle\n2026-10-03 12:00:00 Pickup runner skipped: idle\n2026-10-03 12:00:00 Architect run skipped: idle\n";
    memory
        .0
        .lock()
        .unwrap()
        .files
        .insert(PathBuf::from(ACTIVITY), seed.into());
    skip(&memory, Pass::Pickup, "idle");
    assert_eq!(memory.file(ACTIVITY), format!("{seed}{line}"));
}

#[test]
fn command_creation_failure_warns_once_and_retains_an_existing_path() {
    for existing in [false, true] {
        let memory = Scripted::default();
        {
            let mut files = memory.0.lock().unwrap();
            files.command_open_fails = true;
            if existing {
                files
                    .files
                    .insert(PathBuf::from(COMMAND), "previous content\n".into());
            }
        }
        let mut record = Record::new(memory.clone());
        record.begin(Begin::Run(&issue()));
        record.configured(&config(false));
        record.started(Work::Run(&issue()), &harness());
        assert_eq!(
            record.command_log_path(),
            existing.then(|| PathBuf::from(COMMAND))
        );
        // File writes stay disabled even if the underlying problem clears.
        memory.0.lock().unwrap().command_open_fails = false;
        record.eprint("carried on");
        record.print("result");
        record.started(Work::Run(&issue()), &harness());
        record.ended("ready");
        assert_eq!(
            memory
                .terminal()
                .matches("warning: could not keep the Command log:")
                .count(),
            1
        );
        assert!(
            memory
                .terminal()
                .contains("warning: could not keep the Command log: Command open failed\n")
        );
        assert!(!memory.terminal().contains("logging this command to"));
        assert!(memory.terminal().ends_with("carried on\nresult\n"));
        assert_eq!(
            memory.file(COMMAND),
            if existing { "previous content\n" } else { "" }
        );
        let named = if existing {
            "commands/issue/7-20261003T120000-0400.log"
        } else {
            "no Command log"
        };
        assert_eq!(
            memory.file(ACTIVITY),
            format!(
                "2026-10-03 12:00:00 Run #7 started: {named}, on claude · opus · high\n2026-10-03 12:00:00 Run #7 ended: ready\n"
            )
        );
    }
}

#[test]
fn a_failure_writing_held_lines_retains_the_partial_command_log_path() {
    let memory = Scripted::default();
    memory.0.lock().unwrap().command_bytes_left = Some(3);
    let mut record = Record::new(memory.clone());
    record.begin(Begin::Run(&issue()));
    record.configured(&config(false));
    record.eprint("held detail");
    record.started(Work::Run(&issue()), &harness());
    record.eprint("after failure");
    record.ended("done");
    assert_eq!(record.command_log_path(), Some(PathBuf::from(COMMAND)));
    assert_eq!(memory.file(COMMAND), "thi");
    assert_eq!(
        memory
            .terminal()
            .matches("warning: could not keep the Command log:")
            .count(),
        1
    );
    assert!(memory.terminal().contains(&format!(
        "warning: could not keep the Command log: could not write {COMMAND}: Command write failed\n"
    )));
    assert!(
        memory
            .file(ACTIVITY)
            .contains("Run #7 started: commands/issue/7-20261003T120000-0400.log,")
    );
    assert!(memory.file(ACTIVITY).ends_with("Run #7 ended: done\n"));
}

#[test]
fn an_activity_warning_can_fail_the_command_append_without_recursion_and_each_warns_once() {
    let memory = Scripted::default();
    let mut record = Record::new(memory.clone());
    record.begin(Begin::Run(&issue()));
    record.configured(&config(false));
    record.started(Work::Run(&issue()), &harness());
    let command_before = memory.file(COMMAND);
    {
        let mut faults = memory.0.lock().unwrap();
        faults.command_bytes_left = Some(0);
        faults.activity_write_fails = true;
    }
    record.ended("failed");
    record.ended("failed again");
    record.eprint("carried on");
    let warning = format!(
        "thirdshift: 12:00:00 warning: could not keep the Activity log: Activity write failed\nthirdshift: 12:00:00 warning: could not keep the Command log: could not write {COMMAND}: Command write failed\ncarried on\n"
    );
    assert!(memory.terminal().ends_with(&warning));
    assert_eq!(memory.terminal().matches("warning:").count(), 2);
    assert_eq!(memory.file(COMMAND), command_before);
    assert_eq!(record.command_log_path(), Some(PathBuf::from(COMMAND)));
    {
        let mut faults = memory.0.lock().unwrap();
        faults.command_bytes_left = None;
        faults.activity_write_fails = false;
    }
    record.ended("recovered");
    record.print("result");
    assert_eq!(memory.file(COMMAND), command_before);
    assert_eq!(
        memory.file(ACTIVITY),
        "2026-10-03 12:00:00 Run #7 started: commands/issue/7-20261003T120000-0400.log, on claude · opus · high\n2026-10-03 12:00:00 Run #7 ended: recovered\n"
    );
    assert_eq!(memory.terminal().matches("warning:").count(), 2);
}

#[test]
fn activity_read_and_append_warnings_reveal_quiet_output_and_reset_in_fresh_instances() {
    for read_failure in [false, true] {
        let memory = Scripted::default();
        for (begin, pass, kind) in [
            (Begin::PickupRun, Pass::Pickup, "Pickup run"),
            (Begin::ArchitectRun, Pass::Architect, "Architect run"),
        ] {
            {
                let mut faults = memory.0.lock().unwrap();
                faults.activity_read_fails = read_failure;
                faults.activity_write_fails = !read_failure;
            }
            let before = memory.terminal();
            let mut record = Record::new(memory.clone());
            record.begin(begin);
            record.configured(&config(true));
            record.print("held stdout");
            record.eprint("held stderr");
            assert_eq!(memory.terminal(), before);
            record.skipped(pass, &issue().repo(), "idle");
            record.skipped(pass, &issue().repo(), "idle");
            let error = if read_failure {
                "Activity read failed"
            } else {
                "Activity write failed"
            };
            let expected = format!(
                "{before}thirdshift: 12:00:00 {kind} starting, 2026-10-03 -0400\nheld stdout\nheld stderr\nthirdshift: 12:00:00 warning: could not keep the Activity log: {error}\n"
            );
            assert_eq!(memory.terminal(), expected);
            {
                let mut faults = memory.0.lock().unwrap();
                faults.activity_read_fails = false;
                faults.activity_write_fails = false;
            }
            record.skipped(pass, &issue().repo(), "idle");
            record.print("after warning");
            assert!(
                memory
                    .file(ACTIVITY)
                    .ends_with(&format!("{kind} skipped: idle\n"))
            );
            assert!(memory.terminal().ends_with("after warning\n"));
            assert_eq!(record.command_log_path(), None);
        }
        assert_eq!(
            memory
                .terminal()
                .matches("warning: could not keep the Activity log:")
                .count(),
            2
        );
        assert_eq!(
            memory
                .terminal()
                .matches("warning: could not keep the Command log:")
                .count(),
            0
        );
    }
}

#[test]
fn terminal_failures_are_ignored_with_creation_append_and_activity_failure_combinations() {
    for terminal_failure in [false, true] {
        for activity_failure in [false, true] {
            for command_failure in ["none", "open", "held", "append"] {
                let memory = Scripted::default();
                {
                    let mut faults = memory.0.lock().unwrap();
                    faults.terminal_fails = terminal_failure;
                    faults.activity_write_fails = activity_failure;
                    faults.command_open_fails = command_failure == "open";
                    if command_failure == "held" {
                        faults.command_bytes_left = Some(3);
                    }
                }
                let mut record = Record::new(memory.clone());
                record.begin(Begin::Run(&issue()));
                record.configured(&config(false));
                record.started(Work::Run(&issue()), &harness());
                let created = memory.file(COMMAND);
                if command_failure == "append" {
                    memory.0.lock().unwrap().command_bytes_left = Some(0);
                }
                record.eprint("ordinary output");
                record.ended("done");
                let output = memory.terminal();
                assert_eq!(
                    output
                        .matches("warning: could not keep the Command log:")
                        .count(),
                    usize::from(!terminal_failure && command_failure != "none")
                );
                assert_eq!(
                    output
                        .matches("warning: could not keep the Activity log:")
                        .count(),
                    usize::from(!terminal_failure && activity_failure)
                );
                assert_eq!(
                    record.command_log_path(),
                    (command_failure != "open").then(|| PathBuf::from(COMMAND))
                );
                if command_failure != "none" {
                    assert_eq!(memory.file(COMMAND), created);
                } else {
                    assert!(memory.file(COMMAND).starts_with("thirdshift: 12:00:00 starting on https://github.com/acme/widgets/issues/7, 2026-10-03 -0400\n"));
                    assert!(memory.file(COMMAND).ends_with("ordinary output\n"));
                    assert_eq!(
                        memory.file(COMMAND).matches("warning:").count(),
                        usize::from(activity_failure)
                    );
                }
                if activity_failure {
                    assert_eq!(memory.file(ACTIVITY), "");
                } else {
                    assert!(memory.file(ACTIVITY).ends_with("Run #7 ended: done\n"));
                }
                if terminal_failure {
                    assert_eq!(output, "");
                    memory.0.lock().unwrap().terminal_fails = false;
                    record.ended("still done");
                    record.eprint("terminal reopened");
                    assert_eq!(memory.terminal(), "terminal reopened\n");
                }
            }
        }
    }
}

#[test]
fn concurrent_output_and_start_have_the_same_terminal_and_command_log_order() {
    let memory = Scripted::default();
    let mut record = Record::new(memory.clone());
    record.begin(Begin::PickupRun);
    record.configured(&config(true));
    let record = Arc::new(Mutex::new(record));
    let ready = std::sync::Barrier::new(3);
    std::thread::scope(|threads| {
        for worker in 0..2 {
            let record = record.clone();
            let ready = &ready;
            threads.spawn(move || {
                ready.wait();
                for number in 0..16 {
                    let line = format!("worker {worker} line {number}");
                    let mut record = record.lock().unwrap();
                    if worker == 0 {
                        record.print(&line);
                    } else {
                        record.eprint(&line);
                    }
                }
            });
        }
        threads.spawn(|| {
            ready.wait();
            record
                .lock()
                .unwrap()
                .started(Work::PickupRun(&issue()), &harness());
        });
    });
    record.lock().unwrap().ended("done");
    let output = memory.terminal();
    let path = "/logs/acme/widgets/commands/pickup/7-20261003T120000-0400.log";
    assert_eq!(memory.file(path), output);
    assert_eq!(output.lines().count(), 35);
    assert!(output.contains(&format!("thirdshift: 12:00:00 logging this command to {path}\nthirdshift: 12:00:00 sessions run on claude · opus · high\n")));
    for worker in 0..2 {
        for number in 0..16 {
            let line = format!("worker {worker} line {number}");
            assert_eq!(output.lines().filter(|found| *found == line).count(), 1);
        }
    }
}

#[test]
fn a_pass_can_reveal_output_before_configuration_when_loading_it_fails() {
    let memory = Scripted::default();
    let mut record = Record::new(memory.clone());
    record.begin(Begin::ArchitectRun);
    record.eprint("checking User config");
    assert_eq!(memory.terminal(), "");
    record.show_held();
    record.eprint("invalid User config");
    record.show_held();
    record.ended("failed");
    assert_eq!(
        memory.terminal(),
        "thirdshift: 12:00:00 Architect run starting, 2026-10-03 -0400\nchecking User config\ninvalid User config\n"
    );
    assert_eq!(record.command_log_path(), None);
    assert!(memory.0.lock().unwrap().files.is_empty());
}
