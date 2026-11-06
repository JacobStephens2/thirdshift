#!/usr/bin/env python3
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

root = Path(os.environ["FIXTURE"])
assert sys.stdin.read() == ""
assert sys.argv[1:7] == ["-p", "--permission-mode", "auto", "--output-format", "stream-json", "--verbose"]
assert sys.argv[-1].startswith("/thirdshift-implement ")
skill = Path(".claude/skills/thirdshift-implement")
assert skill.is_symlink() and "Implement" in (skill / "SKILL.md").read_text()
issue = Path(".thirdshift-issue.md").read_text()
assert "Trusted comment" in issue and "UNTRUSTED_PAYLOAD" not in issue
state = json.loads((root / "state.json").read_text())
assert {item["name"] for item in state["labels"]} == {"enhancement", "in-progress"}
print(json.dumps({"type": "system", "subtype": "init", "session_id": "fixture"}), flush=True)
mode = state["mode"]
if mode in {"stubborn", "leader-exits"}:
    def handler(signum, frame):
        with (root / "signals").open("a") as events:
            events.write(f"{role}:{signum}\n")
        if role == "parent" and mode == "leader-exits":
            sys.exit(0)

    signal.signal(signal.SIGINT, handler)
    signal.signal(signal.SIGTERM, handler)
    role = "parent"
    child = os.fork()
    if child == 0:
        role = "child"
        (root / "child-ready").write_text(str(os.getpid()))
    else:
        (root / "parent-ready").write_text(str(os.getpid()))
    while True:
        time.sleep(1)
if mode == "stream":
    while not (root / "finish").exists():
        time.sleep(0.02)
if mode == "malformed":
    print("not JSON", flush=True)
    sys.exit(0)
if mode == "no-result":
    sys.exit(0)
if mode == "error-result":
    print(json.dumps({"type": "result", "is_error": True}), flush=True)
    sys.exit(0)
if mode in {"push", "push-fail"}:
    subprocess.run(["git", "push", "origin", "HEAD"], check=True)
    if mode == "push-fail":
        sys.exit(1)
if mode == "close":
    state["state"] = "closed"
    (root / "state.json").write_text(json.dumps(state))
print(json.dumps({"type": "result", "is_error": False, "result": "done"}), flush=True)
