use super::*;
use crate::issue::IssueUrl;
use crate::logs::effects::OnMachine;

const STAMP: &str = "20261003T120000-0400";

#[test]
fn real_files_keep_interleaved_command_output_and_records_separate() {
    let root = tempfile::tempdir().unwrap();
    let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
    let mut files = OnMachine;
    let first = allocate(&mut files, root.path(), Work::Run(&issue), STAMP).unwrap();
    let first_path = first.owner.log_path().to_path_buf();
    let mut first_writer = first.writer;
    first_writer.write_all(b"first before\n").unwrap();
    let second = allocate(&mut files, root.path(), Work::Run(&issue), STAMP).unwrap();
    let mut second_writer = second.writer;
    second_writer.write_all(b"second before\n").unwrap();
    first_writer.write_all(b"first after\n").unwrap();
    second_writer.write_all(b"second after\n").unwrap();
    assert_ne!(first_path, second.owner.log_path());
    assert_ne!(first.owner.record_path(), second.owner.record_path());
    assert_eq!(
        fs::read(first_path).unwrap(),
        b"first before\nfirst after\n"
    );
    assert_eq!(
        fs::read(second.owner.log_path()).unwrap(),
        b"second before\nsecond after\n"
    );
    for owner in [&first.owner, &second.owner] {
        assert_eq!(record_log(&owner.record_path()), owner.log_path());
        let decoded = decode(owner.log_path()).unwrap();
        assert_eq!(decoded.identity.stamp, STAMP);
        assert!(decoded.identity.token.is_some());
        assert_eq!(decoded.subject, "7");
    }
}

#[test]
fn a_forced_collision_preserves_the_existing_file_and_retries_with_a_fresh_token() {
    struct Colliding {
        occupied: Option<PathBuf>,
    }
    impl Files for Colliding {
        type CommandLog = File;
        fn create_command_log(&mut self, path: &Path) -> io::Result<File> {
            if self.occupied.is_none() {
                fs::create_dir_all(path.parent().unwrap())?;
                fs::write(path, b"another Command's output\n")?;
                self.occupied = Some(path.to_path_buf());
            }
            create(path)
        }
    }
    let root = tempfile::tempdir().unwrap();
    let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
    let mut files = Colliding { occupied: None };
    let mut allocated = allocate(&mut files, root.path(), Work::Run(&issue), STAMP).unwrap();
    allocated.writer.write_all(b"new output\n").unwrap();
    let occupied = files.occupied.unwrap();
    assert_ne!(occupied, allocated.owner.log_path());
    assert_eq!(fs::read(occupied).unwrap(), b"another Command's output\n");
    assert_eq!(
        fs::read(allocated.owner.log_path()).unwrap(),
        b"new output\n"
    );
}

#[test]
fn legacy_and_owned_artifacts_share_one_decoder() {
    for (name, subject, token, kind, log) in [
        (
            "7-20261003T120000-0400.log",
            "7",
            None,
            None,
            "7-20261003T120000-0400.log",
        ),
        (
            "7-20261003T120000-0400.123-start-1-9.command.json",
            "7",
            None,
            None,
            "7-20261003T120000-0400.log",
        ),
        (
            "7-20261003T120000-0400~ownerA.command.json",
            "7",
            Some("ownerA"),
            None,
            "7-20261003T120000-0400~ownerA.log",
        ),
        (
            "20261003T120000-0400~ownerB.log",
            "",
            Some("ownerB"),
            None,
            "20261003T120000-0400~ownerB.log",
        ),
        (
            "7-20261003T120000-0400-implement.jsonl",
            "7",
            None,
            Some("implement"),
            "7-20261003T120000-0400.log",
        ),
        (
            "7-20261003T120000-0400~ownerA-repair-1-resume.jsonl",
            "7",
            Some("ownerA"),
            Some("repair-1-resume"),
            "7-20261003T120000-0400~ownerA.log",
        ),
        (
            "architect-20261003T120000-0400~ownerB-architecture-review.jsonl",
            "architect",
            Some("ownerB"),
            Some("architecture-review"),
            "20261003T120000-0400~ownerB.log",
        ),
        (
            "secure-20261003T120000+0530~ownerC-security-review.jsonl",
            "secure",
            Some("ownerC"),
            Some("security-review"),
            "20261003T120000+0530~ownerC.log",
        ),
    ] {
        let path = Path::new(name);
        let decoded = decode(path).unwrap_or_else(|| panic!("{name}"));
        assert_eq!(decoded.subject, subject);
        assert_eq!(decoded.identity.token, token);
        assert_eq!(decoded.session_kind, kind);
        assert_eq!(decoded.command_log(path), Path::new(log));
    }
    for name in [
        "7-bad.log",
        "7-20261003T120000-0400~.log",
        "7-20261003T120000-0400~owner-.log",
        "7-20261003T120000-0400~owner.jsonl",
        "7-20261003T120000-0400-.jsonl",
    ] {
        assert!(decode(Path::new(name)).is_none(), "{name}");
    }
}
