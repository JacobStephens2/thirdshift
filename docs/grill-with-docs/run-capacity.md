# Grilling: more capacity for Runs and Experiments

Issues: #1979 (provision the shared host, `ready-for-human`, blocked by #1494, blocks #1887); #1494 (`ready-for-agent`, brief posted)
Related: #1887 (build the process-task suite, "once provisioned, the new Experiment host"), #1494 (scheduled Passes compete with Trials), #1486 (Run isolation; its Q1.1 on hosted providers is unanswered), [1484-implement-suite.md](done/1484-implement-suite.md) Q1.8
Research: [run-isolation.md](../research/run-isolation.md) §hosted providers, [sandcastle-fit.md](../research/sandcastle-fit.md), [eval-sandboxes.md](../research/eval-sandboxes.md)

## Round 1 (answered)

**Q1.1 - What the capacity is for.** You said "third shift run capacity" and "more of these experiments". In the glossary those are two things: a **Run** (Ticket run, Spec run, Pass…) delivers through the shared `.git` and pushes to GitHub, while a **Trial** belongs to an **Experiment**, runs in bwrap and stops where it would push (ADR-0027). #1484 Q1.8 already agreed "a new bigger host for Experiments", and #1887 is waiting on it. The options:
- (a) **Experiments only.** The new host runs Experiments and builds Eval tasks; the factory's Runs stay on the five existing machines.
- (b) **Runs only.** More factory throughput; Experiments stay where they are.
- (c) **Both, on one host.** It runs Experiments and also takes Pickup work.

*Recommended:* (a). It is what #1484 Q1.8 settled and what #1887 is blocked on. Nothing in the Fleet shows Runs short of room: five machines already take Pickup work. (c) re-creates #1494 on the new host, where factory builds turn Trials into timeouts that count as failed.

*Answer:* (c)

**Q1.2 - Hosted sandbox providers (Daytona, E2B, Vercel, Modal…).** run-isolation.md said no for Runs: they are "code in, commits out", which breaks the shared `.git` a Run delivers through, two of them drop stdin for Claude Code and Codex (Sandcastle #936), subscription logins on a third-party VM have unsettled terms, and a Security run's write-up would sit on a second vendor's disk (ADR-0015). A Trial *is* closer to "code in, commits out", since it stops before pushing, so the argument against is weaker there. But a Trial's cost is CPU for `cargo build`/`cargo test` three times per task, which a provider bills by the vCPU-second (~$0.05/vCPU-hour on Daytona) with cold toolchains each time, and each Trial would need your Claude/Codex login inside the vendor's VM. Docker Cloud's useful part, Build Cloud, speeds image builds, not sessions, so it isn't a run substrate. The options:
- (a) **Decline hosted providers for both Runs and Trials now.** Record it here and answer #1486 Q1.1 the same way.
- (b) Decline for Runs, but keep a hosted provider as a later Trial backend if one host stops being enough.
- (c) Prototype a Trial on E2B or Daytona before deciding.

*Recommended:* (b). It settles #1486 Q1.1, and keeps the door open only where the fit is real, with a concrete trigger ("one host isn't enough") rather than a vague "later".

*Answer:* agreed

**Q1.3 - Dedicated or shared.** Whatever Q1.1 picks, should the new host run any scheduled Passes (Pickup, Architect, nightly Security, Triage)? #1494 shows what sharing costs: load 21 on 4 CPUs, a Trial timing out at 1800 s.
- (a) **Dedicated.** No scheduled Passes in its crontab; it runs only what you or an Experiment start. It still joins the Fleet so the Lima Console shows it.
- (b) Shared, and fix #1494 first (Passes skip while an Experiment is live).

*Recommended:* (a). It makes #1494 moot for Experiment measurements without code, and keeps #1494 a question for the other hosts only. If the host sits idle between Experiments, that is a reason to resize it, not to fill it with Pickup.

*Answer:* (b)

**Q1.4 - Long-lived droplet or one per Experiment.** Eval tasks live privately on the host that runs Experiments (`~/.thirdshift/evals/`), prepared once and kept read-only (#1484 settled). The options:
- (a) **One long-lived host**, resized up for a big Experiment and down between them. DigitalOcean resizes CPU/RAM in place (disk only grows).
- (b) A droplet created per Experiment from a snapshot, destroyed after; tasks and results restored from backup each time.

*Recommended:* (a). The suite is stateful and private, the A/A batches in #1887 run for hours across days, and per-Experiment provisioning is a tool to build and maintain. Revisit only if the idle cost hurts.

*Answer:* agreed

**Q1.5 - Where this lands on the tracker.** #1887 assumes "the new Experiment host" but no issue provisions it. The options:
- (a) File one `ready-for-human` Ticket, "Provision the Experiment host", and make #1887 step 4 depend on it. Provisioning is your account, your money and your logins, so not an agent's.
- (b) Fold provisioning into #1887.
- (c) No issue; just do it.

*Recommended:* (a). #1887 is already four steps and a judgment call; provisioning is separable and blocks it. Its content follows from Round 2 (size, OS, region, which logins).

*Answer:* agreed

## Round 2 (answered)

**Q2.1 - Who gives way on a shared host: the factory or the Experiment? (#1494's first decision)** Q1.3 (b) shares the host and fixes #1494 first. #1494 asks the Day shift whether factory work or measurements come first. The options:
- (a) **The Experiment.** While an Experiment is live on the machine, each scheduled Pass skips and writes an Activity log line, the way `process` skips on the Pass lock (#1494 option 1). Nothing to remember to turn back on.
- (b) **A pause setting.** `[schedule] paused = true` in the User config, shown by `status` and the Activity log (#1494 option 2). You set it before an Experiment and clear it after.
- (c) Both.

*Recommended:* (a). It is automatic, so a forgotten setting can neither leave the factory paused for days nor let a Pass slip into a measurement. A pause setting can come later as its own Ticket if you want one for other reasons.

*Answer:* agree

**Q2.2 - Runs already live when an Experiment starts.** (a) in Q2.1 stops new Runs, but a Spec run Pickup started an hour ago keeps building and testing, which is exactly #1494's evidence (Pickup took Spec #1456 nine minutes into the A/A). The options:
- (a) **The Experiment waits** until no Run is live on the machine, saying so in its log and `status`, then starts. Passes skip meanwhile, so the wait ends.
- (b) The Experiment refuses to start and names the live Runs; you rerun it later.
- (c) It starts anyway and records how many Runs were live, so the Verdict reader can discount it.

*Recommended:* (a). It needs no babysitting and the measurement is clean. A Spec run can take hours, so the wait should be visible, and `--no-wait` can give (c) for someone who doesn't care.

*Answer:* agree

**Q2.3 - The nightly Security run during an Experiment.** #1494's second decision. The options:
- (a) It skips like the other Passes, and runs the next night.
- (b) It runs anyway; security comes before measurements.

*Recommended:* (a). A missed night costs one day of delay; a Security run's builds during a Trial cost a noisy Verdict. If an Experiment runs for several nights in a row, (a) means several missed nights, so the skip line in the Activity log should say why.

*Answer:* agree

**Q2.4 - Size.** It now runs Pickup work *and* Experiments (Q1.1 c), resized up for big Experiments (Q1.4). The current hosts are 4 vCPU / 8 GB, and #1494 overloaded one. DigitalOcean's dedicated-CPU droplets (approximate list prices, to confirm before provisioning): CPU-Optimized 8 vCPU / 16 GB ≈ $168/month, General Purpose 8 vCPU / 32 GB ≈ $252/month, CPU-Optimized 16 vCPU / 32 GB ≈ $336/month. The options:
- (a) CPU-Optimized 8 vCPU / 16 GB to start, resize up for a large Experiment.
- (b) General Purpose 8 vCPU / 32 GB: same CPU, room for several parallel Trials' memory caps and `cargo` links.
- (c) 16 vCPU / 32 GB from day one.

*Recommended:* (b). Rust links and parallel Trials each with a memory cap run out of RAM before CPU, and 16 GB matches only two 8 GB hosts' worth of work. Dedicated CPU (not Basic shared vCPU) matters because Trials are timed.

*Answer:* agree

**Q2.5 - OS and region.** The options:
- (a) Ubuntu 24.04 LTS, in the same region as the thirdshift.app droplet. Same OS as the Lima VM, where bwrap and systemd user scopes already work for Trials.
- (b) Rocky 9, like orchestrate and dev.

*Recommended:* (a). Trials and their systemd scopes are proven on Ubuntu 24.04 (the Lima VM); the Rocky hosts need a private python3 shim and have #1895-style scope surprises to rediscover.

*Answer:* agree

**Q2.6 - Logins on the host.** Trials, Graders and Pickup Runs all start sessions. The options:
- (a) The same Claude and Codex subscription logins, `gh` login and Resend key as the other factory hosts, under a normal user account with the Console behind its sign-in and added to the Lima VM's Fleet.
- (b) A separate set of accounts for this host, so Experiment spend is apart from factory spend.

*Recommended:* (a). #1932/#1938 already price Trial sessions and cap an Experiment's cost, so separate accounts aren't needed to see the spend. Note that a shared subscription window means a long Experiment and Pickup draw on the same limits; Q2.1 (a) keeps them from overlapping on this host, but not with the other hosts.

*Answer:* agree

**Q2.7 - Backups.** The Eval suite on the host is private and expensive to rebuild (hours of A/A sessions per batch, #1887). The options:
- (a) DigitalOcean's weekly droplet backups (about +20% of the droplet price), plus a copy of `~/.thirdshift/evals/` to the Mac after each approved batch.
- (b) Only the copy to the Mac.
- (c) Only DigitalOcean backups.

*Recommended:* (a). Droplet backups cover the whole host including logins and config; the copy to the Mac is what survives losing the DigitalOcean account and is what #1887 step 4's "backup first" means.

*Answer:* agree

## Round 3 (answered)

**Q3.1 - Which Commands give way.** Q2.1 says "each scheduled Pass skips", but in the glossary a **Pass** is any Command started with no Issue URL, which decides before any work whether it is skipped: Pickup, Architect, Security, Review with `--author`, Fix with no pull request, Triage with no Report, and a Custom process with no issue. The cron doesn't mark which are scheduled. And you can still start a Ticket run or Spec run by hand during an Experiment. The options:
- (a) **Every Pass skips while an Experiment is live on the machine, however it was started. A Command with an Issue URL runs, with a warning on stderr that it will compete with the Experiment's Trials.**
- (b) Only Passes skip; hand-started Commands run silently.
- (c) Everything waits, hand-started Commands included.

*Recommended:* (a). The Pass's whole definition is "decides before any work whether it is skipped", so the rule fits there and needs no scheduled/unscheduled distinction. A Command you start by hand is your deliberate choice, but you should be told. Recorded in the glossary's **Pass** entry as one sentence.

*Answer:* agree

**Q3.2 - Does a waiting Experiment count as live?** Q2.2 (a) makes an Experiment wait for live Runs. If the waiting Experiment didn't count as live, a Pickup Pass could start a new Run while it waits, and the wait might never end. The options:
- (a) Yes: from the moment it starts waiting, Passes skip on that machine.
- (b) No: it is live only once its first Trial starts.

*Recommended:* (a). Only (a) guarantees the wait ends.

*Answer:* agree

## Settled by earlier answers (shout if any is wrong)

- The new host does both factory Runs and Experiments (Q1.1). It joins the Fleet and takes Pickup work.
- Hosted sandbox providers are declined for Runs. One stays a possible later backend for Trials, reopened only if one host stops being enough (Q1.2). This answers #1486 Q1.1 too; that file still needs the answer written in.
- #1494 must land before Experiments run on the new host, since it shares Experiments with scheduled Passes (Q1.3). #1494 goes from `needs-triage` to `ready-for-agent` once Q2.1-Q2.3 decide its open questions. Its fix applies on every machine, the Lima VM included, since the Lima VM also builds tasks while its cron runs Passes.
- One long-lived DigitalOcean droplet, resized for big Experiments (Q1.4).
- A `ready-for-human` Ticket "Provision the Experiment host" blocks #1887 step 4; its contents come from Q2.4-Q2.7 (Q1.5). Since the host is shared, "Experiment host" is a nickname for the issue title, not a glossary term.
- #1494's brief: while an Experiment is live on a machine, Passes skip with an Activity log line naming the Experiment, the nightly Security run included (Q2.1, Q2.3). An Experiment waits for live Runs to end, shown in its log and `status`, and `--no-wait` starts it at once and records how many Runs were live (Q2.2). No pause setting.
- Provisioning Ticket: DigitalOcean General Purpose 8 vCPU / 32 GB dedicated-CPU droplet (price confirmed at provisioning), Ubuntu 24.04 LTS, same region as thirdshift.app, the same Claude, Codex, `gh` and Resend logins as the other factory hosts under a normal user account, Console behind sign-in and added to the Lima VM's Fleet, DigitalOcean weekly backups on, and `~/.thirdshift/evals/` copied to the Mac after each approved batch (Q2.4-Q2.7).
- Every Pass skips while an Experiment is live on its machine, however it was started; a Command with an Issue URL runs, warning on stderr that it competes with the Experiment's Trials (Q3.1). An Experiment is live from the moment it starts waiting (Q3.2). Both go in #1494's brief.
- From #1484 Q1.8: Experiments need Linux with bwrap, so not the Mac; the Lima VM (4 CPU / 8 GiB) builds and screens the first tasks until the new host exists.

## Research results (2026-11-10)

- The Fleet the Lima Console shows is thirdshift (console.thirdshift.app), stephens, test, orchestrate and dev, confirmed from `[[console.fleet]]` in the Lima VM's `~/.thirdshift/config.toml`. The Lima VM itself is 4 CPU / 7 GiB.
- No open issue provisions a new host; #1887 step 1 and step 4 assume one, confirmed by `gh issue view 1887`.
- #1486 Q1.1 (hosted providers for Runs) is still unanswered in [1486-run-isolation.md](1486-run-isolation.md).

## Waiting on research

- DigitalOcean list prices in Q2.4 are from memory; the provisioning Ticket confirms them on the pricing page.
