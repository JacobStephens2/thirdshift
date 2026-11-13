#!/usr/bin/env python3
"""Accept only the owner's tag of an exact Cargo bump of shipped main content."""
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

if sys.version_info < (3, 11):
    sys.exit("Python 3.11 or later is required; found Python "
             f"{'.'.join(map(str, sys.version_info[:3]))} at {sys.executable}")
import tomllib


def git(*args, data=None, env=None):
    return subprocess.check_output(["git", *args], input=data, env=env)


def bump(data, version, package=None):
    """Match release.sh's version-only rewrite, leaving every other line alone."""
    lines = []
    ours = False
    done = False
    for line in data.decode().splitlines():
        if package is None and line.startswith("["):
            ours = line == "[package]"
        elif package is not None:
            if line == "[[package]]":
                ours = False
            elif line == f'name = "{package}"':
                ours = True
        if ours and line.startswith("version = ") and (package is not None or not done):
            line = f'version = "{version}"'
            ours = False
            done = True
        lines.append(line)
    if not done:
        raise ValueError("Cargo package version not found")
    return ("\n".join(lines) + "\n").encode()


def expected_tree(base, version, index):
    env = dict(os.environ, GIT_INDEX_FILE=str(index))
    git("read-tree", base, env=env)
    manifest = git("show", f"{base}:Cargo.toml")
    package = tomllib.loads(manifest.decode())["package"]["name"]
    for path, data in [
        ("Cargo.toml", bump(manifest, version)),
        ("Cargo.lock", bump(git("show", f"{base}:Cargo.lock"), version, package)),
    ]:
        mode = git("ls-tree", base, "--", path).decode().split()[0]
        if mode not in ("100644", "100755"):
            raise ValueError(f"{path} is not a regular file on main")
        blob = git("hash-object", "-w", "--stdin", data=data).decode().strip()
        git("update-index", "--add", "--cacheinfo", mode, blob, path, env=env)
    return git("write-tree", env=env).strip()


def main():
    if os.environ.get("GITHUB_ACTOR") != "JacobStephens2":
        sys.exit("release tag refused: github.actor must be JacobStephens2")
    tag = os.environ.get("GITHUB_REF_NAME", "")
    if not re.fullmatch(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", tag):
        sys.exit("release tag refused: expected vX.Y.Z")
    tagged = git("rev-parse", "--verify", f"refs/tags/{tag}^{{commit}}").decode().strip()
    if tagged != os.environ.get("GITHUB_SHA"):
        sys.exit("release tag refused: tag no longer identifies the push event commit")
    target_tree = git("rev-parse", f"{tagged}^{{tree}}").strip()
    # Search all reachable commits: main may have advanced since selection, and
    # a repaired release head may be a merge with the old bump as first parent.
    candidates = git("rev-list", "refs/remotes/origin/main").decode().splitlines()
    with tempfile.TemporaryDirectory(prefix="release-tag-guard-") as directory:
        index = Path(directory) / "index"
        for base in candidates:
            changed = set(git("diff-tree", "-r", "--name-only", "-z", base, tagged).split(b"\0")) - {b""}
            if changed - {b"Cargo.toml", b"Cargo.lock"}:
                continue
            if expected_tree(base, tag[1:], index) == target_tree:
                print(f"{tag}: exact Cargo version bump of origin/main commit {base}")
                return
    sys.exit("release tag refused: tree is not an exact Cargo version bump of a commit reachable on origin/main")


if __name__ == "__main__":
    main()
