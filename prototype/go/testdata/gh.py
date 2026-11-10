#!/usr/bin/env python3
import json
import os
from pathlib import Path
import re
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
    pr["headRefOid"] = subprocess.check_output(["git", "--git-dir", str(root / "origin.git"), "rev-parse", "refs/heads/" + pr["headRefName"]], text=True).strip()
    return pr
pickup = json.loads((root / "pickup.json").read_text()) if (root / "pickup.json").exists() else {}
match = re.search(r"/issues/(\d+)", args[1]) if args[:1] == ["api"] else None
number = match[1] if match else next((arg[7:] for arg in args if arg.startswith("number=")), "7")
reads_path = root / "reads.json"
reads = json.loads(reads_path.read_text()) if reads_path.exists() else {}
config = pickup.get("issues", {}).get(number, {})
if match and "/events?" not in args[1] and "/comments" not in args[1] and not args[1].endswith("/labels"):
    reads[number] = reads.get(number, 0) + 1
    reads_path.write_text(json.dumps(reads))
if reads.get(number, 0) >= 2:
    config = config | config.get("refresh", {})
if args[:1] == ["api"]:
    if args[1] == "--include":
        code = pickup.get("members", {}).get(args[2].split("/")[-1], 404)
        print(f"HTTP/2.0 {code} Status\n")
        sys.exit(0 if code == 204 else 1)
    elif args[1] == "repos/acme/widgets":
        print(json.dumps({"owner": {"type": "Organization"}}))
    elif args[1] == "graphql":
        facts = {
            "parent": None, "subIssues": {"totalCount": 0},
            "blockedBy": {"nodes": [], "pageInfo": {"hasNextPage": False}},
            "shaped": {"nodes": []},
            "edits": {"nodes": [], "pageInfo": {"hasNextPage": False}}
        } | config.get("facts", {})
        if "edit_pages" in config:
            cursor = next((arg[6:] for arg in args if arg.startswith("after=")), "0")
            page = int(cursor)
            more = page + 1 < len(config["edit_pages"])
            facts["edits"] = {"nodes": config["edit_pages"][page],
                              "pageInfo": {"hasNextPage": more, "endCursor": str(page + 1)}}
        if config.get("bad_graphql"):
            print(json.dumps({"errors": [{"message": "unavailable"}]}))
        else:
            print(json.dumps({"data": {"repository": {"issue": facts}}}))
    elif args[1].endswith("/labels"):
        assert args[args.index("--method") + 1] == "PUT"
        payload = json.load(sys.stdin)
        assert isinstance(payload["labels"], list)
        state["labels"] = [{"name": name} for name in sorted(set(payload["labels"]))]
        (root / "state.json").write_text(json.dumps(state))
        if "in-progress" in payload["labels"] and state["mode"] == "interrupt-claim":
            os.kill(os.getppid(), signal.SIGINT)
            time.sleep(60)
        print(json.dumps(state["labels"]))
    elif "/issues?" in args[1]:
        print(json.dumps(pickup.get("pages", [[{"number": 7}]])))
    elif "/events?" in args[1]:
        # A label applied after the issue snapshot and before Claim. Production
        # gh is not involved; this only sequences the fixture's own state.
        label = pickup.get("poison_label")
        if reads.get(number, 0) >= 2 and isinstance(label, str) and label:
            names = [item.get("name") for item in state.get("labels", [])]
            if label not in names:
                state.setdefault("labels", []).append({"name": label})
                (root / "state.json").write_text(json.dumps(state))
        print(json.dumps(config.get("events", [[{"event": "labeled", "created_at": "2020-01-01T00:00:00Z",
                            "actor": {"login": "acme"}, "label": {"name": "ready-for-agent"}}]])))
    elif "/comments" in args[1]:
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
        if mode == "open-workflow":
            checks = [{"name": "fast", "status": "completed", "conclusion": "success"}]
        print(json.dumps([{"check_runs": checks}]))
    elif "/actions/runs" in args[1]:
        if "head_sha=" not in args[1]:
            sys.exit("workflow runs query missing head_sha")
        head = args[1].split("head_sha=")[1].split("&")[0]
        widget = subprocess.run(["git", "--git-dir", str(root / "origin.git"), "show", head + ":widget.txt"], capture_output=True, text=True).stdout
        reads = root / ("workflow-reads-" + head)
        count = int(reads.read_text()) if reads.exists() else 0
        reads.write_text(str(count + 1))
        runs = []
        if state["mode"] == "open-workflow":
            run = {"id": 1, "name": "CI", "status": "in_progress", "conclusion": None, "workflow_id": 10, "created_at": "2026-01-01T00:00:00Z"}
            if count > 0:
                run["status"] = "completed"
                run["conclusion"] = "failure" if widget == "broken\n" else "success"
            runs = [run]
        print(json.dumps([{"workflow_runs": runs}]))
    elif "/statuses" in args[1]:
        head = args[1].split("/commits/")[1].split("/")[0]
        count = int((root / ("ci-reads-" + head)).read_text())
        statuses = [{"context": "external", "state": "pending" if count == 1 else "success"}] if state["mode"] == "pending-status" else []
        print(json.dumps([statuses]))
    else:
        print(json.dumps(state | config.get("data", {})))
elif args[:2] == ["label", "create"]:
    pass
elif args[:2] == ["pr", "list"]:
    branch = args[args.index("--head") + 1] if "--head" in args else args[args.index("--search") + 1][5:]
    prs = pickup.get("prs", {}).get(branch, [])
    if pr_path.exists():
        pr = pr_data()
        if pr["headRefName"] == branch:
            prs = prs + [pr]
    print(json.dumps(prs))
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
