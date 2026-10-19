# Scheduled work

## Architect runs

`thirdshift architect` starts an **Architect run**: the factory looks for architecture work itself, with no **Issue URL**, publishes a plan for the top opportunity, and implements it. Start it from a clone of the repository, on the **Base branch**, or name the Base branch with `base <branch>`:

```sh
cd ~/repos/widgets
thirdshift architect                              # review the whole codebase, then run the plan
thirdshift architect "the Spec run"               # point the review at an area
thirdshift architect merge parallel 2             # merge the plan's pull request, two Tickets at once
thirdshift architect base-fix                     # the run the plan is dispatched as may start a Base fix
thirdshift architect "the Spec run" --plan-only   # publish the plan, mark it ready and stop
thirdshift architect base main                    # review main and run the plan on it, whatever branch the clone is on
thirdshift architect --email you@example.com      # email how the Architect run ended
```

The focus is optional free text, given as one argument, anywhere among the flags; it goes into the Session prompt. `architect` is a command only as the first argument.

`merge`, `no-merge`, `base-fix`, `no-base-fix`, `parallel <n>`, `harness <name>`, `model <name>` and `effort <level>` (or the same with dashes) are for the run the plan is dispatched as, and mean what they do for `thirdshift <Issue URL>`, so none of them is ever read as the focus. The [Harness, Model and Effort](configuration.md#harness-model-and-effort) are the Architecture review's too. `--plan-only` stops the Architect run once the plan is marked ready, and dispatches nothing. `email`, optionally followed by an address, and `no-email` (or `--email` and `--no-email`) are for the Architect run's own [Run notification](#one-run-notification), with or without `--plan-only`. The word after `email` is the address only if it contains `@`, so a focus without one is never taken for it.

`base <branch>` (or `--base <branch>`) names the Architect run's **Base branch**, so the branch your clone has checked out stops mattering: start it from a clone on another branch, on a detached HEAD, or with uncommitted changes. The Architecture review's worktree starts at the head of `<branch>` on `origin`, and the run the plan is dispatched as takes `<branch>` as its Base branch too: a Run's **Issue branch**, or a Spec run's **Spec branch**, is branched off it, and the pull request or **Spec PR** targets it. It goes before or after the focus and the other flags, `--plan-only` included, and the word after it is always the branch, never the focus. Without `base`, the Base branch is the branch checked out. `base` is a flag of a Pass only: `thirdshift <Issue URL>` doesn't take it. It is what lets an Architect run be started [on a schedule](#on-a-schedule).

A second focus, a repeated flag, `merge` with `no-merge`, `base-fix` with `no-base-fix`, `email` with `no-email`, `parallel` without a whole number from 1 up, `base` with no branch after it, any other argument starting with a dash, or an empty focus is an argument error (exit `2`). So is `merge`, `no-merge`, `base-fix`, `no-base-fix` or `parallel` with `--plan-only`, since nothing is dispatched for them to apply to.

An Architect run:

1. Makes the checks a Run makes that don't need an issue, before creating anything: `origin` is a GitHub repository, git has a `user.name` and `user.email`, HEAD is not detached, and the Base branch exists on `origin` with your local copy not ahead of it. With `base <branch>`, a detached HEAD is fine, since the checkout no longer picks the Base branch, and the other two checks are made on `<branch>`: `base branch <branch> does not exist on origin; push it first with: git push --set-upstream origin 'refs/heads/<branch>:refs/heads/<branch>'` when that branch exists locally (the command quotes the actual refspec safely), or `local <branch> is <n> commit(s) ahead of origin/<branch>; push them first`. If `<branch>` is absent both locally and on origin, the error instead says to create or obtain it first, or select a valid Base branch; no push command is suggested. Preflight makes no push, checkout or branch creation. There is no **Origin match**, since there is no Issue URL: the repository is the one `origin` names. If another Pass on the repository is still running, it is [skipped](#one-at-a-time) here, and if not, it is skipped when an earlier [Architect plan is still open](#one-architect-plan-at-a-time), and if not, when an [Architect idea waits for triage](#an-architect-idea-waits-for-triage), and if not, when the repository has a [Ready issue](#a-ready-issue-goes-first). With `launch.pull = true` in the [User config](configuration.md#user-config), it then fast-forwards your checkout of the Base branch, as a Run does, and so only when the Base branch is the branch checked out: with `base <branch>` from a clone on another branch, your checkout is left alone.
2. Creates a git worktree next to your clone, named `<repo>-architect`, detached at the head of the Base branch on `origin`, with no **Issue branch**. Your checkout, its uncommitted changes and its untracked files are never touched or scanned.
3. Runs the **Architecture review** there: a headless session on the Harness, with the **Factory skills** loaded, started with the [Architecture review prompt](../../prompts/architecture-review.md). It looks for deepening opportunities, skips any an open issue already covers, and takes the top recommendation. If it is Strong, the review publishes it as the plan, labelled `needs-triage`: a **Spec** with **Tickets**, or a single Ticket when one session is enough. It may edit files in the worktree to check an idea, but commits and pushes nothing. It ends its final message with one line naming the plan: `Architecture review plan: <Issue URL>`. thirdshift reads only that line. With [no Strong candidate](#no-strong-candidate) the line names another issue, which thirdshift labels an **Architect idea**, and steps 5 to 7 are skipped.
4. Removes the worktree once the session ends, whatever the outcome. The temporary directory the Factory skills were written out to goes as the Architect run ends.
5. Checks the plan: the issue is in this repository, is open, was created after the Architect run started, and carries no other label that says it is not agent work (`ready-for-human`, `needs-info` or `wontfix`).
6. Marks the plan ready, in one request: `needs-triage` is swapped for `ready-for-agent`, the plan is labelled `architect-plan`, the mark a later Architect run finds an open **Architect plan** by, and its other labels are kept. thirdshift adds that label, never the agent, and first creates it if the repository has none. A Spec's Tickets are left as the review labelled them, without `architect-plan`.
7. Dispatches the plan, unless given `--plan-only`, exactly as `thirdshift <plan URL>` would from the same clone on the Architect run's Base branch: a [Spec run](runs.md#spec-runs) when the plan has sub-issues, a Run otherwise. Its Base branch is the Architect run's, so with `base <branch>` it is `<branch>`, not the branch checked out. `merge` and `no-merge` apply to the Spec PR or the Run's pull request, `parallel <n>` to the Spec run, and `base-fix` and `no-base-fix` to whether that run may start a [Base fix](runs.md#base-fix), as if given to that command, and the [User config](configuration.md#user-config) sets what they leave unsaid: `merge.always`, `spec.parallel`, `base.fix`, `launch.pull` and `logs.dir`. The Architecture review itself never watches CI, so a Base fix can only happen in the dispatched run. It makes the [Claim](runs.md#the-claim) as that command would too: once its checks pass, the plan's `ready-for-agent` is swapped for `in-progress`, and the plan keeps `architect-plan`. The one difference is that it sends no [Run notification](configuration.md#email) of its own, whatever `email.always` says: the Architect run sends [the one](#one-run-notification).

The dispatched run's ending is the Architect run's: its exit code, its pull request's URL alone on stdout, and its last line on stderr, `PR <url> is ready for review` or `PR <url> is merged`. If it fails, the Architect run fails as that Failed run or Failed spec run does, with the cause, the session log and the Command log on stderr and the pull request's URL on stdout if it left one. The plan stays open and `architect-plan`, and `in-progress` unless the run left nothing on `origin` and so [released its Claim](runs.md#when-the-claim-ends), for `thirdshift <plan URL>` to take up again: no later Architect run [retries it](#one-architect-plan-at-a-time). `parallel <n>` on a plan that is a single Ticket fails the same way as it does for `thirdshift <Issue URL>` on an issue that isn't a Spec: `parallel is only for a Spec, and #<n> has no sub-issues`, exit `1`, before any implementing and before the Claim, with the plan left `ready-for-agent`, to run without it.

With `--plan-only`, it instead exits `0` with the plan's URL alone on stdout, and `plan <url> is ready for an agent` as stderr's last line. Read or edit the plan, then run it with `thirdshift <Issue URL>`. The plan is labelled `architect-plan` here too, so until it is finished or closed the next Architect run is [skipped](#one-architect-plan-at-a-time).

Progress lines on stderr say when the review starts, which plan it reported, when the labels are swapped, and when the plan is dispatched: `dispatching the plan <url>, as thirdshift <url> would`. The dispatched run's own progress lines follow. With no Strong candidate, the last line says which issue the Architect run ended on instead.

A review session that fails or is interrupted, a final message without one of the lines the prompt asks for, or a plan that fails a check ends the Architect run as a failure: exit `1`, nothing on stdout, the cause on stderr and then the paths of the session log and the Command log. Nothing is dispatched and no label is changed, so a plan the review did publish stays `needs-triage`, without `architect-plan`, for you to finish or close, and doesn't skip the next Architect run. A review that finds no deepening opportunity at all has no issue to name, so it ends without one of those lines and the Architect run fails this way too; its session log says what it looked at.

### One Run notification

An Architect run asked for a [Run notification](configuration.md#email), by `email` or by `email.always = true` in the [User config](configuration.md#user-config) without `no-email`, sends exactly one, whatever its outcome, with or without `--plan-only`, unless it is skipped. A skipped Architect run, [however](#one-at-a-time) [it](#one-architect-plan-at-a-time) [is](#an-architect-idea-waits-for-triage) [skipped](#a-ready-issue-goes-first), sends none, even when asked, so a schedule can start one every few minutes without flooding your inbox: its line is in the log its scheduler keeps. It makes a Run's checks before any other work all the same, an address and a Resend API key, and stops with exit `1` if either is missing, so a run that would have been skipped fails on them too. The email goes after the outcome is final and printed, and a failed send is only a `warning:` line on stderr: it changes neither the exit code nor stdout.

- **Subject**: `[thirdshift] <owner>/<repo> Architect run: <outcome>`. With a dispatched run, the outcome is that run's: `ready for review`, `merged`, `failed` or `interrupted`. Without one, it is the review's: `plan published` (with `--plan-only`), `idea filed`, `idea already filed`, `review failed` (also for a plan that fails a check) or `interrupted`. The repository is left out if `origin` doesn't name one on GitHub.
- **Body**, plain text: a `Result:` line with the same outcome as the subject; a `Review:` line saying how the Architecture review ended, with the URL of the plan or idea issue it named (`plan published: <url>`, `idea filed: <url>`, `idea already filed: <url>`, `failed` or `interrupted`); a `Description:` line with that issue's title, when known, so you can see what the review proposed without opening the link; when the plan was dispatched, a `Dispatched:` line with that run's outcome; then what a Run's notification holds, for the dispatched run or else the failed review: the pull request URL (if any), the failure cause (if failed), with the [lines after it](runs.md#an-inherited-failure-links-the-base-branchs-checks) of a dispatched run that failed on an Inherited failure, a `Base fix:` line for a dispatched run that started or waited on a [Base fix](runs.md#base-fix), the session log path (if any), the `Command log:` path (if any), the hostname and how long the Architect run took. After a dispatched Spec run, it ends with a line per Ticket, as a Spec run's notification does.

### No Strong candidate

Only a Strong top recommendation becomes a plan. When the review's top recommendation is Worth exploring or Speculative, it publishes no plan, and the Architect run ends in one of two ways, both a success, with or without `--plan-only`: exit `0`, with one issue's URL alone on stdout. thirdshift labels that issue an **Architect idea** and dispatches nothing: there is nothing to run.

- **An idea issue.** The review files its top recommendation as one issue labelled `needs-triage`, for the **Day shift** to flesh out, and ends its final message with `Architecture review idea: <Issue URL>`. stdout carries the idea issue's URL, and stderr's last line is `no Strong candidate: the Architecture review filed the idea <url>`.
- **Already filed.** An open issue already covers that recommendation, so the review files nothing and ends its final message with `Architecture review already filed: <Issue URL>`. stdout carries that issue's URL, and stderr's last line is `no Strong candidate: <url> already covers the Architecture review's top recommendation, so it filed nothing`.

Either way, thirdshift then puts `architect-idea` and `needs-triage` on that issue, in one request that keeps its other labels, with a progress line saying so: `labelling #<n> an Architect idea: adding needs-triage and architect-idea`. thirdshift adds `architect-idea`, never the agent, and first creates it if the repository has none, as it does `architect-plan`. `needs-triage` goes back on an issue that already covered the idea even if it had been triaged, `ready-for-human` say, since the factory again takes it for the best next move. If thirdshift can't label the issue, the Architect run fails as a failed review does, with the cause on stderr, `could not label the Architect idea #<n>: <why>`, and in its [Run notification](#one-run-notification).

### An Architect idea waits for triage

An Architect idea means the factory has run out of Strong ideas, so while any Architect idea is open and still labelled `needs-triage`, every later Architect run on the repository is skipped, before any review, instead of filing the next weaker idea or landing on the same issue again. Both labels count in any case, as every label rule does. The check comes after the [lock](#one-at-a-time) and the [open Architect plan](#one-architect-plan-at-a-time) check, so with both an open plan and a waiting idea, the skip reports the plan:

- **stdout** carries each waiting Architect idea's URL, one a line, newest first.
- **stderr** carries one progress line naming each of them as waiting for triage, several joined by `; `: `Architect idea #<n> "<title>" is waiting for triage: <Issue URL>`.
- **Exit code** `0`: skipped is not a failure. No agent session starts, and nothing else is done.

Any triage decision releases the pause: take `needs-triage` off, whatever replaces it (`ready-for-agent`, `ready-for-human`, `needs-info`, `wontfix` or nothing), or close the issue. The issue keeps `architect-idea`, which only counts alongside `needs-triage`. An idea triaged `ready-for-agent` is then a [Ready issue](#a-ready-issue-goes-first), once it has settled, so the next Architect run is skipped for it in turn, until a Pickup run, or you, takes it.

### A Ready issue goes first

Work a human shaped goes ahead of the factory's own: while the repository has a **Ready issue**, by the very rule a [Pickup run](#pickup-runs) takes one by, every Architect run on it is skipped, before any review. The check comes last, after the [lock](#one-at-a-time), the [open Architect plan](#one-architect-plan-at-a-time) check and the [waiting Architect idea](#an-architect-idea-waits-for-triage) check, so with an open plan or a waiting idea as well, the skip reports that instead. It is only the search: an Architect run makes no [Sweep](#the-sweep) and holds no [Claim limit](#the-claim-limit), which are the Pickup run's.

- **stdout** carries the URL of the lowest-numbered Ready issue, the one a Pickup run would take.
- **stderr** carries, as a Pickup run's does, [a line on each issue labelled `ready-for-agent`](#why-an-issue-was-passed-over) passed over before it, then one progress line naming it: `Ready issue #<n> "<title>" goes first: <Issue URL>`.
- **Exit code** `0`: skipped is not a failure. No agent session starts, and nothing else is done.

An issue labelled `ready-for-agent` that is not a Ready issue never skips an Architect run: one that carries a Claim, was labelled or shaped within the configured settling window (thirty minutes by default), has an open blocker, is a Ticket inside a Spec that is not itself a Ready issue, or is a Base fix's issue, among the rest of the rule. Those get their lines on stderr all the same, and the Architect run goes on.

The rule doesn't ask whether anything will take the Ready issue. A `ready-for-agent` issue that you mean to run by hand, on a repository with no Pickup line in its crontab, keeps every Architect run on it from starting until you run it, which makes the Claim, or take the label off.

### One at a time

A **Pass** is a command started with no Issue URL that decides before any work whether it is skipped. Only one Pass per repository runs at a time on a machine. A Pass started while another on the same repository is still running, the Spec run or Run it dispatched included, is **skipped**: it prints `another Pass is already running on <owner>/<repo>` as a progress line on stderr, prints nothing on stdout, and exits `0`. Skipped is not a failure. A skipped Pass does nothing else: no launch pull, no worktree, no agent session, and no issue filed or labelled. This holds for every Pass, however it was started, so two never build one repository at once.

The repository is the one `origin` names, so two clones of one repository count as the same, and Passes on different repositories run at the same time. A Run or a Spec run started on an **Issue URL** is never skipped this way, and never makes a Pass skip.

thirdshift holds the rule with an operating-system lock on a file under `~/.thirdshift/architect-locks/`, shared by every kind of Pass, which the Pass's process takes without waiting, after its checks in step 1, and holds until it exits. The directory keeps its original name so older thirdshift versions share the lock too. The operating system releases the lock when that process ends for any reason, a crash, a kill or a reboot included, so there is never a stale lock to clear: the next Pass on the repository runs normally. The file itself stays, and means nothing on its own. A review worktree that a killed Architect run left behind is removed by the next one.

A skipped Architect run sends no [Run notification](#one-run-notification), even when asked, and neither does a skipped Pickup run ([sends none](#one-run-notification-for-the-issue-taken)). Both make the notification's checks before the lock is tried all the same, so a broken setup fails the pass with exit `1` rather than being skipped.

### One Architect plan at a time

An Architect run is also skipped while an earlier **Architect plan** is still open, so a new refactor is never planned while the last one is unfinished: its pull request waiting for review, its run failed, or its plan published with `--plan-only` and not started.

thirdshift marks every Architect plan with the label `architect-plan`, in step 6: the Spec, or the standalone Ticket, never a Spec's Tickets. Once the one-at-a-time lock is its own, and before the launch pull, the worktree and the Architecture review, an Architect run lists the repository's open issues labelled `architect-plan`. If there are any, it is skipped:

- **stdout** carries each open Architect plan's URL, one a line, newest first.
- **stderr** carries one progress line naming each of them with the command that picks it up, several joined by `; `: `Architect plan #<n> "<title>" is still open: pick it up with thirdshift <plan URL>`.
- **Exit code** `0`: skipped is not a failure. No agent session starts, and nothing else is done.

It applies to every Architect run, with or without `--plan-only`, and no flag overrides it. The rule is released by finishing the Architect plan, since merging its work closes the issue, by closing the issue, or by removing its `architect-plan` label. The check reads GitHub, so unlike the lock it holds across machines.

An Architect run never retries or dispatches an existing Architect plan. One whose dispatched run failed stays open for you: run `thirdshift <plan URL>` to continue whatever branch and pull request it left, or remove its `in-progress` Claim and then apply `ready-for-agent` to hand it back to Pickup after settling. The other Ready issue gates still apply. What isn't labelled `architect-plan` never counts here: a plan that a failed or interrupted review left `needs-triage`, or an [Architect idea](#no-strong-candidate), which [pauses Architect runs](#an-architect-idea-waits-for-triage) by its own rule, only while it waits for triage.

The check comes after the lock so that the plan of an Architect run that is still running, labelled already while its Spec run or Run goes on, is reported as [already running](#one-at-a-time), not as an unfinished plan. A skipped run sends no [Run notification](#one-run-notification), even when asked: its stdout carries the open Architect plans' URLs, and its stderr line their commands.

### On a schedule

thirdshift has no scheduler of its own: the operating system's scheduler runs the ordinary command. This crontab entry, written for a Linux machine with cron, starts an Architect run every night at 02:00. Add it with `crontab -e`, with your own paths in place of `/home/you` and `~/repos/thirdshift`:

```
PATH=/home/you/.local/bin:/home/you/.cargo/bin:/usr/local/bin:/usr/bin:/bin
0 2 * * * cd ~/repos/thirdshift && thirdshift architect base main >> ~/.thirdshift/logs/cron.log 2>&1
```

- **`PATH`** is set because cron doesn't read your shell profile, and `thirdshift`, the Harness's CLI (`claude` unless you chose another), `gh`, `git` and the repository's build tools must all be found. List every directory that holds one, written out in full: cron expands neither `~` nor `$HOME` on that line. `type thirdshift claude gh git` in your own shell shows where they are.
- **`cd`** makes a clone of the repository the **Launch directory**, as for a hand-typed Architect run, and **`base main`** names the [Base branch](#architect-runs), so the branch checked out in the clone doesn't matter: it runs from the clone you work in, whatever you left it on.
- **With a Pickup line on the same minutes**, add `&& sleep 20` between `cd` and `thirdshift architect`, as in the [whole crontab](#a-pickup-run-on-a-schedule) below. This gives the Pickup run a 20-second head start to take the [shared lock](#one-at-a-time) before the Architect run tries it.
- **One line per repository, at different hours**, so the Architect runs don't compete for the machine. thirdshift keeps no list of repositories.
- **The log file** takes everything the command prints. A failure before the [Run notification](#one-run-notification)'s checks have passed, such as a broken [User config](configuration.md#user-config), a missing Resend key or a bad argument, shows up only there: no email is sent for it. Use one fixed file for every line, in a folder that already exists, such as `~/.thirdshift/logs/`: the shell opens it before thirdshift starts, so a line whose folder is missing never starts the command, and nothing reports it. What each pass did on its repository is in that repository's [Activity log](observing.md#logs), in folders thirdshift creates itself, and with `activity.quiet_skips = true` the log file holds no skips at all.
- **The Harness's CLI and `gh` must already be logged in** for the user the schedule runs as, with git able to push, as the [Prerequisites](installation.md#prerequisites) say. Without a terminal an Architect run behaves as it does from one, except that it never offers Setup.

The command's flags and the User config decide what a scheduled Architect run does, as for a hand-typed one: with `merge.always` it merges the Architect plan's pull request, and without it the pull request is left for review. These are the cautious variants, whatever the User config says:

```
0 2 * * * cd ~/repos/thirdshift && thirdshift architect base main no-merge >> ~/.thirdshift/logs/cron.log 2>&1
0 2 * * * cd ~/repos/thirdshift && thirdshift architect base main --plan-only >> ~/.thirdshift/logs/cron.log 2>&1
```

The first leaves the pull request for you to review, and the second stops at the Architect plan, for you to read and run with `thirdshift <Issue URL>`.

A scheduled Architect run is skipped when another Pass on the repository is [still running](#one-at-a-time), the Spec run or Run it dispatched included, when an [Architect plan is still open](#one-architect-plan-at-a-time), when an [Architect idea waits for triage](#an-architect-idea-waits-for-triage), or when the repository has a [Ready issue](#a-ready-issue-goes-first), so a `ready-for-agent` issue with no Pickup line to take it holds off every night until you run it: no new refactor is planned until last night's Architect plan is closed or loses its `architect-plan` label. Merging its pull request closes it; if its run failed, pick it up with `thirdshift <plan URL>` first. Closing the pull request alone leaves the Architect plan open. A skip exits `0`, so the scheduler sees no failure, and the repository's [Activity log](observing.md#logs) has the line that says why, as does the log file unless `activity.quiet_skips` is set. With Run notifications on, by `email.always = true` in the User config or `email` on the line, an Architect run that did work sends one email, and a skipped one sends none, so a crontab line can fire every few minutes, as for [Weeding](#weeding), without flooding your inbox. A night without an email means it was skipped, or failed before the notification's checks passed: look in the log file.

Any scheduler that runs the command works; a systemd user timer needs lingering on (`loginctl enable-linger`) to fire while you are logged out.

### Weeding

**Weeding** is the factory clearing what features leave behind in a codebase as they land, with nobody starting it: Architect runs started often enough that each one begins as soon as the last one's Architect plan is merged. thirdshift still never schedules itself ([ADR-0009](../../docs/adr/0009-the-operating-system-schedules-thirdshift.md)): it is the line from [On a schedule](#on-a-schedule), fired every five minutes instead of nightly, with `merge.always = true` in the [User config](configuration.md#user-config) so each Architect plan's pull request is merged without waiting for you:

```
PATH=/home/you/.local/bin:/home/you/.cargo/bin:/usr/local/bin:/usr/bin:/bin
*/5 * * * * cd ~/repos/thirdshift && thirdshift architect base main >> ~/.thirdshift/logs/cron.log 2>&1
```

The `PATH` line, the `cd` and `base main` and the logins are as in [On a schedule](#on-a-schedule). Set `activity.quiet_skips = true` in the User config too, so the skips leave the log file alone: each is in the repository's [Activity log](observing.md#logs), once per change of reason.

Most passes are skipped, and that is what paces Weeding. A pass is skipped while:

- **a run is still going**: a Pass on the repository, the Spec run or Run it dispatched included ([One at a time](#one-at-a-time));
- **an Architect plan is open**: one whose pull request waits for review, one whose run failed, or one published with `--plan-only` and not yet built ([One Architect plan at a time](#one-architect-plan-at-a-time));
- **an Architect idea waits for triage**, since the factory has run out of Strong ideas ([An Architect idea waits for triage](#an-architect-idea-waits-for-triage));
- **the repository has a Ready issue**, so work a human shaped goes first ([A Ready issue goes first](#a-ready-issue-goes-first)).

A skip exits `0` and sends no email, even with Run notifications on, so the scheduler sees no failure and your inbox only hears of passes that did work. The repository's [Activity log](observing.md#logs) has the line that says why, once for each change of reason. An Architect plan whose run failed stays open, so Weeding pauses for the **Day shift** until you pick it up with `thirdshift <plan URL>`, close it, or take its `architect-plan` label off; an Architect idea pauses it until you triage the idea.

On a repository that also has a [Pickup line](#a-pickup-run-on-a-schedule), put `--plan-only` on the Weeding line instead: the Architect run publishes the Architect plan and marks it ready, and a Pickup run builds it as an ordinary Ready issue, under the [Claim limit](#the-claim-limit), and merges it with `merge.always`, as in the [whole crontab](#a-pickup-run-on-a-schedule) below.
## Security runs

`thirdshift secure` starts a **Security run**, report-only by default: it audits the **Base branch** at its head on `origin` and records each **Security finding** privately as a draft repository security advisory. Start it from a clone on the Base branch, or name that branch from any checkout:

```sh
thirdshift secure
thirdshift secure base main harness claude
thirdshift secure base main harness codex model gpt-6.1-sol effort high
```

The command takes the Harness, Model and Effort words and the shared Pass words (`merge`, `no-merge`, `base-fix`, `no-base-fix`, `parallel`, `email`, `no-email`, `security-fix` and `no-security-fix`, with or without dashes). After recording findings, it tries to reproduce each untriaged one in a fresh throwaway worktree at the recorded audited commit, and keeps the outcome, severity when reproduced, notes and test in the private record. It preserves findings already triaged by the Day shift. It never publishes or closes an advisory.

Fixing is off by default. `security-fix` on the command or `fix = true` under `[security]` allows one fix; `no-security-fix` overrides the setting. Both words are accepted on every command that starts Runs and passed to those Runs. With fixing allowed, a Security run takes the most severe reproduced finding still awaiting a fix before auditing, with ties in private-record order. After an audit reproduces findings, it goes on to the most severe one.

A publishing session reads the private record and follows the reproduction's fix-size decision: a single **Ticket**, or a **Spec** with **Tickets** through `thirdshift-to-spec` and `thirdshift-to-tickets`. Every new issue is terse, saying only what the fix changes and linking the private record. thirdshift checks all the issues, swaps the top issue's `needs-triage` for `ready-for-agent`, adds `security-fix` to every fix issue (creating the label when missing), records the fix link privately and dispatches its **Run** or **Spec run**. The Security run ends as that run ends, including a **Merge run** when asked by the command or the User config.

On a private repository, the finding's own issue is the fix's Ticket or Spec, keeping its write-up and proof-of-concept. A one-session fix needs no publishing session or second issue: thirdshift marks the finding's issue ready and dispatches it. For a bigger fix, the publishing session adds terse Tickets as that issue's native sub-issues, with their blocking links.

A failed fix keeps its **Claim**, even if it pushed nothing, and stays open for the **Day shift**. No later Security run or Pickup run retries it. Security runs pause while that fix's issue is open; closing the issue lets them go on.

`email`, optionally followed by an address, or `email.always` in the User config asks for one **Run notification** when the Security run ends, whether the audit succeeded, failed or was interrupted. `no-email` overrides the default. The notification says how the audit ended and lists each finding it recorded or matched to an existing private record: its severity when known, title and private link. It includes no finding write-up, trace or evidence, since it passes through Resend. Detailed failure causes stay in the local logs; the notification gives the audit's status and log paths. A recording failure still lists the records reached before it failed. When a run leaves a reproduced finding unfixed and neither the command nor the User config decided against fixing, its notification and stderr offer both `thirdshift secure security-fix` and `fix = true` under `[security]`. With `no-security-fix`, or the setting turned off, it offers nothing. A skipped Security run sends none. The usual address and Resend API key checks run before the skip checks or any work; a failed send is a warning and never changes the run's outcome.

A Security run chooses its Harness from the command's `harness` word, then `[security] harness` in the User config, then `harness.default`, then Claude Code. When `security.harness` is blank or missing, it uses `harness.default`; when `harness.default` is also missing, it uses Claude Code. Model and Effort come from the chosen Harness's own `[harness.<name>]` section, with command words taking precedence. For example, `harness claude` overrides `[security] harness = "codex"` and uses `[harness.claude]`.

Codex security sessions and their Resumes set `agents.max_concurrent_threads_per_session=8` and ask for fresh sub-agents with `fork_turns: "none"`, so the skill's verifiers stay independent.

A Security run checks its gates in order: another **Pass** on the repository running on the machine, a **Ready issue**, a failed Security fix whose issue is still open, a **Security finding** waiting for the **Day shift**, then an unchanged **Base branch** since the last completed Security audit. An unreproduced draft advisory with no severity waits; closing it, publishing it or assigning severity ends that wait. A reproduced finding waits while fixing is disabled until its record is closed or published, or its fix Ticket is closed. On a private repository, an open finding issue with `needs-triage` waits until that label is removed or the issue is closed. The audited commit comes from the skill's own `run-metadata.json` under the audit root, with no separate state file. A skip exits `0`, starts no session, keeps no Command log and sends no Run notification; its reason goes into the repository's **Activity log** only when it changes, with the usual `activity.quiet_skips` behavior. Before starting work, thirdshift checks the chosen Harness and that **Node.js** is on `PATH`, since the embedded skill's report validators require it.

The Security audit runs in a throwaway worktree detached at origin's Base branch head, leaving the Launch directory and local work untouched, including when `launch.pull` is set. Its Session prompt runs `thirdshift-security-audit` in full audit mode with the `quick` profile, auditing the whole repository except vendored and third-party code. It names a `SECURITY.md` or conventional threat-model document when present, reads compatible earlier runs and ends `incomplete` instead of asking for input.

Each audit keeps a new output directory under `<logs.dir>/<owner>/<repo>/audits/`, outside the worktree. The audit artifacts and private advisory descriptions contain evidence; keep the logs directory private. thirdshift runs both embedded validators before recording anything. A missing final line, an incomplete session or an invalid report fails the run and names its Session log. Command logs and Session logs are in `commands/secure/`; Session logs are named `secure-<stamp>-security-audit.jsonl`; the Activity log records the start and ending as for other Passes.

A finding's title becomes the advisory summary. Its description preserves its write-up, trace, evidence and validation plan, with the fingerprint and audited commit. thirdshift claims no severity, CWE or affected version range. It uses the package in the repository's manifest, or the `other` ecosystem when none is identified. Matching fingerprints in any advisory state are kept rather than recorded again. Rejected candidates stay in the local report and produce no advisory. Draft advisories require GitHub repository security manager or administrator access. On a private repository whose advisory endpoint is unavailable, the private record is an issue labelled `security-finding` and `needs-triage`. It is never replaced by a public finding issue.

### Security review

`security-review`, or `review = true` under `[security]`, enables one guidance review after a Run's opening session or a Spec run's Spec review, before Delivery pushes and enters the Repair loop. A Spec run reviews the whole Spec branch against its Base branch once; its Tickets' Runs get none. `no-security-review` overrides the setting. Both words, with or without dashes, are accepted on commands that start Runs and follow dispatched Runs and Base fixes. Setup asks about the setting; it is off by default.

The session reads the relevant attack-class guidance and supporting code without delegated auditors or the full audit workflow. It fixes only findings reproduced by a failing proof-of-concept test, keeps that test and commits the fix. Unaddressed introduced findings appear under Security in the PR's Unaddressed findings. A Merge run, including a Spec run asked to merge its Spec PR, leaves the PR ready for review and fails with the findings named if any remain, or if the review was refused or incomplete. A Run or Spec run not asked to merge records the outcome and continues. An old finding, whose proof-of-concept also fails at the pinned merge base, is reported in a private file under the repository's logs (`security-reviews/`). thirdshift records it as a draft advisory, or a `security-finding`/`needs-triage` issue on a private repository, reusing existing fingerprints and preserving the Day shift's grades. New records include scored reproduction and fix-size metadata so a later Security run can take the fix when allowed. Its details stay out of the PR and it does not hold Self-merge; fixing belongs to a separate Security run. The separate Security audit retains its full audit workflow.

### Fencing

**Fencing** is the factory finding and fixing vulnerabilities with nobody starting it: Security runs started on a schedule with fixing allowed. thirdshift never schedules itself ([ADR-0009](../../docs/adr/0009-the-operating-system-schedules-thirdshift.md)). This Linux crontab entry starts one every five minutes; use your own paths, with `claude` and `gh` already logged in and Git able to push:

```cron
PATH=/home/you/.local/bin:/usr/local/bin:/usr/bin:/bin
*/5 * * * * cd /home/you/repos/widgets && thirdshift secure base main harness claude security-fix >> /home/you/thirdshift-secure.log 2>&1
```

Each pass yields to a **Ready issue**, so work the **Day shift** shaped goes first. It is skipped while another Pass on the repository is running, while a fix has failed and its issue stays open, while a finding waits for triage and no reproduced finding can be fixed, or while the Base branch is unchanged and no fix is waiting. A failed fix pauses Fencing for the Day shift until its issue is closed; it is never retried by a later Security run or Pickup run. Triaging a finding lets audits go on; fixing allowed can still take a reproduced finding ahead of a different finding waiting for triage.

A skip exits `0` and sends no Run notification. Its reason appears in the **Activity log** once per change of reason. Set `activity.quiet_skips = true` in the [User config](configuration.md#user-config) to keep those skips out of the scheduler's log. `merge` on the command, or `merge.always = true` in the User config, lets each fix's Run merge its pull request; otherwise it stays ready for review.
## Pickup runs

`thirdshift pickup` starts a **Pickup run**: one pass, with no **Issue URL**, that takes the lowest-numbered **Ready issue** in the repository and runs it, so that an issue you have already marked ready needs no command typed for it. Start it from a clone of the repository, on the **Base branch**, or name the Base branch with `base <branch>`:

```sh
cd ~/repos/widgets
thirdshift pickup                      # take the lowest-numbered Ready issue and run it
thirdshift pickup merge parallel 2     # merge its pull request; two Tickets at once if it is a Spec
thirdshift pickup base main no-merge   # run it on main, whatever branch the clone is on, and leave its pull request for review
```

A Ready issue is an open issue that:

- is labelled `ready-for-agent`: an issue without the label is never taken;
- has none of the labels that make an **Unready Ticket**, `ready-for-human`, `needs-info`, `wontfix` and `needs-triage`, so a contradictory label errs on the side of not running;
- carries no [Claim](runs.md#the-claim): it is not labelled `in-progress`;
- is not a sub-issue. A sub-issue is a **Ticket** of a **Spec**, and is never run on its own, whatever the Spec's labels: it is reached through its Spec, when the Spec is itself a Ready issue, so its work always goes through the **Spec branch**. Labelling one Ticket never promotes its Spec, either;
- is not labelled `base-fix` or `security-fix`: the Run that opened a [Base fix](runs.md#base-fix)'s issue, or the Security run that dispatched a fix, owns it. A failed Security fix is still passed over if GitHub prevented its Claim from being made;
- has no open blocker, by GitHub's "blocked by" links, never the text of its body. Once every blocker is closed, it can be taken;
- either was never started (no **Issue branch** on `origin` and no pull request from one, open, merged or closed), or you handed it back by applying `ready-for-agent` after its last [Claim](runs.md#the-claim) ended. For started work, the full label history must show `in-progress` applied, then removed, then `ready-for-agent` applied. Work never Claimed, or without that later hand-back, stays with you. An eligible branch with no PR or an open PR follows the existing [Continuation](runs.md#continuation) rule; merged or closed PRs follow ADR 0002 and start a fresh numbered branch. This is deliberate hand-back, not automatic retries;
- is not a **Spec** whose **Tickets** are all closed. With nothing started on it, such a Spec was done some other way, and the [Spec run](runs.md#spec-runs) it would be dispatched as stops with nothing to do, so a Pickup run passes it over rather than take it on every pass. Waiting never makes it a Ready issue, so this comes before the next condition, however lately the Spec was labelled: close it, or take its `ready-for-agent` off. A Spec with at least one open Ticket is taken;
- is settled: by default, thirty minutes have passed since the latest of `ready-for-agent` being applied to it, a sub-issue being added to it or removed, and a "blocked by" link being added to it or removed. So a Spec is not taken while its Tickets are still being attached. These are read from the issue's timeline, since adding a sub-issue does not change the issue's update time, and `pickup.wait_minutes` in the [User config](configuration.md#user-config) sets the window in whole minutes from 0 up, within the supported duration range; 0 disables waiting. The setting also controls this same Ready issue search for Architect runs. A later pass takes the issue once it has settled.

A Pickup run:

1. Makes the checks an [Architect run](#architect-runs) makes, before anything else but the [Run notification](#one-run-notification-for-the-issue-taken)'s own: `origin` is a GitHub repository, git has a `user.name` and `user.email`, HEAD is not detached unless `base <branch>` names the Base branch, and the Base branch exists on `origin` with your local copy not ahead of it. A check that fails stops the pass with exit `1`, before any label is read or changed.
2. Tries the Pass lock, and is [skipped](#one-at-a-time) if another Pass on the repository is still running on the machine.
3. Takes `in-progress` off the repository's closed issues: the [Sweep](#the-sweep).
4. Counts the repository's open issues labelled `in-progress`, and is skipped if the repository is at its [Claim limit](#the-claim-limit).
5. Lists the repository's open issues labelled `ready-for-agent`, lowest number first, and takes the first that is a Ready issue, saying so on stderr: `taking Ready issue #<n> "<title>", as thirdshift <Issue URL> would`. Each one it passes over on the way gets [a line saying why](#why-an-issue-was-passed-over). One issue a pass: the rest wait for the next.
6. Atomically reserves the selected issue on `origin`, then rechecks that it is still Ready and no Claim has been made since selection. If another Pickup run holds the reservation, the issue ceased to be Ready, or an intervening Run has already taken it and returned it to Ready, skips with exit `0`, no Claim, no session and no notification. This also protects two hosts using the same GitHub identity, with settling disabled or clocks out of sync.
7. Dispatches it exactly as `thirdshift <Issue URL>` would from the same clone on the Pickup run's Base branch: a [Spec run](runs.md#spec-runs) when the issue has sub-issues, a Run otherwise. That run makes the Claim, so the issue's `ready-for-agent` is swapped for `in-progress` and no later pass takes it again. The reservation lasts through the dispatched run's ending, including failed-work preservation and Claim ending, then is released with an exact ownership check, even after interruption.

Every Pickup host must run a version that honors reservations. The reservation branch is `thirdshift/pickup/<issue-number>`; direct Issue URL commands and other Pass kinds keep their existing behavior. A crash or an uncertain remote failure can leave a reservation behind. Before releasing one, confirm its host's Command has stopped. Inspect `git ls-remote origin refs/heads/thirdshift/pickup/7`, fetch that ref, and read its commit message for the launch path, process and start time. Then delete it with the observed SHA as an exact lease: `git push --force-with-lease=refs/heads/thirdshift/pickup/7:<observed-sha> origin :refs/heads/thirdshift/pickup/7`. Never delete a live owner's reservation or remove one on a timer. Repository permissions must allow creating and deleting these branches. See [ADR 0017](../../docs/adr/0017-pickup-reserves-issues-on-origin.md).

`merge`, `no-merge`, `base-fix`, `no-base-fix`, `parallel <n>`, `harness <name>`, `model <name>` and `effort <level>`, with or without dashes, are for the dispatched run, and mean what they do for `thirdshift <Issue URL>`, as do `email`, optionally followed by an address, and `no-email` for the Pickup run's [Run notification](#one-run-notification-for-the-issue-taken). The [User config](configuration.md#user-config) sets what they leave unsaid: `merge.always`, `base.fix`, `spec.parallel`, `email.always`, `launch.pull` and `logs.dir`. A User config a Run would refuse stops the pass the same way, before any check. `parallel <n>` applies when the Ready issue is a Spec and is ignored, with no error, when it isn't, unlike on an Issue URL: the command can't know which it will take. `base <branch>` (or `--base <branch>`) names the Base branch as it does for [`architect`](#architect-runs), and the dispatched run takes it: its Issue branch or Spec branch is branched off `<branch>`, and its pull request targets it. With `base <branch>`, a Pickup run started [on a schedule](#a-pickup-run-on-a-schedule) runs from the clone you work in, whatever branch you left it on.

`pickup` is a command only as the first argument, and takes nothing but those flags, each at most once: a focus, `--plan-only`, an Issue URL, a repeated or contradictory flag, or any other argument is an argument error (exit `2`).

The dispatched run's ending is the Pickup run's: its exit code, its pull request's URL alone on stdout, and its last line on stderr, `PR <url> is ready for review` or `PR <url> is merged`. If it fails, the Pickup run fails as that [Failed run](runs.md#failed-runs) or Failed spec run does. After an [Inherited failure](runs.md#an-inherited-failure-links-the-base-branchs-checks), the command it offers is `thirdshift <Issue URL>` with the dispatched run's flags and `base-fix`, since another Pickup run needs you to remove its Claim and then apply `ready-for-agent` before it can take started work again.

A pass is **skipped** when the lock is held, when the repository is at its Claim limit, when the repository has no Ready issue, or when the selected issue is reserved or stale: it exits `0` with stdout empty and a reason on stderr. A reserved or stale selection says `#<n> is reserved by another Pickup run or is no longer Ready`, with the reservation ref to inspect if it stays reserved. Skipped is not a failure. A skipped pass starts no agent session, makes no Claim, and sends no Run notification. One skipped for the lock changes no label at all; the others have made the Sweep, whose lines come before the reason.

### The Claim limit

A Pickup run takes nothing while the repository is at its **Claim limit**: as many open issues carry a [Claim](runs.md#the-claim), that is, are labelled `in-progress`, as the limit, which is 3 unless you set it. It keeps a broken Base branch from failing every Ready issue in turn, one a pass, and pull requests from piling up unreviewed: once that many issues wait on you, the factory waits too.

- The count is every open issue labelled `in-progress` in the repository, whoever started it: a Run you started by hand counts as one a Pickup run dispatched does. Closed issues don't count.
- `pickup.limit` in the [User config](configuration.md#user-config) sets the limit, a whole number from 1 up, by default 3. Raise it on a machine where you review quickly; `limit = 1` takes one issue at a time. There is no command-line flag for it.
- At or over the limit, the pass is skipped: exit `0`, stdout empty, and the reason on stderr with the count and the limit, such as `at the Claim limit on acme/widgets: 3 open issue(s) labelled in-progress, pickup.limit is 3`. No Ready issue is looked for, and no session is started.
- An issue leaves the count when it is closed, as merging its pull request does, or when you take its `in-progress` label off.
- If the issues can't be counted, the pass stops with exit `1` and `gh`'s error.

### The Sweep

In the **Sweep**, each Pickup run, once it holds the lock and before it counts, lists the repository's closed issues labelled `in-progress` and takes the label off each, so an issue whose pull request you merged by hand doesn't look taken for ever.

- Only `in-progress` is removed, in one request per issue: the closed issue's other labels stay. stderr says `taking in-progress off #<n>, which is closed`.
- It runs on every pass that gets the lock, one that is then skipped for the Claim limit or for having no Ready issue included, and not on one skipped because the lock is held.
- A failure, to list the closed issues or to take the label off one, is a `warning:` line on stderr, such as `warning: could not take in-progress off #<n>: <gh's error>`, and the pass carries on: the next pass tries again.

### Why an issue was passed over

Each open issue labelled `ready-for-agent` that a pass looks at and does not take gets one line on stderr, naming the issue and the first reason that applies, in the order of the Ready issue rule above. The lines come before the line that says what the pass did, so the cron log explains why nothing started:

```
thirdshift: 03:00:01 Pickup run starting, 2026-10-03 -0400
thirdshift: 03:00:02 #18 labelled needs-info
thirdshift: 03:00:02 #19 labelled in-progress
thirdshift: 03:00:03 #21 is a Ticket of #20, which is not ready
thirdshift: 03:00:03 #26 labelled base-fix
thirdshift: 03:00:04 #30 blocked by #29
thirdshift: 03:00:05 #31 started: issue-31 is on origin; no ready-for-agent hand-back after the last Claim ended
thirdshift: 03:00:06 #32 started: PR https://github.com/acme/widgets/pull/37; no ready-for-agent hand-back after the last Claim ended
thirdshift: 03:00:06 #33 every Ticket is closed
thirdshift: 03:00:07 #34 not settled: labelled ready-for-agent less than 30 minutes ago
thirdshift: 03:00:07 #35 not settled: a sub-issue added or removed less than 30 minutes ago
thirdshift: 03:00:08 #36 not settled: a "blocked by" link added or removed less than 30 minutes ago
thirdshift: 03:00:08 no Ready issue on acme/widgets
```

A line with `blocked by` names every open blocker. A Ticket's line names its Spec: the Ticket runs when its Spec does. A Ticket whose Spec is a Ready issue gets no line: its Spec is taken, by this pass or a later one. A started issue without a recorded hand-back gets the `started` line. Even with a hand-back, a Spec with every Ticket closed is passed over with `every Ticket is closed`. When a later issue is a Ready issue, the lines on the earlier ones are printed and it is taken:

```
thirdshift: 03:00:01 Pickup run starting, 2026-10-03 -0400
thirdshift: 03:00:02 #18 labelled needs-info
thirdshift: 03:00:03 #20 blocked by #17
thirdshift: 03:00:04 taking Ready issue #22 "Sharpen the widgets", as thirdshift https://github.com/acme/widgets/issues/22 would
thirdshift: 03:00:04 logging this command to ~/.thirdshift/logs/acme/widgets/commands/pickup/22-20261003T030001-0400.log
```

### One Run notification for the issue taken

A Pickup run asked for a [Run notification](configuration.md#email), by `email` or by `email.always = true` in the [User config](configuration.md#user-config) without `no-email`, sends exactly one when it took a Ready issue: the notification the Spec run or Run it dispatched would have sent started by hand. An address after `email` wins over `email.to`, as for a Run.

- **Subject**: that run's, `[thirdshift] <owner>/<repo>#<n> <issue title>: <outcome>`, naming the issue taken, with the outcome `ready for review`, `merged`, `failed` or `interrupted`.
- **Body**: that run's too: what a Run's notification holds, with `Issue:` immediately after `Result:` linking the issue it took (the parent Spec when it dispatched a Spec run), and after a Spec run a line per Ticket. The time it gives is the whole pass's, the search for the Ready issue included.

The email goes after the outcome is final and printed, and a failed send is only a `warning:` line on stderr: it changes neither the exit code nor stdout. The dispatched run sends no notification of its own, whatever the User config says, as with an [Architect run](#one-run-notification), nor do a Spec run's Ticket Runs.

A skipped pass sends no notification, even when one was asked for: a pass every half hour would otherwise send dozens a day. A skipped Architect run sends none either.

The notification's checks, an address and a Resend API key, are made before any other work on every pass, before the checks in step 1, the lock and any label read or changed. A broken setup therefore stops the pass with exit `1`, naming what is missing, even a pass that would have been skipped, so it shows in the scheduler's log on the first pass, not only once an issue is ready. A pass that asks for no notification makes none of these checks.

### A Pickup run on a schedule

thirdshift has no scheduler of its own ([ADR-0009](../../docs/adr/0009-the-operating-system-schedules-thirdshift.md)): the operating system's scheduler runs the ordinary command, one pass each time. This crontab entry, written for a Linux machine with cron, starts a pass every half hour. Add it with `crontab -e`, with your own paths in place of `/home/you` and `~/repos/widgets`:

```
PATH=/home/you/.local/bin:/home/you/.cargo/bin:/usr/local/bin:/usr/bin:/bin
*/30 * * * * cd ~/repos/widgets && thirdshift pickup base main >> ~/.thirdshift/logs/cron.log 2>&1
```

The `PATH` line, the `cd` and `base main`, the log file's directory, the logins the Harness's CLI and `gh` need, and schedulers other than cron are as for an Architect run: see its [On a schedule](#on-a-schedule). What differs for a Pickup run:

- **The interval** sets how soon a Ready issue is taken and how much work is started: a pass takes one issue, so `*/30` starts at most one every half hour. A pass lasts as long as the run it dispatched, and the passes that fire meanwhile are skipped, so the schedule builds one issue of a repository at a time on a machine, and the first pass after it ends takes the next, unless the repository is by then at its [Claim limit](#the-claim-limit): each pull request left for review, and each failure left for you, holds a Claim, and at 3 of them, or the `pickup.limit` you set, the passes take nothing until you have dealt with one. An issue you have just labelled also waits thirty minutes by default, or the `pickup.wait_minutes` you set, until it is settled. A Run you start by hand on an Issue URL is outside all this: it neither waits for a pass nor makes one skip.
- **One line per repository.** thirdshift keeps no list of repositories. Each Pass on one repository skips while another is still running, the Spec run or Run it dispatched included, so both lines can go in one crontab and two Passes never build that repository at once. Passes on different repositories do run at the same time: give their lines different minutes, such as `15,45`, if they would compete for the machine.
- **A pass is skipped** when the [lock is held](#one-at-a-time), when the repository has no Ready issue, and when it is at its Claim limit. A skip exits `0`, so the scheduler sees no failure, and sends no email, even with Run notifications on. The repository's [Activity log](observing.md#logs) records a skip when its reason changes. Unless `activity.quiet_skips` is set, the log file holds each skip's line too, and before the line of a pass that found no Ready issue, as before the line of one that took an issue, [why each issue was passed over](#why-an-issue-was-passed-over). A pass skipped for the lock or the Claim limit looks at no issue, so it has no such lines.
- **A Run notification**, by `email.always = true` in the [User config](configuration.md#user-config) or `email` on the line, is sent [only for an issue taken](#one-run-notification-for-the-issue-taken). So a day without email doesn't say the passes are running: a pass that fails before it takes an issue, on a broken User config, a missing Resend key or a failed check, shows up only in the log file.

A whole crontab with both Weeding and a Pickup run on one repository looks like this, with `activity.quiet_skips = true` in the User config, every line sending its output to the one log file, which catches only what went wrong, so `tail ~/.thirdshift/logs/acme/widgets/activity.log` shows what the factory has been doing on the repository lately:

```
# cron reads no shell profile: list every directory that holds thirdshift, claude, gh, git and the build tools
PATH=/home/you/.local/bin:/home/you/.cargo/bin:/usr/local/bin:/usr/bin:/bin

# Weeding, every five minutes: an Architect run publishes each Architect plan, and a Pickup run builds it
*/5 * * * * cd ~/repos/widgets && sleep 20 && thirdshift architect base main --plan-only >> ~/.thirdshift/logs/cron.log 2>&1

# Pickup runs, every half hour
*/30 * * * * cd ~/repos/widgets && thirdshift pickup base main >> ~/.thirdshift/logs/cron.log 2>&1
```

For one Architect run a night instead of Weeding, put the nightly line from [On a schedule](#on-a-schedule) in place of the Weeding line: a repository needs one or the other, since Weeding is Architect runs started more often. Adding a repository is adding its lines. Add the lines with `crontab -e` rather than by joining files, and end each with a newline: a line run together with the comment after it, such as `... 2>&1# Edit this file`, is a shell syntax error, so cron fires it but the pass never starts and nothing reaches its log.

The command's flags and the User config decide what a pass does with the issue it takes, as for a hand-typed `thirdshift <Issue URL>`: with `merge.always` it merges the pull request, and without it the pull request is left for review. This is the cautious variant, whatever the User config says:

```
*/30 * * * * cd ~/repos/widgets && thirdshift pickup base main no-merge >> ~/.thirdshift/logs/cron.log 2>&1
```

An issue whose run failed [keeps its Claim](runs.md#when-the-claim-ends) and waits for the **Day shift**: it stays `in-progress`, with the Issue branch and any draft pull request the run left, no later pass takes it, and it counts towards the Claim limit until it is closed or you take the label off. To send it round again by hand, run `thirdshift <Issue URL>` from the clone, with the Base branch checked out, since that command takes no `base <branch>`: it picks up where the failed run stopped, as a [Continuation](runs.md#continuation) does. To hand it back to Pickup instead, remove `in-progress` and then apply `ready-for-agent`: after settling, a pass can continue it if every other Ready issue gate still permits it. Applying the ready label before the last Claim ends does not hand it back. For an ordinary Run, the one failure a later pass does retry is one that left nothing on `origin`, such as a usage limit or an expired login: its Claim is released, so the issue is a Ready issue again once it has settled. A failed Security fix is always left for the Day shift, even with nothing on `origin`.

When shaping issues with `to-spec` and `to-tickets` from [mattpocock/skills](https://github.com/mattpocock/skills), keep a **Spec** labelled `needs-triage` until every **Ticket** is published, attached as a sub-issue and linked to its blockers; then replace it with `ready-for-agent`. A Spec with no Tickets yet looks exactly like a standalone Ticket, so a pass would start a plain Run on it. This repository's Day shift follows the publication procedure in [`docs/agents/triage-labels.md`](../../docs/agents/triage-labels.md), required by [`AGENTS.md`](../../AGENTS.md), which overrides the imported skills' defaults and survives their updates. The settling window (thirty minutes by default) is only a second line of defence: it covers Tickets attached within minutes of the label, not a Spec left labelled and without Tickets for longer.
