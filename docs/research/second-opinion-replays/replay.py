#!/usr/bin/env python3
"""Replays of past thirdshift reviews, for #429 (Second opinion).

Each unit is one read-only agent session over past work, in a throwaway
repository holding only the history up to the commit under review:

  bugs6   the PRs behind six timing bugs, reviewed as merged (forward)
  bugs3r  the PRs behind three bugs in Codex's work (reverse)
  plansF  20 plans Claude's Architecture reviews wrote, checked (forward)
  plansR  20 plans Codex's Architecture reviews wrote, checked (reverse)
  specs   30 Spec branches as they stood before their Spec reviews

The design, bars and arms are settled in #429's grilling log
(.scratch/grill-with-docs/429-multi-family-review.md).

  replay.py context          fetch each issue body the sessions need
  replay.py status           what has run, per replay and arm
  replay.py run STAGE        run every pending unit on STAGE's Harness,
                             one at a time, stopping at a limit warning
  replay.py show UNIT        print one unit's prompt and command

Raw output stays outside the repository, under ROOT.
"""

import json
import os
import re
import shutil
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

HOME = Path.home()
ROOT = HOME / ".thirdshift/replays/2026-10-09"
FORWARD = HOME / ".thirdshift/replays/inputs-2026-10-07/manifest.json"
REVERSE = ROOT / "inputs/manifest.json"
CHECKLIST = ROOT / "timing-checklist.md"
SKILLS = ROOT / "skills"
CONTEXT = ROOT / "context"
RUNS = ROOT / "runs"
OWNER = "JacobStephens2"
TIMEOUT = 9000  # seconds per session

# Each arm's Harness, Model and Effort (#429 Q9, Q31). Claude runs at max where
# it is the baseline and at high where it is the Second opinion being tried.
SETTINGS = {
    "claude-max": ("claude", "claude-opus-5-5", "max"),
    "claude-high": ("claude", "claude-opus-5-5", "high"),
    "codex-xhigh": ("codex", "gpt-6.1-sol", "xhigh"),
    # The new Harnesses, each at its best measured setting (#429 Q17).
    "agy-high": ("agy", "gemini-3.8-flash", "high"),
    "grok-xhigh": ("grok", "grok-4.7", "xhigh"),
    "muse-max": ("muse", "muse-spark-1.3", "max"),
    "opencode-mimo": ("opencode", "primalabs/primalabs-ai/MiMo-V2.6-Pro", None),
}

# Stop a stage when a session's weekly window passes this, or warns.
CODEX_WEEKLY_STOP = 85.0


# ---------------------------------------------------------------- the units


def load(path):
    return json.loads(path.read_text()) if path.exists() else None


def units():
    """Every unit, in the order its stage runs them."""
    forward = load(FORWARD)
    reverse = load(REVERSE) or {}
    out = []

    # bugs6: one item per introducing PR; #22 carries #57 and #102.
    prs = {}
    for bug in forward["timing_bugs"]:
        pr = bug["introducing_pr"]
        item = prs.setdefault(pr["number"], {
            "item": f"thirdshift-pr{pr['number']}",
            "repo": "thirdshift",
            "head": pr["head_as_merged"],
            "base": pr["merge_base"],
            "branch": pr["head_ref"],
            "issues": [pr["implements_issue"]],
            "bugs": [],
        })
        item["bugs"].append(bug["id"])
    for arm, settings, extra in [
        ("fresh", "claude-max", None),
        ("checklist", "claude-max", "checklist"),
        ("other", "codex-xhigh", None),
        ("agy", "agy-high", None),
        ("grok", "grok-xhigh", None),
        ("muse", "muse-max", None),
        ("opencode", "opencode-mimo", None),
    ]:
        for item in prs.values():
            out.append(dict(item, replay="bugs6", arm=arm, settings=settings,
                            kind="review", skill="today", extra=extra))

    # bugs3r: the PRs behind #559, #603 and #607.
    seen = {}
    for bug in reverse.get("codex_bugs", []):
        for pr in bug["introducing_prs"]:
            item = seen.setdefault(pr["number"], {
                "item": f"thirdshift-pr{pr['number']}",
                "repo": "thirdshift",
                "head": pr["head_as_merged"],
                "base": pr["merge_base"],
                "branch": pr["head_ref"],
                "issues": [pr["implements_issue"]],
                "bugs": [],
            })
            item["bugs"].append(bug["id"])
    for arm, settings in [("other", "claude-high"), ("fresh", "codex-xhigh")]:
        for item in seen.values():
            out.append(dict(item, replay="bugs3r", arm=arm, settings=settings,
                            kind="review", skill="today", extra=None))

    # plansF and plansR.
    def plan_items(plans):
        return [{
            "item": f"{p['repo']}-{p['plan_issue']}",
            "repo": p["repo"],
            "head": p["base_commit"],
            "base": None,
            "branch": p["base_branch"],
            "issues": [p["plan_issue"]],
            "bugs": [],
        } for p in plans]

    for arm, settings in [("other", "codex-xhigh"), ("fresh", "claude-max")]:
        for item in plan_items(forward["plans"]):
            out.append(dict(item, replay="plansF", arm=arm, settings=settings,
                            kind="plan", skill=None, extra=None))
    # othermax: Claude at max on Codex's plans, added by #429 Q34 (c) after
    # Claude at high stopped too few to pass.
    for arm, settings in [("other", "claude-high"), ("fresh", "codex-xhigh"), ("othermax", "claude-max")]:
        for item in plan_items(reverse.get("codex_plans", [])):
            out.append(dict(item, replay="plansR", arm=arm, settings=settings,
                            kind="plan", skill=None, extra=None))

    # specs: the old skill, to match the recorded Spec reviews (#429 Q26).
    # fresh, a Claude session that never saw the work, separates a second look
    # from a second family.
    for arm, settings in [("other", "codex-xhigh"), ("fresh", "claude-max")]:
        for s in forward["spec_reviews"]:
            out.append({
                "item": f"{s['repo']}-{s['spec_issue']}",
                "repo": s["repo"],
                "head": s["pre_review_head"],
                "base": s["merge_base"],
                "branch": s["spec_branch"],
                "issues": [s["spec_issue"]],
                "bugs": [],
                "replay": "specs", "arm": arm, "settings": settings,
                "kind": "review", "skill": "old", "extra": None,
            })

    # specsR: fresh Codex on the Spec branches Codex's own Spec reviews saw, a
    # pairing no other replay tried (#429 Q35). Each keeps the skill its
    # recorded review ran. #486 was reviewed twice; only the first review is
    # independent, so only it is replayed.
    codex_specs = load(ROOT / "inputs/codex-specs/manifest.json") or {}
    seen = set()
    for s in sorted(codex_specs.get("codex_spec_reviews", []), key=lambda s: s["spec_review_started_at"]):
        if (s["repo"], s["spec_issue"]) in seen:
            continue
        seen.add((s["repo"], s["spec_issue"]))
        out.append({
            "item": f"{s['repo']}-{s['spec_issue']}",
            "repo": s["repo"],
            "head": s["pre_review_head"],
            "base": s["merge_base"],
            "branch": s["spec_branch"],
            "issues": [s["spec_issue"]],
            "bugs": [],
            "replay": "specsR", "arm": "fresh", "settings": "codex-xhigh",
            "kind": "review",
            "skill": "today" if s["review_skill"].startswith("evidence") else "old",
            "extra": None,
        })

    for u in out:
        u["id"] = f"{u['replay']}/{u['arm']}/{u['item']}"
        u["harness"] = SETTINGS[u["settings"]][0]
    return out


# ------------------------------------------------------------ issue bodies


QUERY = """query($o:String!,$r:String!,$n:Int!){repository(owner:$o,name:$r){
issue(number:$n){number title body parent{number}
subIssues(first:50){nodes{number}}
userContentEdits(first:100){nodes{editedAt diff}}}}}"""


def fetch_issue(repo, number):
    """The issue's title and body as first published, its parent and Tickets."""
    path = CONTEXT / repo / f"{number}.json"
    if path.exists():
        return json.loads(path.read_text())
    raw = subprocess.run(
        ["gh", "api", "graphql", "-f", f"query={QUERY}", "-F", f"o={OWNER}",
         "-F", f"r={repo}", "-F", f"n={number}"],
        check=True, capture_output=True, text=True).stdout
    issue = json.loads(raw)["data"]["repository"]["issue"]
    edits = sorted(issue["userContentEdits"]["nodes"], key=lambda e: e["editedAt"])
    first = edits[0]["diff"] if edits and edits[0]["diff"] is not None else issue["body"]
    record = {
        "number": number,
        "title": issue["title"],
        "body": first,
        "edited": len(edits) > 0,
        "parent": issue["parent"]["number"] if issue["parent"] else None,
        "tickets": [n["number"] for n in issue["subIssues"]["nodes"]],
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(record, indent=1))
    return record


def spec_text(repo, numbers):
    """The spec a reviewer gets: each issue, its parent Spec, and a Spec's Tickets."""
    parts, done = [], set()

    def add(n, role):
        if n in done:
            return
        done.add(n)
        i = fetch_issue(repo, n)
        parts.append(f"# {role} #{n}: {i['title']}\n\n{i['body'].strip()}\n")
        return i

    for n in numbers:
        i = add(n, "Issue")
        if i and i["parent"]:
            add(i["parent"], "Parent Spec")
        if i:
            for t in i["tickets"]:
                add(t, "Ticket")
    return "\n\n".join(parts)


def plan_text(repo, number):
    i = fetch_issue(repo, number)
    text = f"# Plan #{number}: {i['title']}\n\n{i['body'].strip()}\n"
    for t in i["tickets"]:
        ti = fetch_issue(repo, t)
        text += f"\n\n# Ticket #{t}: {ti['title']}\n\n{ti['body'].strip()}\n"
    return text


# ---------------------------------------------------------------- prompts


RULES = """Rules for this session:
- Only read. Use read-only commands such as git, grep, rg, ls, cat, sed and find.
- Don't build, compile, run tests or install anything. Where a finding needs a test, write it out in full in the report; don't run it.
- Change nothing in the repository: no edits, no commits, no branches.
- Don't use gh, the network or the web. Everything you need is in the repository and in {ctx}.
- Write only in {out}."""

CODEX_FRESH = ("Start fresh sub-agents with `fork_turns: \"none\"`, giving each only "
               "its own task and necessary evidence, so the reviewers stay independent.")

REVIEW = """/thirdshift-code-review Review branch `{branch}` with `base` as the fixed point: this is a review-only replay of a past change, for an experiment.

The originating spec is in `{ctx}/spec.md`: pass that path as the spec. There is no issue tracker in this session.

{rules}

Write the reviewers' reports, with each axis's final files-read list, to `{out}/standards.md` (Standards) and `{out}/spec.md` (Spec). Write both even when an axis has no findings.{extra}
End with the skill's one-line summary."""

CHECKLIST_EXTRA = """

Give both reviewers the timing checklist below, pasted in full into their prompts, and ask each to check the diff against it as well as its own brief.

{checklist}
"""

PLAN = """This is a read-only check of a plan an Architecture review published, for an experiment. The repository is checked out at the commit the plan was made on. The plan, as published, is in `{ctx}/plan.md`.

The Architecture review rated this plan Strong. A candidate is Strong only when all of these hold:
- The friction is real and you can point to it in the code.
- The deepening passes the deletion test.
- Its whole design can be settled from the code, `CONTEXT.md` and the ADRs. A candidate with a decision only a human can make is not Strong.
- It contradicts no ADR, or the friction is real enough to warrant reopening that ADR.

The `thirdshift-codebase-design` skill holds the design vocabulary, including the deletion test.

Check the plan yourself: re-apply the four tests, reading the plan, `CONTEXT.md` if there is one, the ADRs and the code. Check too that the plan's decisions hold up against the code: anything that would make the change fail or go wrong when it is built. Then decide whether the plan should be stopped before thirdshift marks it ready for the factory to build.

{rules}

Write your check to `{out}/verdict.md`: each test with pass or fail and its evidence (file and line), then any decision in the plan that is wrong and why, and as the very last line exactly `Verdict: stop` or `Verdict: pass`."""


def skill_prompt(prompt, harness):
    """How each Harness loads a skill named at the start of a prompt (src/harness/adapter.rs)."""
    rest = prompt.removeprefix("/thirdshift-")
    if rest == prompt or harness in ("claude", "agy", "grok"):
        return prompt
    if harness == "codex":
        return "$thirdshift-" + rest
    skill, _, remaining = rest.partition(" ")
    return f"Load thirdshift-{skill} with your skill tool. {remaining}"


def prompt_for(u, ws):
    ctx, out = ws / "context", ws / "out"
    rules = RULES.format(ctx=ctx, out=out)
    if u["kind"] == "review":
        extra = ""
        if u["extra"] == "checklist":
            extra = CHECKLIST_EXTRA.format(checklist=CHECKLIST.read_text().strip())
        text = REVIEW.format(branch=u["branch"], ctx=ctx, out=out, rules=rules, extra=extra)
    else:
        text = PLAN.format(ctx=ctx, out=out, rules=rules)
    if u["harness"] == "codex":
        text += "\n\n" + CODEX_FRESH
    return skill_prompt(text, u["harness"])


# ------------------------------------------------------------ the sessions


def command_for(u, ws, prompt):
    harness, model, effort = SETTINGS[u["settings"]]
    if harness == "claude":
        return ["claude", "-p",
                "--add-dir", str(ws / "out"), str(ws / "context"),
                "--disallowedTools", "WebFetch", "WebSearch",
                "--permission-mode", "auto", "--model", model, "--effort", effort,
                "--strict-mcp-config", "--output-format", "stream-json", "--verbose",
                prompt]
    if harness == "codex":
        return ["codex", "exec", "--json", "--dangerously-bypass-approvals-and-sandbox",
                "-m", model, "-c", f'model_reasoning_effort="{effort}"',
                "-c", 'project_doc_fallback_filenames=["CLAUDE.md"]', prompt]
    if harness == "agy":
        # Since agy 1.3.3, -p takes the prompt as its value.
        return ["agy", "-p", prompt, "--dangerously-skip-permissions", "--output-format", "stream-json",
                "--model", model, "--effort", effort]
    if harness == "grok":
        return ["grok", "--always-approve", "--sandbox", "off",
                "--output-format", "streaming-messages-json",
                "-m", model, "--reasoning-effort", effort, "-p", prompt]
    if harness == "muse":
        return ["muse", "exec", "--json", "--yolo", "--model", model,
                "--reasoning-effort", effort, prompt]
    if harness == "opencode":
        return ["opencode", "run", "--standalone", "--format", "json", "--auto", "-m", model, prompt]
    raise SystemExit(f"no command for {harness}")


def clean_env():
    """The environment a cron-started session would have, with gh signed out."""
    env = {k: v for k, v in os.environ.items()
           if not (k.startswith(("CLAUDE", "HERDR_")) or k in (
               "AI_AGENT", "GIT_EDITOR", "GH_TOKEN", "GITHUB_TOKEN",
               "COREPACK_ENABLE_AUTO_PIN", "NoDefaultCurrentDirectoryInExePath"))}
    gh = ROOT / ".gh-signed-out"
    gh.mkdir(parents=True, exist_ok=True)
    env["GH_CONFIG_DIR"] = str(gh)
    env.update(GROK_DISABLE_AUTOUPDATER="1", MUSE_NO_AUTO_UPDATE="1", OPENCODE_DISABLE_AUTOUPDATE="1")
    # In print mode Claude Code ends the turn while the review's sub-agents
    # run, then stops them after ten minutes; wait for them instead.
    env["CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS"] = "0"
    return env


def git(repo, *args, **kw):
    return subprocess.run(["git", "-C", str(repo), *args], check=True,
                          capture_output=True, text=True, **kw).stdout.strip()


def build_workspace(u, ws):
    """A repository holding only the history up to the commit under review."""
    repo, ctx, out = ws / "repo", ws / "context", ws / "out"
    shutil.rmtree(repo, ignore_errors=True)
    shutil.rmtree(out, ignore_errors=True)
    ctx.mkdir(parents=True, exist_ok=True)
    out.mkdir(parents=True)
    clone = HOME / "repos" / u["repo"]
    subprocess.run(["git", "init", "-q", "-b", "replay-init", str(repo)], check=True)
    refspecs = [f"{u['head']}:refs/heads/{u['branch']}"]
    if u["base"]:
        refspecs.append(f"{u['base']}:refs/heads/base")
    git(repo, "fetch", "-q", "--no-tags", str(clone), *refspecs)
    git(repo, "checkout", "-q", u["branch"])
    skills = []
    if u["kind"] == "review":
        skills.append(SKILLS / u["skill"] / "thirdshift-code-review")
    else:
        skills.append(SKILLS / "design" / "thirdshift-codebase-design")
    for place in (".claude/skills", ".agents/skills"):
        for s in skills:
            dest = repo / place / s.name
            shutil.rmtree(dest, ignore_errors=True)
            shutil.copytree(s, dest)
    with open(repo / ".git/info/exclude", "a") as f:
        f.write("/.claude/skills/thirdshift-*\n/.agents/skills/thirdshift-*\n")
    if u["kind"] == "review":
        (ctx / "spec.md").write_text(spec_text(u["repo"], u["issues"]))
    else:
        (ctx / "plan.md").write_text(plan_text(u["repo"], u["issues"][0]))
    return git(repo, "status", "--porcelain")


def finished(u, ws):
    out = ws / "out"
    if u["kind"] == "review":
        return all((out / f).exists() and (out / f).stat().st_size > 0
                   for f in ("standards.md", "spec.md"))
    v = out / "verdict.md"
    return v.exists() and re.search(r"^Verdict: (stop|pass)\s*$", v.read_text(), re.M)


def limits(u, ws):
    """What the session's stream says about its Harness's limits."""
    seen = {"stop": False, "wait_until": None, "notes": []}
    events = ws / "events.jsonl"
    if not events.exists():
        return seen
    for line in events.read_text(errors="replace").splitlines():
        if u["harness"] == "claude" and '"rate_limit_event"' in line:
            info = json.loads(line).get("rate_limit_info", {})
            kind, status = info.get("rateLimitType"), info.get("status")
            if status in ("allowed_warning", "rejected"):
                seen["notes"].append(f"{kind} {status} {info.get('utilization')}")
                if kind == "five_hour" and status == "rejected":
                    seen["wait_until"] = info.get("resetsAt")
                else:
                    seen["stop"] = True
        elif u["harness"] == "codex" and '"rate_limits"' in line:
            m = re.search(r'"rate_limits":(\{.*?"plan_type":[^}]*\})', line)
            if not m:
                continue
            rl = json.loads(m.group(1))
            primary = rl.get("primary") or {}
            if rl.get("rate_limit_reached_type") or (primary.get("used_percent") or 0) >= CODEX_WEEKLY_STOP:
                seen["stop"] = True
                seen["notes"].append(f"codex {primary.get('used_percent')}% {rl.get('rate_limit_reached_type')}")
    text = (ws / "stderr.log").read_text(errors="replace") if (ws / "stderr.log").exists() else ""
    if re.search(r"usage limit|rate limit|hit your limit", text, re.I):
        seen["stop"] = True
        seen["notes"].append("limit message on stderr")
    return seen


def commands(ws):
    """Every shell command and tool call the session made, from its stream."""
    found = []
    events = ws / "events.jsonl"
    if not events.exists():
        return found
    for line in events.read_text(errors="replace").splitlines():
        try:
            e = json.loads(line)
        except json.JSONDecodeError:
            continue
        # Claude: assistant tool_use blocks. Codex: command_execution items.
        for c in (e.get("message") or {}).get("content") or []:
            if isinstance(c, dict) and c.get("type") == "tool_use":
                found.append((c.get("name"), c.get("input", {}).get("command") or json.dumps(c.get("input"))))
        item = e.get("item") or {}
        if item.get("type") == "command_execution":
            found.append(("shell", item.get("command", "")))
    return found


# A tool in command position: at the start, or after ; & | ( or a subshell's $(.
AT = r"(?:^|(?<!\\)[;&|(]|\$\()\s*(?:\w+=\S*\s+)*"  # not a grep pattern's \\|


def flags(ws):
    """Signs a session broke the read-only rules, from the shell commands it ran."""
    found = set()
    for tool, cmd in commands(ws):
        if tool in ("WebFetch", "WebSearch"):
            found.add("web tool")
        if tool in ("Edit", "Write", "NotebookEdit") and "/out/" not in (cmd or ""):
            found.add("edit")
        if tool not in ("Bash", "shell"):
            continue
        for pattern, name in [
            (AT + r"gh\s", "gh"),
            (AT + r"(curl|wget)\s", "network"),
            (AT + r"(cargo\s+(build|test|check|run|clippy|nextest)|(npm|pnpm|yarn)\s+(test|run|install|ci)|pytest|go\s+test)\b", "build or test"),
            (AT + r"git\s+(-C\s+\S+\s+)?(commit|push|reset|stash\s+(push|pop|apply|drop|save)|rebase|merge|cherry-pick|apply|add)(\s|$)", "git write"),
        ]:
            if re.search(pattern, cmd or "", re.M):
                found.add(name)
    return sorted(found)


def run_unit(u):
    ws = RUNS / u["id"]
    ws.mkdir(parents=True, exist_ok=True)
    status_before = build_workspace(u, ws)
    prompt = prompt_for(u, ws)
    cmd = command_for(u, ws, prompt)
    (ws / "prompt.txt").write_text(prompt)
    (ws / "command.json").write_text(json.dumps([a if a != prompt else "<prompt.txt>" for a in cmd], indent=1))
    repo = ws / "repo"
    head = git(repo, "rev-parse", "HEAD")
    version = subprocess.run([cmd[0], "--version"], capture_output=True, text=True).stdout.strip()
    start = time.time()
    stdin_prompt = u["harness"] == "opencode"  # OpenCode reads its prompt on stdin
    argv = cmd[:-1] if stdin_prompt else cmd
    with open(ws / "events.jsonl", "w") as ev, open(ws / "stderr.log", "w") as er:
        code = subprocess.run(["timeout", "-k", "120", str(TIMEOUT), *argv], cwd=repo,
                              env=clean_env(), input=prompt if stdin_prompt else None,
                              stdin=None if stdin_prompt else subprocess.DEVNULL,
                              stdout=ev, stderr=er, text=True).returncode
    meta = {
        "id": u["id"], "unit": u, "cli_version": version,
        "started": datetime.fromtimestamp(start, timezone.utc).isoformat(),
        "wall_seconds": round(time.time() - start), "exit_status": code,
        "timed_out": code in (124, 137),
        "head_before": head, "head_after": git(repo, "rev-parse", "HEAD"),
        "repo_changed": git(repo, "status", "--porcelain") != status_before,
        "repo_diff": git(repo, "diff", "--stat"),
        "finished": bool(finished(u, ws)), "flags": flags(ws), "limits": limits(u, ws),
    }
    (ws / "meta.json").write_text(json.dumps(meta, indent=1))
    shutil.rmtree(repo)  # rebuildable from the unit; the reports are in out/
    return meta


def state(u):
    meta = load(RUNS / u["id"] / "meta.json")
    if meta is None:
        return "pending"
    if meta["finished"] and meta["exit_status"] == 0:
        return "done"
    if meta["limits"]["stop"] or meta["limits"]["wait_until"]:
        return "limited"
    return "failed"


def ready(u):
    """Whether the unit's inputs exist yet."""
    return u["head"] and (u["kind"] != "review" or u["extra"] != "checklist" or CHECKLIST.exists())


def cmd_run(stage, only=None):
    """Run the stage's next pending unit until none is left, picking up units
    whose inputs arrive while it runs."""
    n = 0
    while True:
        queue = [u for u in units() if u["harness"] == stage and state(u) in ("pending", "limited")
                 and ready(u) and (only is None or u["replay"] in only)]
        if not queue:
            print(f"stage done after {n} units", flush=True)
            return 0
        u, n = queue[0], n + 1
        print(f"[{datetime.now():%H:%M:%S}] #{n}, {len(queue)} left: {u['id']} ({u['settings']})", flush=True)
        meta = run_unit(u)
        print(f"    exit {meta['exit_status']} in {meta['wall_seconds']}s, finished={meta['finished']}"
              f" flags={meta['flags']} limits={meta['limits']['notes']}", flush=True)
        lim = meta["limits"]
        if lim["wait_until"] and not lim["stop"]:
            pause = max(0, lim["wait_until"] - time.time()) + 120
            print(f"    five-hour window full: waiting {pause / 60:.0f} minutes", flush=True)
            time.sleep(pause)
        elif lim["stop"]:
            print(f"    stopping at a limit warning: {lim['notes']}", flush=True)
            return 3


def cmd_retry(ids):
    """Set failed units aside as attempts, so the next run repeats them."""
    for u in units():
        if (u["id"] in ids or "failed" in ids) and state(u) == "failed":
            ws = RUNS / u["id"]
            n = 1
            while ws.with_name(f"{ws.name}.attempt{n}").exists():
                n += 1
            ws.rename(ws.with_name(f"{ws.name}.attempt{n}"))
            print("set aside", u["id"], f"as attempt {n}")


def cmd_status():
    rows = {}
    for u in units():
        key = (u["replay"], u["arm"], u["settings"])
        rows.setdefault(key, {"pending": 0, "done": 0, "failed": 0, "limited": 0})
        rows[key][state(u)] += 1
    for (replay, arm, settings), c in rows.items():
        print(f"{replay:7} {arm:9} {settings:12} " + " ".join(f"{k}={v}" for k, v in c.items() if v))


def cmd_context():
    for u in units():
        if u["kind"] == "review":
            spec_text(u["repo"], u["issues"])
        else:
            plan_text(u["repo"], u["issues"][0])
    print("context fetched into", CONTEXT)


def cmd_show(unit_id):
    u = next(u for u in units() if u["id"] == unit_id)
    ws = RUNS / u["id"]
    print(json.dumps(u, indent=1))
    prompt = prompt_for(u, ws)
    print(" ".join(command_for(u, ws, "<prompt>")))
    print(prompt)


if __name__ == "__main__":
    args = sys.argv[1:]
    if not args:
        sys.exit(__doc__)
    if args[0] == "retry":
        sys.exit(cmd_retry(set(args[1:])))
    if args[0] == "flags":
        for u in units():
            if (RUNS / u["id"] / "meta.json").exists():
                print(u["id"], flags(RUNS / u["id"]))
        sys.exit()
    if args[0] == "run":
        sys.exit(cmd_run(args[1], set(args[2].split(",")) if len(args) > 2 else None))
    {"status": cmd_status, "context": cmd_context}.get(args[0], lambda: cmd_show(args[1]))()
