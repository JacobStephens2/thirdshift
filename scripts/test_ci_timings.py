"""Exercise timing reports and coverage accounting through their CLI."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import re
import textwrap
import unittest


SCRIPT = Path(__file__).with_name("ci-timings.py")


class CiTimings(unittest.TestCase):
    def test_actual_workflow_gates_reject_failed_cancelled_skipped_or_missing_work(self):
        from test_workflow_timeouts import ROOT, job_blocks

        gates = [job_blocks(ROOT / ".github/workflows/ci.yml")[name]
                 for name in ["test", "test-linux-arm"]]
        gates.append(job_blocks(ROOT / ".github/workflows/rust-tests.yml")["complete"])
        for gate in gates:
            command = re.search(r'run: (test "\$RESULT" = success)', gate)[1]
            for status in ["success", "failure", "cancelled", "skipped", ""]:
                with self.subTest(gate=command, result=status):
                    result = subprocess.run(
                        ["bash", "-e", "-c", textwrap.dedent(command)],
                        env=dict(os.environ, RESULT=status), timeout=10)
                    self.assertEqual(result.returncode == 0, status == "success")

    def test_partition_union_and_execution_must_match_current_inventory(self):
        def listing(names, selected):
            return {"rust-suites": {"thirdshift": {"testcases": {
                name: {"ignored": name == "ignored", "filter-match": {
                    "status": "matches" if name in selected else "mismatch"}}
                for name in names}}}}

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            shards = [root / "1", root / "2"]
            for shard, name in zip(shards, ["unit", "integration"]):
                shard.mkdir()
                (shard / "full.json").write_text(json.dumps(
                    listing(["unit", "integration", "ignored"], ["unit", "integration"])))
                (shard / "partition.json").write_text(json.dumps(
                    listing(["unit", "integration", "ignored"], [name])))
                (shard / "junit.xml").write_text(
                    f'<testsuites><testsuite name="thirdshift">'
                    f'<testcase name="{name}" time="1"/></testsuite></testsuites>')

            def run(paths=shards):
                return subprocess.run(
                    ["python3", str(SCRIPT), "verify", "--count", "2", *map(str, paths)],
                    capture_output=True, text=True, timeout=10)

            self.assertEqual(run().returncode, 0, run().stderr)
            self.assertNotEqual(run(shards[:1]).returncode, 0)
            (shards[1] / "partition.json").write_text((shards[0] / "partition.json").read_text())
            self.assertNotEqual(run().returncode, 0, "duplicate assignment must fail")
            (shards[1] / "partition.json").write_text(json.dumps(
                listing(["unit", "integration", "ignored"], ["integration"])))
            (shards[1] / "junit.xml").unlink()
            self.assertNotEqual(run().returncode, 0, "missing execution must fail")

    def test_summary_preserves_passing_test_times_without_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "junit.xml").write_text(
                '<testsuites><testsuite name="thirdshift">'
                '<testcase name="fast" time="1.25"/>'
                '<testcase name="slow" time="4.5"><failure>failed</failure></testcase>'
                '</testsuite><testsuite name="integration">'
                '<testcase name="run" time="2"/></testsuite></testsuites>'
            )
            result = subprocess.run(
                ["python3", str(SCRIPT), "summary", str(root)],
                capture_output=True, text=True, timeout=10,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            report = json.loads((root / "summary.json").read_text())
            self.assertEqual(report["test_seconds"], 7.75)
            self.assertEqual(report["binaries"], {"thirdshift": 5.75, "integration": 2.0})
            self.assertEqual(report["slowest"][0], ["thirdshift", "slow", 4.5])
            self.assertIn("7.750", result.stdout)


if __name__ == "__main__":
    unittest.main()
