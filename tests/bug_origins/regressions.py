#!/usr/bin/env python3
"""Isolated CLI reproducers; no repository source or tests are modified."""
import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import tomllib


binary = Path(sys.argv[1]).resolve()
mode = sys.argv[2]
assert mode in {"merged-base", "no-issue", "reclosed", "dotted-repo", "save-failure"}
slug = "acme/widget.tools" if mode == "dotted-repo" else "acme/widgets"
with tempfile.TemporaryDirectory(prefix="thirdshift-spec-") as temporary:
    root = Path(temporary)
    home, bins, repository = (root / name for name in ("home", "bin", "repository"))
    for directory in (home, bins, repository):
        directory.mkdir()
    env = {
        "PATH": f"{bins}:{os.environ['PATH']}",
        "HOME": str(home),
        "XDG_CONFIG_HOME": str(home / ".config"),
        "GIT_CONFIG_NOSYSTEM": "1",
        "REVIEW_FIXTURE": str(root / "metadata.json"),
    }

    def git(*args):
        result = subprocess.run(["git", "-C", str(repository), *args], env=env,
                                text=True, capture_output=True)
        assert result.returncode == 0, result.stderr
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
    actual_review_base = base
    if mode == "merged-base":
        git("checkout", "-q", "main")
        (repository / "upstream-only").write_text("unrelated base-branch work\n")
        git("add", ".")
        git("commit", "-qm", "Unrelated upstream change")
        actual_review_base = git("rev-parse", "HEAD")
        git("checkout", "-q", "feature")
        git("merge", "--no-ff", "-qm", "Merge main before review", "main")
        head = git("rev-parse", "HEAD")
        assert git("diff", "--name-only", f"{actual_review_base}...{head}") == "answer"
    git("checkout", "-q", "main")
    git("merge", "--no-ff", "-qm", "Merge reviewed feature", "feature")
    merge = git("rev-parse", "HEAD")
    (home / ".gitconfig").write_text(
        f'[url "{repository}"]\n    insteadOf = https://github.com/{slug}.git\n'
        '[protocol "file"]\n    allow = always\n'
    )
    issue = f"https://github.com/{slug}/issues/8"
    pr = f"https://github.com/{slug}/pull/23"
    connection = lambda nodes: {"nodes": nodes, "pageInfo": {"hasNextPage": False}}
    metadata = {
        "body": "Reviewed feature", "headRefOid": head,
        "mergeCommit": {"oid": merge, "parents": {"nodes": [{"oid": actual_review_base}]}},
        "baseRefName": "main", "headRefName": "feature",
        "mergedAt": "2026-10-21T00:00:00Z",
        "commits": {"nodes": [{"commit": {"parents": {"nodes": [{"oid": base}]}}}]},
        "closingIssuesReferences": connection([] if mode == "no-issue" else [{
            "url": f"https://github.com/{slug}/issues/7", "labels": connection([])
        }]),
    }
    (root / "metadata.json").write_text(json.dumps(metadata))
    fake = bins / "gh"
    fake.write_text('''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
if len(sys.argv) >= 3 and sys.argv[1] == "issue" and sys.argv[2] == "view":
    marker = Path(os.environ["REVIEW_FIXTURE"]).parent / "block-checkpoint"
    if marker.exists():
        marker.unlink()
        checkpoint = Path(os.environ["HOME"]) / ".thirdshift/evals/scoring/first.json"
        checkpoint.rename(checkpoint.with_suffix(".saved"))
        checkpoint.mkdir()
    print(json.dumps({"labels": []}))
    raise SystemExit(0)
query = next((a for a in sys.argv if a.startswith("query=")), "")
if "BugOriginReview" in query:
    field, node = "pullRequest", json.load(open(os.environ["REVIEW_FIXTURE"]))
elif "ScoringBody" in query or "ScoringTitle" in query:
    field, node = "issue", {"title": "Feature", "body": "Implement the feature.",
        "lastEditedAt": None, "edits": {"nodes": [],
        "pageInfo": {"hasNextPage": False}}}
else:
    sys.exit("Unexpected gh invocation: " + repr(sys.argv))
print(json.dumps({"data": {"repository": {field: node}}}))
''')
    fake.chmod(0o755)
    scoring = home / ".thirdshift/evals/scoring"
    scoring.mkdir(parents=True)
    original = {
        "issue": issue, "closed_at": "2026-10-22T00:00:00Z", "state": "proposed",
        "commit": head, "reason": "The introducing PR writes 41 instead of 42.",
        "pull_requests": [{"url": pr, "run": None}],
    }
    week = {"week": "2026-W43", "window_start": "2026-10-15T00:00:00Z",
            "window_end": "2026-10-23T00:00:00Z", "seed": 1,
            "sample": [], "bug_origins": [original]}
    (scoring / "first.json").write_text(json.dumps(week))

    def run(*args):
        return subprocess.run([str(binary), *args], env=env, cwd=root,
                              text=True, capture_output=True)

    confirmed = run("bug-origins", "8", "--confirm", "--json")
    assert confirmed.returncode == 0, (
        "A merged introducing PR should be confirmable without a linked issue:\n"
        + confirmed.stderr
    )
    confirmed_origin = json.loads(confirmed.stdout)[0]
    reference = confirmed_origin["settlement"]["eval_task"]
    task_dir = home / ".thirdshift/evals/suites" / reference["suite"] / "tasks" / reference["task"]
    if mode == "merged-base":
        task = tomllib.loads((task_dir / "task.toml").read_text())
        patch_files = subprocess.check_output([
            "git", "-C", str(task_dir / "repository"), "diff", "--name-only",
            task["base"], task["change"]
        ], env=env, text=True).splitlines()
        assert patch_files == ["answer"], (
            f"Reviewed diff changes only answer; proposed Eval also contains {patch_files}"
        )
    elif mode == "save-failure":
        # The retained handle is loaded before GitHub runs. Make its atomic
        # replacement fail after preparing a correction to a different PR.
        (root / "block-checkpoint").touch()
        failed = run("bug-origins", "8", "--correct", "24", "--json")
        assert failed.returncode != 0, "checkpoint replacement should fail"
        assert "Is a directory" in failed.stderr, failed.stderr
        assert task_dir.is_dir(), "failed settlement withdrew the retained decision's Eval proposal"
        (scoring / "first.json").rmdir()
        (scoring / "first.saved").rename(scoring / "first.json")
        retained = run("bug-origins", "8", "--json")
        assert retained.returncode == 0, retained.stderr
        assert json.loads(retained.stdout)[0] == confirmed_origin
    elif mode == "reclosed":
        # A reopened bug closes again in a later Scoring window, which retains
        # a new unsettled proposal alongside the old confirmed checkpoint.
        later = copy.deepcopy(week)
        later.update(seed=2, window_start=week["window_end"],
                     window_end="2026-10-24T00:00:00Z")
        later["bug_origins"][0]["closed_at"] = "2026-10-23T12:00:00Z"
        (scoring / "second.json").write_text(json.dumps(later))
        rejected = run("bug-origins", "8", "--none", "--json")
        assert rejected.returncode == 0, rejected.stderr
        assert json.loads(rejected.stdout)[-1]["state"] == "rejected"
        listed = run("evals", "list")
        assert listed.returncode == 0, listed.stderr
        pending = [task for suite in json.loads(listed.stdout)
                   for task in suite["tasks"] if task["state"] == "proposed"]
        assert pending == [], f"Rejected latest Bug origin retains unapproved Eval tasks: {pending}"
    print(f"PASS: {mode}")
