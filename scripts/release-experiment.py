"""Render a release Experiment snapshot; stdout is the script's state and ID."""
import json
import re
import sys


def main():
    view = json.load(sys.stdin)
    if view.get("release_skip") in {"no_suites", "baseline_without_trial"}:
        print(f"release: {view['reason']}", file=sys.stderr)
        print("skip")
        return
    experiment = view["id"]
    if not re.fullmatch(r"[0-9a-f]{64}", experiment) or view["state"] != "completed":
        raise ValueError("release Experiment must be completed with a valid ID")
    outcome = view["verdict"]["verdict"]
    if outcome not in {"pass", "fail", "inconclusive", "waiting_on_rechecks"}:
        raise ValueError(f"unexpected release Experiment Verdict: {outcome}")
    print(f"release: Experiment {experiment}: {outcome}", file=sys.stderr)
    for name, suite in view["verdict"]["suites"].items():
        for key, label in [("accuracy", "accuracy"), ("wrong_findings", "wrong findings")]:
            assessment = suite[key]
            if "interval" in assessment:
                intervals = [(label, assessment["interval"])]
            else:
                intervals = [
                    (f"{label} {kind} ({suite['bar'][kind + '_level']:.0%})",
                     assessment[kind + "_interval"])
                    for kind in ("pass", "fail")
                ]
            for interval_label, interval in intervals:
                print(f"release: {name}: {interval_label} [{interval['lower']}, {interval['upper']}]", file=sys.stderr)
    if outcome == "waiting_on_rechecks":
        pending = view["pending_rechecks"]
        if not pending:
            raise ValueError("waiting Experiment has no pending Scores")
        print("release: waiting for your rechecks:", file=sys.stderr)
        for score in pending:
            print(
                f"release: {score['id']}: {score['suite']}/{score['task']} "
                f"{score['arm']} Trial {score['trial']} "
                f"(borderline={score['borderline']}, decisive={score['decisive']})\n"
                f"release: thirdshift recheck {experiment} {score['id']} pass|fail --reason '…'",
                file=sys.stderr,
            )
    elif view["pending_rechecks"]:
        raise ValueError("release Verdict still has pending Scores")
    print(outcome, experiment)


if __name__ == "__main__":
    main()
