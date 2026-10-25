#!/usr/bin/env python3
"""Private Scoring samples, real local Git and fake GitHub for route/browser tests."""
import json
import os
from pathlib import Path
import subprocess
import sys

home = Path(sys.argv[1]).resolve()
private = home / ".thirdshift"
private.mkdir(parents=True, exist_ok=True)
(private / "config.toml").write_text("[console]\nport = 0\n")
repository = home / "repository"
repository.mkdir()
env = {**os.environ, "HOME": str(home), "GIT_CONFIG_NOSYSTEM": "1",
       "GIT_AUTHOR_DATE": "2026-10-19T00:00:00Z", "GIT_COMMITTER_DATE": "2026-10-19T00:00:00Z"}


def git(*args):
    return subprocess.run(["git", "-C", str(repository), *args], env=env,
                          text=True, capture_output=True, check=True).stdout.strip()


git("init", "-q", "--initial-branch=main", "--template=")
git("config", "user.name", "Console Fixture")
git("config", "user.email", "console@example.invalid")
git("config", "commit.gpgsign", "false")
(repository / "answer").write_text("42\n")
git("add", ".")
git("commit", "-qm", "Initial")
base = git("rev-parse", "HEAD")
(repository / "answer").write_text("41\n")
git("commit", "-qam", "Introduce wrong answer")
head = git("rev-parse", "HEAD")
(home / ".gitconfig").write_text(
    f'[url "{repository}"]\n    insteadOf = https://github.com/acme/widgets.git\n'
    '[protocol "file"]\n    allow = always\n[gc]\n    auto = 0\n[maintenance]\n    auto = false\n')
metadata = {
    "body": "Reviewed feature\nBuilt with codex 1.2.3 · builder · high <!-- thirdshift:built-with -->",
    "headRefOid": head, "baseRefName": "main", "headRefName": "feature",
    "mergeCommit": {"oid": head, "parents": {"nodes": [{"oid": base}]}},
    "mergedAt": "2026-10-20T00:00:00Z",
    "commits": {"nodes": [{"commit": {"parents": {"nodes": [{"oid": base}]}}} ]},
    "closingIssuesReferences": {"nodes": [], "pageInfo": {"hasNextPage": False}},
}
(home / "metadata.json").write_text(json.dumps(metadata))
bins = home / "bin"
bins.mkdir()
fake = bins / "gh"
fake.write_text('''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
home = Path(os.environ["HOME"])
if "issue" in sys.argv and "view" in sys.argv:
    print(json.dumps({"labels": []}))
elif any("BugOriginReview" in arg for arg in sys.argv):
    print(json.dumps({"data": {"repository": {"pullRequest": json.loads((home / "metadata.json").read_text())}}}))
else:
    sys.exit("Unexpected fake GitHub call: " + repr(sys.argv[1:]))
''')
fake.chmod(0o755)
criteria = ["Correct", "Scoped", "Tested", "Honest", "Standards"]
sample = {"repository": "acme/widgets", "number": 23, "url": "https://github.com/acme/widgets/pull/23",
          "merged_at": "2026-10-20T00:00:00Z", "harness": "codex", "model": "builder",
          "harness_version": "1.2.3", "effort": "high", "for_you": True,
          "assessment": {"grader": {"harness": "claude", "model": "grader"}, "scores": [
              {"criterion": c, "passed": True, "reason": "Model secret <script>unsafe()</script>"} for c in criteria]}}
origins = [{"issue": f"https://github.com/acme/widgets/issues/{number}",
            "closed_at": "2026-10-22T00:00:00Z", "state": "proposed", "commit": head,
            "reason": "The answer changed to 41. <script>unsafe()</script>",
            "pull_requests": [{"url": sample["url"], "run": None}]} for number in [8, 9, 10]]
directory = private / "evals/scoring"
directory.mkdir(parents=True)
(directory / "2026-W43.json").write_text(json.dumps({
    "week": "2026-W43", "window_start": "2026-10-17T00:00:00Z",
    "window_end": "2026-10-24T00:00:00Z", "seed": 7, "sample": [sample], "bug_origins": origins,
}))
