//! Local filesystem observations; real child execution needs no user manager.

use super::*;
use std::cell::Cell;
use std::rc::Rc;

thread_local! {
    static GRADER: RefCell<Option<Rc<Fixture>>> = const { RefCell::new(None) };
}

/// Exercise a caller's complete Session interface with local kernel evidence.
/// The override belongs only to this thread and is restored even on panic.
pub(crate) fn with_grader<T>(fixture: &Rc<Fixture>, exercise: impl FnOnce() -> T) -> T {
    struct Restore(Option<Rc<Fixture>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            GRADER.with(|slot| slot.replace(self.0.take()));
        }
    }
    let _restore = Restore(GRADER.with(|slot| slot.replace(Some(Rc::clone(fixture)))));
    exercise()
}

pub(super) fn grader_command(program: &Path, args: &[String], directory: &Path) -> Option<Launch> {
    GRADER.with(|slot| {
        let fixture = slot.borrow();
        let fixture = fixture.as_ref()?;
        let mut command = Command::new(program);
        command.args(args).current_dir(directory);
        Some(fixture.launch(command))
    })
}

pub(crate) struct Fixture {
    pub directory: tempfile::TempDir,
    journal_reads: Cell<usize>,
    pub journal_kill: Cell<bool>,
}

impl Fixture {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            directory: tempfile::tempdir().unwrap(),
            journal_reads: Cell::new(0),
            journal_kill: Cell::new(false),
        })
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.directory.path().join(name)
    }

    pub fn launch(self: &Rc<Self>, mut command: Command) -> Launch {
        command
            .env("EVENTS", self.path("events"))
            .env("OBSERVED", self.path("observed"));
        Launch {
            command,
            scope: Scope {
                unit: "fixture.scope".into(),
                watch: RefCell::new(None),
                observation: Box::new(Rc::clone(self)),
            },
        }
    }

    pub fn journal_reads(&self) -> usize {
        self.journal_reads.get()
    }
}

impl Observation for Rc<Fixture> {
    fn events(&self, _unit: &str) -> Option<PathBuf> {
        Some(self.path("events"))
    }

    fn acknowledge(&self) {
        let _ = std::fs::write(self.path("observed"), "observed");
    }

    fn journal_oom_killed(&self, _unit: &str) -> bool {
        self.journal_reads.set(self.journal_reads.get() + 1);
        self.journal_kill.get()
    }
}

/// The child cannot delete the evidence until the supervisor has read it.
/// A finite handshake makes a missing observer fail instead of hanging tests.
pub(crate) const REMOVE_OBSERVED_EVENTS: &str = r#"
for attempt in $(seq 1 200); do
    if [ -f "$OBSERVED" ]; then rm "$EVENTS"; exit 137; fi
    sleep 0.01
done
echo 'memory observation was never acknowledged' >&2
exit 99
"#;
