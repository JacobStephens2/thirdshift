"""Run the ARM release smoke shell steps at their external process seam."""
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile
import textwrap
import unittest

from test_workflow_timeouts import ROOT, job_blocks


class ArmLinuxSmoke(unittest.TestCase):
    def test_selects_the_arm_archive_and_build_artifact_from_the_plan(self):
        # Include another architecture and a checksum so selecting the first
        # artifact or every artifact for ARM cannot silently pass.
        plan = {
            "artifacts": {
                "x86.tar.xz": {"kind": "executable-zip", "name": "x86.tar.xz",
                               "target_triples": ["x86_64-unknown-linux-musl"]},
                "arm.tar.xz.sha256": {"kind": "checksum", "name": "arm.tar.xz.sha256",
                                      "target_triples": ["aarch64-unknown-linux-musl"]},
                "arm.tar.xz": {"kind": "executable-zip", "name": "arm.tar.xz",
                               "target_triples": ["aarch64-unknown-linux-musl"]},
            },
            "ci": {"github": {"artifacts_matrix": {"include": [
                {"targets": ["x86_64-unknown-linux-musl"]},
                {"targets": ["aarch64-unknown-linux-musl"]},
            ]}}},
        }
        gate = job_blocks(ROOT / ".github/workflows/release-smoke.yml")["gate"]
        script = gate.split("      - id: arm\n", 1)[1].split("        run: |\n", 1)[1]
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output"
            result = subprocess.run(
                ["bash", "-eo", "pipefail", "-c", textwrap.dedent(script)],
                env=dict(os.environ, PLAN=json.dumps(plan), GITHUB_OUTPUT=str(output)),
                capture_output=True, text=True, timeout=10,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(output.read_text(),
                             "archive=arm\nartifact=artifacts-build-local-aarch64-unknown-linux-musl\n")

    def run_smoke(self, linkage="statically linked", version="thirdshift 1.2.3"):
        job = job_blocks(ROOT / ".github/workflows/release-smoke.yml")["arm-linux"]
        scripts = re.findall(r"^        run: \|\n((?:          .*\n|\n)+)", job, re.MULTILINE)
        self.assertTrue(scripts, "ARM smoke must execute the release binary")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = "thirdshift-aarch64-unknown-linux-musl"
            binary = root / archive / "thirdshift"
            binary.parent.mkdir()
            binary.write_text(f"#!/bin/sh\ntest \"$1\" = version || exit 1\necho '{version}'\n")
            binary.chmod(0o755)
            with tarfile.open(root / f"{archive}.tar.xz", "w:xz") as bundle:
                bundle.add(binary.parent, arcname=archive)
            binary.unlink()
            binary.parent.rmdir()
            commands = root / "commands"
            commands.mkdir()
            file_command = commands / "file"
            file_command.write_text(f"#!/bin/sh\necho 'ELF 64-bit ARM aarch64, {linkage}'\n")
            file_command.chmod(0o755)
            env = dict(os.environ, PATH=f"{commands}:{os.environ['PATH']}",
                       ARCHIVE=archive, EXPECTED="thirdshift 1.2.3")
            for script in scripts:
                result = subprocess.run(
                    ["bash", "-eo", "pipefail", "-c", textwrap.dedent(script)],
                    cwd=root, env=env, capture_output=True, text=True, timeout=10,
                )
                if result.returncode:
                    return result
            return result

    def test_static_arm_archive_runs_the_expected_version(self):
        result = self.run_smoke()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("thirdshift 1.2.3", result.stdout)

    def test_static_pie_archive_runs_the_expected_version(self):
        result = self.run_smoke(linkage="static-pie linked")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_dynamic_binary_blocks_the_release(self):
        result = self.run_smoke(linkage="dynamically linked")
        self.assertNotEqual(result.returncode, 0)

    def test_wrong_version_blocks_the_release(self):
        result = self.run_smoke(version="thirdshift 0.0.1")
        self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
