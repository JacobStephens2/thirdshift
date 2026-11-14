"""Exercise the generator CLI against a stand-in for the external dist tool."""
import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


GENERATOR = Path(__file__).with_name("generate-release.py").resolve()
TAG_GUARD = GENERATOR.with_name("release-tag-guard.py")


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
            upstream = (
                "name: Release\non:\n  pull_request:\n  push:\n    tags: ['v*']\n"
                "\njobs:\n  # Run 'dist plan' (or host) to determine what tasks we need to do\n"
                "  plan:\n    runs-on: ubuntu-24.04\n"
                "  build-local-artifacts:\n    runs-on: ubuntu-24.04\n"
                "    steps:\n      - name: Install dependencies\n        run: echo install\n"
                "  build-global-artifacts:\n    runs-on: ubuntu-24.04\n"
                "  custom-release-smoke:\n    uses: ./.github/workflows/release-smoke.yml\n"
                "    with:\n      plan: val\n    secrets: inherit\n"
                "  custom-publish-crates:\n    uses: ./.github/workflows/publish-crates.yml\n"
                "    with:\n      plan: val\n    secrets: inherit\n"
            )
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
            self.assertIn("      - 'processes/**'", expected)
            self.assertIn("  push:\n    tags: ['v*']", expected)
            self.assertIn("  release-tag-guard:\n", expected)
            self.assertIn('run: test "$RELEASE_ACTOR" = JacobStephens2', expected)
            self.assertIn("python3 scripts/release-tag-guard.py", expected)
            self.assertIn("fetch-depth: 0", expected)
            self.assertIn("  plan:\n    needs: [release-tag-guard]\n", expected)
            # The guard goes above plan's comment, which describes plan.
            self.assertIn(
                "\njobs:\n  release-tag-guard:\n", expected)
            self.assertIn(
                "\n  # Run 'dist plan' (or host) to determine what tasks we need to do\n"
                "  plan:\n", expected)
            self.assertIn(
                "  plan:\n    needs: [release-tag-guard]\n    permissions:\n      \"actions\": \"write\"\n"
                "      \"contents\": \"read\"\n    timeout-minutes: 5\n"
                "    runs-on: ubuntu-24.04",
                expected,
            )
            self.assertIn("    timeout-minutes: 20\n", expected)
            self.assertIn(
                "      - name: Resolve Ubuntu package hosts\n"
                "        run: bash scripts/release-dns.sh\n"
                "      - name: Install dependencies\n",
                expected,
            )
            self.assertEqual(expected.count("    timeout-minutes: 5\n"), 3)
            self.assertIn(
                "  build-local-artifacts:\n    permissions:\n"
                "      \"actions\": \"write\"\n      \"contents\": \"read\"\n",
                expected,
            )
            self.assertIn(
                "  build-global-artifacts:\n    permissions:\n"
                "      \"actions\": \"write\"\n      \"contents\": \"read\"\n",
                expected,
            )
            smoke_start = expected.index("  custom-release-smoke:\n")
            publish_start = expected.index("  custom-publish-crates:\n")
            self.assertNotIn("secrets: inherit", expected[smoke_start:publish_start])
            self.assertIn("secrets: inherit", expected[publish_start:])
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


class HardenReleaseWorkflow(unittest.TestCase):
    def load(self):
        spec = importlib.util.spec_from_file_location(
            "generate_release", GENERATOR
        )
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module.harden_release_workflow

    def test_missing_build_job_fails_loudly(self):
        harden = self.load()
        with self.assertRaises(RuntimeError):
            harden("jobs:\n  plan:\n    runs-on: ubuntu-24.04\n")

    def test_renamed_build_job_gets_no_silent_pass(self):
        harden = self.load()
        upstream = (
            "jobs:\n  plan:\n    runs-on: ubuntu-24.04\n"
            "  build-local-artifacts-RENAMED:\n    runs-on: ubuntu-24.04\n"
        )
        with self.assertRaises(RuntimeError):
            harden(upstream)


class ReleaseTagGuard(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.git("init", "-q", "-b", "main")
        self.git("config", "user.email", "test@example.com")
        self.git("config", "user.name", "Release test")
        (self.root / "Cargo.toml").write_text(
            '[package]\nname = "thirdshift"\nversion = "0.2.0"\n'
        )
        (self.root / "Cargo.lock").write_text(
            'version = 4\n\n[[package]]\nname = "thirdshift"\nversion = "0.2.0"\n'
            '\n[[package]]\nname = "dependency"\nversion = "1.0.0"\n'
        )
        (self.root / "source.rs").write_text("reviewed code\n")
        self.commit("Reviewed source")
        self.base = self.git("rev-parse", "HEAD")
        self.git("update-ref", "refs/remotes/origin/main", self.base)
        for name in ["Cargo.toml", "Cargo.lock"]:
            file = self.root / name
            file.write_text(file.read_text().replace('"0.2.0"', '"0.3.0"'))
        self.commit("Release 0.3.0")
        self.git("tag", "-a", "v0.3.0", "-m", "Release")
        self.event_commit = self.git("rev-parse", "HEAD")

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.root, text=True).strip()

    def commit(self, message):
        self.git("add", ".")
        self.git("commit", "-qm", message)

    def guard(self, actor="JacobStephens2", tag="v0.3.0", sha=None):
        return subprocess.run(
            ["python3", str(TAG_GUARD)], cwd=self.root,
            env=dict(os.environ, GITHUB_ACTOR=actor, GITHUB_REF_NAME=tag,
                     GITHUB_SHA=sha or self.event_commit),
            capture_output=True, text=True,
        )

    def test_accepts_exact_bump_of_reachable_main_content(self):
        # main can move after the release candidate was selected.
        self.git("checkout", "-q", "main")
        self.git("reset", "--hard", self.base)
        (self.root / "later.rs").write_text("later reviewed code\n")
        self.commit("Later main")
        self.git("update-ref", "refs/remotes/origin/main", "HEAD")
        result = self.guard()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("exact Cargo version bump", result.stdout)
        self.assertEqual(self.git("status", "--porcelain"), "")

    def test_refuses_other_actors_and_non_release_tags(self):
        for actor, tag, reason in [
            ("colleague", "v0.3.0", "github.actor"),
            ("JacobStephens2", "thirdshift/0.3.0", "expected vX.Y.Z"),
            ("JacobStephens2", "v0.03.0", "expected vX.Y.Z"),
        ]:
            with self.subTest(actor=actor, tag=tag):
                result = self.guard(actor, tag)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(reason, result.stderr)

    def retag(self):
        self.commit("Changed candidate")
        self.git("tag", "-f", "v0.3.0")
        self.event_commit = self.git("rev-parse", "HEAD")

    def test_refuses_a_tag_that_moved_since_the_push_event(self):
        result = self.guard(sha=self.base)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("push event", result.stderr)

    def test_refuses_source_changes_outside_main(self):
        (self.root / "source.rs").write_text("unreviewed code\n")
        self.retag()
        result = self.guard()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not an exact Cargo version bump", result.stderr)

    def test_refuses_dependency_changes_inside_cargo_files(self):
        lock = self.root / "Cargo.lock"
        lock.write_text(lock.read_text().replace('"1.0.0"', '"2.0.0"'))
        self.retag()
        result = self.guard()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not an exact Cargo version bump", result.stderr)

    def test_refuses_versions_that_disagree_with_the_tag(self):
        manifest = self.root / "Cargo.toml"
        manifest.write_text(manifest.read_text().replace('"0.3.0"', '"0.4.0"'))
        self.retag()
        result = self.guard()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not an exact Cargo version bump", result.stderr)

    def test_refuses_file_mode_changes(self):
        (self.root / "Cargo.toml").chmod(0o755)
        self.retag()
        result = self.guard()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not an exact Cargo version bump", result.stderr)


if __name__ == "__main__":
    unittest.main()
