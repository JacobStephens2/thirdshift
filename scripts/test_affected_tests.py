"""Exercise affected-tests through its CLI with real Git and stand-in runners."""
import json
import os
from pathlib import Path
import re
import runpy
import shutil
import subprocess
import sys
import tempfile
import unittest


SELECTOR = Path(__file__).with_name("affected-tests").resolve()


def assert_doc_references_covered(root):
    """Guard literal Markdown inputs in repository code, including new files.

    Only existing docs count: target-repository threat models and example paths
    aren't inputs here. The selector and this test inventory paths, not read them.
    Ignore comment-only lines and URLs; neither makes a doc a build/test input.
    """
    exceptions = {os.fsdecode(path) for path in runpy.run_path(str(SELECTOR))["DOC_CHECKS"]}
    paths = subprocess.check_output(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        cwd=root,
    ).decode().split("\0")
    reference = re.compile(
        r'''["'](?:\./|\.\./)*((?:docs/[^"'\n]+|GLOSSARY|CODING_STANDARDS)\.md)["']'''
    )
    code_suffixes = {".rs", ".py", ".js", ".cjs", ".mjs", ".ts", ".tsx",
                     ".go", ".sh", ".toml", ".yml", ".yaml"}
    missing = []
    for name in sorted(set(paths) - {"scripts/affected-tests", "scripts/test_affected_tests.py", ""}):
        source = root / name
        if not source.is_file() or (source.suffix not in code_suffixes
                                    and not name.startswith("scripts/")):
            continue
        for number, line in enumerate(source.read_text(errors="replace").splitlines(), 1):
            if line.lstrip().startswith(("#", "//", "*")):
                continue
            for doc in reference.findall(line):
                if (root / doc).is_file() and doc not in exceptions:
                    missing.append(f"{name}:{number} reads {doc}; add it to DOC_CHECKS")
    if missing:
        raise AssertionError("\n".join(missing))


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
        for tool in ["cargo", "npm", "python3", "go", "bun"]:
            path = self.tools / tool
            path.write_text(
                f"#!{sys.executable}\n"
                "import json, os, pathlib, sys\n"
                "expected = pathlib.Path(os.environ['EXPECT_ROOT'])\n"
                "if pathlib.Path(sys.argv[0]).name == 'go': expected /= 'prototype/go'\n"
                "assert pathlib.Path.cwd() == expected\n"
                "if sys.argv[1:] == ['nextest', '--version']:\n"
                "    sys.exit(0 if os.environ['NEXTEST'] == 'yes' else 1)\n"
                "with open(os.environ['CALLS'], 'a') as calls:\n"
                "    calls.write(json.dumps([pathlib.Path(sys.argv[0]).name, *sys.argv[1:]]) + '\\n')\n"
                "sys.stdout.write(os.environ.get('RUNNER_STDOUT', ''))\n"
                "sys.stderr.write(os.environ.get('RUNNER_STDERR', ''))\n"
                "sys.exit(int(os.environ.get('RUNNER_EXIT', '0')))\n"
            )
            path.chmod(0o755)
        self.git("init", "-q")
        (self.root / ".git/info/exclude").write_text("/tools/\n/calls.jsonl\n")
        (self.root / ".gitignore").write_bytes(SELECTOR.parent.parent.joinpath(".gitignore").read_bytes())
        self.git("add", ".gitignore")
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

    def select_for_only(self, path):
        self.git("branch", "-f", "base", "HEAD")
        self.change(path)
        result, calls = self.run_selector()
        self.calls.unlink(missing_ok=True)
        self.assertEqual(result.returncode, 0, result.stderr)
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

    def test_go_prototype_only_runs_go_in_its_module(self):
        self.change("prototype/go/main.go")
        result, calls = self.run_selector()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [["go", "test", "./..."]])

    def test_go_and_site_keep_targeted_checks_without_full_rust(self):
        self.change("prototype/go/main.go", "site/index.html")
        result, calls = self.run_selector()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [
            ["go", "test", "./..."],
            ["npm", "test", "--prefix", "site"],
            ["cargo", "test", "--test", "share_card", "--test", "site_deploy"],
            ["cargo", "test", "--bin", "thirdshift", "prompts_page"],
        ])

    def test_typescript_only_runs_its_typecheck_and_tests(self):
        self.change("prototypes/typescript/src/ticket.ts")
        result, calls = self.run_selector()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [
            ["bun", "run", "--cwd", "prototypes/typescript", "typecheck"],
            ["bun", "test", "--cwd", "prototypes/typescript"],
        ])

    def test_typescript_and_rust_run_both_suites(self):
        self.change("prototypes/typescript/src/ticket.ts", "src/main.rs")
        result, calls = self.run_selector()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [
            ["bun", "run", "--cwd", "prototypes/typescript", "typecheck"],
            ["bun", "test", "--cwd", "prototypes/typescript"],
            ["cargo", "nextest", "run"],
        ])

    def test_typescript_and_site_do_not_select_full_rust(self):
        self.change("prototypes/typescript/src/ticket.ts", "site/index.html")
        result, calls = self.run_selector()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [
            ["bun", "run", "--cwd", "prototypes/typescript", "typecheck"],
            ["bun", "test", "--cwd", "prototypes/typescript"],
            ["npm", "test", "--prefix", "site"],
            ["cargo", "test", "--test", "share_card", "--test", "site_deploy"],
            ["cargo", "test", "--bin", "thirdshift", "prompts_page"],
        ])

    def test_go_and_rust_run_both_suites(self):
        self.change("prototype/go/main.go", "src/main.rs")
        result, calls = self.run_selector()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [["go", "test", "./..."], ["cargo", "nextest", "run"]])

    def test_both_prototypes_and_site_keep_targeted_checks_without_full_rust(self):
        self.change("prototype/go/main.go", "prototypes/typescript/src/ticket.ts", "site/index.html")
        result, calls = self.run_selector()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [
            ["go", "test", "./..."],
            ["bun", "run", "--cwd", "prototypes/typescript", "typecheck"],
            ["bun", "test", "--cwd", "prototypes/typescript"],
            ["npm", "test", "--prefix", "site"],
            ["cargo", "test", "--test", "share_card", "--test", "site_deploy"],
            ["cargo", "test", "--bin", "thirdshift", "prompts_page"],
        ])

    def test_passing_commands_print_summaries_and_keep_distinct_ignored_logs(self):
        self.change("site/index.html")
        self.env.update(RUNNER_STDOUT="raw passing test output\n", RUNNER_STDERR="raw stderr\n")
        result, calls = self.run_selector()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("raw passing test output", result.stdout + result.stderr)
        self.assertNotIn("raw stderr", result.stdout + result.stderr)
        summaries = [line for line in result.stdout.splitlines() if line.startswith("PASS ")]
        self.assertEqual(len(summaries), len(calls))
        logs = []
        for command, summary in zip(calls, summaries):
            self.assertIn(" ".join(command), summary)
            self.assertRegex(summary, r"\(\d+\.\d{2}s; log: .+\)$")
            log = Path(summary.split("; log: ", 1)[1][:-1])
            logs.append(log)
            self.assertEqual(log.parent, self.root / ".affected-tests")
            self.assertIn("-".join(command), log.name)
            self.assertIn("raw passing test output\n", log.read_text())
            self.assertIn("raw stderr\n", log.read_text())
            self.assertIn(f"log: {log}", result.stdout.split(summary)[0])
            self.assertEqual(self.git("check-ignore", str(log)), str(log))
        self.assertEqual(len(set(logs)), len(calls))
        again, _ = self.run_selector()
        self.assertEqual(again.returncode, 0, again.stderr)
        self.assertEqual(len(list((self.root / ".affected-tests").glob("*.log"))), 2 * len(calls))
        self.git("branch", "-f", "base", "HEAD")
        clean, _ = self.run_selector()
        self.assertIn("No changed files", clean.stdout)

    def test_release_only_runs_release_checks(self):
        self.change("scripts/release.sh")
        result, calls = self.run_selector()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [
            ["cargo", "test", "--test", "release"],
            ["python3", "-m", "unittest", "discover", "-s", "scripts", "-p", "test_generate_release.py"],
        ])

    def test_console_browser_test_changes_run_only_the_console_browser_tests(self):
        for path in ["console/tests/inbox.test.cjs", "console/tests/fixtures/experiments.cjs",
                     "console/package-lock.json"]:
            with self.subTest(path=path):
                result, calls = self.select_for_only(path)
                self.assertEqual(calls, [
                    ["cargo", "build", "--locked"],
                    ["npm", "test", "--prefix", "console"],
                ])
                self.assertIn(f"Console browser tests for {path!r}.", result.stdout)

    def test_console_source_runs_the_console_browser_tests_and_full_rust_suite(self):
        for path in ["src/console.rs", "src/console/inbox.html", "src/console/assets/console.css",
                     "src/console/assets/console.js", "src/config/console.rs",
                     "src/config/console/themes.rs", "tests/console/scoring_fixture.py",
                     "tests/fixtures/waterfall-logs.jsonl", "tests/fixtures/waterfall-spans.jsonl"]:
            with self.subTest(path=path):
                result, calls = self.select_for_only(path)
                self.assertEqual(calls, [
                    ["cargo", "build", "--locked"],
                    ["npm", "test", "--prefix", "console"],
                    ["cargo", "nextest", "run"],
                ])
                self.assertIn(f"Console browser tests and full Rust suite for {path!r}.", result.stdout)

    def test_console_and_site_changes_keep_both_browser_checks_and_the_full_suite(self):
        self.change("site/index.html", "console/tests/inbox.test.cjs")
        result, calls = self.run_selector()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [
            ["npm", "test", "--prefix", "site"],
            ["cargo", "build", "--locked"],
            ["npm", "test", "--prefix", "console"],
            ["cargo", "nextest", "run"],
        ])

    def test_missing_npm_fails_the_console_selection_visibly(self):
        self.change("console/tests/inbox.test.cjs")
        (self.tools / "npm").unlink()
        system = self.root / "system"
        system.mkdir()
        (system / "git").symlink_to(shutil.which("git"))
        self.env["PATH"] = f"{self.tools}:{system}"
        result, calls = self.run_selector()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("npm", result.stderr)
        self.assertEqual(calls, [["cargo", "build", "--locked"]])

    def test_broad_or_unrecognised_paths_run_the_full_rust_suite(self):
        for path in ["src/main.rs", "tests/support/mod.rs", "tests/fakes/gh.rs",
                     "Cargo.toml", "Cargo.lock", ".github/workflows/ci.yml",
                     ".config/nextest.toml", "new-area/unknown file\nname"]:
            with self.subTest(path=path):
                result, calls = self.select_for_only(path)
                self.assertEqual(calls, [["cargo", "nextest", "run"]])
                self.assertIn("cargo nextest run", result.stdout)

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

    def test_unread_markdown_runs_no_checks(self):
        for path in ["docs/adr/example.md", "docs/research/note.md",
                     "docs/grill-with-docs/plan.md", "docs/agents/example.md",
                     "docs/testing.md", "CODING_STANDARDS.md"]:
            with self.subTest(path=path):
                result, calls = self.select_for_only(path)
                self.assertEqual(calls, [])
                self.assertIn(f"No checks for docs-only {path!r}.", result.stdout)

    def test_docs_do_not_broaden_other_areas(self):
        for path in ["src/main.rs", "site/index.html", "scripts/release.sh",
                     "console/tests/inbox.test.cjs", "prototype/go/main.go",
                     "prototypes/typescript/src/ticket.ts"]:
            with self.subTest(path=path):
                _, expected = self.select_for_only(path)
                self.change("docs/adr/example.md", commit=False)
                result, calls = self.run_selector()
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(calls, expected)
                (self.root / "docs/adr/example.md").unlink()
                self.calls.unlink(missing_ok=True)

    def test_docs_consumers_and_non_markdown_keep_checks(self):
        for path in ["docs/experiment-results.md",
                     "docs/adr/0026-each-console-serves-an-api-that-can-do-what-the-cli-can.md",
                     "docs/research/go-vs-rust-prototype.md",
                     "docs/research/replay.py", "README.md", "AGENTS.md"]:
            with self.subTest(path=path):
                _, calls = self.select_for_only(path)
                self.assertEqual(calls, [["cargo", "nextest", "run"]])

    def test_glossary_gets_its_site_consumers_checks(self):
        _, calls = self.select_for_only("GLOSSARY.md")
        self.assertEqual(calls, [
            ["npm", "test", "--prefix", "site"],
            ["cargo", "test", "--test", "share_card", "--test", "site_deploy"],
            ["cargo", "test", "--bin", "thirdshift", "prompts_page"],
        ])

    def test_repository_doc_references_are_listed(self):
        assert_doc_references_covered(SELECTOR.parent.parent)

    def test_new_doc_consumer_fails_the_reference_guard(self):
        self.change("docs/adr/example.md", "tests/reads_doc.rs")
        (self.root / "tests/reads_doc.rs").write_text(
            'const DOC: &str = include_str!("../docs/adr/example.md");\n'
        )
        with self.assertRaisesRegex(AssertionError, r"tests/reads_doc.rs:1 reads docs/adr/example.md"):
            assert_doc_references_covered(self.root)

    def test_reference_guard_ignores_comments_and_external_docs(self):
        self.change("docs/adr/example.md", "src/example.rs")
        (self.root / "src/example.rs").write_text(
            '// See "docs/adr/example.md"\n'
            'const URL: &str = "https://example.com/docs/adr/example.md";\n'
            'const TARGET: &str = "docs/THREAT-MODEL.md";\n'
        )
        assert_doc_references_covered(self.root)

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

    def test_failed_rust_tests_print_names_and_multiline_panics_for_both_runners(self):
        self.change("src/main.rs")
        for runner in ["yes", "no"]:
            with self.subTest(nextest=runner):
                self.env.update(
                    NEXTEST=runner, RUNNER_EXIT="101",
                    RUNNER_STDOUT=(
                        "running 2 tests\n"
                        "test config::loads_defaults ... ok\n"
                        "test config::rejects_invalid ... FAILED\n"
                        "\nfailures:\n\n---- config::rejects_invalid stdout ----\n"
                        "unrelated captured output\n\n"
                        "thread 'config::rejects_invalid' panicked at src/config.rs:42:9:\n"
                        "assertion `left == right` failed: invalid setting\n"
                        "  left: 1\n right: 2\n"
                        "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\n"
                        "\nfailures:\n    config::rejects_invalid\n\n"
                        "test result: FAILED. 1 passed; 1 failed; 0 ignored\n"
                    ) if runner == "no" else "",
                    RUNNER_STDERR=(
                        "        FAIL [   0.012s] thirdshift config::rejects_invalid\n"
                        "--- STDOUT:              thirdshift config::rejects_invalid ---\n"
                        "unrelated captured output\n"
                        "--- STDERR:              thirdshift config::rejects_invalid ---\n"
                        "thread 'config::rejects_invalid' panicked at src/config.rs:42:9:\n"
                        "assertion `left == right` failed: invalid setting\n"
                        "  left: 1\n right: 2\n"
                        "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\n"
                        "\n------------\n"
                    ) if runner == "yes" else "",
                )
                result, _ = self.run_selector()
                self.assertNotEqual(result.returncode, 0)
                self.assertRegex(result.stdout, r"(?m)^FAIL cargo (nextest run|test) \(\d+\.\d{2}s; log: .+\)$")
                self.assertIn("config::rejects_invalid", result.stdout)
                self.assertIn("panicked at src/config.rs:42:9:", result.stdout)
                self.assertIn("assertion `left == right` failed: invalid setting", result.stdout)
                self.assertIn("left: 1", result.stdout)
                self.assertIn("right: 2", result.stdout)
                self.assertNotIn("unrelated captured output", result.stdout)
                self.assertNotIn("running 2 tests", result.stdout)

    def test_failed_browser_cases_are_printed_for_tap_and_spec_reporters(self):
        self.change("site/index.html")
        for output in [
            "TAP version 13\n# Subtest: navigation opens the manual\n"
            "not ok 1 - navigation opens the manual\n  ---\n  duration_ms: 15.4\n"
            "  failureType: 'testCodeFailure'\n  error: 'wrong page'\n  ...\n",
            "✔ homepage loads (3.2ms)\n✖ navigation opens the manual (15.4ms)\n"
            "ℹ tests 2\nℹ fail 1\n✖ failing tests:\n"
            "test at tests/pages.test.cjs:10:1\n"
            "✖ navigation opens the manual (15.4ms)\n  Error: wrong page\n",
        ]:
            with self.subTest(output=output):
                self.env.update(RUNNER_STDOUT=output, RUNNER_EXIT="1")
                result, _ = self.run_selector()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("FAIL npm test --prefix site (", result.stdout)
                self.assertIn("navigation opens the manual", result.stdout)
                self.assertNotIn("homepage loads", result.stdout)
                self.assertNotIn("duration_ms", result.stdout)
                self.assertNotIn("failureType", result.stdout)

    def test_command_that_cannot_start_still_reports_failure_and_keeps_a_log(self):
        self.change("site/index.html")
        (self.tools / "git").symlink_to(shutil.which("git"))
        self.env["PATH"] = str(self.tools)
        (self.tools / "npm").write_text("#!/no-such-affected-tests-interpreter\n")
        result, calls = self.run_selector()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, [])
        self.assertIn("FAIL npm test --prefix site (", result.stdout)
        log = next((self.root / ".affected-tests").glob("npm-*.log"))
        self.assertIn("No such file or directory", log.read_text())
        self.assertIn("No such file or directory", result.stdout)

    def test_forced_color_failures_are_readable_and_logs_preserve_the_original(self):
        self.change("site/index.html")
        output = "\x1b[31m✖ navigation opens the manual (15.4ms)\x1b[0m\n"
        self.env.update(RUNNER_STDOUT=output, RUNNER_EXIT="1")
        result, _ = self.run_selector()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("✖ navigation opens the manual (15.4ms)", result.stdout)
        self.assertNotIn("\x1b", result.stdout)
        log = next((self.root / ".affected-tests").glob("*.log"))
        self.assertEqual(log.read_text(), output)

    def test_signal_terminated_rust_test_is_named(self):
        self.change("src/main.rs")
        self.env.update(
            RUNNER_EXIT="100",
            RUNNER_STDERR=(
                "     SIGABRT [   0.376s] (1/1) thirdshift abort_test_must_be_named\n"
                "error: test run failed\n"
            ),
        )
        result, _ = self.run_selector()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("abort_test_must_be_named", result.stdout)

    def test_failed_release_generator_tests_are_named(self):
        self.change("scripts/release.sh")
        for status in ("FAIL", "ERROR"):
            with self.subTest(status=status):
                output = (
                    f"{status}: test_release_notes "
                    "(test_generate_release.ReleaseTests.test_release_notes)\n"
                    "----------------------------------------------------------------------\n"
                    "AssertionError: missing notes\n"
                )
                (self.tools / "python3").write_text(
                    f"#!{sys.executable}\n"
                    "import sys\n"
                    f"sys.stderr.write({output!r})\n"
                    "sys.exit(1)\n"
                )
                result, _ = self.run_selector()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("test_release_notes", result.stdout)


if __name__ == "__main__":
    unittest.main()
