#!/usr/bin/env python3
"""Retain nextest timings and prove partition coverage from nextest's inventory."""
import argparse
from collections import Counter
from datetime import datetime
import json
from pathlib import Path
import subprocess
import sys
import xml.etree.ElementTree as ET


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def cases(path):
    report = ET.parse(path).getroot()
    return [(suite.attrib["name"], case.attrib["name"],
             float(case.get("time", "0")))
            for suite in report.iter("testsuite") for case in suite.findall("testcase")
            if case.find("skipped") is None]


def summary(directory):
    tests = cases(directory / "junit.xml")
    binaries = Counter()
    for binary, _, seconds in tests:
        binaries[binary] += seconds
    total = sum(binaries.values())
    slowest = sorted(tests, key=lambda test: test[2], reverse=True)[:20]
    write_json(directory / "summary.json", {
        "tests": len(tests), "test_seconds": total,
        "binaries": dict(binaries), "slowest": slowest,
    })
    print(f"### {directory.name}: {len(tests)} tests, {total:.3f} test-seconds\n")
    print("| Binary | Test-seconds |\n| --- | ---: |")
    for binary, seconds in binaries.most_common():
        print(f"| {binary} | {seconds:.3f} |")
    print("\n| Slowest tests | Seconds |\n| --- | ---: |")
    for binary, name, seconds in slowest:
        print(f"| {binary}::{name} | {seconds:.3f} |")


def inventory(directory, partition):
    directory.mkdir(parents=True, exist_ok=True)
    for name, arguments in (("full", []), ("partition", ["--partition", f"hash:{partition}"])):
        result = subprocess.run(
            ["cargo", "nextest", "list", "--profile", "ci", "--message-format", "json", *arguments],
            check=True, capture_output=True, text=True,
        )
        write_json(directory / f"{name}.json", json.loads(result.stdout))


def selected(path):
    report = json.loads(path.read_text())
    return {(binary, name) for binary, suite in report["rust-suites"].items()
            for name, case in suite["testcases"].items()
            if case["filter-match"]["status"] == "matches"}


def verify(directories, count):
    if len(directories) != count:
        raise ValueError(f"expected {count} partitions, found {len(directories)}")
    full = selected(directories[0] / "full.json")
    revision = (directories[0] / "revision.txt").read_text().strip()
    if not full:
        raise ValueError("empty unsharded inventory")
    assigned = Counter()
    work = []
    for directory in directories:
        if not revision or (directory / "revision.txt").read_text().strip() != revision:
            raise ValueError(f"source revisions differ: {directory}")
        if selected(directory / "full.json") != full:
            raise ValueError(f"unsharded inventories differ: {directory}")
        partition = selected(directory / "partition.json")
        assigned.update(partition)
        executed = [(binary, name) for binary, name, _ in cases(directory / "junit.xml")]
        if Counter(executed) != Counter(partition):
            raise ValueError(f"execution differs from selected inventory: {directory}")
        work.append(sum(seconds for _, _, seconds in cases(directory / "junit.xml")))
    if set(assigned) != full or any(number != 1 for number in assigned.values()):
        raise ValueError("partition union must assign every selected test exactly once")
    print(f"Coverage: {len(full)} selected tests assigned and executed exactly once across {count} partitions.")
    print("Partition test-seconds: " + ", ".join(f"{seconds:.3f}" for seconds in work))


def benchmark(directory):
    run = json.loads((directory / "run.json").read_text())
    jobs = json.loads((directory / "jobs.json").read_text())
    created = datetime.fromisoformat(run["created_at"])
    groups = {}
    for job in jobs:
        if not job["name"].startswith("benchmark ("):
            continue
        group = groups.setdefault(job["name"].split(" / ")[0], {"jobs": []})
        start = datetime.fromisoformat(job["started_at"])
        end = datetime.fromisoformat(job["completed_at"])
        steps = {step["name"]: (datetime.fromisoformat(step["completed_at"]) -
                                datetime.fromisoformat(step["started_at"])).total_seconds()
                 for step in job["steps"]
                 if step.get("started_at") and step.get("completed_at")}
        group["jobs"].append({
            "name": job["name"], "conclusion": job["conclusion"],
            "runner": job["runner_name"],
            "start_delay_seconds": (start - created).total_seconds(),
            "execution_seconds": (end - start).total_seconds(),
            "end_to_end_seconds": (end - created).total_seconds(), "steps": steps,
        })
    print("| Runner / partitions | Elapsed seconds incl. start delay | Runner minutes | Results |")
    print("| --- | ---: | ---: | --- |")
    for name, group in groups.items():
        group["runner_minutes"] = sum(job["execution_seconds"] for job in group["jobs"]) / 60
        group["critical_path_seconds"] = max(job["end_to_end_seconds"] for job in group["jobs"])
        results = ", ".join(sorted({job["conclusion"] for job in group["jobs"]}))
        print(f'| {name} | {group["critical_path_seconds"]:.1f} | {group["runner_minutes"]:.2f} | {results} |')
    write_json(directory / "benchmark.json", {"revision": run["head_sha"], "groups": groups})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    report = commands.add_parser("summary")
    report.add_argument("directory", type=Path)
    measured = commands.add_parser("benchmark")
    measured.add_argument("directory", type=Path)
    listing = commands.add_parser("inventory")
    listing.add_argument("directory", type=Path)
    listing.add_argument("--partition", required=True)
    check = commands.add_parser("verify")
    check.add_argument("directories", type=Path, nargs="+")
    check.add_argument("--count", required=True, type=int)
    args = parser.parse_args()
    try:
        if args.command == "summary":
            summary(args.directory)
        elif args.command == "benchmark":
            benchmark(args.directory)
        elif args.command == "inventory":
            inventory(args.directory, args.partition)
        else:
            verify(args.directories, args.count)
    except (ValueError, OSError, ET.ParseError, subprocess.CalledProcessError) as error:
        print(f"ci-timings: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
