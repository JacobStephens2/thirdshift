#!/usr/bin/env python3
import json
import os
from pathlib import Path
import signal
import sys
import time

root = Path(os.environ["FIXTURE"])
args = sys.argv[1:]
with (root / "calls.jsonl").open("a") as calls:
    calls.write(json.dumps(args) + "\n")
state = json.loads((root / "state.json").read_text())
if args[:1] == ["api"]:
    if "/comments" in args[1]:
        print(json.dumps([[{"author_association": "OWNER", "user": {"login": "owner"}, "body": "Trusted comment"},
                           {"author_association": "NONE", "user": {"login": "stranger"}, "body": "UNTRUSTED_PAYLOAD"}]]))
    else:
        print(json.dumps(state))
elif args[:2] == ["label", "create"]:
    pass
elif args[:2] == ["issue", "edit"]:
    labels = {item["name"] for item in state["labels"]}
    for index, arg in enumerate(args):
        if arg == "--add-label": labels.update(args[index + 1].split(","))
        if arg == "--remove-label": labels.difference_update(args[index + 1].split(","))
    state["labels"] = [{"name": name} for name in sorted(labels)]
    (root / "state.json").write_text(json.dumps(state))
    if "--add-label" in args and args[args.index("--add-label") + 1] == "in-progress" and state["mode"] == "interrupt-claim":
        os.kill(os.getppid(), signal.SIGINT)
        time.sleep(60)
elif args[:2] == ["pr", "list"]:
    print("[]")
else:
    sys.exit("unexpected gh invocation: " + repr(args))
