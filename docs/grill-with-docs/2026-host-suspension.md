# Grilling: #2026, Stage times and health count time the host was suspended

Issue: https://github.com/JacobStephens2/thirdshift/issues/2026
Research: see "Research results" below

## Round 1 (answered)

**Q1.1 - The term.** We need one word for the time a Command's host was not running at all, such as a laptop asleep or a VM paused while its own host sleeps. It is measured by the running Command as wall-clock elapsed minus monotonic elapsed. The term has to stay distinct from **Silent**, where the host runs but the logs stop growing, and from a Pass simply not firing. The options:
- (a) **Suspended time**, with the host "suspended". _Avoid_: asleep, paused, downtime.
- (b) **Host sleep**.
- (c) **Downtime**.

*Recommended:* (a). "Suspend" is what both Linux and macOS call it, it covers a paused VM as well as a sleeping laptop, and "sleep" is easy to confuse with `sleep` in scripts and with Silent.

*Answer:* agree

**Q1.2 - What a stage's time means.** Today a stage's time is the wall-clock gap between its `began_at` and the next stage's (`StageTime::seconds`). #1975's implement stage reads 10 h, of which 9 h 40 m was Suspended time. The options:
- (a) **Subtract.** A stage's time is wall time minus Suspended time. This is wrong for stages that wait on something outside the host, such as CI watch: CI kept running on GitHub while the laptop slept.
- (b) **Annotate only.** Keep wall time and show the Suspended time next to it, e.g. "10 h 6 m, 9 h 40 m suspended". Usual ranges still take the inflated sample.
- (c) **Annotate, and leave suspended stages out of Usual ranges.** Keep wall time and show the Suspended time. A stage with Suspended time is not a sample for its Usual range, because the time no longer says how long that stage takes.

*Recommended:* (c). It stays honest for stages that wait on something outside the host, and it keeps Usual ranges clean. A suspended stage is rare enough on always-on hosts that dropping its sample costs nothing.

*Answer:* agree

**Q1.3 - Where Suspended time is recorded.** Only the running Command can measure it: the Console reads files and cannot tell a sleeping host from a stalled one. The options:
- (a) **Both.** A per-stage Suspended time in the **Command record**, updated when a suspension is detected, plus one **Command log** line per suspension (e.g. "host was suspended 1 h 38 m"), so `show` and a reader of the log both see it.
- (b) **Command log line only.** Readers would have to parse the log to get stage times right.
- (c) **Command record only.** The log, which a person reads first, would say nothing.

*Recommended:* (a). The record carries the number that stage times and Usual ranges need. The log line is the explanation a person looks for, which this time took power-log forensics to find.

*Answer:* agree

**Q1.4 - Which Commands measure it.** Ticket runs are where it showed up, but any Command can span a suspension: a Pass, a Security run, an Experiment, a Process eval. The options:
- (a) **Every Command**, since the measurement belongs to the running process, not to one kind of run.
- (b) **Ticket runs only.**

*Recommended:* (a). The measurement is the same everywhere, and Experiments and Process evals compare times, so a suspension skews them just as badly.

*Answer:* agree

## Round 2 (answered)

**Q2.1 - Slow and Suspended time.** Slow compares a live stage's age with its Usual range. Right after #1975's laptop woke, the implement stage's wall-clock age was about 10 h, about 20× its range, though it had been working for 29 minutes. This builds on Q1.2 and Q1.3: the Command record now holds the stage's Suspended time. The options:
- (a) **Compare the age without Suspended time.** A stage is Slow when its age minus its Suspended time passes the top of its Usual range. For CI watch this makes Slow fire late, since CI kept running while the host slept.
- (b) **Never Slow after a suspension.** A stage with any Suspended time is never Slow.
- (c) **Leave it.** Compare wall-clock age as today.

*Recommended:* (a). It asks the question Slow is for: has this stage been working longer than usual? Firing late on a CI watch the host slept through is harmless.

*Answer:* agree

**Q2.2 - Silent and Suspended time.** Silent counts how long the logs have not grown. When a host wakes, the quiet time includes the whole suspension until the next log line, so a run can read "silent 90 min · past 30" for a moment. The options:
- (a) **Leave Suspended time out of the quiet time.** The logs had no chance to grow while nothing ran.
- (b) **Leave it.** The next log line comes within seconds of waking anyway.

*Recommended:* (a). It is the same rule as Q2.1. It also holds when the Console is read from another host while this one is still asleep: a frozen run does not read as stalled, as long as the Suspended time reaches the record before the Console is read (see Q2.3).

*Answer:* agree

**Q2.3 - How big a jump counts.** NTP corrections step or slew the wall clock by milliseconds to seconds, which also shows up as wall − monotonic drift. The options:
- (a) **One minute.** A gap under a minute is not recorded.
- (b) **Five minutes.**
- (c) **Configurable in the User config.**

*Recommended:* (a). It is far above clock corrections and far below anything that moves a stage time or a health state. Nobody needs to tune it.

*Answer:* agree

**Q2.4 - How it shows.** The options:
- (a) **Next to the stage time** in `thirdshift show`, the Console's run page and the Stages page, e.g. "10 h 6 m · 9 h 40 m suspended", and nowhere else. A suspension is not itself a Needs you row.
- (b) Also a Needs you row, or an Inbox note, saying the host was suspended.

*Recommended:* (a). A suspension needs no action from the Day shift. It only explains a time.

*Answer:* agree

**Q2.5 - Experiments and Process evals.** Q1.4 measures every Command, and Experiments compare Trial times. The options:
- (a) **Mark the Trial and leave its time out of time comparisons**, like a Usual range sample (Q1.2). Its other results, such as pass or fail and cost, still count.
- (b) **Mark it only.**
- (c) **Rerun the Trial.**

*Recommended:* (a). It follows Q1.2's rule. A rerun would cost a whole Trial to fix one number.

*Answer:* agree

## Settled by earlier answers (shout if any is wrong)

- The term is **Suspended time**, added to `GLOSSARY.md`, and **Usual range** now says suspended stages are left out. (Q1.1, Q1.2)
- A stage's time stays wall-clock time; nothing subtracts Suspended time from it. (Q1.2)
- Every place that shows a stage time can show its Suspended time beside it. (Q1.2)
- A stage with Suspended time is not a sample for its repository's Usual range. (Q1.2)
- The Command record gets a per-stage Suspended time, and the Command log gets one line per suspension. (Q1.3)
- Only the running Command measures it; the Console never infers it. (Q1.3)
- A record written before this change reads as having no Suspended time. (Q1.3)
- Every Command measures it: Passes, Ticket runs, Spec runs, Security runs, Experiments and Process evals. (Q1.4)
- The Command record must take Suspended time during a live stage, not only at stage transitions, so that Q2.1 and Q2.2 can use it. (Q1.3)
- Slow compares a stage's age minus its Suspended time with the top of its Usual range, so a CI watch the host slept through turns Slow late. (Q2.1)
- Silent's quiet time leaves out Suspended time. (Q2.2)
- A Command records a suspension once wall-clock time has run at least one minute ahead of monotonic time. A shorter gap, such as an NTP correction, is not recorded, and the threshold is not configurable. (Q2.3)
- Suspended time shows next to the stage time in `thirdshift show`, the Console's run page and the Stages page, e.g. "10 h 6 m · 9 h 40 m suspended". A suspension never makes a Needs you row or an Inbox note by itself. (Q2.4)
- A Trial with Suspended time is marked, and its time is left out of an Experiment's or Process eval's time comparisons. Its outcome and cost still count. (Q2.5)
- Nothing is left open: the design tree is complete.

## Research results (2026-11-11)

- In the Lima `vz` guest, the monotonic clock stops while the VM is paused and `CLOCK_BOOTTIME` doesn't count the pause either (boot − monotonic = 0). On each wake the wall clock is stepped forward; the journal logs `systemd-resolved: Clock change detected`. Confirmed on lima-thirdshift.
- On macOS, Rust's `Instant` reads `CLOCK_UPTIME_RAW`, which stops during sleep, so the same wall − monotonic measurement works on a native Mac Command. From the Rust std docs; not yet tested on the Mac.
- Stage times are wall-clock differences of each Stage's `began_at` (`src/logs/command_record.rs`, `StageTime::seconds`). Health uses the stage's wall age and the logs' quiet age (`src/logs/reading/health.rs`).
- Usual ranges are the 25th–75th percentiles of past stage times (`src/logs/usual_range.rs`), so a single suspended sample barely moves them.
- In #1975, the "slow" row on CI watch was genuine: it was GitHub's CI time, and the Mac was awake. The suspension inflated only the implement stage.

## Waiting on research

- None.
