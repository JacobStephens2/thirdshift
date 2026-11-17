"""Isolated review regressions: no external fleet, GitHub or Console calls."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[4]
CHECK = ROOT / ".agents/skills/fleet-steward/scripts/fleet-check.sh"
ARTIFACTS = CHECK.parent


class FleetStewardSpecRegressions(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="fleet-spec-", dir=ARTIFACTS)
        self.addCleanup(self.temporary.cleanup)
        self.sandbox = Path(self.temporary.name)
        self.home = self.sandbox / "home"
        self.bin = self.sandbox / "bin"
        self.bin.mkdir()
        self.fleet = self.home / ".thirdshift/fleet"
        self.fleet.mkdir(parents=True)
        self.state = self.fleet / "last-tick"
        self.original_checkpoint = "2000-01-01 00:00:00\n"
        self.state.write_text(self.original_checkpoint)
        self.config = self.fleet / "fleet.conf"
        self.config.write_text("OWNERS=(); HOSTS=(); CONSOLE_URL=''; IGNORE=()\n")
        self.environment = dict(os.environ)
        self.environment.update({
            "HOME": str(self.home),
            "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
            "THIRDSHIFT_FLEET_CONFIG": str(self.config),
            "THIRDSHIFT_LOGS": str(self.home / ".thirdshift/logs"),
            "TZ": "UTC",
        })
        self.fake("gh", "print('quota core 4999/5000 reset 00:00; search 30')\n")
        self.fake("ps", "pass\n")
        self.fake("ssh", "import sys\nprint('ssh: host unavailable', file=sys.stderr)\nsys.exit(255)\n")
        self.fake("limactl", "import sys\nsys.exit(1)\n")
        self.fake("curl", "import sys\nprint('unexpected Console request', file=sys.stderr)\nsys.exit(1)\n")

    def fake(self, name, body):
        target = self.bin / name
        target.write_text("#!" + sys.executable + "\n" + body)
        target.chmod(0o755)

    def run_check(self):
        return subprocess.run(["/bin/bash", str(CHECK)], env=self.environment,
                              text=True, capture_output=True, timeout=20)

    def test_all_host_events_since_last_tick_are_printed(self):
        self.config.write_text("OWNERS=(); HOSTS=(local); CONSOLE_URL=''; IGNORE=()\n")
        activity = self.home / ".thirdshift/logs/acme/widgets/activity.log"
        activity.parent.mkdir(parents=True)
        activity.write_text("2001-01-01 00:00:01 Ticket run #1 failed: unique lost failure\n" +
                            "".join(f"2001-01-01 00:00:{i:02d} Ticket run #{i} merged\n"
                                    for i in range(2, 42)))
        result = self.run_check()
        self.assertIn("unique lost failure", result.stdout,
                      "A failure followed by 40 events disappeared from the tick")

    def test_failed_host_read_keeps_checkpoint(self):
        self.config.write_text("OWNERS=(); HOSTS=(unreachable); CONSOLE_URL=''; IGNORE=()\n")
        self.run_check()
        self.assertEqual(self.state.read_text(), self.original_checkpoint,
                         "The next tick will skip unread events on the unreachable host")

    def test_every_failed_issue_is_listed(self):
        self.config.write_text("OWNERS=(acme); HOSTS=(); CONSOLE_URL=''; IGNORE=()\n")
        self.fake("gh", """import sys
args = sys.argv[1:]
if 'search/issues' in args:
    fields = [args[i + 1] for i, value in enumerate(args[:-1]) if value == '-f']
    fields = dict(value.split('=', 1) for value in fields)
    size = min(int(fields.get('per_page', 30)), 100)
    page = int(fields.get('page', 1))
    numbers = range(1, 202) if '--paginate' in args else range(1 + (page-1)*size, min(page*size, 201)+1)
    for number in numbers:
        print(f'acme/widgets#{number} failed [agent-failed]')
else:
    print('quota core 4999/5000 reset 00:00; search 30')
""")
        result = self.run_check()
        self.assertIn("acme/widgets#201 ", result.stdout,
                      "Only the first GitHub search page was inspected")

    def test_sign_in_page_is_not_an_empty_needs_you_group(self):
        parser = ROOT / ".agents/skills/fleet-steward/scripts/needs-you.py"
        sign_in = (ROOT / "src/console/sign_in.html").read_text()
        result = subprocess.run([sys.executable, str(parser), "https://console.invalid"],
                                input=sign_in, text=True, capture_output=True, timeout=20)
        self.assertNotEqual(result.returncode, 0,
                            "The sign-in page was accepted as a successfully read empty Inbox: " + result.stdout)

    def test_needs_you_keeps_machine_and_command_identity(self):
        page = '''<details data-group="needs you">
<li data-machine="worker-2" data-command="pickup-20261117T120000-0500">
<span>Failed</span> acme/widgets#7
<ul><li>Command log and Session trace</li></ul></li></details>
<details data-group="done"><li data-command="unrelated">Done</li></details>'''
        parser = CHECK.with_name("needs-you.py")
        result = subprocess.run([sys.executable, str(parser)], input=page,
                                text=True, capture_output=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("1 rows", result.stdout)
        self.assertIn("worker-2", result.stdout)
        self.assertIn("pickup-20261117T120000-0500", result.stdout)
        self.assertNotIn("unrelated", result.stdout)

    def test_authenticated_console_check_updates_checkpoint(self):
        cookies = self.sandbox / "cookies.txt"
        cookies.write_text("# Netscape HTTP Cookie File\n")
        self.config.write_text("OWNERS=(); HOSTS=(); IGNORE=()\n"
                               "CONSOLE_URL=https://console.invalid\n"
                               f"CONSOLE_COOKIE_FILE='{cookies}'\n")
        self.fake("curl", """import sys
assert '--cookie' in sys.argv
from pathlib import Path
assert Path(sys.argv[sys.argv.index('--cookie') + 1]).is_file()
assert '--fail' in sys.argv
print('<details data-group="needs you"></details>')
""")
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("0 rows", result.stdout)
        self.assertNotEqual(self.state.read_text(), self.original_checkpoint)

    def test_failed_console_check_keeps_checkpoint(self):
        self.config.write_text("OWNERS=(); HOSTS=(); IGNORE=(); CONSOLE_URL=https://console.invalid\n")
        result = self.run_check()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.state.read_text(), self.original_checkpoint)

    def test_idle_host_with_no_matching_events_is_successful(self):
        self.config.write_text("OWNERS=(); HOSTS=(local); CONSOLE_URL=''; IGNORE=()\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("running: idle", result.stdout)
        self.assertNotEqual(self.state.read_text(), self.original_checkpoint)

    def test_failed_github_read_keeps_checkpoint(self):
        self.fake("gh", "import sys\nsys.exit(1)\n")
        result = self.run_check()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.state.read_text(), self.original_checkpoint)


if __name__ == "__main__":
    unittest.main()
