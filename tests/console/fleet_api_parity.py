import json
import os
import pathlib
import select
import subprocess
import tempfile
import urllib.request

import sys
binary = sys.argv[1] if len(sys.argv) > 1 else str(pathlib.Path("target/debug/thirdshift").resolve())
servers = []
with tempfile.TemporaryDirectory(prefix="fleet-api-parity-") as root:
    def env(home):
        return dict(os.environ, HOME=str(home))

    def run(home, *args, token=None):
        return subprocess.run(
            [binary, *args], env=env(home), input=token,
            text=True, capture_output=True, check=True,
        ).stdout

    def start(home):
        (home / ".thirdshift").mkdir(parents=True)
        (home / ".thirdshift/config.toml").write_text("[console]\nport = 0\nfleet = []\n")
        child = subprocess.Popen(
            [binary, "console", "--no-sign-in"], env=env(home),
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
        )
        servers.append(child)
        assert select.select([child.stdout], [], [], 15)[0], "Console startup timeout"
        address = child.stdout.readline().strip()
        assert address.startswith("http://127.0.0.1:"), address
        return address

    try:
        caller = pathlib.Path(root) / "caller"
        owner = pathlib.Path(root) / "owner"
        caller_address = start(caller)
        owner_address = start(owner)
        token = run(owner, "console", "token", "create", "watcher").strip()
        run(caller, "console", "fleet", "add", "remote", owner_address, token=token + "\n")
        fleet = json.loads(run(caller, "status", "--json"))
        assert len(fleet["machines"]) == 2 and all(m["reachability"] == "ok" for m in fleet["machines"]), fleet
        cli = json.loads(run(caller, "status", "--local", "--json"))
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        with opener.open(caller_address + "/api/v1/status", timeout=15) as response:
            api = json.load(response)
        cli_names = sorted(machine["name"] for machine in cli["machines"])
        api_names = sorted(machine["name"] for machine in api["machines"])
        assert api_names == cli_names, {
            "api_machine_names": api_names,
            "cli_machine_names": cli_names,
        }
    finally:
        for child in servers:
            child.terminate()
        for child in servers:
            try:
                child.wait(timeout=15)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
