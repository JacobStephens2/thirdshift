use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, FixedOffset};

use super::*;
use crate::harness::{ChosenBy, Harness};
use crate::issue::IssueUrl;
use crate::logs::effects::Tail;

const STAMP: &str = "20261003T120000-0400";
const ACTIVITY: &str = "/logs/acme/widgets/activity.log";

#[derive(Default)]
struct Memory {
    now: Option<DateTime<FixedOffset>>,
    terminal: Vec<(Stream, String)>,
    files: BTreeMap<PathBuf, String>,
    command_open_fails: bool,
    existing_on_open_failure: Option<PathBuf>,
    command_bytes_left: Option<usize>,
    activity_read_fails: bool,
    activity_write_fails: bool,
    terminal_fails: bool,
    record_write_fails: bool,
    reused_pid: bool,
}

#[derive(Clone, Default)]
struct Scripted(Arc<Mutex<Memory>>);

impl Scripted {
    fn command_record(&self) -> CommandRecord {
        let memory = self.0.lock().unwrap();
        let records: Vec<_> = memory
            .files
            .iter()
            .filter(|(path, _)| path.to_string_lossy().ends_with(".command.json"))
            .collect();
        assert_eq!(records.len(), 1);
        serde_json::from_str(records[0].1).unwrap()
    }

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

impl owner::Files for Scripted {
    type CommandLog = Writer;

    fn create_command_log(&mut self, path: &Path) -> io::Result<Writer> {
        let mut memory = self.0.lock().unwrap();
        if memory.command_open_fails {
            if memory.existing_on_open_failure.is_some() {
                memory
                    .files
                    .insert(path.into(), "previous content\n".into());
                memory.existing_on_open_failure = Some(path.into());
            }
            return Err(io::Error::other("Command open failed"));
        }
        if memory.files.contains_key(path) {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        memory.files.insert(path.to_path_buf(), String::new());
        Ok(Writer {
            memory: self.clone(),
            path: path.to_path_buf(),
        })
    }
}

impl Outside for Scripted {
    fn stop_request(&mut self, record: &Path) -> Option<crate::logs::stop::Request> {
        let memory = self.0.lock().unwrap();
        serde_json::from_str(memory.files.get(&record.with_extension("stop.json"))?).ok()
    }

    fn hostname(&mut self) -> String {
        "factory".into()
    }
    fn process_start(&mut self, _: u32) -> Option<String> {
        Some(
            if self.0.lock().unwrap().reused_pid {
                "start-2"
            } else {
                "start-1"
            }
            .into(),
        )
    }
    fn replace_record(&mut self, path: &Path, bytes: &[u8]) -> Result<()> {
        if self.0.lock().unwrap().record_write_fails {
            anyhow::bail!("disk full");
        }
        self.0
            .lock()
            .unwrap()
            .files
            .insert(path.into(), String::from_utf8(bytes.to_vec())?);
        Ok(())
    }

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

#[test]
fn a_stop_request_in_the_file_adapter_is_logged_before_the_ending() {
    let memory = Scripted::default();
    let mut record = Record::new(memory.clone());
    record.begin(Begin::Run(&issue()));
    record.configured(&config(false));
    record.started(Work::Run(&issue()), &harness());
    memory.0.lock().unwrap().files.insert(
        record.record_path().unwrap().with_extension("stop.json"),
        r#"{"who":"jacob","requeue":false}"#.into(),
    );
    record.ended("interrupted");
    let activity = memory.file(ACTIVITY);
    let stopped = activity
        .find("Run #7 stopped from the CLI by jacob")
        .expect(&activity);
    assert!(stopped < activity.find("Run #7 ended: interrupted").unwrap());
}

#[test]
fn a_command_record_write_failure_warns_once_and_keeps_the_outcome() {
    let memory = Scripted::default();
    memory.0.lock().unwrap().record_write_fails = true;
    let mut record = Record::new(memory.clone());
    record.begin(Begin::Run(&issue()));
    record.configured(&config(false));
    record.started(Work::Run(&issue()), &harness());
    record.stage(StageName::Implement, None);
    record.stage(StageName::PullRequest, None);
    // A later successful atomic write still retains completion facts.
    memory.0.lock().unwrap().record_write_fails = false;
    let ended = crate::run::Ended {
        outcome: Ok(crate::run::Reached {
            pr_url: "https://github.com/acme/widgets/pull/1".into(),
            goal: crate::run::Goal::ReadyForReview,
            log: None,
            ticket_lines: Vec::new(),
        }),
        base_fix: None,
        advice: Vec::new(),
    };
    let ending = crate::command::Ending::Run(ended);
    let account = crate::run_ending::read(&ending).ok().unwrap();
    record.finished(&account);
    record.ended(account.summary());
    assert_eq!(
        memory
            .terminal()
            .matches("warning: could not keep the Command record:")
            .count(),
        1
    );
    assert!(
        memory
            .file(ACTIVITY)
            .contains("PR https://github.com/acme/widgets/pull/1 is ready for review")
    );
    let facts = memory.command_record();
    assert_eq!(facts.outcome.as_deref(), Some("ready for review"));
}

#[test]
fn a_reused_process_id_is_not_a_live_command() {
    let mut memory = Scripted::default();
    let mut record = Record::new(memory.clone());
    record.begin(Begin::Run(&issue()));
    record.configured(&config(false));
    record.started(Work::Run(&issue()), &harness());
    let facts = memory.command_record();
    assert!(facts.live(&mut memory));
    memory.0.lock().unwrap().reused_pid = true;
    assert!(!facts.live(&mut memory));
}

#[test]
fn commands_started_on_the_same_issue_in_one_second_keep_separate_output_and_records() {
    let memory = Scripted::default();
    let issue = issue();
    let mut first = Record::new(memory.clone());
    first.begin(Begin::Run(&issue));
    first.configured(&config(false));
    first.started(Work::Run(&issue), &harness());
    first.issue_title(Some("first Command".into()));
    first.print("first before second starts");
    let first_log = first.command_log_path().unwrap();
    let first_record = first.record_path().unwrap();

    let mut second = Record::new(memory.clone());
    second.begin(Begin::Run(&issue));
    second.configured(&config(false));
    second.started(Work::Run(&issue), &harness());
    second.issue_title(Some("second Command".into()));
    second.print("second before interleaving");
    first.print("first after second starts");
    second.print("second after interleaving");
    let second_log = second.command_log_path().unwrap();
    assert!(
        memory
            .file(&first_log)
            .contains("first before second starts\n"),
        "starting the second Command erased the first Command output"
    );
    assert_ne!(first_log, second_log);
    assert_ne!(first_record, second.record_path().unwrap());
    for (path, own, other) in [
        (&first_log, "first", "second"),
        (&second_log, "second", "first"),
    ] {
        let bytes = memory.file(path);
        assert_eq!(
            bytes.lines().filter(|line| line.starts_with(own)).count(),
            2
        );
        assert!(!bytes.lines().any(|line| line.starts_with(other)));
    }

    // The effects adapter keeps its clock fixed, so both owners have the same
    // issue, repository, and second-resolution Command-log stamp.
    let records: Vec<CommandRecord> = memory
        .0
        .lock()
        .unwrap()
        .files
        .iter()
        .filter(|(path, _)| path.to_string_lossy().ends_with(".command.json"))
        .map(|(_, bytes)| serde_json::from_str(bytes).unwrap())
        .collect();
    let mut titles: Vec<_> = records
        .iter()
        .map(|record| record.issue.as_ref().unwrap().title.as_deref().unwrap())
        .collect();
    titles.sort();
    assert_eq!(titles, ["first Command", "second Command"]);
}

#[test]
fn concurrent_child_writes_keep_both_runs_and_leave_the_parent_untouched() {
    let memory = Scripted::default();
    let mut parent = Record::new(memory.clone());
    parent.begin(Begin::Run(&issue()));
    parent.configured(&config(false));
    parent.started(Work::SpecRun(&issue()), &harness());
    let path = parent.record_path().unwrap();
    let before = memory.file(&path);
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|threads| {
        for (number, stage) in [(8, StageName::CiWatch), (9, StageName::Repair)] {
            let memory = memory.clone();
            let path = path.clone();
            let barrier = &barrier;
            threads.spawn(move || {
                let issue =
                    IssueUrl::parse(&format!("https://github.com/acme/widgets/issues/{number}"))
                        .unwrap();
                let mut child = Record::new(memory);
                child.child_of(Some(path));
                child.begin(Begin::ChildRun(STAMP, crate::logs::CommandKind::Issue));
                child.configured(&config(false));
                child.started(Work::Run(&issue), &harness());
                barrier.wait();
                for _ in 0..100 {
                    child.issue_title(Some(format!("Ticket {number}")));
                    child.stage(stage, None);
                }
            });
        }
    });
    assert_eq!(memory.file(&path), before);
    let memory = memory.0.lock().unwrap();
    let children: Vec<CommandRecord> = memory
        .files
        .iter()
        .filter(|(path, _)| path.to_string_lossy().ends_with(".child.json"))
        .map(|(_, bytes)| serde_json::from_str(bytes).unwrap())
        .collect();
    assert_eq!(children.len(), 2);
    for (child, (number, stage)) in children.iter().zip([(8, "CI watch"), (9, "Repair")]) {
        assert_eq!(child.parent_record.as_ref(), Some(&path));
        assert_eq!(child.issue.as_ref().unwrap().number, number);
        assert_eq!(
            child.issue.as_ref().unwrap().title.as_deref(),
            Some(format!("Ticket {number}").as_str())
        );
        assert_eq!(child.current_stage().unwrap().name, stage);
    }
}

#[test]
fn children_of_different_dispatches_keep_separate_records_for_the_same_issue() {
    let memory = Scripted::default();
    let parent = PathBuf::from("/logs/acme/widgets/commands/secure/pass.command.json");
    for number in [20, 30] {
        let mut child = Record::new(memory.clone());
        child.child_of(Some(parent.clone()));
        child.child_dispatch(Some(format!(
            "https://github.com/acme/widgets/issues/{number}"
        )));
        child.begin(Begin::ChildRun(STAMP, crate::logs::CommandKind::Secure));
        child.configured(&config(false));
        child.started(Work::BaseFix(&issue()), &harness());
    }
    let memory = memory.0.lock().unwrap();
    let children: Vec<CommandRecord> = memory
        .files
        .values()
        .map(|bytes| serde_json::from_str(bytes).unwrap())
        .collect();
    assert_eq!(children.len(), 2);
    for (child, number) in children.iter().zip([20, 30]) {
        assert_eq!(child.parent_record.as_ref(), Some(&parent));
        assert_eq!(
            child.parent_dispatch.as_deref(),
            Some(format!("https://github.com/acme/widgets/issues/{number}").as_str())
        );
        assert_eq!(child.kind, "Base fix");
    }
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
    let path = record.command_log_path().unwrap();
    let relative = path
        .strip_prefix(record.root(&issue.repo()))
        .unwrap()
        .display();
    let output = format!(
        "thirdshift: 12:00:00 starting on https://github.com/acme/widgets/issues/7, 2026-10-03 -0400\nchecking Origin match\nthirdshift: 12:00:00 logging this command to {}\nthirdshift: 12:00:00 sessions run on claude · opus · high\nhttps://github.com/acme/widgets/pull/8\n",
        path.display()
    );
    assert_eq!(memory.terminal(), output);
    assert_eq!(memory.file(&path), output);
    assert_eq!(
        memory.file(ACTIVITY),
        format!(
            "2026-10-03 12:00:00 Run #7 started: {relative}, on claude · opus · high\n2026-10-03 12:00:00 Run #7 ended: ready for review\n"
        )
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
        let path = record.command_log_path().unwrap();
        assert_eq!(owner::decode(&path).unwrap().identity.stamp, STAMP);
        assert_eq!(
            path.parent(),
            Path::new("/logs/acme/widgets").join(relative).parent()
        );
        let relative = path.strip_prefix("/logs/acme/widgets").unwrap().display();
        let output = format!(
            "thirdshift: 12:00:00 {beginning}, 2026-10-03 -0400\nthirdshift: 12:00:00 logging this command to {}\nthirdshift: 12:00:00 sessions run on claude · opus · high\n",
            path.display()
        );
        assert_eq!(memory.terminal(), output);
        assert_eq!(memory.file(&path), output);
        assert_eq!(
            memory.file(ACTIVITY),
            format!(
                "2026-10-03 12:00:00 {subject} started: {relative}, on claude · opus · high\n2026-10-03 12:00:00 {subject} ended: ready\n"
            )
        );
        assert_eq!(memory.0.lock().unwrap().files.len(), 3);
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
        let path = record.command_log_path().unwrap();
        let path = path.to_str().unwrap();
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
    record.begin(Begin::ChildRun(
        "parent-stamp",
        crate::logs::CommandKind::Issue,
    ));
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
    assert_eq!(
        owner::decode(&first.command_log_path().unwrap())
            .unwrap()
            .identity
            .stamp,
        STAMP
    );
    assert!(memory.terminal().starts_with("thirdshift: 13:01:02 starting on https://github.com/acme/widgets/issues/7, 2026-10-04 +0530\n"));
    assert!(
        memory
            .file(ACTIVITY)
            .starts_with("2026-10-04 13:01:02 Run #7 started:")
    );

    let mut second = Record::new(memory.clone());
    assert_eq!(second.stamp(), "20261004T130102+0530");
    second.begin(Begin::ChildRun(
        "ignored-after-lazy-query",
        crate::logs::CommandKind::Issue,
    ));
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
    let run_path = run.command_log_path().unwrap();
    let run_path = run_path
        .strip_prefix("/logs/acme/widgets")
        .unwrap()
        .display();
    let pickup_path = pickup.command_log_path().unwrap();
    let pickup_path = pickup_path
        .strip_prefix("/logs/acme/widgets")
        .unwrap()
        .display();
    assert_eq!(
        memory.file(ACTIVITY),
        format!(
            "2026-10-03 12:00:00 Pickup run skipped: idle\n2026-10-03 12:00:00 Architect run skipped: a plan is open\n2026-10-03 12:00:00 Run #7 started: {run_path}, on claude · opus · high\n2026-10-03 12:00:00 Run #7 ended: ready\n2026-10-03 12:00:00 Pickup run skipped: at the Claim limit\n2026-10-03 12:00:00 Pickup run skipped: idle\n2026-10-03 12:00:00 Pickup run #7 started: {pickup_path}, on claude · opus · high\n2026-10-03 12:00:00 Pickup run skipped: idle\n2026-10-03 12:00:00 Pickup run #7 ended: ready\n2026-10-03 12:00:00 Pickup run skipped: idle\n"
        )
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
fn command_creation_failure_warns_once_and_never_adopts_an_existing_path() {
    for existing in [false, true] {
        let memory = Scripted::default();
        {
            let mut files = memory.0.lock().unwrap();
            files.command_open_fails = true;
            if existing {
                files.existing_on_open_failure = Some(PathBuf::new());
            }
        }
        let mut record = Record::new(memory.clone());
        record.begin(Begin::Run(&issue()));
        record.configured(&config(false));
        record.started(Work::Run(&issue()), &harness());
        assert_eq!(record.command_log_path(), None);
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
        if existing {
            let path = memory
                .0
                .lock()
                .unwrap()
                .existing_on_open_failure
                .clone()
                .unwrap();
            assert_eq!(memory.file(&path), "previous content\n");
            assert_eq!(owner::record_log(&record.record_path().unwrap()), path);
        }
        let named = "no Command log";
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
    let path = record.command_log_path().unwrap();
    assert_eq!(memory.file(&path), "thi");
    assert_eq!(
        memory
            .terminal()
            .matches("warning: could not keep the Command log:")
            .count(),
        1
    );
    assert!(memory.terminal().contains(&format!(
        "warning: could not keep the Command log: could not write {}: Command write failed\n",
        path.display()
    )));
    assert!(memory.file(ACTIVITY).contains(&format!(
        "Run #7 started: {},",
        path.strip_prefix("/logs/acme/widgets").unwrap().display()
    )));
    assert!(memory.file(ACTIVITY).ends_with("Run #7 ended: done\n"));
}

#[test]
fn an_activity_warning_can_fail_the_command_append_without_recursion_and_each_warns_once() {
    let memory = Scripted::default();
    let mut record = Record::new(memory.clone());
    record.begin(Begin::Run(&issue()));
    record.configured(&config(false));
    record.started(Work::Run(&issue()), &harness());
    let path = record.command_log_path().unwrap();
    let command_before = memory.file(&path);
    {
        let mut faults = memory.0.lock().unwrap();
        faults.command_bytes_left = Some(0);
        faults.activity_write_fails = true;
    }
    record.ended("failed");
    record.ended("failed again");
    record.eprint("carried on");
    let warning = format!(
        "thirdshift: 12:00:00 warning: could not keep the Activity log: Activity write failed\nthirdshift: 12:00:00 warning: could not keep the Command log: could not write {}: Command write failed\ncarried on\n",
        path.display()
    );
    assert!(memory.terminal().ends_with(&warning));
    assert_eq!(memory.terminal().matches("warning:").count(), 2);
    assert_eq!(memory.file(&path), command_before);
    assert_eq!(record.command_log_path(), Some(path.clone()));
    {
        let mut faults = memory.0.lock().unwrap();
        faults.command_bytes_left = None;
        faults.activity_write_fails = false;
    }
    record.ended("recovered");
    record.print("result");
    assert_eq!(memory.file(&path), command_before);
    assert_eq!(
        memory.file(ACTIVITY),
        format!(
            "2026-10-03 12:00:00 Run #7 started: {}, on claude · opus · high\n2026-10-03 12:00:00 Run #7 ended: recovered\n",
            path.strip_prefix("/logs/acme/widgets").unwrap().display()
        )
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
                let path = record.command_log_path();
                let created = path.as_ref().map(|path| memory.file(path));
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
                assert_eq!(record.command_log_path(), path);
                if command_failure == "open" {
                    assert!(path.is_none());
                } else if command_failure != "none" {
                    assert_eq!(Some(memory.file(path.as_ref().unwrap())), created);
                } else {
                    let bytes = memory.file(path.as_ref().unwrap());
                    assert!(bytes.starts_with("thirdshift: 12:00:00 starting on https://github.com/acme/widgets/issues/7, 2026-10-03 -0400\n"));
                    assert!(bytes.ends_with("ordinary output\n"));
                    assert_eq!(
                        bytes.matches("warning:").count(),
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
    let path = record.lock().unwrap().command_log_path().unwrap();
    assert_eq!(memory.file(&path), output);
    assert_eq!(output.lines().count(), 35);
    assert!(output.contains(&format!("thirdshift: 12:00:00 logging this command to {}\nthirdshift: 12:00:00 sessions run on claude · opus · high\n", path.display())));
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
