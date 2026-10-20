#!/usr/bin/env python3
"""A live Session reader must reach its Command's Stop confirmation."""
import datetime
import html
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import urllib.error
import urllib.parse
import urllib.request

root = Path(__file__).resolve().parents[2]
with tempfile.TemporaryDirectory(prefix="thirdshift-review-session-stop-") as temporary:
    home = Path(temporary)
    folder = home / ".thirdshift/logs/acme/widgets/commands/issue"
    folder.mkdir(parents=True)
    (home / ".thirdshift/config.toml").write_text("[console]\nport = 0\n")
    command_log = folder / "7-fixture.log"
    session_log = folder / "7-fixture-implement.jsonl"
    command_log.write_text("Waiting for Stop\n")
    session_log.write_text('{"type":"unfamiliar","text":"waiting"}\n')
    stat = Path(f"/proc/{os.getpid()}/stat").read_text().rsplit(") ", 1)[1].split()
    identity = Path("/proc/sys/kernel/random/boot_id").read_text().strip() + ":" + stat[19]
    record = dict(version=1, kind="Run", name="acme/widgets#7", repository="acme/widgets",
                  hostname="fixture", issue=dict(number=7, url="https://github.com/acme/widgets/issues/7", title="Live Command"),
                  process_id=os.getpid(), process_start=identity, harness="codex",
                  started_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  ended_at=None, outcome=None, session_log=str(session_log), stages=[])
    (folder / "7-fixture.command.json").write_text(json.dumps(record))
    server = subprocess.Popen([str(root / "target/debug/thirdshift"), "console", "--no-sign-in"],
                              env={**os.environ, "HOME": str(home)}, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, text=True)
    try:
        origin = server.stdout.readline().strip()
        assert origin.startswith("http://127.0.0.1:"), origin
        def get(route):
            try:
                with urllib.request.urlopen(origin + route, timeout=10) as response:
                    return response.status, response.read().decode()
            except urllib.error.HTTPError as error:
                return error.code, error.read().decode()
        status, command = get("/peek?" + urllib.parse.urlencode({"command": str(command_log)}))
        assert status == 200 and ">Stop</button>" in command, (status, command)
        status, inventory = get("/logs")
        assert status == 200
        session_row = next(row for row in re.findall(r"<li\b.*?</li>", inventory, re.S)
                           if "Session log" in row and session_log.name in row)
        reader_url = html.unescape(re.search(r'href="([^"]+)"', session_row).group(1))
        status, reader = get(reader_url)
        assert status == 200
        details_url = html.unescape(re.search(r'class="peek-link" href="([^"]+)"', reader).group(1))
        status, details = get(details_url)
        print(f"Session reader's Command details: HTTP {status}; {details}")
        assert status == 200 and ">Stop</button>" in details, (
            "Live Session reader must expose its owning Command's Stop control", status, details)
    finally:
        server.terminate()
        server.communicate(timeout=10)
