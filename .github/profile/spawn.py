"""Temporary diagnostic for #894: what one process costs on this runner.

Times `N` runs of each command, one after another, in a scratch clone like
the tests' launch clone, and prints the median and mean in milliseconds.
"""

import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

N = 200


def timed(argv, cwd, env):
    times = []
    for _ in range(N):
        started = time.perf_counter()
        subprocess.run(argv, cwd=cwd, env=env, stdout=subprocess.DEVNULL, check=True)
        times.append((time.perf_counter() - started) * 1000)
    return statistics.median(times), statistics.mean(times)


def main():
    fakes, thirdshift = sys.argv[1], sys.argv[2]
    root = tempfile.mkdtemp()
    env = {
        "PATH": os.environ["PATH"],
        "HOME": root,
        "GIT_CONFIG_NOSYSTEM": "1",
        "FAKE_GH_STATE": os.path.join(root, "gh-state.json"),
        "FAKE_GH_RECORD": os.path.join(root, "gh-calls.json"),
    }
    with open(os.path.join(root, "gh-state.json"), "w") as f:
        f.write('{"repo":"acme/widgets","issues":{},"pulls":[]}')
    git = shutil.which("git")
    subprocess.run([git, "init", "-q", "repo"], cwd=root, env=env, check=True)
    repo = os.path.join(root, "repo")
    subprocess.run(
        [git, "-c", "user.name=a", "-c", "user.email=a@b", "commit", "-q", "--allow-empty", "-m", "x"],
        cwd=repo,
        env=env,
        check=True,
    )
    commands = {
        "/usr/bin/true": ["/usr/bin/true"],
        "/bin/sh -c true": ["/bin/sh", "-c", "true"],
        "git --version": [git, "--version"],
        "/usr/bin/git --version": ["/usr/bin/git", "--version"],
        "git rev-parse --git-common-dir": [git, "rev-parse", "--git-common-dir"],
        "git worktree list --porcelain -z": [git, "worktree", "list", "--porcelain", "-z"],
        "fake gh (debug) issue list": [fakes + "-debug", "issue", "list", "--repo", "acme/widgets", "--json", "number"],
        "fake gh (opt) issue list": [fakes + "-opt", "issue", "list", "--repo", "acme/widgets", "--json", "number"],
        "thirdshift (debug) --version": [thirdshift, "--version"],
    }
    print(f"git = {git}")
    for label, argv in commands.items():
        if label.startswith("fake"):
            gh = os.path.join(root, "gh")
            if os.path.lexists(gh):
                os.unlink(gh)
            os.symlink(argv[0], gh)
            argv = [gh] + argv[1:]
        try:
            median, mean = timed(argv, repo, env)
            print(f"{label}: median {median:.2f} ms, mean {mean:.2f} ms")
        except (subprocess.CalledProcessError, OSError) as error:
            print(f"{label}: {error}")


main()
