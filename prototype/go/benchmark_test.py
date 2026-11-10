"""Contract tests at the benchmark CLI/tool and published-report seams."""
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest


RUNNER = Path(__file__).with_name("benchmark.py")


class BenchmarkCLI(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="benchmark-contract-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.env = dict(os.environ, GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_NOSYSTEM="1")
        for args in [("init", "-q"), ("config", "user.name", "Test"),
                     ("config", "user.email", "test@example.com")]:
            self.git(*args)
        for name in ["prototype/go/main.go", "src/main.rs", "tests/pickup_model.rs"]:
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("// fixture\n")
        self.git("add", ".")
        self.git("-c", "core.hooksPath=/dev/null", "commit", "-qm", "fixture")
        self.tools = self.root / "tools"
        self.tools.mkdir()
        # Only external tool processes are faked. Git worktrees, logs, timings,
        # source counting, serialization and cleanup run for real.
        tool = self.tools / "tool"
        tool.write_text('''#!/usr/bin/env python3
import json, os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ["BENCH_CALLS"], "a") as out:
    out.write(json.dumps({"tool": name, "args": args, "cwd": os.getcwd(),
        "gocache": os.environ.get("GOCACHE"), "target": os.environ.get("CARGO_TARGET_DIR")}) + "\\n")
if "--version" in args or args == ["version"] or args == ["-vV"]:
    print("host: x86_64-unknown-linux-gnu" if args == ["-vV"] else "fixture 1.0")
elif os.environ.get("BENCH_FAIL") == name and "build" in args:
    print("intentional tool failure", file=sys.stderr)
    sys.exit(19)
elif name == "readelf":
    print("INTERP" if os.environ.get("BENCH_DYNAMIC") else "ELF fixture; no dynamic section")
elif name == "go":
    output = pathlib.Path(args[args.index("-o") + 1])
    output.write_text("#!/bin/sh\\nexit 0\\n")
    output.chmod(0o755)
elif name == "cargo":
    if "fetch" in args:
        sys.exit(0)
    target = pathlib.Path(os.environ["CARGO_TARGET_DIR"])
    if "test" in args:
        for test in ["fresh_run", "pickup_run", "harness", "ci", "pickup_model"]:
            output = target / "debug" / test
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text("#!/bin/sh\\nexit 0\\n")
            output.chmod(0o755)
            print(json.dumps({"reason": "compiler-artifact", "target": {"name": test},
                "profile": {"test": True}, "executable": str(output)}))
    elif "--release" in args:
        output = target / args[args.index("--target") + 1] / "release" / "thirdshift"
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_bytes(b"static fixture")
''')
        tool.chmod(0o755)
        for name in ["go", "cargo", "rustc", "readelf"]:
            (self.tools / name).symlink_to(tool)
        self.calls = self.root / "calls.jsonl"
        self.env.update(PATH=str(self.tools) + ":" + os.environ["PATH"], BENCH_CALLS=str(self.calls))

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.repo), *args], env=self.env, text=True)

    def run_benchmark(self, **environment):
        return subprocess.run(["python3", str(RUNNER), "--repo", str(self.repo),
                               "--samples", "2", "--output", str(self.root / "results")],
                              env=dict(self.env, **environment), text=True, capture_output=True)

    def test_runner_records_both_languages_and_removes_worktrees(self):
        result = self.run_benchmark()
        self.assertEqual(result.returncode, 0, result.stderr)
        data = json.loads((self.root / "results/results.json").read_text())
        self.assertEqual(len(data["samples"]), 2)
        for sample in data["samples"]:
            for language in ["go", "rust"]:
                row = sample[language]
                for metric in ["cold_build", "incremental_build", "test_compile", "test_execution", "model_check", "static_build"]:
                    self.assertGreaterEqual(row["seconds"][metric], 0)
                self.assertGreater(row["static_bytes"], 0)
        self.assertEqual(data["source_lines"]["go"]["lines"], 1)
        calls = [json.loads(line) for line in self.calls.read_text().splitlines()]
        caches = {call["gocache"] for call in calls if call["tool"] == "go" and "build" in call["args"]}
        self.assertEqual(len(caches), 2, "each sample needs a private cache")
        self.assertTrue(all(str(self.repo) != call["cwd"] for call in calls if call["tool"] == "go" and "build" in call["args"]))
        self.assertEqual(self.git("worktree", "list", "--porcelain").count("worktree "), 1)
        self.assertEqual(self.git("status", "--porcelain"), "")
        self.assertTrue(list((self.root / "results").glob("*.log")))

    def test_tool_failure_is_nonzero_and_cleans_up(self):
        result = self.run_benchmark(BENCH_FAIL="cargo")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("intentional tool failure", result.stderr)
        self.assertFalse((self.root / "results/results.json").exists())
        self.assertEqual(self.git("worktree", "list", "--porcelain").count("worktree "), 1)

    def test_dynamic_binary_is_rejected(self):
        result = self.run_benchmark(BENCH_DYNAMIC="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("static", result.stderr)


class PublishedReport(unittest.TestCase):
    def test_report_is_complete_and_numbers_match_measured_artifact(self):
        root = RUNNER.parents[2]
        report = (root / "docs/research/go-vs-rust-prototype.md").read_text()
        artifact = root / "docs/research/go-vs-rust-benchmark-2026-11-06/results.json"
        data = json.loads(artifact.read_text())
        self.assertIn("go-vs-rust-benchmark-2026-11-06/results.json", report)
        for topic in ["Reproduce", "Scope", "Subprocess", "Exhaustiveness", "Model checking", "ADR-0001"]:
            self.assertIn(topic, report)
        self.assertRegex(data["revision"], r"^[0-9a-f]{40}$")
        self.assertGreaterEqual(len(data["samples"]), 2)
        for metric in ["cold_build", "incremental_build", "test_compile", "test_execution", "model_check", "static_build"]:
            row = re.search(r"^\| " + metric + r" \| (.+) \|$", report, re.MULTILINE)
            self.assertIsNotNone(row, metric)
            for language in ["go", "rust"]:
                for sample in data["samples"]:
                    value = sample[language]["seconds"][metric]
                    self.assertGreater(value, 0)
                    self.assertIn(f"{value:.3f}", row.group(1), metric)
        for language in ["go", "rust"]:
            self.assertIn(str(data["source_lines"][language]["lines"]), report)
            for sample in data["samples"]:
                self.assertIn(str(sample[language]["static_bytes"]), report)


if __name__ == "__main__":
    unittest.main()
