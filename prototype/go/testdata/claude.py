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
names = {item["name"] for item in state["labels"]}
assert "in-progress" in names and "ready-for-agent" not in names, names
with (root / "sessions.jsonl").open("a") as sessions:
    sessions.write(json.dumps({"cwd": str(Path.cwd()), "prompt": sys.argv[-1]}) + "\n")
print(json.dumps({"type": "system", "subtype": "init", "session_id": "fixture"}), flush=True)
mode = state["mode"]
if mode == "reservation":
    number = sys.argv[-1].split("/issues/")[1].split()[0]
    ref = "refs/heads/thirdshift/pickup/" + number
    token = subprocess.check_output(["git", "ls-remote", "origin", ref], text=True).split()[0]
    subprocess.run(["git", "fetch", "origin", ref], check=True, capture_output=True)
    assert subprocess.check_output(["git", "show", "-s", "--format=%P", token], text=True).strip() == ""
    assert subprocess.check_output(["git", "ls-tree", "-r", token], text=True).strip() == ""
    message = subprocess.check_output(["git", "show", "-s", "--format=%B", token], text=True)
    assert all(field in message for field in ["Token:", "Host:", "Launch:", "Process:", "Started:"])
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
