"""Exercise affected-tests through its CLI with real Git and stand-in runners."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SELECTOR = Path(__file__).with_name("affected-tests").resolve()


class AffectedTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.tools = self.root / "tools"
        self.tools.mkdir()
        self.calls = self.root / "calls.jsonl"
        self.env = dict(
            os.environ,
            PATH=f"{self.tools}:{os.environ['PATH']}",
            GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_NOSYSTEM="1",
            GIT_AUTHOR_NAME="Test", GIT_AUTHOR_EMAIL="test@example.com",
            GIT_COMMITTER_NAME="Test", GIT_COMMITTER_EMAIL="test@example.com",
            CALLS=str(self.calls), NEXTEST="yes",
            EXPECT_ROOT=str(self.root),
        )
        for tool in ["cargo", "npm", "python3"]:
            path = self.tools / tool
            path.write_text(
                f"#!{sys.executable}\n"
                "import json, os, pathlib, sys\n"
                "assert os.getcwd() == os.environ['EXPECT_ROOT']\n"
                "if sys.argv[1:] == ['nextest', '--version']:\n"
                "    sys.exit(0 if os.environ['NEXTEST'] == 'yes' else 1)\n"
                "with open(os.environ['CALLS'], 'a') as calls:\n"
                "    calls.write(json.dumps([pathlib.Path(sys.argv[0]).name, *sys.argv[1:]]) + '\\n')\n"
                "sys.exit(int(os.environ.get('RUNNER_EXIT', '0')))\n"
            )
            path.chmod(0o755)
        self.git("init", "-q")
        (self.root / ".git/info/exclude").write_text("/tools/\n/calls.jsonl\n")
        self.git("-c", "core.hooksPath=/dev/null", "commit", "-qm", "base", "--allow-empty")
        self.git("branch", "base")

    def git(self, *arguments):
        return subprocess.check_output(
            ["git", *arguments], cwd=self.root, env=self.env, text=True,
        ).strip()

    def change(self, *paths, commit=True):
        for name in paths:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("changed\n")
        if commit:
            self.git("add", "--", *paths)
            self.git("-c", "core.hooksPath=/dev/null", "commit", "-qm", "change")

    def run_selector(self, base="base", cwd=None):
        result = subprocess.run(
            [sys.executable, str(SELECTOR), base],
            cwd=cwd or self.root, env=self.env, capture_output=True, text=True,
        )
        calls = [json.loads(line) for line in self.calls.read_text().splitlines()] if self.calls.exists() else []
        return result, calls

    def test_site_only_runs_browser_and_rust_site_checks(self):
        self.change("site/index.html")
        result, calls = self.run_selector()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [
            ["npm", "test", "--prefix", "site"],
            ["cargo", "test", "--test", "share_card", "--test", "site_deploy"],
            ["cargo", "test", "--bin", "thirdshift", "prompts_page"],
        ])

    def test_release_only_runs_release_checks(self):
        self.change("scripts/release.sh")
        result, calls = self.run_selector()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [
            ["cargo", "test", "--test", "release"],
            ["python3", "-m", "unittest", "discover", "-s", "scripts", "-p", "test_generate_release.py"],
        ])

    def test_broad_or_unrecognised_paths_run_the_full_rust_suite(self):
        for path in ["src/main.rs", "tests/support/mod.rs", "tests/fakes/gh.rs",
                     "Cargo.toml", "Cargo.lock", ".github/workflows/ci.yml",
                     ".config/nextest.toml", "new-area/unknown file\nname"]:
            with self.subTest(path=path):
                self.git("branch", "-f", "base", "HEAD")
                self.change(path)
                result, calls = self.run_selector()
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(calls, [["cargo", "nextest", "run"]])
                self.assertIn("cargo nextest run", result.stdout)
                self.calls.unlink()

    def test_mixed_site_release_and_rust_changes_keep_all_checks(self):
        self.change("site/index.html", "scripts/release.sh", "src/main.rs")
        result, calls = self.run_selector()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [
            ["npm", "test", "--prefix", "site"],
            ["cargo", "nextest", "run"],
            ["python3", "-m", "unittest", "discover", "-s", "scripts", "-p", "test_generate_release.py"],
        ])

    def test_guide_source_gets_the_site_checks(self):
        self.change("docs/guide.md")
        result, calls = self.run_selector()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [
            ["npm", "test", "--prefix", "site"],
            ["cargo", "test", "--test", "share_card", "--test", "site_deploy"],
            ["cargo", "test", "--bin", "thirdshift", "prompts_page"],
        ])

    def test_nextest_unavailable_prints_the_cargo_test_fallback(self):
        self.change("src/main.rs")
        self.env["NEXTEST"] = "no"
        result, calls = self.run_selector()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [["cargo", "test"]])
        self.assertIn("nextest unavailable; falling back to cargo test", result.stdout)

    def test_mixed_narrow_areas_also_get_the_full_suite(self):
        self.change("site/index.html", "scripts/release.sh")
        result, calls = self.run_selector()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [
            ["npm", "test", "--prefix", "site"],
            ["cargo", "nextest", "run"],
            ["python3", "-m", "unittest", "discover", "-s", "scripts", "-p", "test_generate_release.py"],
        ])

    def test_unstaged_staged_and_untracked_changes_are_included(self):
        self.change("site/index.html")
        self.git("branch", "-f", "base", "HEAD")
        (self.root / "site/index.html").write_text("unstaged\n")
        self.change("scripts/release.sh", commit=False)
        self.git("add", "scripts/release.sh")
        self.change("unknown.txt", commit=False)
        result, calls = self.run_selector(cwd=self.root / "site")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [
            ["npm", "test", "--prefix", "site"],
            ["cargo", "nextest", "run"],
            ["python3", "-m", "unittest", "discover", "-s", "scripts", "-p", "test_generate_release.py"],
        ])

    def test_rename_keeps_the_checks_for_the_old_path(self):
        self.change("site/old.html")
        self.git("branch", "-f", "base", "HEAD")
        self.git("mv", "site/old.html", "unknown.html")
        result, calls = self.run_selector()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [
            ["npm", "test", "--prefix", "site"],
            ["cargo", "nextest", "run"],
        ])

    def test_invalid_base_runs_nothing_and_fails(self):
        self.change("site/index.html")
        result, calls = self.run_selector("no-such-ref")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, [])

    def test_clean_checkout_runs_nothing(self):
        result, calls = self.run_selector()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [])
        self.assertIn("No changed files", result.stdout)

    def test_test_runner_failure_fails_the_entry_point(self):
        self.change("src/main.rs")
        self.env["RUNNER_EXIT"] = "7"
        result, calls = self.run_selector()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, [["cargo", "nextest", "run"]])


if __name__ == "__main__":
    unittest.main()
