//! The fake `gh`, `claude`, `codex`, `agy`, `grok` and `muse`, built from
//! `tests/fakes.rs` with plain `rustc` the first time a test needs them and
//! kept in Cargo's scratch directory for tests. They aren't a binary target,
//! so neither the crate nor a release ships them.

use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// The fakes' sources. They name the built executable, and including them
/// makes Cargo rebuild the tests when they change.
const SOURCES: [&str; 10] = [
    include_str!("../fakes.rs"),
    include_str!("../fakes/gh.rs"),
    include_str!("../fakes/claude.rs"),
    include_str!("../fakes/codex.rs"),
    include_str!("../fakes/agy.rs"),
    include_str!("../fakes/muse.rs"),
    include_str!("../fakes/json.rs"),
    include_str!("../fakes/grok.rs"),
    include_str!("../fixtures/grok-models.json"),
    include_str!("../fixtures/grok-models.txt"),
];

/// Put the fake `gh`, `claude`, `codex`, `agy`, `grok` and `muse` in `bin`.
pub fn install(bin: &Path) {
    for name in [
        "gh",
        "claude",
        "codex",
        "agy",
        "grok",
        "muse",
        "detached-command",
    ] {
        std::os::unix::fs::symlink(executable(), bin.join(name)).unwrap();
    }
}

fn executable() -> &'static Path {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(build)
}

/// Build the fakes, unless a test process has already built these sources.
fn build() -> PathBuf {
    let mut hasher = DefaultHasher::new();
    SOURCES.hash(&mut hasher);
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let built = dir.join(format!("fakes-{:016x}", hasher.finish()));
    if built.exists() {
        return built;
    }
    // Test processes can get here at once (nextest runs each test in its
    // own), so each builds under its own name and renames the result.
    let partial = dir.join(format!("fakes-{}.partial", std::process::id()));
    let output = Command::new(std::env::var_os("RUSTC").unwrap_or("rustc".into()))
        .args(["--edition", "2024", "--crate-name", "fakes"])
        .args(["-C", "debuginfo=0", "-o"])
        .arg(&partial)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fakes.rs"))
        .output()
        .expect("could not run rustc to build the fakes");
    assert!(
        output.status.success(),
        "building the fakes failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::rename(&partial, &built).unwrap();
    built
}
