#!/usr/bin/env python3
"""Snapshot Factory skills inside the Go module (go:embed cannot reach ../..)."""
import argparse
import json
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--check", action="store_true")
args = parser.parse_args()
module = Path(__file__).resolve().parent
source = module.parent.parent / "skills"
files = {path.relative_to(source).as_posix(): path.read_text()
         for path in sorted(source.rglob("*")) if path.is_file()}
snapshot = json.dumps(files, ensure_ascii=False, indent=2) + "\n"
target = module / "factory-skills.json"
if args.check:
    if target.read_text() != snapshot:
        raise SystemExit("Factory skills snapshot is stale; run go generate")
else:
    target.write_text(snapshot)
