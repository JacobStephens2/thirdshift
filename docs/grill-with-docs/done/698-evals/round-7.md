# Round 7 (answered)

Part of the [#698 grilling](README.md).

*Q7.1 and Q7.2 follow from Q6.1's release gate. Q7.3 names Round 6's ideas, and Q7.4 revisits Q4.13's Specs, since the Scoring run needs neither Session traces nor Experiments.*

**Q7.1 - How a release waits for its Experiment.** Under Q6.1, a release that touches an Eval suite waits for an Experiment on the commit being released. Releases are cut on the `thirdshift` host, where Experiments run, and `scripts/release.sh` already waits there for CI with no overall timeout. An Experiment takes far longer: dozens of sessions two at a time, paused at any plan-limit warning (Q3.7). The options:
- (a) **The script starts the Experiment and waits for it.**
  - Before it opens the release pull request, it works out which Eval suites the changes since the last release touch (Q3.3). If any are touched, it starts `thirdshift experiment` on the commit it will release, with the last release as the Baseline arm, and waits for the verdict, printing progress.
  - A pass carries on. A fail or an inconclusive verdict shows the interval, and asks whether to release anyway, as `--review` asks about the summary.
  - If it's interrupted, running it again picks up the same Experiment, as it picks up a release pull request now.
- (b) **You start the Experiment yourself,** and the script refuses to release a commit without a passing verdict unless you confirm.
- (c) **As (a), but the Experiment posts its verdict on the commit as a GitHub status,** and the script waits for that like a CI check. That puts the verdict on GitHub, though nothing else.

*Recommended:* (a). A release is already one command that waits for what it needs, and the release fixes which commit the Experiment tests. Nothing new leaves the host.

*Answer:* agree

**Q7.2 - Changes that reach production without a release.** Q6.1 gates releases, but three things change production without one:
- **Harness updates.** Claude Code updated itself five times on this host between 16 and 20 October, from 2.1.302 to 2.1.306, and the other Harnesses update on their own schedules. thirdshift doesn't record a Harness's version itself.
- **User config changes.** A new Harness, Model or Effort default takes effect on the next Run.
- **Providers** can change a model behind the same name.

Whichever option you choose, an Experiment runs all its Trials on the Harness versions it started with, so an update partway through can't reach one Arm and not the other. The options:
- (a) **Record versions, watch for drift, and run an Experiment before a deliberate change.**
  - Each Run records its Harness's version, in its Session traces and in its pull request's Built-with line, so the Scoring run can split Scores by version for both hosts.
  - Before you change a Harness, Model or Effort default in the User config, you run an Experiment on one build with different flags (Q4.3). Nothing enforces it.
  - Harnesses keep updating themselves.
- (b) **As (a), and pin each Harness's version.** Auto-update is off, and each upgrade gets an Experiment first, with the current version as the Baseline arm and the new one as the Candidate. At Claude Code's pace that's about one Experiment a day, and security fixes in a Harness wait for a verdict.
- (c) **As (a), and gate User config changes.** thirdshift refuses a changed Harness, Model or Effort default until an Experiment on it has passed.

*Recommended:* (a). The changes you make on purpose get an Experiment, and the rest are watched, since a provider's change can't be pinned anyway. Pinning would turn every Harness release into an Experiment. If the Scoring run shows a drop after an update, an Experiment between the two versions settles it.

*Answer:* agree

**Q7.3 - Names for Round 6's ideas.** Three words from Round 6 aren't in `GLOSSARY.md`, and Q6.8 changes two definitions:
- A **Void Trial** (Q6.5) failed for a reason outside its Arm, under rules fixed with the bar. It's re-run, and kept out of the verdict. The eval tools share no word for it: Inspect records such a sample as an error, and Phoenix's Harbor plugin scores it `infra_ok`.
- A **Verdict** is what an Experiment ends in under its bar: pass, fail or inconclusive.
- The **Bar** is the rules an Experiment's verdict follows, fixed before it starts: the margin, the credible level, the prior, the seed and draw count, the Trials per Eval task, and the void rules.
- **Trial** says a Trial "belongs to no **Command**". Under Q6.8 it belongs to its Experiment, which is a Command, but it writes no Command record of its own.
- **Command** gains the Experiment, as it gained the Scoring run.

The options:
- (a) **Add Void Trial, Verdict and Bar,** and change Trial and Command as above.
  - Void Trial: _Avoid_ errored Trial, infrastructure failure.
  - Verdict: _Avoid_ result, and outcome, which is a Run's.
  - Bar: _Avoid_ threshold, gate, acceptance criteria.
- (b) **As (a), without Bar,** which stays plain English in the Specs.
- (c) **Only the changes to Trial and Command.** Void, verdict and bar stay plain English.

*Recommended:* (a). All three will appear in the Specs, the CLI and the Console, and agents need one meaning for each. Once you agree, they go into #943 before it merges, with ADR 0019's amendment for Q5.9 and ADR 0016's for Q6.9.

*Answer:* agree

**Q7.4 - The Specs, now that the Scoring run stands alone.** Q4.13 settled three Specs, and the Settled list puts Run metrics last because they "need Session-trace signals and Graders calibrated in Experiments". Since then, two things have changed:
- The Scoring run (Q5.1) scores from GitHub, and calibrates against your own Scores (Q5.4). So it needs neither Session traces nor Experiments, only the sandbox its Graders run in.
- Run metrics (Q4.5) need Session traces for half their signals, and nothing from Experiments. The other half comes from the Command record, which exists today.

The options:
- (a) **Four Specs: Session traces, Experiments, the Scoring run, and Run metrics.**
  - The Scoring run's Grader Ticket is blocked only by the Experiments Spec's sandbox Ticket. Its other Tickets can start at once.
  - Only Run metrics' Session-trace Tickets wait for the Session-trace Spec.
  - All four are published together, as #921 allows (Q4.13's note).
- (b) **Three Specs, with the Scoring run inside Run metrics,** as the Settled list has it.
- (c) **Three Specs, with the Scoring run inside Experiments,** since both use Graders and the sandbox.

*Recommended:* (a). Production scoring answers the second half of Q1.1, and under (a) it starts as soon as the sandbox exists, rather than after both other Specs. Each Spec also stays about one thing.

*Answer:* agree

