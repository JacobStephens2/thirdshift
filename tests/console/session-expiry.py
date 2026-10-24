#!/usr/bin/env python3
"""Open Inbox and log streams must stop accepting changes after their session expires."""
import argparse
from datetime import datetime
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import threading
import time
from urllib.parse import urlsplit

parser = argparse.ArgumentParser()
parser.add_argument("--binary", required=True, type=Path)
parser.add_argument("--settle-seconds", type=float, default=65)
args = parser.parse_args()
assert args.settle_seconds > 60, "Allow the complete 60-second recovery interval"
folder = Path(__file__).resolve().parent
with tempfile.TemporaryDirectory(prefix="thirdshift-session-expiry-") as temporary:
    temporary = Path(temporary)
    library = temporary / "session-clock.so"
    subprocess.run(["cc", "-shared", "-fPIC", str(folder / "session-clock.c"),
                    "-o", str(library), "-ldl"], check=True)
    home = temporary / "home"
    root = home / ".thirdshift"
    logs = root / "logs"
    logs.mkdir(parents=True)
    probe = logs / "probe"
    probe.write_text("before\n")
    # This script is the live writer, so the log stays open until expiry.
    commands = logs / "acme/widgets/commands/issue"
    commands.mkdir(parents=True)
    started = datetime.now().astimezone().isoformat()
    ticks = Path("/proc/self/stat").read_text().rsplit(") ", 1)[1].split()[19]
    boot = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
    (commands / "7-fixture.command.json").write_text(json.dumps({
        "version": 1, "kind": "Spec run", "repository": "acme/widgets",
        "issue": {"number": 7, "url": "https://github.com/acme/widgets/issues/7", "title": "Expiry"},
        "hostname": "fixture", "process_id": os.getpid(), "process_start": f"{boot}:{ticks}",
        "harness": "codex", "model": "gpt-6", "effort": "high",
        "started_at": started, "ended_at": None,
        "stages": [{"name": "implement", "began_at": started, "session_log": None}],
        "session_log": None, "outcome": None, "cause": None, "issue_branch": None,
        "pull_request": None, "tickets": [],
    }))
    log = commands / "7-fixture.log"
    log.write_text("before\n")
    (root / "config.toml").write_text(
        '[console]\nport = 0\nallowed_emails = ["owner@example.test"]\n')
    token = "a" * 64
    (root / "console-accounts.json").write_text(json.dumps({
        "owner@example.test": {"password_hash": "", "verified": True,
                               "sessions": {hashlib.sha256(token.encode()).hexdigest(): int(time.time())}}
    }))
    offset = temporary / "clock-offset"
    offset.write_text("0")
    environment = dict(os.environ, HOME=str(home), LD_PRELOAD=str(library),
                       THIRDSHIFT_REVIEW_CLOCK_OFFSET=str(offset))
    child = subprocess.Popen([str(args.binary.resolve()), "console"], env=environment,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    connections = []
    readers = []
    try:
        origin = child.stdout.readline().strip()
        assert origin.startswith("http://127.0.0.1:"), origin
        parsed = urlsplit(origin)
        cookie = {"Cookie": "__Host-thirdshift-session=" + token}

        def get(path):
            connection = http.client.HTTPConnection(parsed.hostname, parsed.port, timeout=90)
            connections.append(connection)
            connection.request("GET", path, headers=cookie)
            response = connection.getresponse()
            assert response.status == 200, (path, response.status)
            return response

        inventory = get("/logs").read().decode()
        listed = re.search(r'href="(/logs/[^"]+)">[^<]*7-fixture\.log</a>', inventory)
        assert listed, "The live log is missing from the Logs inventory"
        link = listed.group(1)
        streams = {"inbox": (get("/events"), b"event: commands"),
                   "log": (get(link + "/stream/0"), b"event: lines")}
        notices = {name: [] for name in streams}
        closed = {name: threading.Event() for name in streams}
        reader_errors = []

        def read_notices(name, response, event):
            try:
                while True:
                    line = response.readline()
                    if not line:
                        closed[name].set()
                        return
                    if line.startswith(event):
                        notices[name].append(time.monotonic())
            except Exception as error:
                reader_errors.append(f"{name}: {error!r}")
                closed[name].set()

        for name, (response, event) in streams.items():
            reader = threading.Thread(target=read_notices, args=(name, response, event))
            reader.start()
            readers.append(reader)
        deadline = time.monotonic() + 5
        while not all(notices.values()) and time.monotonic() < deadline:
            time.sleep(0.01)
        assert all(notices.values()), f"No initial notice: {notices}"
        assert not any(event.is_set() for event in closed.values()), "A stream closed early"
        time.sleep(0.5)
        offset.write_text(str(30 * 24 * 60 * 60 + 2))
        print("Shifted the Console child 30 days forward; waiting for one recovery interval", flush=True)
        time.sleep(args.settle_seconds)
        changed_at = time.monotonic()
        probe.write_text("after\n")
        with log.open("a") as appended:
            appended.write("after\n")
        deadline = time.monotonic() + 3
        while (not all(event.is_set() for event in closed.values())
               and not any(at >= changed_at for at in sum(notices.values(), []))
               and time.monotonic() < deadline):
            time.sleep(0.01)
        delivered = {name: any(at >= changed_at for at in at_times)
                     for name, at_times in notices.items()}
        stream_closed = {name: event.is_set() for name, event in closed.items()}
        print(json.dumps({"expired_session_received_changes": delivered,
                          "stream_closed": stream_closed, "reader_errors": reader_errors}), flush=True)
        assert not any(delivered.values()), "Expired session received fresh changes after the recovery interval"
        assert all(stream_closed.values()), "An expired session's stream remained open after the recovery interval"
    finally:
        child.terminate()
        try:
            child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait()
        for connection in connections:
            connection.close()
        for reader in readers:
            reader.join(timeout=5)
            assert not reader.is_alive(), "A stream reader did not stop"
