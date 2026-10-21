# Grilling: #698, evals

Issue: https://github.com/JacobStephens2/thirdshift/issues/698
Research: `docs/research/eval-tools.md`, `docs/research/eval-tools-landscape.md` (#741), `docs/research/eval-viewers.md`, `docs/research/eval-runners.md`, `docs/research/otlp-file-writer.md`, `docs/research/eval-sandboxes.md`

The grilling follows `../../TEMPLATE.md`, with one file per round and three that every round updates:
- each round's questions, answers and `Checked:` notes go in its own `round-<n>.md`;
- what follows from the answers goes in `settled.md`;
- findings go in `research.md`, and pending work in `waiting.md`.

Questions are numbered `<round>.<question>`, so Q6.2 is Round 6's second question. A new round starts again at 1: Round 8's first question is Q8.1.

## Rounds

- [Round 1](round-1.md) (answered): what the evals are for, and where eval data may live. Q1.1–Q1.2.
- [Round 2](round-2.md) (answered): Session logs and Traces, the first Eval suite, the runner, and names. Q2.1–Q2.7.
- [Round 3](round-3.md) (answered): Trials, the bar, grading, keeping Trials honest, promptfoo, and Traces. Q3.1–Q3.20.
- [Round 4](round-4.md) (answered): the Console, the Baseline arm, measuring "worse", Run metrics, production grading, Bug origins and the Specs. Q4.1–Q4.14.
- [Round 5](round-5.md) (answered): the Scoring run, names, verdicts, calibration, the viewer, the runner, the interval, the OTLP writer and Codex's diffs. Q5.1–Q5.10.
- [Round 6](round-6.md) (answered): releases, Self-merge settings, the sandbox, void Trials, memory, the implement suite, the Console and the data folder. Q6.1–Q6.11.
- [Round 7](round-7.md) (answered): how a release waits for its Experiment, changes without a release, names, and the Specs. Q7.1–Q7.4.

## Status

Done on 2026-10-21. Every question is answered, and the decisions became four Specs, each broken into Tickets with `to-tickets`:
- #700, **Session traces:** OTLP JSON Lines in place of Session logs;
- #977, **Experiments,** which measure a change before it reaches production;
- #978, **the Scoring run,** which scores a weekly sample of production work;
- #981, **Run metrics,** the signals each Run leaves, kept apart.

Two decisions went to issues of their own, outside the Specs: #966, Self-merge settings for each kind of command (Q6.2), and #937, Claude Code's own attribution on factory pull requests.

## Every round

- [Settled by earlier answers](settled.md)
- [Research results](research.md)
- [Waiting on research](waiting.md)
