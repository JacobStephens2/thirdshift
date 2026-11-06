#!/usr/bin/env python3
import json
import os
from pathlib import Path
import re
import signal
import sys
import time

root = Path(os.environ["FIXTURE"])
args = sys.argv[1:]
with (root / "calls.jsonl").open("a") as calls:
    calls.write(json.dumps(args) + "\n")
state = json.loads((root / "state.json").read_text())
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
        print(json.dumps(config.get("events", [[{"event": "labeled", "created_at": "2020-01-01T00:00:00Z",
                            "actor": {"login": "acme"}, "label": {"name": "ready-for-agent"}}]])))
    elif "/comments" in args[1]:
        print(json.dumps([[{"author_association": "OWNER", "user": {"login": "owner"}, "body": "Trusted comment"},
                           {"author_association": "NONE", "user": {"login": "stranger"}, "body": "UNTRUSTED_PAYLOAD"}]]))
    else:
        print(json.dumps(state | config.get("data", {})))
elif args[:2] == ["label", "create"]:
    pass
elif args[:2] == ["pr", "list"]:
    branch = args[args.index("--head") + 1] if "--head" in args else args[args.index("--search") + 1][5:]
    print(json.dumps(pickup.get("prs", {}).get(branch, [])))
else:
    sys.exit("unexpected gh invocation: " + repr(args))
