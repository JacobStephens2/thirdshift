#!/usr/bin/env python3
"""A security-fix bug must not become a Bug-origin Eval task.

Harmless local git and a fake gh only. No network. Exit 0 when this binary
has no bug-origins command, or when confirmation refuses the labelled bug and
leaves the proposal unsettled. Exit 1 when confirmation accepts it.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


binary = Path(sys.argv[1]).resolve()
help_run = subprocess.run(
    [str(binary), "help"], text=True, capture_output=True, check=False
)
if "bug-origins" not in help_run.stdout:
    print("PASS: bug-origins is absent, so confirmation cannot write an Eval task")
    raise SystemExit(0)

with tempfile.TemporaryDirectory(prefix="thirdshift-security-fix-") as temporary:
    root = Path(temporary)
    home, bins, repository = (root / name for name in ("home", "bin", "repository"))
    for directory in (home, bins, repository):
        directory.mkdir()
    env = {
        "PATH": f"{bins}:{os.environ['PATH']}",
        "HOME": str(home),
        "XDG_CONFIG_HOME": str(home / ".config"),
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_TERMINAL_PROMPT": "0",
    }

    def git(*args):
        result = subprocess.run(
            ["git", "-C", str(repository), *args],
            env=env,
            text=True,
            capture_output=True,
            check=False,
        )
        if result.returncode != 0:
            sys.stderr.write(result.stderr)
            raise SystemExit("local git fixture failed")
        return result.stdout.strip()

    git("init", "-q", "--initial-branch=main")
    git("config", "user.name", "Review Fixture")
    git("config", "user.email", "review@example.invalid")
    (repository / "answer").write_text("42\n")
    git("add", ".")
    git("commit", "-qm", "Initial")
    base = git("rev-parse", "HEAD")
    git("checkout", "-qb", "feature")
    (repository / "answer").write_text("41\n")
    git("commit", "-qam", "Introduce wrong answer")
    head = git("rev-parse", "HEAD")
    git("checkout", "-q", "main")
    git("merge", "--no-ff", "-qm", "Merge reviewed feature", "feature")
    merge = git("rev-parse", "HEAD")
    slug = "acme/widgets"
    (home / ".gitconfig").write_text(
        f'[url "{repository}"]\n'
        f"    insteadOf = https://github.com/{slug}.git\n"
        '[protocol "file"]\n'
        "    allow = always\n"
    )
    issue = f"https://github.com/{slug}/issues/8"
    pr = f"https://github.com/{slug}/pull/23"
    connection = lambda nodes: {"nodes": nodes, "pageInfo": {"hasNextPage": False}}
    metadata = {
        "body": "Reviewed feature",
        "headRefOid": head,
        "mergeCommit": {
            "oid": merge,
            "parents": {"nodes": [{"oid": base}]},
        },
        "baseRefName": "main",
        "headRefName": "feature",
        "mergedAt": "2026-10-21T00:00:00Z",
        "commits": {"nodes": [{"commit": {"parents": {"nodes": [{"oid": base}]}}}]},
        "closingIssuesReferences": connection([]),
    }
    (root / "metadata.json").write_text(json.dumps(metadata))
    fake = bins / "gh"
    fake.write_text(
        r'''#!/usr/bin/env python3
import json, os, sys
if len(sys.argv) >= 3 and sys.argv[1] == "issue" and sys.argv[2] == "view":
    # The bug carries security-fix now, whatever its labels were when the
    # proposal was saved.
    print(json.dumps({"labels": [{"name": "bug"}, {"name": "security-fix"}]}))
    raise SystemExit(0)
query = next((a for a in sys.argv if a.startswith("query=")), "")
if "BugOriginReview" in query:
    field, node = "pullRequest", json.load(open(os.environ["REVIEW_FIXTURE"]))
elif "ScoringBody" in query or "ScoringTitle" in query:
    field, node = "issue", {
        "title": "Feature",
        "body": "Implement the feature.",
        "lastEditedAt": None,
        "edits": {"nodes": [], "pageInfo": {"hasNextPage": False}},
    }
else:
    sys.stderr.write("Unexpected gh invocation: " + repr(sys.argv) + "\n")
    raise SystemExit(2)
print(json.dumps({"data": {"repository": {field: node}}}))
'''
    )
    fake.chmod(0o755)
    env["REVIEW_FIXTURE"] = str(root / "metadata.json")
    scoring = home / ".thirdshift/evals/scoring"
    scoring.mkdir(parents=True)
    (scoring / "first.json").write_text(
        json.dumps(
            {
                "week": "2026-W43",
                "window_start": "2026-10-15T00:00:00Z",
                "window_end": "2026-10-23T00:00:00Z",
                "seed": 1,
                "sample": [],
                "bug_origins": [
                    {
                        "issue": issue,
                        "closed_at": "2026-10-22T00:00:00Z",
                        "state": "proposed",
                        "commit": head,
                        "reason": "The introducing PR writes 41 instead of 42.",
                        "pull_requests": [{"url": pr, "run": None}],
                    }
                ],
            }
        )
    )

    confirmed = subprocess.run(
        [str(binary), "bug-origins", "8", "--confirm", "--json"],
        env=env,
        cwd=root,
        text=True,
        capture_output=True,
        check=False,
    )
    tasks = home / ".thirdshift/evals/suites"
    created = sorted(path for path in tasks.rglob("*") if path.is_dir()) if tasks.exists() else []
    saved = json.loads((scoring / "first.json").read_text())
    state = saved["bug_origins"][0]["state"]
    if confirmed.returncode == 0 or state == "confirmed" or created:
        sys.stderr.write(confirmed.stdout)
        sys.stderr.write(confirmed.stderr)
        sys.stderr.write(
            f"security-fix bug was confirmed (exit {confirmed.returncode}, "
            f"state {state}, eval dirs {len(created)})\n"
        )
        raise SystemExit(1)
    if "Security fixes cannot" not in confirmed.stderr:
        sys.stderr.write(confirmed.stderr)
        sys.stderr.write("confirmation failed without the security-fix refusal\n")
        raise SystemExit(1)
    print("PASS: security-fix confirmation refused and wrote no Eval task")
