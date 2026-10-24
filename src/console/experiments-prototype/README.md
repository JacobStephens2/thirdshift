# Experiment pages prototype (#1019): throwaway

The question: **what should the Console's Experiments list and an Experiment's page look like, at phone and laptop widths?** The answer goes on #1020. This folder never merges to main.

The pages don't exist yet, and the Console renders on the server, so this is a static copy of its chrome (header, nav, Peek, footer) with the real `console.css` and Themes, as #995's span-tree prototype was. The data is invented, in the shape `thirdshift experiments --json` prints on #977's branch (`src/evals/experiment/store.rs`), and each Verdict is computed by `src/verdict.rs`'s method: a paired Dirichlet with the standing Bar, at 2,500 draws so the page loads quickly.

| Key | Experiment page (`?variant=`) |
| --- | --- |
| A | Compare table: both Arms side by side, then each suite's intervals, then Eval tasks by Arm with a Trial selector, after Phoenix's compare view |
| B | Report: one column of sentences and facts, like the Console's Command details; nothing hidden |
| C | Trial grid: every Trial a cell, Eval tasks in rows and Arms in columns; a cell opens its details in the Peek |
| D | Differences: each Eval task's difference on the interval's axis, as a forest plot, with the suite's interval as its last row |

| Key | Experiments list (`?list=`) |
| --- | --- |
| 1 | Log list: the Cron jobs page's pattern, one fact list per Experiment |
| 2 | Table: one row per Experiment, with each suite's accuracy interval drawn |
| 3 | Inbox groups: Running, Needs you and Done, with the Inbox's rows |

`&x=` picks the Experiment: `running`, `paused` (at a plan-limit warning), `waiting` (on 26 rechecks, provisionally a pass), `release` (inconclusive: code review can't settle), `inconclusive` (by its interval, with a memory-cap finding), `fail`, `dropped` (four Eval tasks void) and `pass`.

## Run it

```sh
python3 -m http.server 8019 -b 127.0.0.1 -d src/console
# open http://127.0.0.1:8019/experiments-prototype/?variant=A&list=1&x=waiting
```

It is also published at https://staging.thirdshift.app/. The bottom bar is the prototype's, not the design's: ← → step the Experiment page layouts, Shift+← → the list layouts; the menus pick an Experiment and a Theme; "Live" replays the running Experiment's remaining Trials, its model Graders and its Verdict; "390 px" shows the page in a phone frame. Links to Session traces and Command logs are stubs.

## What it found

These came out of building the data, before any design was chosen:

- **The code-review suite can't pass the standing Bar** (#1252). The fixture's nine code-review Eval tasks give an interval of about −14 to +14 points with no difference at all, wider than the −10-point margin; the host's ten approved tasks give about −13 to +13. So every code-review suite ends inconclusive. It needs at least fourteen Eval tasks, and about twenty to leave room for a real difference.
- **A pass waits on a recheck of nearly every Candidate Trial** (#1253). With thirty Spec-review Eval tasks and no wrong findings, one wrong finding moves the interval to about −13 to +4. So each Candidate Trial is decisive under `rechecks.rs`'s flip rule: 26 Trials wait in `waiting`, and all 30 were rechecked in `pass`.
- **Rechecks stay in the CLI** (ADR 0016 lists no recheck action), and each command carries two 64-character IDs.
- **`thirdshift experiments --json` lacks what the pages need to name and order Experiments:** the refs given (it keeps only commits), a title, the end time, running Trials (it lists finished ones), the command line that resumes, and which flip makes a Score decisive. It also sorts by ID rather than by time.
- **Until the model Graders run, a Trial shows only its code checks,** so live, an Experiment has no differences or interval to draw.
