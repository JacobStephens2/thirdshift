"""Remote cron inventory is authoritative, including older payloads."""
import http.server
import json
import os
from pathlib import Path
import select
import subprocess
import sys
import tempfile
import threading
import urllib.request


binary = sys.argv[1]
with tempfile.TemporaryDirectory(prefix="cronjobs-remote-") as temp:
    home = Path(temp)
    root = home / ".thirdshift"
    root.mkdir()
    (root / "config.toml").write_text("[console]\nport = 0\n")
    checkout = home / "local-checkout"
    subprocess.run(["git", "init", "-q", str(checkout)], check=True)
    subprocess.run(["git", "-C", str(checkout), "remote", "add", "origin",
                    "https://github.com/local/wrong.git"], check=True)
    job = {"schedule": "@daily", "command": f"cd {checkout} && thirdshift pickup"}
    inventory = {"jobs": [job], "pass_starts": []}

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path == "/api/v1/whoami":
                value = dict(version=1, api_version="v1", hostname="owner",
                             thirdshift_version="fixture", scopes=["read"])
            elif self.path == "/api/v1/cronjobs":
                value = inventory
            else:
                self.send_error(404)
                return
            body = json.dumps(value).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    owner = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=owner.serve_forever, daemon=True)
    thread.start()
    child = None
    try:
        env = dict(os.environ, HOME=str(home))
        subprocess.run([binary, "console", "fleet", "add", "owner",
                        f"http://127.0.0.1:{owner.server_port}"], env=env,
                       input="fixture-token\n", text=True, capture_output=True, check=True)
        child = subprocess.Popen([binary, "console", "--no-sign-in"], env=env,
                                 stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        assert select.select([child.stdout], [], [], 15)[0], "Console startup timeout"
        address = child.stdout.readline().strip()
        assert address.startswith("http://127.0.0.1:"), address
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        for repository in [None, "remote/widgets"]:
            if repository is not None:
                job["repository"] = repository
            with opener.open(address + "/cronjobs?machine=owner", timeout=15) as response:
                html = response.read().decode()
            heading = repository or "Other"
            assert f"<h2>{heading}</h2>" in html, html
            assert "local/wrong" not in html, html
            assert "Run Passes now" not in html, html
            assert "Last fired by repository" in html, html
        print("Remote and legacy cron inventories retain owning-machine association")
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
