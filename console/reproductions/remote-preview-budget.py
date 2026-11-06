#!/usr/bin/env python3
"""Assert that opening remote Peek does not fetch over 4 KiB of log text."""
import json
import os
from pathlib import Path
import selectors
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.request import urlopen
from urllib.parse import parse_qs, urlsplit

repo = Path(__file__).resolve().parents[2]
binary = Path(os.environ.get("THIRDSHIFT_BINARY", repo / "target/debug/thirdshift")).resolve()
log = "x" * (1024 * 1024) + "\nlast remote line\n"
served_log_bytes = []


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path.startswith("/api/v1/peek?"):
            value = {"version": 1, "command": {"kind": "Run", "repository": "acme/widgets", "name": "fixture", "state": "ended", "outcome": "failed"}}
        elif self.path.startswith("/api/v1/log?"):
            # The existing log endpoint's CLI twin now supports a bounded
            # tail. An ordinary full-log request still exposes the regression.
            contents = log[-4096:] if parse_qs(urlsplit(self.path).query).get('tail') == ['true'] else log
            served_log_bytes.append(len(contents.encode()))
            value = {"version": 1, "contents": contents}
        else:
            self.send_error(404)
            return
        body = json.dumps(value).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_):
        pass


owner = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
thread = threading.Thread(target=owner.serve_forever, daemon=True)
thread.start()
try:
    with tempfile.TemporaryDirectory(prefix="thirdshift-preview-budget-") as home:
        root = Path(home) / ".thirdshift"
        root.mkdir()
        (root / "config.toml").write_text('[console]\nport = 0\nfleet = [{name = "remote", address = "http://127.0.0.1:%d"}]\n' % owner.server_port)
        credentials = root / "credentials.toml"
        credentials.write_text('[fleet]\nremote = "fixture"\n')
        credentials.chmod(0o600)
        caller = subprocess.Popen([str(binary), "console", "--no-sign-in"], env={**os.environ, "HOME": home}, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            with selectors.DefaultSelector() as ready:
                ready.register(caller.stdout, selectors.EVENT_READ)
                assert ready.select(15), "Console did not announce its address"
            origin = caller.stdout.readline().strip()
            with urlopen(origin + "/peek?machine=remote&command=widgets%237", timeout=15) as response:
                html = response.read().decode()
            assert "last remote line" in html, html
            count = sum(served_log_bytes)
            print(f"Remote Peek fetched {count} log-text bytes for its preview")
            assert count <= 4096, f"remote preview fetched {count} bytes; documented budget is 4096"
        finally:
            caller.terminate()
            try:
                caller.communicate(timeout=10)
            except subprocess.TimeoutExpired:
                caller.kill()
                caller.communicate()
finally:
    owner.shutdown()
    owner.server_close()
    thread.join()
