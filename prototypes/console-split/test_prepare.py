"""Exercise the delivered split through its command-line interface."""
import json
import io
from pathlib import Path
import subprocess
import tempfile
import tarfile
import unittest

from prepare import BASE, REPO


HERE = Path(__file__).resolve().parent


class PrepareTests(unittest.TestCase):
    def test_workspace_has_one_way_dependency_and_preserves_integration_tests(self):
        with tempfile.TemporaryDirectory() as scratch:
            tree = Path(scratch) / "split"
            subprocess.run(["python3", str(HERE / "prepare.py"), str(tree)], check=True)
            metadata = json.loads(subprocess.check_output(
                ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], cwd=tree))
            packages = {p["name"]: p for p in metadata["packages"]}
            self.assertEqual(set(packages), {"thirdshift", "thirdshift-core"})
            for package in packages.values():
                self.assertEqual(package["repository"], "https://github.com/JacobStephens2/thirdshift")
            self.assertIn("thirdshift-core", {d["name"] for d in packages["thirdshift"]["dependencies"]})
            self.assertNotIn("thirdshift", {d["name"] for d in packages["thirdshift-core"]["dependencies"]})
            self.assertNotIn("mod console;", (tree / "src/core.rs").read_text())
            self.assertNotIn("pub fn parse(", (tree / "src/core_args.rs").read_text())
            self.assertIn("pub fn parse(", (tree / "src/args.rs").read_text())
            original = subprocess.check_output(["git", "archive", BASE, "tests"], cwd=REPO)
            with tarfile.open(fileobj=io.BytesIO(original)) as archive:
                for source in archive:
                    if source.isfile():
                        self.assertEqual(archive.extractfile(source).read(), (tree / source.name).read_bytes())
            for root, kind in [("main.rs", "bin"), ("core.rs", "lib")]:
                platform_root = Path(scratch) / root
                platform_root.write_text((tree / "src" / root).read_text().replace('target_os = "linux"', "thirdshift_linux"))
                for testing in [[], ["--test"]]:
                    command = ["rustc", "--edition=2024", "--crate-type", kind,
                               "--emit=metadata", "--error-format=json", *testing,
                               str(platform_root), "-o", str(Path(scratch) / "platform.rmeta")]
                    result = subprocess.run(command, capture_output=True, text=True)
                    self.assertNotEqual(result.returncode, 0)
                    messages = [json.loads(line) for line in result.stderr.splitlines()]
                    errors = [m["message"] for m in messages if m.get("level") in {"error", "warning"}
                              and not m["message"].startswith("aborting due to")]
                    self.assertEqual(errors, ["thirdshift supports Linux only. On macOS, run it inside an Ubuntu 24.04 Linux VM (such as Lima). On Windows, use WSL2."])

    def test_refuses_nonempty_destination(self):
        with tempfile.TemporaryDirectory() as scratch:
            sentinel = Path(scratch) / "keep"
            sentinel.write_text("keep")
            result = subprocess.run(["python3", str(HERE / "prepare.py"), scratch], capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(b"destination must be empty", result.stderr)
            self.assertEqual(sentinel.read_text(), "keep")


if __name__ == "__main__":
    unittest.main()
