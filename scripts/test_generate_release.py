"""Exercise the generator CLI against a stand-in for the external dist tool."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


GENERATOR = Path(__file__).with_name("generate-release.py").resolve()


class GenerateRelease(unittest.TestCase):
    def test_generate_and_check_preserve_the_workflow(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / ".github/workflows").mkdir(parents=True)
            (root / ".github/workflows/ci.yml").write_text(
                GENERATOR.parent.parent.joinpath(".github/workflows/ci.yml").read_text()
            )
            workflow = root / ".github/workflows/release.yml"
            (root / "dist-workspace.toml").write_text('[dist]\ncargo-dist-version = "0.33.0"\nallow-dirty = ["ci"]\n')
            upstream = "name: Release\non:\n  pull_request:\n  push:\n    tags: ['v*']\n\njobs:\n  plan:\n    runs-on: ubuntu-24.04\n"
            workflow.write_text("stale workflow\n")
            dist = root / "dist"
            dist.write_text(
                "#!/usr/bin/env python3\nimport os, pathlib, sys\n"
                "if os.environ.get('DIST_FAIL'):\n    pathlib.Path('.github/workflows/release.yml').write_text('partial')\n    sys.exit(1)\n"
                "if '--version' in sys.argv:\n    print('cargo-dist 0.33.0')\n"
                f"else:\n    assert 'allow-dirty' not in pathlib.Path('dist-workspace.toml').read_text()\n    pathlib.Path('.github/workflows/release.yml').write_text({upstream!r})\n"
            )
            dist.chmod(0o755)
            env = dict(os.environ, PATH=f"{root}:{os.environ['PATH']}")

            def run(*arguments):
                return subprocess.run(
                    ["python3", str(GENERATOR), *arguments],
                    cwd=root, env=env, capture_output=True, text=True,
                )

            stale = run("--check")
            self.assertNotEqual(stale.returncode, 0)
            self.assertEqual(workflow.read_text(), "stale workflow\n")
            generated = run()
            self.assertEqual(generated.returncode, 0, generated.stderr)
            expected = workflow.read_text()
            self.assertIn("concurrency:\n", expected)
            self.assertIn("github.event.pull_request.number", expected)
            self.assertIn("cancel-in-progress: ${{ github.event_name == 'pull_request' }}", expected)
            self.assertIn("    paths:\n", expected)
            self.assertIn("      - 'src/**'", expected)
            self.assertIn("  push:\n    tags: ['v*']", expected)
            self.assertIn("jobs:\n  plan:\n    runs-on: ubuntu-24.04", expected)
            config_before = (root / "dist-workspace.toml").read_bytes()
            checked = run("--check")
            self.assertEqual(checked.returncode, 0, checked.stderr)
            self.assertEqual(workflow.read_text(), expected)
            self.assertEqual((root / "dist-workspace.toml").read_bytes(), config_before)
            # Fail after writing a partial workflow, as dist itself can do.
            dist.write_text(dist.read_text().replace("if os.environ.get('DIST_FAIL'):", "if '--version' not in sys.argv:"))
            failed = run()
            self.assertNotEqual(failed.returncode, 0)
            self.assertEqual(workflow.read_text(), expected)
            self.assertEqual((root / "dist-workspace.toml").read_bytes(), config_before)


if __name__ == "__main__":
    unittest.main()
