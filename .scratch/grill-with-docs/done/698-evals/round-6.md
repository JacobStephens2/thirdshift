# Round 6 (answered)

Part of the [#698 grilling](README.md).

*Q6.1 and Q6.2 bring back Q5.3 with two facts it missed. Q6.3–Q6.7 turn the sandbox research into decisions, Q6.8–Q6.10 place Experiments beside the rest of thirdshift, and Q6.11 closes a gap in Q4.4's rules.*

**Q6.1 - Where an Experiment stops a change: at the merge or at the release.** Q1.1 wants to know whether a change makes the agents worse before it reaches production, and production is the release, not `main` (Q5.3's note). You answered Q5.3 with (a): a Run whose change touches an Eval suite leaves its pull request for review, unless the User config lets it Self-merge. The options:
- (a) **At the release.** Runs Self-merge as now. A release whose changes since the last one touch an Eval suite (Q3.3) waits for an Experiment on the commit being released, with the last release as the Baseline arm, both built as Q4.3 builds Arms.
  - A pass releases. A fail or an inconclusive verdict waits for you, as `scripts/release.sh --review` does now.
  - One Experiment covers every change in the release, so a fail doesn't say which change caused it. You'd revert, or run an Experiment on each change.
- (b) **At the merge,** as you answered Q5.3, with a User config key for whether such Runs may Self-merge. Each pull request gets an Experiment of its own, and a release needs none.
- (c) **Both:** (a) always, and (b)'s key for anyone who wants each change judged alone.

*Recommended:* (a). It measures exactly what reaches production, against exactly what production runs, and the factory keeps Self-merging meanwhile. It also needs fewer Experiments: the 16 changes since 1 October went out in 10 releases, and only the ones touching a suite would need one. Q6.2's keys can still hold any kind of Run for review.

*Answer:* agree

*Checked (2026-10-21):* not in CI. The Experiment runs on the `thirdshift` host, like every Experiment (Settled, "Running"), and `scripts/release.sh` waits for its verdict, as it waits for CI now. Q7.1 settles how the script gets the verdict. GitHub's CI can't run an Experiment:
- the Eval tasks are private files on that host (Q2.5);
- Trials run on your subscriptions, through the Harness logins on that host, and putting those logins and the sessions on GitHub's runners would break Q2.2's rule that eval data stays on machines you run;
- an Experiment is dozens of sessions, two at a time, that pause at a plan-limit warning and resume later (Q3.7).

A self-hosted runner on the host would get around the first two, but GitHub advises against self-hosted runners on public repositories like thirdshift, since a pull request from anyone could run code on the machine.

**Q6.2 - Self-merge settings for each kind of command.** Your Q5.3 answer also asks for User config settings that decide, for each kind of Run, whether it Self-merges. Today `merge.always` decides for every Run, and `merge` or `no-merge` on a command overrides it, a Pass's for everything that Pass starts. On the `thirdshift` host, the Architect run's crontab line stops at its plan (`--plan-only`), and the Pickup and Security lines Self-merge, because `merge.always` and `security.fix` are on. A Ticket's Run always merges into its Spec branch, so for a Spec run the setting is about its Spec PR. The options:
- (a) **A key under `[merge]` for each kind,** each defaulting to `merge.always`: `architect`, `pickup` and `security` for what those Passes start, and `spec` for every Spec PR.
  - A Run or Spec PR Self-merges only when every key that applies allows it, so `spec = false` holds a Spec PR whichever Pass started it.
  - A command's own words still win.
  - It's an issue of its own, outside #698's Specs, since it needs nothing from evals.
- (b) **As (a), inside the Experiments Spec.**
- (c) **No new keys.** Each crontab line keeps choosing with its words, and the guide says so.

*Recommended:* (a). It puts each machine's merge policy in one file that Setup can ask about. `spec` adds a choice that crontab words can't make, since a Pickup run takes Specs and single issues alike. Filed separately, it can ship before the eval work.

*Answer:* agree

**Q6.3 - How a Trial reaches only its provider.** The Settled list gives each Trial a network of its own. It still has to reach its provider and nothing else: seven of the eight repositories are public on GitHub, later fixes included. Whichever option you choose, the Trial also gets its own PID namespace (`--unshare-pid`). It then can't read the command lines of the host's processes, which carry Session prompts, and everything it leaves running ends with it. The options:
- (a) **An allowlisting proxy over a Unix socket, written in thirdshift.**
  - bwrap's `--unshare-net` leaves the Trial only its own loopback.
  - A proxy outside listens on a socket bound into the sandbox, and a relay inside turns a local port into that socket. The Harness reaches its provider through `HTTPS_PROXY`.
  - The proxy allows only the hosts on that Harness's list, and refuses names that resolve to loopback or to the host's own addresses.
  - It is the design of Anthropic's sandbox-runtime and of Codex's managed proxy, in a few hundred lines of blocking Rust.
  - A Harness that ignores `HTTPS_PROXY` finds no route and fails, so its Trial is void (Q6.5), never leaky. Claude Code's and Grok's docs say they honour it, and Codex's imply it. OpenCode, Muse and agy wait on a live check.
  - Web search that a Harness runs at its provider passes the proxy, so it stays turned off by flag, as #429 turned off Claude's.
- (b) **pasta joined to bwrap's namespaces,** with its loopback mappings turned off. Every Harness works unchanged, but GitHub and the host's public addresses stay reachable.
- (c) **Anthropic's sandbox-runtime itself.** It brings Node, and lets a sandbox read everywhere by default, the opposite of Q3.6's bind list.

*Recommended:* (a). It's the only option that closes all three leaks: the host's loopback, its public addresses and GitHub. It also enforces Q2.2's rule that nothing leaves the host but the sessions, and blocks `curl`, `git clone` and WebFetch inside a Trial without a flag for each.

*Answer:* agree

**Q6.4 - Logins and homes inside the sandbox.** Q3.6 binds only the Harness's login files into a Trial, and keeps Harness memory off. The sandbox research found two traps. Codex replaces its refresh token on every refresh, so a copy of its login that refreshes leaves the factory's file holding a used token. And each Harness's home folder holds session histories, including the Graders' and those that wrote the later fixes. The options:
- (a) **A fresh home per Trial, with the one real login file bound in place.**
  - Only the login and the user-level instructions, rules, settings and hooks a Run's session reads are bound in, the last four read-only. That turns memory off by construction, and tells a Trial what a Run is told.
  - Codex's `auth.json` is bound read-write, so a refresh reaches the file production uses.
  - Claude needs no file. `claude setup-token` makes a one-year subscription token that "can only make model requests". It's kept in thirdshift's credentials file, as the Resend key is (ADR 0007), and passed as `CLAUDE_CODE_OAUTH_TOKEN`.
  - The other Harnesses bind their one file, once a live check shows each logs in and refreshes that way.
- (b) **As (a), but Claude binds its credentials file too,** if a live check shows Claude writes it in place. No new token.
- (c) **A copy of each login per Trial.** It's simplest, but it breaks Codex's next refresh in production.

*Recommended:* (a). A Trial then holds nothing but its login and what a Run reads, and the factory's logins keep working. The setup token is the only new credential, and it can reach nothing but the models.

*Answer:* agree

**Q6.5 - Void Trials.** A Trial can fail for reasons outside its Arm, and counting those as fails would blame the Arm for the host. The sandbox research proposes **void** Trials, decided by rules fixed with the bar before the Experiment starts (Q3.4). Q3.5 already voids a Trial that changed its repository. The options:
- (a) **Void only for named causes outside the agent.**
  - Void: the sandbox didn't start, the Harness couldn't reach its provider, the login failed or expired, the provider returned a server error before any work, Q3.7's plan-limit stop, or the Trial was killed by the host, or by a memory cap that Runs don't have (Q6.6).
  - Not void: a timeout, a context-window overflow, a refusal, or anything the agent did inside its sandbox. Those count as fails, as they would in a Run.
  - A void Trial is re-run as a fresh Trial, up to twice. It is shown with its cause and counted per Arm, and an Arm that keeps hitting its cap is reported as a finding.
  - An Eval task with a Trial still void after its re-runs drops out of Q4.4's interval for both Arms, so the pairing holds. If more than a tenth of the Eval tasks drop out, the verdict is inconclusive.
- (b) **No void Trials.** Every failure counts against its Arm, as Harbor's metrics count it.
- (c) **Every error is void,** timeouts included, as Phoenix's `infra_ok` marks it.

*Recommended:* (a). It keeps the host's failures out of the verdict without hiding the agent's. A timeout stays a fail, because a Run that never finishes fails too.

*Answer:* agree

**Q6.6 - Capping a Trial's memory.** Q3.7 runs at most two Trials beside the factory on the 8 GB host, but nothing stops a Trial from taking the factory's memory. The factory runs from cron, inside `cron.service`, which jacob can't subdivide, and with lingering off there's no user service manager to ask for a scope. The options:
- (a) **Run `loginctl enable-linger jacob` once.**
  - Each Trial then runs in a `systemd-run --user --scope` of its own, with `MemoryHigh`, `MemoryMax` and `MemorySwapMax`, so a Trial that outgrows its cap dies alone, and is void (Q6.5).
  - Each Trial also raises its own `oom_score_adj`, so the kernel's OOM killer picks Trials before the factory.
  - The cap is set once a Trial's real footprint has been measured.
- (b) **No host change.** Trials raise their own `oom_score_adj`, and thirdshift holds the next Trial back while available memory is low.
- (c) **Resize the `thirdshift` host,** as you offered in Q5.5, and keep (b).

*Recommended:* (a). One command makes memory a limit rather than a hope, and the user service manager it leaves running from boot would also serve rootless containers if Q6.7 ever needs them. (c) can follow if measured Trials need more room than the factory leaves.

*Answer:* agree

**Q6.7 - Where the implement suite runs.** Q3.20 makes the implement session the third suite, graded against the merged fix and its tests. Its Trials have to build each repository at an old commit. The benchmarks build an image per task, but SWE-bench asks for 120 GB of disk against 32 GB free here, and each image would need all six Harnesses at production's versions, rebuilt whenever those change. The options:
- (a) **The host's toolchains, in bwrap.**
  - Each Eval task is prepared once at its base commit: built and tested with network, checked that the merged fix's tests fail there and pass at the fix, and kept read-only. Each Trial works on a copy.
  - Its clone is time-safe, as SWE-bench's are. It is reset to the base with `origin` removed, later tags and branches are deleted, and the reflog is expired and pruned. Preparation fails if `git log --all` shows any later commit.
  - Code Graders run the fix's tests with no network at all.
  - Per-repository images come only when a commit stops building on the host, a repository needs services, as keeplore might, or Experiments move to a bigger machine.
- (b) **Per-repository images under rootless Podman,** on a host resized for the disk. Old commits keep building, at the cost of an image per repository with six Harnesses in it.
- (c) **Docker on the `thirdshift` host.** Its group is root-equivalent, and every unattended session runs as jacob.

*Recommended:* (a). The suite's Eval tasks are fixes a few weeks old, in repositories this host builds every day, and rustup installs whatever each commit pins. It needs no new software, and an image can come later for the one repository that needs it.

*Answer:* agree

**Q6.8 - Whether an Experiment is a Command.** Q5.1 made the Scoring run a Command, so cron starts it, and the Console shows it and can stop it. An Experiment has no such place yet. Trials write no Command record, so that the Inbox, Usual ranges and Silent ignore them (Settled), and nothing says what `thirdshift experiment` itself writes. The options:
- (a) **An Experiment is a Command.**
  - `thirdshift experiment` writes a Command record and a Command log, and its Trials and Graders appear beneath it, never as Runs.
  - The Console shows it live and can Stop it, and the Inbox shows its verdict, a failed Experiment included.
  - Its Trace holds its Trials and Graders beneath it, as a Pass's holds its Runs.
  - Its Trials still write no Command records of their own, so the Inbox, Usual ranges and Silent still ignore them.
- (b) **An Experiment writes only its results folder** (Q6.10), which the Console reads. Each Trial's and Grader's session is then a Trace of its own, since none belongs to a Command.

*Recommended:* (a). Stop, the Inbox and the live view come with being a Command, as they do for the Scoring run, and one Trace per Experiment shows the whole of it in one tree.

*Answer:* agree

**Q6.9 - What the Console lets you do for evals.** ADR 0016 limits the Console to reading, Stop and its own look, and pairs every view and action with a CLI command with `--json`. Evals add four things only you do: scoring Runs for calibration (Q5.4), confirming or correcting a Bug origin (Q4.8), approving a proposed Eval task (Q4.9), and starting an Experiment. #919 already asks for another action, dismissing items in Needs you, so ADR 0016 changes either way. The options:
- (a) **Scoring, confirming and approving in the Console, each also a CLI command.**
  - The Console hides the model's Scores until you've entered yours, as Q5.4's blind scoring needs.
  - Starting an Experiment stays in the CLI, since ADR 0016 keeps the Console from starting work.
  - ADR 0016 is amended once, for these and #919's.
- (b) **CLI only for now.** The Console shows Scores and Bug origins, and the actions move into it after a few weeks of use.
- (c) **As (a), and the Console starts Experiments too,** which reverses ADR 0016's rule that it never starts work.

*Recommended:* (a). Scoring means reading an issue, a diff and a pull request, which a page does better than a terminal, and Q5.4's first 20 Runs are a lot of reading. The rule that the Console never starts work stays whole.

*Answer:* agree

**Q6.10 - The folder for eval data.** Q2.5 keeps Eval tasks, Trials and Scores private under `~/.thirdshift/replays/`, which already holds #429's manifests and grading files. Replay is now Experiment, and the folder will also hold the Scoring run's Scores and Bug origins (Q5.1). The options:
- (a) **`~/.thirdshift/evals/`,** with three folders:
  - `suites/` for each Eval suite's Eval tasks;
  - `experiments/` for each Experiment's Trials, Scores and verdict;
  - `scoring/` for the Scoring runs' Scores and Bug origins.

  #429's files move into `suites/` once the Eval task format exists, and `replays/` stays until then.
- (b) **`~/.thirdshift/experiments/`** for all of it.
- (c) **Keep `~/.thirdshift/replays/`.**

*Recommended:* (a). Eval is the word that covers both the offline Experiments and the online Scoring run (#943), and each folder inside is named for what it holds.

*Answer:* agree

**Q6.11 - A sure loss inside the margin.** Q4.4's rules overlap. The Candidate fails when the whole interval is below zero, and passes when the whole interval is above the margin, 10 points worse (Q5.3). An interval from 8 points worse to 2 points worse does both. It can't happen with 9 Eval tasks, whose intervals are far wider, but it can as Q4.9 grows the suites. The options:
- (a) **A sure loss fails, even inside the margin.** The Candidate passes only when the whole interval is above the margin and some of it is at or above zero. You can still release or merge it after reading the interval.
- (b) **The margin wins,** as in non-inferiority trials. A loss known to be smaller than the margin passes, and only an interval wholly below the margin fails.
- (c) **A fourth verdict,** "worse, within the margin", which waits for you like an inconclusive one.

*Recommended:* (a). A change that is surely worse has no reason to pass on its own, and Q1.1 puts accuracy first. The margin keeps the job Q4.4 gave it: letting a change pass when the data can't tell a small loss from none.

*Answer:* agree
