#!/usr/bin/env python3
"""Materialize a mechanical Console/core split of one pinned production commit."""
import argparse
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile


BASE = "9283d0b8379aa9cf5b645bfe95699813846cb3a0"
REPO = Path(__file__).resolve().parents[2]


def between(text, start, end):
    """Select an exact, uniquely anchored region of the pinned source."""
    assert text.count(start) == 1, start
    assert text.count(end) == 1, end
    return text[text.index(start):text.index(end)]


def split(tree):
    src = tree / "src"
    main = (src / "main.rs").read_text()
    declarations = main[:main.index('#[cfg(target_os = "linux")]\nuse std::process::ExitCode;')]
    core = declarations
    for name in ["args", "console"]:
        core = core.replace(f'#[cfg(target_os = "linux")]\nmod {name};\n', "")
    core = core.replace('#[cfg(not(target_os = "linux"))]\nfn main() {}\n', "")
    core = re.sub(r"^mod (\w+);", r"pub mod \1;", core, flags=re.M)
    core = core.replace('#[cfg(test)]\n#[cfg(target_os = "linux")]\npub mod test_support;',
                        '#[cfg(any(test, feature = "test-support"))]\n#[cfg(target_os = "linux")]\npub mod test_support;')
    core += ('\n#[cfg(target_os = "linux")]\n#[path = "core_args.rs"]\npub mod args;\n'
             '#[cfg(target_os = "linux")]\npub mod console_contract;\n')
    core = ('// The prototype exposes existing constructors mechanically; adding Default\n'
            '// implementations would redesign the production interface.\n'
            '#![allow(clippy::new_without_default)]\n' + core)
    (src / "core.rs").write_text(core)
    # The binary imports core modules at its root, preserving Console's crate:: paths.
    binary = declarations
    for name in re.findall(r"^mod (\w+);", declarations, re.M):
        if name not in {"args", "console"}:
            binary = binary.replace(f"mod {name};", f"pub use thirdshift_core::{name};")
    binary = binary.replace('#[cfg(test)]\n#[cfg(target_os = "linux")]\npub use thirdshift_core::prompts_page;\n', "")
    binary += main[len(declarations):]

    args = (src / "args.rs").read_text()
    regions = [
        between(args, "/// What `thirdshift process` was asked to do.", "/// What thirdshift was asked to do."),
        between(args, "#[derive(Default)]\npub struct MetricsArgs", "/// The flag that stops an Architect run"),
        between(args, "/// Retry the original Issue URL", "/// Parse the arguments after the program name."),
        between(args, "pub(crate) fn fix_retry", "fn parse_process"),
        between(args, "/// One path component:", "/// Parse the arguments after `architect`"),
        between(args, "/// Is `arg`, after `email`", "#[cfg(test)]\nmod tests"),
    ]
    header = """//! Shared command values and retry rendering; parsing stays in the binary.
use crate::asks::Flags;
use crate::base_fix::BaseFixAsk;
use crate::harness;
use crate::issue::{IssueInput, IssueUrl};
use crate::notification::NotificationAsk;
use crate::run::Goal;
const BASE_FIX: &str = "base-fix";
"""
    (src / "core_args.rs").write_text(header + "\n".join(regions))
    for region in regions:
        args = args.replace(region, "")
    args = args.replace("use crate::issue::{IssueInput, IssueUrl};", "use crate::issue::IssueInput;\n#[cfg(test)]\nuse crate::issue::IssueUrl;")
    args = args.replace("use crate::child_run::{self, Given};", "use crate::child_run::{self, Given};\npub use thirdshift_core::args::*;")
    # The parity inventory reads the moved ProcessAction definition too.
    parity = src / "console/parity.rs"
    parity.write_text(parity.read_text().replace('include_str!("../args.rs"), "ProcessAction"',
                                                'include_str!("../core_args.rs"), "ProcessAction"'))
    (src / "args.rs").write_text(args)

    console = (src / "console.rs").read_text()
    scope = between(console, "#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]",
                    "#[derive(Clone, Copy)]\nenum View")
    fleet_path = src / "console/fleet.rs"
    fleet = fleet_path.read_text()
    pairing = between(fleet, "pub(crate) fn pairing_address", "#[derive(Deserialize)]\npub(super) struct Whoami")
    (src / "console_contract.rs").write_text(
        "//! Values and address checks shared with the Console.\n"
        "use anyhow::{Context, Result, ensure};\nuse crate::config::console::fleet;\n" + scope + pairing)
    (src / "console.rs").write_text(console.replace(scope, "pub(crate) use thirdshift_core::console_contract::Scope;\n\n"))
    fleet_path.write_text(fleet.replace(pairing, "use thirdshift_core::console_contract::pairing_address;\n\n"))

    # These tests cross the parser/child-run seam; keep them on the binary side.
    child = (src / "child_run.rs").read_text()
    tests = child[child.index("#[cfg(test)]\nmod tests {"):]
    tests = tests.replace('#[cfg(test)]', '#[cfg(all(test, target_os = "linux"))]', 1)
    child = child[:child.index("#[cfg(test)]\nmod tests {")]
    child = child.replace("fn start_from(", "pub fn start_from(")
    child = child.replace("    fn to_args(", "    pub fn to_args(")
    (src / "child_run.rs").write_text(child)
    tests = tests.replace("    use super::*;", """    use thirdshift_core::child_run::*;
    use crate::base_fix::BaseFixAsk;
    use crate::harness::{Choice, ChosenBy, Harness};
    use crate::issue::IssueUrl;
    use crate::logs;
    use std::path::Path;
    use anyhow::Result;""", 1)
    binary += "\n" + tests
    (src / "main.rs").write_text(binary)

    core_files = [p for p in src.rglob("*.rs")
                  if p not in {src / "main.rs", src / "args.rs", src / "console.rs"}
                  and p.relative_to(src).parts[0] != "console"]
    # config/console belongs to core, while src/console belongs to the binary.
    for path in core_files:
        text = path.read_text()
        text = text.replace("pub(crate)", "pub")
        text = text.replace("crate::console::Scope", "crate::console_contract::Scope")
        text = text.replace("crate::console::fleet::pairing_address", "crate::console_contract::pairing_address")
        # Sources stay in src/, so include_str! relative paths need no changes.
        text = text.replace('env!("CARGO_MANIFEST_DIR")', 'concat!(env!("CARGO_MANIFEST_DIR"), "/..")')
        text = text.replace('$CARGO_MANIFEST_DIR/', '$CARGO_MANIFEST_DIR/../')
        path.write_text(text)

    manifest = (tree / "Cargo.toml").read_text()
    dependencies = between(manifest, "[dependencies]", "[lints.clippy]")
    (tree / "core").mkdir()
    lints = between(manifest, "[lints.clippy]", "# Line tables")
    repository = re.search(r"^repository = .+$", manifest, re.M).group()
    (tree / "core/Cargo.toml").write_text(
        '[package]\nname = "thirdshift-core"\nversion = "0.24.1"\nedition = "2024"\n' + repository + '\n'
        '[lib]\npath = "../src/core.rs"\n[features]\ntest-support = []\n' + dependencies + lints)
    manifest = manifest.replace("[dependencies]", '[dependencies]\nthirdshift-core = { path = "core" }', 1)
    manifest = manifest.replace("[dev-dependencies]", '[dev-dependencies]\nthirdshift-core = { path = "core", features = ["test-support"] }', 1)
    manifest += '\n[workspace]\nmembers = ["core"]\ndefault-members = [".", "core"]\nresolver = "3"\n'
    (tree / "Cargo.toml").write_text(manifest)
    # No dependency versions change: add just the local package and edge to the lockfile.
    lock_path = tree / "Cargo.lock"
    lock = lock_path.read_text()
    package = re.search(r'\[\[package\]\]\nname = "thirdshift"\n.*?(?=\[\[package\]\])', lock, re.S).group()
    core_package = package.replace('name = "thirdshift"', 'name = "thirdshift-core"', 1)
    binary_package = package.replace('dependencies = [\n', 'dependencies = [\n "thirdshift-core",\n', 1)
    lock_path.write_text(lock.replace(package, binary_package + core_package))
    subprocess.run(["cargo", "fmt", "--all"], cwd=tree, check=True)


def prepare(destination):
    destination = Path(destination).resolve()
    if destination.exists() and (not destination.is_dir() or any(destination.iterdir())):
        raise ValueError("destination must be empty")
    destination.mkdir(parents=True, exist_ok=True)
    # git archive includes committed sources only, never this worktree's edits.
    with tempfile.TemporaryFile() as archive:
        subprocess.run(["git", "archive", "--format=tar", BASE], cwd=REPO, stdout=archive, check=True)
        archive.seek(0)
        with tarfile.open(fileobj=archive) as contents:
            contents.extractall(destination, filter="data")
    split(destination)
    print(destination)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    prepare(parser.parse_args().destination)
