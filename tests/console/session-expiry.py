#!/usr/bin/env python3
"""An open Inbox stream must stop accepting notices after its session expires."""
import argparse
import hashlib
import http.client
import json
import os
from pathlib import Path
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
    connection = None
    reader = None
    try:
        origin = child.stdout.readline().strip()
        assert origin.startswith("http://127.0.0.1:"), origin
        parsed = urlsplit(origin)
        connection = http.client.HTTPConnection(parsed.hostname, parsed.port, timeout=90)
        connection.request("GET", "/events", headers={"Cookie": "__Host-thirdshift-session=" + token})
        response = connection.getresponse()
        assert response.status == 200, response.status
        notices = []
        closed = threading.Event()
        reader_errors = []

        def read_notices():
            try:
                while True:
                    line = response.readline()
                    if not line:
                        closed.set()
                        return
                    if line.startswith(b"event: commands"):
                        notices.append(time.monotonic())
            except Exception as error:
                reader_errors.append(repr(error))
                closed.set()

        reader = threading.Thread(target=read_notices)
        reader.start()
        deadline = time.monotonic() + 5
        while not notices and time.monotonic() < deadline:
            time.sleep(0.01)
        assert notices, "No initial Inbox notice"
        time.sleep(0.5)
        offset.write_text(str(30 * 24 * 60 * 60 + 2))
        print("Shifted the Console child 30 days forward; waiting for one recovery interval", flush=True)
        time.sleep(args.settle_seconds)
        changed_at = time.monotonic()
        probe.write_text("after\n")
        deadline = time.monotonic() + 3
        while not closed.is_set() and not any(at >= changed_at for at in notices) and time.monotonic() < deadline:
            time.sleep(0.01)
        delivered = any(at >= changed_at for at in notices)
        print(json.dumps({"expired_session_received_commands": delivered,
                          "stream_closed": closed.is_set(), "reader_errors": reader_errors}), flush=True)
        assert not delivered, "Expired session received a fresh commands notice after the recovery interval"
        assert closed.is_set(), "Expired session's stream remained open after the recovery interval"
    finally:
        child.terminate()
        try:
            child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait()
        if connection:
            connection.close()
        if reader:
            reader.join(timeout=5)
            assert not reader.is_alive(), "Inbox reader did not stop"
