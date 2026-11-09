# Who decides a Verdict is fixed in the Bar, and a Grader panel may stand in for people

An Experiment that grades pairs of Trials has two settings in its Bar, fixed before the first Trial: one for the Grader panel and one for the Day shift's people. Each is *off*, *beside* the Verdict, or *counted* in it, and the people's setting also fixes their seeded sample of pairs and the overlap between readers. With only the panel counted, the people's calls calibrate it. With only the people counted on a sample, the Verdict rests on the sampled pairs alone, and a small sample usually comes out inconclusive rather than wrong. With both counted, the panel's merge call stands in for the people's on the pairs they didn't read. The sample is drawn by pair, so both Arms in a pair always have the same reader. People–panel agreement on the sample is always reported, and when both are counted, agreement below the Bar's level makes the Verdict inconclusive. Further rules guard the stand-in, each a Bar setting on by default:
- a Grader from either Arm's family doesn't vote while another family is available;
- the voting panel must span a set number of families, two by default;
- the panel votes one vote per family.

Grilled in #1484, Q4.1, Q5.1, Q6.1, Q6.3 and Q9.1–Q9.2, with evidence in `docs/research/eval-grader-panel.md`.

## Considered Options

- **People always read every pair.** Rejected: at 20 Eval tasks and three Trials that's 60 pairs per Experiment, too many to read for every change.
- **The panel decides with no human check.** Rejected: a Grader can prefer its own family's work, and nothing would show whether the panel judges as a person would.
- **Stand-in only, with no other combinations.** Rejected: an Experiment testing whether the panel's judgment can be trusted needs the panel and the people kept apart.

## Consequences

- A Verdict may rest mostly on model calls. Anyone reading it needs to see which combination the Bar fixed, and how well the panel and the people agreed.
