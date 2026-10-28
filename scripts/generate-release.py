#!/usr/bin/env python3
"""Generate cargo-dist's workflow plus the repository's event policy.

cargo-dist's supported allow-dirty=["ci"] opt-out permits customization; this
command replaces its skipped freshness check. Run from the repository root.
"""
import argparse
import difflib
import re
from pathlib import Path
import subprocess
import sys
import tomllib


# Cargo packages src/ and skills/ and dist also includes README and LICENSE.
# Pipeline/toolchain changes must exercise the full release contract too.
RELEASE_PATHS = [
    "src/**", "skills/**", "Cargo.toml", "Cargo.lock", "dist-workspace.toml",
    "rust-toolchain.toml", "README.md", "LICENSE", ".github/workflows/**",
    "scripts/**",
]


# PR-reachable concrete jobs: minutes, sized from successful hosted runs.
# See docs/ci-performance.md. Reusable smoke jobs set their own bounds.
PR_JOB_TIMEOUTS = {
    "plan": 5,
    "build-local-artifacts": 20,
    "build-global-artifacts": 5,
}

LEAST_PRIVILEGE_PERMISSIONS = (
    '    permissions:\n'
    '      "actions": "write"\n'
    '      "contents": "read"\n'
)


def harden_release_workflow(generated):
    """Bound PR-reachable jobs and keep secrets off the PR smoke path.

    The plan and build jobs run on pull requests, so they get job-level
    least-privilege permissions (read the checkout, read/download and
    write/upload workflow artifacts), with execution timeouts. The publishing
    jobs keep the workflow top-level write permission. The smoke reusable call needs no
    secrets, so drop its `secrets: inherit` instead of forwarding every
    secret to a PR-reachable run; the publish-only reusable calls keep
    theirs.
    """
    for job, timeout in PR_JOB_TIMEOUTS.items():
        anchor = f"  {job}:\n"
        if generated.count(anchor) != 1:
            raise RuntimeError(
                f"cargo-dist workflow shape changed: expected one {anchor!r}"
            )
        permissions_anchor = f"{anchor}{LEAST_PRIVILEGE_PERMISSIONS}"
        if permissions_anchor not in generated:
            generated = generated.replace(anchor, permissions_anchor, 1)
        bounded_anchor = f"{permissions_anchor}    timeout-minutes: {timeout}\n"
        if bounded_anchor not in generated:
            generated = generated.replace(permissions_anchor, bounded_anchor, 1)
    smoke_anchor = "  custom-release-smoke:\n"
    if generated.count(smoke_anchor) == 1:
        start = generated.index(smoke_anchor)
        match = re.search(r"^  \S", generated[start + len(smoke_anchor):], re.MULTILINE)
        end = start + len(smoke_anchor) + match.start() if match else len(generated)
        block = generated[start:end]
        block = block.replace("    secrets: inherit\n", "")
        generated = generated[:start] + block + generated[end:]
    return generated


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    config_path = Path("dist-workspace.toml")
    config_bytes = config_path.read_bytes()
    config = tomllib.loads(config_bytes.decode())["dist"]
    version = config["cargo-dist-version"]
    installed = subprocess.check_output(["dist", "--version"], text=True).strip()
    if installed != f"cargo-dist {version}":
        raise RuntimeError(f"expected cargo-dist {version}, got {installed}")
    workflow = Path(".github/workflows/release.yml")
    original = workflow.read_bytes() if workflow.exists() else None
    try:
        # dist's --allow-dirty skips generation too. Temporarily remove only
        # our supported opt-out so the real generator and version check run.
        if config.get("allow-dirty") != ["ci"]:
            raise RuntimeError('expected allow-dirty = ["ci"] in dist-workspace.toml')
        config_text = config_bytes.decode()
        opt_out = 'allow-dirty = ["ci"]\n'
        if config_text.count(opt_out) != 1:
            raise RuntimeError("expected one allow-dirty line")
        config_path.write_text(config_text.replace(opt_out, ""))
        subprocess.run(["dist", "generate", "--mode", "ci"], check=True)
        generated = workflow.read_text()
    finally:
        config_path.write_bytes(config_bytes)
        if original is None:
            workflow.unlink(missing_ok=True)
        else:
            workflow.write_bytes(original)
    for anchor in ["  pull_request:\n", "\njobs:\n"]:
        if generated.count(anchor) != 1:
            raise RuntimeError(f"cargo-dist workflow shape changed: expected one {anchor!r}")
    # CI owns the shared event identity; carry its complete root block into
    # Release so the two workflows cannot silently drift apart.
    blocks = re.findall(r"^concurrency:\n(?:[ \t]+[^\n]*\n)+", Path(".github/workflows/ci.yml").read_text(), re.MULTILINE)
    if len(blocks) != 1:
        raise RuntimeError("expected one root concurrency block in CI")
    concurrency = blocks[0]
    paths = "    paths:\n" + "".join(f"      - '{path}'\n" for path in RELEASE_PATHS)
    generated = generated.replace("  pull_request:\n", "  pull_request:\n" + paths)
    generated = generated.replace("\njobs:\n", "\n" + concurrency + "\njobs:\n")
    generated = harden_release_workflow(generated)
    generated = "# Regenerate with python3 scripts/generate-release.py; do not edit.\n" + generated
    if args.check:
        previous = original.decode() if original else ""
        if previous != generated:
            sys.stderr.writelines(difflib.unified_diff(
                previous.splitlines(keepends=True), generated.splitlines(keepends=True),
                fromfile=str(workflow), tofile="regenerated",
            ))
            return 1
    else:
        workflow.write_text(generated)
    return 0


if __name__ == "__main__":
    sys.exit(main())
