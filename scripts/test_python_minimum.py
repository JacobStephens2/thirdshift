"""Scripts that import tomllib name the Python they need instead of a traceback."""
import ast
from pathlib import Path
import subprocess
import sys
import unittest


ROOT = Path(__file__).resolve().parent.parent
SCRIPTS = [ROOT / "scripts/generate-release.py", ROOT / "tests/bug_origins/regressions.py"]


def run_as(version, script, *arguments, **options):
    """Run `script` with sys.version_info reporting an interpreter of `version`."""
    wrapper = (
        "import runpy, sys\n"
        f"sys.version_info = {(*version, 'final', 0)!r}\n"
        f"sys.argv = {[str(script), *arguments]!r}\n"
        f"runpy.run_path({str(script)!r}, run_name='__main__')\n"
    )
    return subprocess.run([sys.executable, "-c", wrapper], capture_output=True, text=True, **options)


def rejection(version):
    found = ".".join(map(str, version))
    return f"Python 3.11 or later is required; found Python {found} at {sys.executable}\n"


class PythonMinimum(unittest.TestCase):
    def test_older_interpreters_get_one_line_naming_both_versions(self):
        for script in SCRIPTS:
            for version in [(3, 9, 18), (3, 10, 14)]:
                with self.subTest(script=script.name, version=version):
                    # regressions.py reads two arguments at module level; the
                    # check must exit before they are used.
                    result = run_as(version, script, "unused", "unused")
                    self.assertNotEqual(result.returncode, 0)
                    self.assertEqual(result.stderr, rejection(version))

    def test_checked_scripts_still_parse_as_python_3_9(self):
        for script in [*SCRIPTS, ROOT / "scripts/affected-tests"]:
            with self.subTest(script=script.name):
                ast.parse(script.read_text(), str(script), feature_version=(3, 9))


if __name__ == "__main__":
    unittest.main()
