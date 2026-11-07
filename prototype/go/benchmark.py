#!/usr/bin/env python3
"""Measure the committed Go vertical slice and its Rust owners in fresh worktrees.

Requires Linux, Python 3, Git, Go, Cargo, the native Rust musl target/linker,
and readelf. Dependency downloads are prepared outside timed regions.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import shlex
import subprocess
import tempfile
import time


RUST_TESTS = ["fresh_run", "pickup_run", "harness", "ci", "pickup_model"]
RUST_OWNERS = ["main", "run", "delivery", "pickup", "ready", "reservation",
               "claim", "process", "session", "harness", "ci", "pull_request", "interrupt"]


def capture(command, cwd, env=None):
    return subprocess.check_output(command, cwd=cwd, env=env, text=True, stderr=subprocess.STDOUT).strip()


def run_logged(command, cwd, env, log):
    try:
        subprocess.run(command, cwd=cwd, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    except subprocess.CalledProcessError as error:
        log.flush()
        error.output = Path(log.name).read_text()[-4000:]
        raise


def source_lines(worktree):
    go = sorted((worktree / "prototype/go").glob("*.go"))
    rust = []
    for owner in RUST_OWNERS:
        rust.extend((worktree / "src").glob(owner + ".rs"))
        rust.extend((worktree / "src" / owner).rglob("*.rs"))
    return {
        language: {"lines": sum(len(path.read_text().splitlines()) for path in paths),
                   "files": [str(path.relative_to(worktree)) for path in paths]}
        for language, paths in {
            "go": [path for path in go if not path.name.endswith("_test.go")],
            "rust": sorted(path for path in rust if not path.stem.endswith(("test", "tests", "test_support", "testing"))),
        }.items()
    }


class Sample:
    def __init__(self, worktree, output, index, jobs, target, scratch):
        self.worktree, self.output, self.index = worktree, output, index
        self.jobs, self.target, self.scratch = jobs, target, scratch
        self.env = os.environ.copy()
        for name in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_TARGET",
                     "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "GOFLAGS", "GOOS", "GOARCH"]:
            self.env.pop(name, None)
        self.env.update({"CGO_ENABLED": "0", "GOMAXPROCS": str(jobs), "GOFLAGS": "-p=" + str(jobs),
                         "GOCACHE": str(scratch / "go-cache"), "GOMODCACHE": str(scratch / "go-mod"),
                         "CARGO_TARGET_DIR": str(scratch / "rust-target"), "CARGO_BUILD_JOBS": str(jobs),
                         "CARGO_PROFILE_RELEASE_STRIP": "symbols"})
        self.rows = {language: {"seconds": {}, "commands": []} for language in ["go", "rust"]}

    def measure(self, language, metric, commands):
        row = self.rows[language]
        log_path = self.output / f"{self.index}-{language}-{metric}.log"
        cwd = self.worktree / "prototype/go" if language == "go" else self.worktree
        print(f"sample {self.index}: {language} {metric} -> {log_path}", flush=True)
        started = time.perf_counter()
        with log_path.open("w") as log:
            for command in commands:
                row["commands"].append({"metric": metric, "argv": list(map(str, command)),
                                         "cwd": str(cwd), "log": log_path.name})
                log.write("$ " + shlex.join(map(str, command)) + "\n")
                log.flush()
                run_logged(command, cwd, self.env, log)
        row["seconds"][metric] = time.perf_counter() - started
        return log_path

    def verify_static(self, language, binary):
        headers = capture(["readelf", "-lW", str(binary)], self.worktree, self.env)
        dynamic = capture(["readelf", "-dW", str(binary)], self.worktree, self.env)
        if "INTERP" in headers or "(NEEDED)" in dynamic:
            raise ValueError(f"{language} binary is not static: {binary}")
        (self.output / f"{self.index}-{language}-static.log").write_text(headers + "\n" + dynamic + "\n")
        self.rows[language]["static_bytes"] = binary.stat().st_size

    def run(self):
        go_binary = self.scratch / "thirdshift-go"
        go_build = ["go", "build", "-trimpath", "-o", str(go_binary), "."]
        self.measure("go", "cold_build", [go_build])
        rust_build = ["cargo", "build", "--locked", "--bin", "thirdshift"]
        self.measure("rust", "cold_build", [rust_build])
        # A content edit forces package recompilation; an unchanged build is a
        # cache-hit benchmark instead. Only this disposable worktree is edited.
        for language, source, command in [("go", "prototype/go/main.go", go_build),
                                           ("rust", "src/main.rs", rust_build)]:
            path = self.worktree / source
            original = path.read_bytes()
            path.write_bytes(original + b"\n// benchmark: incremental source edit\n")
            try:
                self.measure(language, "incremental_build", [command])
            finally:
                path.write_bytes(original)

        go_tests = self.scratch / "go-tests"
        self.measure("go", "test_compile", [["go", "test", "-c", "-o", str(go_tests), "."]])
        # Exclude the runner/report contract to prevent recursive benchmarking.
        self.measure("go", "test_execution", [[str(go_tests), "-test.skip=^TestBenchmark", "-test.count=1",
                                                  f"-test.parallel={self.jobs}"]])
        self.measure("go", "model_check", [[str(go_tests), "-test.run=^TestPickupModel", "-test.count=1", "-test.v"]])
        test_command = ["cargo", "test", "--locked", "--no-run", "--message-format=json"]
        for name in RUST_TESTS:
            test_command += ["--test", name]
        compiled = self.measure("rust", "test_compile", [test_command])
        executables = {}
        for line in compiled.read_text().splitlines():
            if line.startswith("{"):
                message = json.loads(line)
                if message.get("reason") == "compiler-artifact" and message.get("executable") and message.get("profile", {}).get("test"):
                    executables[message["target"]["name"]] = message["executable"]
        if set(executables) != set(RUST_TESTS):
            raise ValueError(f"expected Rust test targets {RUST_TESTS}, got {sorted(executables)}")
        self.measure("rust", "test_execution", [[executables[name], f"--test-threads={self.jobs}"] for name in RUST_TESTS])
        self.measure("rust", "model_check", [[executables["pickup_model"], "--test-threads=1", "--nocapture"]])
        self.measure("go", "static_build", [["go", "build", "-trimpath", "-ldflags=-s -w", "-o", str(go_binary), "."]])
        self.verify_static("go", go_binary)
        self.measure("rust", "static_build", [["cargo", "build", "--locked", "--release", "--target", self.target, "--bin", "thirdshift"]])
        self.verify_static("rust", self.scratch / "rust-target" / self.target / "release/thirdshift")
        return self.rows


def positive(value):
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("must be positive")
    return number


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--ref", default="HEAD", help="committed revision to measure (uncommitted edits excluded)")
    parser.add_argument("--output", type=Path, required=True, help="new or empty results directory")
    parser.add_argument("--samples", type=positive, default=3)
    parser.add_argument("--jobs", type=positive, default=2)
    args = parser.parse_args()
    repo = args.repo.resolve()
    revision = capture(["git", "rev-parse", "--verify", args.ref + "^{commit}"], repo)
    host = capture(["rustc", "-vV"], repo).split("host: ")[-1].splitlines()[0]
    target = host.replace("-gnu", "-musl")
    if platform.system() != "Linux" or target == host:
        raise ValueError("run on native GNU/Linux with its Rust musl target installed")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if any(output.iterdir()):
        raise ValueError("output directory must be empty")
    data = {"revision": revision, "timestamp_utc": datetime.now(timezone.utc).isoformat(),
            "host": platform.platform(), "cpu_count": os.cpu_count(), "load_average": os.getloadavg(),
            "jobs": args.jobs, "rust_static_target": target,
            "tools": {name: capture(command, repo) for name, command in {
                "go": ["go", "version"], "rustc": ["rustc", "--version"], "cargo": ["cargo", "--version"],
                "git": ["git", "--version"], "readelf": ["readelf", "--version"]}.items()},
            "samples": []}
    cpuinfo = Path("/proc/cpuinfo").read_text()
    data["cpu"] = next((line.split(":", 1)[1].strip() for line in cpuinfo.splitlines() if line.startswith("model name")), platform.machine())
    for index in range(1, args.samples + 1):
        with tempfile.TemporaryDirectory(prefix="thirdshift-benchmark-") as temp:
            scratch = Path(temp)
            worktree = scratch / "worktree"
            subprocess.run(["git", "worktree", "add", "--detach", str(worktree), revision], cwd=repo, check=True,
                           stdout=subprocess.DEVNULL)
            try:
                if index == 1:
                    data["source_lines"] = source_lines(worktree)
                    data["input_sha256"] = {path: hashlib.sha256((worktree / path).read_bytes()).hexdigest()
                                             for path in ["Cargo.lock", "prototype/go/go.mod", "prototype/go/factory-skills.json"]
                                             if (worktree / path).exists()}
                    # Downloads do not contaminate cold compiler timings.
                    with (output / "dependency-fetch.log").open("w") as log:
                        run_logged(["cargo", "fetch", "--locked"], worktree, None, log)
                sample = Sample(worktree, output, index, args.jobs, target, scratch)
                data["samples"].append(sample.run())
            finally:
                subprocess.run(["git", "worktree", "remove", "--force", str(worktree)], cwd=repo, check=True)
    (output / "results.json").write_text(json.dumps(data, indent=2) + "\n")
    print(f"completed: {output / 'results.json'}", flush=True)


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        raise SystemExit(error.output or f"command failed: {error.cmd}; see output logs")
    except (OSError, ValueError) as error:
        raise SystemExit(str(error))
