#!/usr/bin/env python3
"""Generate cargo-dist's workflow plus the repository's event policy.

cargo-dist's supported allow-dirty=["ci"] opt-out permits customization; this
command replaces its skipped freshness check. Run from the repository root.
"""
import argparse
import difflib
from pathlib import Path
import subprocess
import sys
import tomllib


CONCURRENCY = """# PR numbers distinguish forks with the same branch name. Unique non-PR
# groups preserve even pending main/tag runs (GitHub replaces pending runs).
concurrency:
  group: ${{ github.workflow }}-${{ github.event_name }}-${{ github.event.pull_request.number || github.ref }}-${{ github.event_name != 'pull_request' && github.run_id || '' }}
  cancel-in-progress: ${{ github.event_name == 'pull_request' }}

"""

# Cargo packages src/ and skills/ and dist also includes README and LICENSE.
# Pipeline/toolchain changes must exercise the full release contract too.
RELEASE_PATHS = [
    "src/**", "skills/**", "Cargo.toml", "Cargo.lock", "dist-workspace.toml",
    "rust-toolchain.toml", "README.md", "LICENSE", ".github/workflows/**",
    "scripts/**",
]


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
    paths = "    paths:\n" + "".join(f"      - '{path}'\n" for path in RELEASE_PATHS)
    generated = generated.replace("  pull_request:\n", "  pull_request:\n" + paths)
    generated = generated.replace("\njobs:\n", "\n" + CONCURRENCY + "jobs:\n")
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
