#!/usr/bin/env python3
"""PROTOTYPE (#995), throwaway: build fixture.js for the span-tree prototype.

Reads Command records and their Session traces from ~/.thirdshift/logs and
writes one nested tree per Command: Command > Run > stage > session > spans.

    python3 -I src/console/span-tree-prototype/extract.py <command.json>...
"""
import hashlib, json, os, re, sys
from datetime import datetime

HERE = os.path.dirname(os.path.abspath(__file__))
SECRET = re.compile(r"(gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|sk-[A-Za-z0-9_-]{20,}"
                    r"|AKIA[0-9A-Z]{16}|xox[baprs]-[A-Za-z0-9-]+|-----BEGIN [A-Z ]*PRIVATE KEY-----)")
LIMIT = 600


def ms(text):
    return int(datetime.fromisoformat(text).timestamp() * 1000) if text else None


def clip(value):
    text = value if isinstance(value, str) else json.dumps(value)
    text = SECRET.sub("[redacted]", text)
    return text if len(text) <= LIMIT else text[:LIMIT] + f"… ({len(text) - LIMIT} more characters)"


def value(any_value):
    return next(iter(any_value.values())) if any_value else None


def requests(path):
    try:
        with open(path) as f:
            for line in f:
                if line.strip():
                    try:
                        yield json.loads(line)
                    except json.JSONDecodeError:
                        pass  # a live append may expose half a line
    except FileNotFoundError:
        return


def session(log, now):
    base = log[:-len(".jsonl")] if log.endswith(".jsonl") else log
    spans, resource = [], {}
    for r in requests(base + ".spans.jsonl"):
        for rs in r["resourceSpans"]:
            resource = {a["key"]: value(a["value"]) for a in rs.get("resource", {}).get("attributes", [])}
            for ss in rs["scopeSpans"]:
                spans += ss["spans"]
    if not spans and not os.path.exists(base + ".logs.jsonl"):
        return None
    nodes, root = {}, None
    for s in spans:
        attrs = {a["key"]: value(a["value"]) for a in s.get("attributes", [])}
        op = attrs.get("gen_ai.operation.name")
        if op == "invoke_agent":
            kind, name = "session", s["name"].removeprefix("invoke_agent ")
        elif op == "chat":
            kind, name = "model", attrs.get("gen_ai.response.model") or attrs.get("gen_ai.request.model") or "model call"
        elif op == "execute_tool":
            kind, name = "tool", attrs.get("gen_ai.tool.name") or s["name"]
            try:
                args = json.loads(attrs.get("gen_ai.tool.call.arguments") or "{}")
            except json.JSONDecodeError:
                args = {}
            detail = isinstance(args, dict) and (args.get("command") or args.get("description") or args.get("file_path") or args.get("pattern"))
            if detail:
                name += " " + str(detail).replace("/bin/bash -lc ", "")[:160]
        elif s["name"].startswith("agent "):
            kind, name = "agent", attrs.get("thirdshift.description") or "sub-agent"
        elif s["name"].startswith("turn "):
            kind, name = "turn", s["name"].replace("turn-", "")
        else:
            kind, name = "span", s["name"]
        start = int(s.get("startTimeUnixNano") or 0) // 1_000_000 or None
        end = int(s.get("endTimeUnixNano") or 0) // 1_000_000 or None
        code = (s.get("status") or {}).get("code", 0)
        node = {
            "id": s["spanId"], "kind": kind, "name": name, "start": start, "end": end,
            "status": {1: "ok", 2: "error"}.get(code, "unset"),
            "statusMessage": (s.get("status") or {}).get("message"),
            "estimated": attrs.get("thirdshift.time.estimated") in (True, "True", "true") or resource.get("thirdshift.migration") == "legacy",
            "attrs": {k: clip(v) for k, v in attrs.items() if k != "thirdshift.event"},
            "parent": s.get("parentSpanId"), "children": [],
        }
        nodes[s["spanId"]] = node
        if kind == "session":
            root = node
    # Tool calls still running exist only as log records until they end.
    ended = {n["attrs"].get("gen_ai.tool.call.id") for n in nodes.values()}
    for r in requests(base + ".logs.jsonl"):
        for rl in r["resourceLogs"]:
            for sl in rl["scopeLogs"]:
                for rec in sl["logRecords"]:
                    attrs = {a["key"]: value(a["value"]) for a in rec.get("attributes", [])}
                    for event in json.loads(attrs.get("thirdshift.events") or "[]"):
                        if event.get("type") != "tool_call_started" or event.get("id") in ended:
                            continue
                        ended.add(event.get("id"))
                        detail = event.get("input") if isinstance(event.get("input"), dict) else {}
                        node = {
                            "id": "rec-" + str(event.get("id")), "kind": "tool",
                            "name": f"{event.get('tool', 'Tool')} {str(detail.get('command', ''))[:160]}".strip(),
                            "start": int(rec.get("timeUnixNano") or rec.get("observedTimeUnixNano") or 0) // 1_000_000,
                            "end": None, "status": "running", "statusMessage": "still running",
                            "estimated": False,
                            "attrs": {"gen_ai.operation.name": "execute_tool", "gen_ai.tool.name": event.get("tool"),
                                      "gen_ai.tool.call.id": event.get("id"), "gen_ai.tool.call.arguments": clip(event.get("input"))},
                            "parent": rec.get("spanId"), "children": [],
                        }
                        nodes[node["id"]] = node
    if root is None:
        # A live session's own span is written when it ends.
        starts = [n["start"] for n in nodes.values() if n["start"]]
        root = {"id": "session-" + os.path.basename(base), "kind": "session",
                "name": base.rsplit("-", 1)[-1], "start": min(starts) if starts else None, "end": None,
                "status": "running", "statusMessage": "still running", "estimated": False,
                "attrs": {k: clip(v) for k, v in resource.items()}, "parent": None, "children": []}
        nodes[root["id"]] = root
    for node in nodes.values():
        if node is root:
            continue
        parent = nodes.get(node.pop("parent"), root)
        parent["children"].append(node)
    root.pop("parent", None)
    root["attrs"].update({f"resource {k}": clip(v) for k, v in resource.items()})
    root["file"] = os.path.basename(base) + ".spans.jsonl"
    for node in nodes.values():
        node["children"].sort(key=lambda n: n["start"] or 0)
    return root


def run(record, now, depth, path_of):
    ended = ms(record.get("ended_at"))
    issue = record.get("issue") or {}
    node = {
        "id": hashlib.sha256((record["name"] + record["started_at"]).encode()).hexdigest()[:16], "kind": "run" if depth else "command",
        "name": f"{record['kind']} · {record['name']}", "title": issue.get("title"),
        "start": ms(record["started_at"]), "end": ended,
        "status": "running" if not ended else ("error" if record.get("outcome") in ("failed", "interrupted") else "ok"),
        "statusMessage": record.get("cause") or record.get("outcome") or "live",
        "estimated": False,
        "attrs": {k: clip(record[k]) for k in ("kind", "harness", "model", "effort", "outcome", "cause", "pull_request", "issue_branch", "hostname") if record.get(k)},
        "children": [],
    }
    stages = record.get("stages", [])
    seen = set()
    children = list(record.get("dispatched", []))
    children += [json.load(open(p)) for p in path_of(record)]
    for i, stage in enumerate(stages):
        start = ms(stage["began_at"])
        end = ms(stages[i + 1]["began_at"]) if i + 1 < len(stages) else ended
        s = {"id": hashlib.sha256(f"{node['id']}{stage['name']}{stage['began_at']}".encode()).hexdigest()[:16], "kind": "stage", "name": stage["name"], "start": start, "end": end,
             "status": "running" if end is None else "ok", "statusMessage": None, "estimated": False,
             "attrs": {"stage": stage["name"], "began_at": stage["began_at"], "session_log": os.path.basename(stage.get("session_log") or "") or None},
             "children": []}
        log = stage.get("session_log")
        if log and log not in seen:
            seen.add(log)
            traced = session(log, now)
            if traced:
                s["children"].append(traced)
        for child in children:
            if child.get("_placed"):
                continue
            began = ms(child["started_at"])
            if began >= start and (end is None or began < end):
                child["_placed"] = True
                s["children"].append(run(child, now, depth + 1, path_of))
        s["children"].sort(key=lambda n: n["start"] or 0)
        node["children"].append(s)
    for child in children:
        if not child.pop("_placed", False):
            node["children"].append(run(child, now, depth + 1, path_of))
    return node


def main(paths):
    now = int(datetime.now().timestamp() * 1000)
    commands = []
    for path in paths:
        record = json.load(open(path))
        folder = os.path.dirname(path)
        stem = os.path.basename(path).removesuffix(".command.json")

        def path_of(r, stem=stem, folder=folder):
            # Ticket Runs are kept as `<stem>.command.dispatch-*.child.json` beside the Command.
            if r.get("kind") != "Spec run":
                return []
            return sorted(os.path.join(folder, f) for f in os.listdir(folder)
                          if f.startswith(stem + ".command.dispatch-") and f.endswith(".child.json"))
        tree = run(record, now, 0, path_of)
        selector = stem.split("-", 1)[1]
        commands.append({"selector": selector, "subject": record["name"], "tree": tree,
                         "cli": f"thirdshift show '{selector}' --json"})
    with open(os.path.join(HERE, "fixture.js"), "w") as f:
        f.write("// PROTOTYPE (#995) fixture, generated by extract.py from real Session traces. Wipe me.\n")
        f.write(f"window.FIXTURE = {json.dumps({'now': now, 'commands': commands})};\n")


if __name__ == "__main__":
    main(sys.argv[1:])
