"""Fleet schedules share CLI/page data and isolate failures at owning APIs."""
import http.server
from html import unescape
import json
import os
from pathlib import Path
import select
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request


binary = sys.argv[1]
with tempfile.TemporaryDirectory(prefix="cronjobs-fleet-") as temp:
    home = Path(temp)
    root = home / ".thirdshift"
    root.mkdir()
    (home / "bin").mkdir()
    crontab = home / "bin/crontab"
    crontab.write_text('#!/bin/sh\ncat "$HOME/crontab"\n')
    crontab.chmod(0o755)
    (home / "crontab").write_text('@daily thirdshift "$UNKNOWN"\n')
    (root / "config.toml").write_text("[console]\nport = 0\n")
    calls = []
    active = False
    job = {"repository": "acme/widgets", "command_kind": "Pickup run",
           "schedule": "*/5 * * * *", "command": "cd /remote/only && thirdshift pickup"}
    inventories = {
        "east": {"jobs": [job, dict(job, schedule="@daily")], "pass_starts": []},
        "west": {"jobs": [dict(job, command_kind="Security run"),
                          dict(job, repository="acme/tools"),
                          {"schedule": "@reboot", "command": 'thirdshift "<script>"',
                           "command_kind": "Future run"}],
                 "pass_starts": []},
        "empty": {"jobs": [], "pass_starts": []},
        "old": {"jobs": "incompatible"},
    }

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            name = self.path.split('/')[1]
            calls.append((name, self.path, self.headers.get("x-thirdshift-person")))
            status = 200
            if self.path.endswith("/whoami"):
                value = dict(version=1, api_version="v1", hostname=name,
                             thirdshift_version="fixture", scopes=["read"])
            elif active and name == "refused":
                status, value = 403, {"error": "read refused"}
            elif active and name == "timeout":
                time.sleep(4)
                value = inventories["empty"]
            else:
                value = inventories.get(name, inventories["empty"])
            body = json.dumps(value).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            try:
                self.wfile.write(body)
            except BrokenPipeError:
                pass

        def log_message(self, *args):
            pass

    owner = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=owner.serve_forever, daemon=True)
    thread.start()
    child = None
    try:
        env = dict(os.environ, HOME=str(home), PATH=f"{home / 'bin'}:{os.environ['PATH']}")
        for name in ["east", "west", "empty", "old", "refused", "timeout"]:
            subprocess.run([binary, "console", "fleet", "add", name,
                            f"http://127.0.0.1:{owner.server_port}/{name}"], env=env,
                           input="fixture-token\n", text=True, capture_output=True, check=True)
        with (root / "config.toml").open("a") as config:
            config.write('\n[[console.fleet]]\nname = "link"\naddress = "https://link.example.test"\n')
        active = True
        calls.clear()
        output = subprocess.run([binary, "cronjobs", "--fleet", "--json"], env=env,
                                text=True, capture_output=True, timeout=30)
        assert output.returncode == 0, output.stderr
        fleet = json.loads(output.stdout)
        states = {machine["name"]: machine for machine in fleet["machines"]}
        for name in ["east", "west", "empty"]:
            assert states[name]["status"] == "ok", states
        for name, state in [("old", "incompatible"), ("refused", "refused"),
                            ("timeout", "unavailable"), ("link", "link-only")]:
            assert states[name]["status"] == state, states
        assert all(name != "link" and person is None for name, _, person in calls), calls
        groups = fleet["repositories"]
        assert [group["repository"] for group in groups] == ["acme/tools", "acme/widgets", None], groups
        widgets = groups[1]["machines"]
        assert [machine["name"] for machine in widgets] == ["east", "west"], widgets
        assert [entry["schedule"] for entry in widgets[0]["jobs"]] == ["*/5 * * * *", "@daily"]
        assert widgets[1]["jobs"][0]["command_kind"] == "Security run"
        assert len(groups[2]["machines"]) == 2, groups[2]
        assert groups[2]["machines"][1]["jobs"][0]["command_kind"] is None, groups[2]
        child = subprocess.Popen([binary, "console", "--no-sign-in"], env=env,
                                 stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        assert select.select([child.stdout], [], [], 15)[0], "Console startup timeout"
        address = child.stdout.readline().strip()
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        with opener.open(address + "/cronjobs?fleet=true", timeout=30) as response:
            html = response.read().decode()
        for text in ["acme/widgets", "acme/tools", "east", "west", "Pickup run", "Security run",
                     "Other", "empty", "No thirdshift cron jobs", "incompatible", "refused",
                     "unavailable", "link-only", "https://link.example.test"]:
            assert text in html, (text, html)
        assert 'thirdshift "<script>"' in unescape(html) and "<script>" not in html, html
        assert "Last fired" not in html, html
        calls.clear()
        for query in ["?fleet=true", "?machine=east", "?fleet=true&machine=east"]:
            try:
                opener.open(address + "/api/v1/cronjobs" + query, timeout=15)
            except urllib.error.HTTPError as error:
                assert error.code in [400, 422], error.code
            else:
                raise AssertionError("API accepted a Fleet relay")
        assert not calls, calls
        # Both the hostname and its first local fallback are valid Fleet names.
        # Keep local and remote schedules identifiable even under that collision.
        local_hostname = socket.gethostname()
        for name, route in [(local_hostname, "east"), (local_hostname + " (local)", "west")]:
            subprocess.run([binary, "console", "fleet", "add", name,
                            f"http://127.0.0.1:{owner.server_port}/{route}"], env=env,
                           input="fixture-token\n", text=True, capture_output=True, check=True)
        output = subprocess.run([binary, "cronjobs", "--fleet", "--json"], env=env,
                                text=True, capture_output=True, timeout=30)
        assert output.returncode == 0, output.stderr
        collision = json.loads(output.stdout)
        other = next(group for group in collision["repositories"] if group["repository"] is None)
        assert len(other["machines"]) == 3, "Local and remote schedules were merged: " + json.dumps(other)
        names = [machine["name"] for machine in collision["machines"]]
        assert len(set(names)) == len(names), names
        print("Fleet CLI/page grouping, empty inventories, partial failures and API isolation passed")
    finally:
        if child is not None:
            child.terminate()
            try:
                child.wait(timeout=15)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
        owner.shutdown()
        owner.server_close()
        thread.join(timeout=15)
