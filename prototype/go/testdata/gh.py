#!/usr/bin/env python3
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

root = Path(os.environ["FIXTURE"])
args = sys.argv[1:]
with (root / "calls.jsonl").open("a") as calls:
    calls.write(json.dumps(args) + "\n")
state = json.loads((root / "state.json").read_text())
pr_path = root / "pr.json"
def pr_data():
    pr = json.loads(pr_path.read_text())
    pr["headRefOid"] = subprocess.check_output(["git", "--git-dir", str(root / "origin.git"), "rev-parse", "refs/heads/issue-7"], text=True).strip()
    return pr
if args[:1] == ["api"]:
    if "/comments" in args[1]:
        print(json.dumps([[{"author_association": "OWNER", "user": {"login": "owner"}, "body": "Trusted comment"},
                           {"author_association": "NONE", "user": {"login": "stranger"}, "body": "UNTRUSTED_PAYLOAD"}]]))
    elif "/check-runs" in args[1]:
        head = args[1].split("/commits/")[1].split("/")[0]
        widget = subprocess.run(["git", "--git-dir", str(root / "origin.git"), "show", head + ":widget.txt"], capture_output=True, text=True).stdout
        reads = root / ("ci-reads-" + head)
        count = int(reads.read_text()) if reads.exists() else 0
        reads.write_text(str(count + 1))
        mode = state["mode"]
        conclusion = "failure" if (mode == "ci-repair" and widget == "broken\n") or (mode == "ci-still-red" and widget) else "success"
        checks = [{"name": "test", "status": "completed", "conclusion": conclusion}]
        if mode in {"pending-check", "checks-disappear"} and count == 0:
            checks[0].update(status="in_progress", conclusion=None)
        if mode == "no-ci" or (mode == "checks-disappear" and count > 0):
            checks = []
        if mode == "pr-closes":
            pr = pr_data()
            pr["state"] = "CLOSED"
            pr_path.write_text(json.dumps(pr))
        if mode == "partial-register":
            # First read is a fast success. The next read, still inside the
            # registration grace, is when the failing check appears.
            if count == 0:
                checks = [{"name": "fast", "status": "completed", "conclusion": "success"}]
            else:
                conclusion = "failure" if widget == "broken\n" else "success"
                checks = [
                    {"name": "fast", "status": "completed", "conclusion": "success"},
                    {"name": "security", "status": "completed", "conclusion": conclusion},
                ]
        print(json.dumps([{"check_runs": checks}]))
    elif "/statuses" in args[1]:
        head = args[1].split("/commits/")[1].split("/")[0]
        count = int((root / ("ci-reads-" + head)).read_text())
        statuses = [{"context": "external", "state": "pending" if count == 1 else "success"}] if state["mode"] == "pending-status" else []
        print(json.dumps([statuses]))
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
    print(json.dumps([pr_data()] if pr_path.exists() else []))
elif args[:2] == ["pr", "create"]:
    assert "--draft" in args
    pr_path.write_text(json.dumps({"number": 8, "url": "https://github.com/acme/widgets/pull/8", "state": "OPEN", "isDraft": True,
                                  "headRefName": args[args.index("--head") + 1], "baseRefName": args[args.index("--base") + 1], "mergeable": "MERGEABLE"}))
    print("https://github.com/acme/widgets/pull/8")
elif args[:2] == ["pr", "view"]:
    print(json.dumps(pr_data()))
elif args[:2] == ["pr", "ready"]:
    pr = pr_data()
    pr["isDraft"] = "--undo" in args
    pr_path.write_text(json.dumps(pr))
else:
    sys.exit("unexpected gh invocation: " + repr(args))
