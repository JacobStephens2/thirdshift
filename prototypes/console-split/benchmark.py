#!/usr/bin/env python3
"""Validate and time the pinned Base commit versus the Console/core split."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import time

from prepare import BASE, REPO, export_base, prepare


EDITS = {"console": "src/console/commands.rs", "ticket": "src/run.rs", "logs": "src/logs.rs"}


def capture(command, cwd=REPO, env=None):
    return subprocess.check_output(command, cwd=cwd, env=env, text=True, stderr=subprocess.STDOUT).strip()


class Runner:
    def __init__(self, output, jobs):
        self.output = output
        self.env = os.environ.copy()
        # A wrapper, inherited flags or profile overrides would change the experiment.
        for key in list(self.env):
            if key.startswith(("CARGO_", "RUST")):
                self.env.pop(key)
        self.env.update(CARGO_BUILD_JOBS=str(jobs), CARGO_INCREMENTAL="1")
        self.rows = []

    def command(self, tree, target, label, argv, measured=False):
        env = self.env | {"CARGO_TARGET_DIR": str(target)}
        log_path = self.output / (label + ".log")
        print(f"{label}: {' '.join(argv)} -> {log_path}", flush=True)
        started = time.perf_counter()
        with log_path.open("w") as log:
            result = subprocess.run(argv, cwd=tree, env=env, stdout=log, stderr=subprocess.STDOUT)
        seconds = time.perf_counter() - started
        output = log_path.read_text()
        row = {"label": label, "command": argv, "seconds": seconds,
               "exit_code": result.returncode, "measured": measured,
               "log": log_path.name, "sha256": hashlib.sha256(log_path.read_bytes()).hexdigest(),
               "results": [line for line in output.splitlines() if line.startswith("test result:")],
               "tail": output.splitlines()[-8:]}
        self.rows.append(row)
        (self.output / "progress.json").write_text(json.dumps(self.rows, indent=2) + "\n")
        print(f"{label}: exit {result.returncode}, {seconds:.3f}s", flush=True)
        if result.returncode:
            raise RuntimeError(f"{label} failed; see {log_path}\n{output[-4000:]}")

    def validate(self, tree, target):
        for label, argv in [
            ("build", ["cargo", "build", "--locked"]),
            ("clippy", ["cargo", "clippy", "--all-targets", "--", "-D", "warnings"]),
            ("eval-metadata", ["cargo", "test", "--test", "evals",
                               "replay_import_completes_trial_inputs_and_preserves_local_edits", "--", "--exact"]),
            ("platform", ["cargo", "test", "--test", "packaging",
                          "non_linux_compilation_reports_only_the_platform_error", "--", "--exact"]),
            ("test", ["cargo", "test", "--no-fail-fast"]),
        ]:
            self.command(tree, target, "validation-" + label, argv)

    def edits(self, tree, target, prefix, argv):
        # Restore and rebuild before each location so a measurement invalidates only
        # that location, rather than also invalidating the preceding restored file.
        for location, relative in EDITS.items():
            self.command(tree, target, f"{prefix}-{location}-warm", argv)
            path = tree / relative
            original = path.read_bytes()
            try:
                path.write_bytes(original + b"\n// Console split edit-rebuild measurement.\n")
                self.command(tree, target, f"{prefix}-{location}", argv, measured=True)
            finally:
                path.write_bytes(original)

    def sample(self, tree, scratch, sample, layout):
        prefix = f"{sample}-{layout}"
        target = scratch / "build-target"
        build = ["cargo", "build", "--locked", "--bin", "thirdshift"]
        self.command(tree, target, prefix + "-cold-build", build, measured=True)
        self.edits(tree, target, prefix + "-build", build)
        self.edits(tree, target, prefix + "-check", ["cargo", "check", "--locked", "--bin", "thirdshift"])
        shutil.rmtree(target)
        # Test compilation really is cold, including dependencies; it never reuses
        # the target from the cold binary build, checks, validation or other samples.
        self.command(tree, scratch / "test-target", prefix + "-cold-test",
                     ["cargo", "test", "--locked", "--no-run"], measured=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="new or empty results directory")
    parser.add_argument("--samples", type=int, default=3)
    parser.add_argument("--jobs", type=int, default=os.cpu_count())
    args = parser.parse_args()
    if args.samples < 3 or args.jobs < 1:
        parser.error("at least 3 samples and 1 build job are required")
    output = args.output.resolve()
    if output.exists() and any(output.iterdir()):
        parser.error("output must be empty")
    output.mkdir(parents=True, exist_ok=True)
    runner = Runner(output, args.jobs)
    # Record the actual default linker selection, without changing flags.
    with tempfile.TemporaryDirectory(prefix="console-split-linker-") as probe:
        source = Path(probe) / "probe.rs"
        source.write_text("fn main() {}\n")
        linker = capture(["rustc", str(source), "-o", str(Path(probe) / "probe"),
                          "--print", "link-args"], env=runner.env)
    host = {"platform": platform.platform(), "uname": list(platform.uname()),
            "cpu_count": os.cpu_count(), "cpu": capture(["lscpu"]),
            "rustc": capture(["rustc", "-Vv"], env=runner.env),
            "cargo": capture(["cargo", "-V"], env=runner.env),
            "linker_command": linker, "cc": capture(["cc", "--version"]),
            "environment": {k: v for k, v in runner.env.items() if k.startswith(("CARGO_", "RUST"))}}
    metadata = {"base": BASE, "started": datetime.now(timezone.utc).isoformat(),
                "host": host, "samples": args.samples, "jobs": args.jobs,
                "edits": EDITS, "method": "sequential fresh targets; warm commands excluded; OS page cache retained"}
    (output / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    # Validate once before measuring. This private target is removed before timing.
    with tempfile.TemporaryDirectory(prefix="console-split-validate-") as tmp:
        scratch = Path(tmp)
        prepare(scratch / "tree")
        runner.validate(scratch / "tree", scratch / "target")
    for sample in range(1, args.samples + 1):
        # Alternate which layout goes first; compilation never runs concurrently.
        for layout in (["base", "split"] if sample % 2 else ["split", "base"]):
            with tempfile.TemporaryDirectory(prefix="console-split-sample-") as tmp:
                scratch = Path(tmp)
                tree = scratch / "tree"
                if layout == "split":
                    prepare(tree)
                else:
                    export_base(tree)
                # Download and registry setup are excluded from the timed regions.
                runner.command(tree, scratch / "fetch-target", f"{sample}-{layout}-fetch", ["cargo", "fetch", "--locked"])
                runner.sample(tree, scratch, sample, layout)
    metadata["finished"] = datetime.now(timezone.utc).isoformat()
    metadata["runs"] = runner.rows
    (output / "results.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print(output / "results.json", flush=True)


if __name__ == "__main__":
    main()
