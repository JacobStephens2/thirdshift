# thirdshift

A factory that turns a GitHub issue into a ready-for-review pull request, or on request a merged one, by running unattended agent sessions against it: the humans are the day shift, the agents work the third shift. Quality over quantity: a pull request marked ready for review, or merged by the factory, is one the factory stands behind.

## Language

**Day shift**:
The human half of the workflow: shaping work into issues the factory can take on (grilling, specs, tickets), and reviewing the pull requests it delivers, except those a **Merge run** merges itself.
_Avoid_: planning phase

**Issue URL**:
The GitHub issue link the factory is given to implement, e.g. `https://github.com/<owner>/<repo>/issues/<n>`.
_Avoid_: ticket, spec link

**Launch directory**:
The directory a **Run** is started from. It must pass **Origin match**, and its checked-out branch is normally the **Base branch**. A Run works in its own worktree, never in the Launch directory.
_Avoid_: originating workspace, launch repo

**Origin match**:
The rule that the **Issue URL** must belong to the same GitHub repository as the `origin` remote of the **Launch directory**. A mismatch stops the run before any work happens.

**Base branch**:
The branch the work branches off and the pull request targets. Normally the branch checked out in the **Launch directory**; for an **Architect run** or a **Pickup run** whose command names a branch, that branch, whatever is checked out; for a **Ticket**'s **Run** in a **Spec run**, the **Spec branch**; in a **Continuation** with an open pull request, that pull request's base instead.

**Factory skills**:
The skills in `skills/`, each named `thirdshift-<skill>` and placed in each **Run**'s worktree for its **Harness** to find: adapted copies of Matt Pocock's skills, designed to run headless with no human in the loop.
_Avoid_: the skills (ambiguous with `.claude/skills/`)

**Session prompt**:
The message thirdshift starts or resumes an agent session with. It carries the **Run**'s facts (issue, **Base branch**, **Issue branch**, pull request) and names the **Factory skills** to use. Written by thirdshift, not by a human.
_Avoid_: instructions, system prompt

**Harness**:
The headless agent CLI that runs every agent session of a **Command**: Claude Code (`claude`), Codex (`codex`) or Grok Build (`grok`). One per Command, its child Runs and every **Resume** and **Repair** included. Chosen by the command, else the **User config**, else Claude Code.
_Avoid_: agent, provider, backend

**Model**:
The model the **Harness** runs its sessions on, such as Opus 5.5 or GPT-6.1-Sol. One per **Command**, chosen by the command, else the **User config**'s choice for that Harness, else the Harness's own default.

**Effort**:
How hard the **Model** reasons in each session, such as `high` or `max`. One per **Command**, chosen the way the Model is.
_Avoid_: reasoning level, thinking budget

**Standards finding**:
A finding from the Standards axis of a code review: the change breaks a documented coding standard or shows a baseline code smell.

**Spec finding**:
A finding from the Spec axis of a code review: the change is missing, gets wrong, or goes beyond what the issue asked for.

**Unaddressed finding**:
A **Standards finding** or **Spec finding** the agent chose not to fix. It is listed in the pull request body so a human can decide on it.

**Spec**:
An issue describing a multi-session piece of work, what is being built rather than how each session does its share, made of **Tickets**: its GitHub sub-issues. See [Spec](https://www.aihero.dev/ai-coding-dictionary/spec) in Matt Pocock's AI Coding Dictionary.
_Avoid_: PRD, parent issue

**Ticket**:
An issue scoping one session of work, standing alone or as a sub-issue of a **Spec**. A Ticket in a Spec can block or be blocked by sibling Tickets; the order of work falls out of those GitHub "blocked by" links. See [Ticket](https://www.aihero.dev/ai-coding-dictionary/ticket) in Matt Pocock's AI Coding Dictionary.
_Avoid_: task, sub-task

**Run**:
One invocation of the factory on a single issue that is not a **Spec**, from launch to cleanup, ending in one pull request. Started directly on an **Issue URL**, by a **Spec run** for one of its **Tickets**, by an **Architect run** for the Ticket its **Architecture review** published, or by a **Pickup run** for the **Ready issue** it took.

**Claim**:
The mark that the factory has taken an issue: thirdshift labels the issue `in-progress`, in place of `ready-for-agent` if it has that, when a **Run** or a **Spec run** starts on it, whether started on its **Issue URL** or by an **Architect run** or a **Pickup run**. A **Ticket**'s Run in a Spec run and a **Base fix** make none. A Claim is released, `ready-for-agent` put back, only when the Run ends with nothing on `origin` to take over: no **Issue branch** and no pull request. Otherwise it stays while the issue is open, whether its pull request is ready for review or the Run failed, until the **Day shift** relabels it, and thirdshift takes the label off once the issue is closed.
_Avoid_: lock (a lock is per machine and ends with its process), assignment

**Spec run**:
One invocation of the factory on a **Spec**'s **Issue URL**: it works through the Spec's **Tickets** in dependency order, starting a **Run** for each Ticket once the Tickets blocking it are done, several at once when the graph allows.
_Avoid_: batch run, spec implementation

**Architect run**:
One invocation of the factory with no **Issue URL**: an **Architecture review** of the **Base branch**, then, unless asked to stop there, a **Spec run** or a **Run** on the **Architect plan** it published. One pass per invocation. It is skipped, doing nothing, when another Architect run or a **Pickup run** on the same repository is still running on the machine, its Spec run or Run included, when an earlier Architect plan is still open, when an **Architect idea** is waiting for triage, or when the repository has a **Ready issue**, so that work a human shaped goes first. Skipped is not a failure, and sends no **Run notification**, so that it can be started every few minutes and so follow the last one as soon as its plan is merged.
_Avoid_: improve run, architecture run

**Weeding**:
The factory clearing the detritus that features leave in a repository's codebase as they land, with no human starting it: **Architect runs** started often enough that each follows the last as soon as its **Architect plan** is merged. It yields to work a human shaped, and pauses for the **Day shift** whenever a plan fails or an **Architect idea** waits for triage.
_Avoid_: upkeep, continuous architect, daemon, loop

**Pickup run**:
One invocation of the factory with no **Issue URL**: it takes the lowest-numbered **Ready issue** in the repository and starts a **Spec run** or a **Run** on it, as the command on that issue's URL would. One issue per invocation, and thirdshift never schedules itself: the operating system's scheduler starts each one. It is skipped, doing nothing, when another Pickup run or an **Architect run** on the same repository is still running on the machine, its Spec run or Run included, when there is no Ready issue, or when the repository is at its **Claim limit**. Skipped is not a failure, and sends no **Run notification**.
_Avoid_: watch, daemon, queue run, poll

**Pass**:
An **Architect run** or a **Pickup run**: a **Command** started with no **Issue URL**, which decides before any work whether it is skipped.
_Avoid_: tick, cycle

**Claim limit**:
The number of open issues carrying a **Claim** at which a **Pickup run** takes no more, set in the **User config**. It keeps a broken **Base branch** from failing every **Ready issue** in turn, and pull requests from piling up unreviewed.
_Avoid_: WIP limit, concurrency (it counts issues waiting on the **Day shift**, not Runs in flight)

**Sweep**:
A **Pickup run**'s removal of `in-progress` from every closed issue in the repository that still has it, such as one whose pull request was merged by hand, with the issue's other labels kept. Each Pickup run that is not skipped for the lock makes one, before it holds the repository against its **Claim limit**. A Sweep that fails is a warning, never a failure.
_Avoid_: cleanup, garbage collection

**Ready issue**:
An open issue a **Pickup run** may take: labelled `ready-for-agent`, with no label that makes an **Unready Ticket** and no **Claim**, not a sub-issue, not a **Base fix**'s issue, with no open blocker, never started (no **Issue branch** and no pull request), not a **Spec** whose **Tickets** are all closed, and left untouched long enough that whoever is shaping it has finished. On a Spec, the label says its Tickets are published. A `ready-for-agent` Ticket inside a Spec is not one: it is reached through its Spec, when the Spec is itself a Ready issue.
_Avoid_: queued issue, backlog item

**Architecture review**:
The agent session that opens an **Architect run**: it scans the **Base branch** for deepening opportunities, skips any already covered by an open issue, and publishes the top recommendation as the **Architect plan**, but only when that recommendation is Strong. Otherwise it files the top recommendation as an **Architect idea** for the **Day shift**, or names the open issue that already covers it, and nothing is implemented. It changes nothing in the repository.
_Avoid_: planning session

**Architect plan**:
The **Spec**, or the standalone **Ticket** when one session is enough, that an **Architecture review** published: one or the other, never a third kind of issue. thirdshift labels it `architect-plan` once the review has ended cleanly, and it stays open until its work is merged or someone closes it. A failed **Run** or **Spec run** on it leaves it open for the **Day shift** to take over; no later **Architect run** or **Pickup run** retries it.
_Avoid_: plan, plan doc

**Architect idea**:
The issue an **Architecture review** ends on when its top recommendation is not Strong: the one it filed, or the open issue that already covered it. thirdshift labels it `architect-idea` and `needs-triage`, the second put back on an issue that already covered the idea and had been triaged, since the factory again takes it for the best next move. While any Architect idea is open and still labelled `needs-triage`, every **Architect run** on the repository is skipped: the factory has run out of Strong ideas, and waits for the **Day shift** to triage one, whatever the decision.
_Avoid_: weak plan, suggestion

**Spec branch**:
The **Issue branch** of the **Spec** in a **Spec run**, branched off the **Base branch**. Each **Ticket**'s **Run** is a **Merge run** into it, so the Spec's work gathers there before it reaches the Base branch.
_Avoid_: integration branch, feature branch

**Spec PR**:
The pull request from the **Spec branch** into the **Base branch** that closes the **Spec**. It is left ready for review once every **Ticket** is done, or merged by a **Self-merge** when the **Spec run** was asked to merge; a draft listing what is missing otherwise. It opens as a draft once the first Ticket lands, with the **Tickets checklist** in its body.

**Tickets checklist**:
The part of the **Spec PR**'s body, between fixed markers, that thirdshift writes: one line per **Ticket**, ticked once it is done, with its pull request, or saying whether it is not started, running, failed, interrupted, blocked, in a cycle or unready. It is rewritten as each Ticket starts and ends, and put back after the **Spec review** rewrites the body.

**Unready Ticket**:
An open **Ticket** labelled with a triage role that says it is not agent work: `ready-for-human`, `needs-info`, `wontfix` or `needs-triage`. A **Spec run** never starts a **Run** for one, nor for a Ticket it blocks. An open Ticket with no triage label is taken. A Ticket with sub-issues of its own is treated as one too.

**Spec review**:
The agent session a **Spec run** starts once every **Ticket** is done: it reviews the whole **Spec branch** against the **Base branch** and the **Spec**, fixes what it agrees with, and writes the **Spec PR**'s description, listing the rest as **Unaddressed findings**.

**Failed spec run**:
A **Spec run** that ends, including by interruption, with any of its **Tickets** not done, or with its **Spec PR** not ready, mergeable and green (or, when asked to merge, not merged). It still takes every Ticket it can reach, and a failed Ticket stops only the Tickets it blocks.

**Merge run**:
A **Run** asked to end with its pull request merged rather than left for review, by the command it was started with or by the **User config**. It does everything a **Run** does, then a **Self-merge**. A **Ticket**'s Run in a **Spec run** is always a Merge run into the **Spec branch**; the command's ask applies to the **Spec PR**.
_Avoid_: auto-merge (GitHub's own feature, which thirdshift does not use)

**Run notification**:
A message thirdshift sends when a **Run**, a **Spec run** or an **Architect run** ends, whatever its outcome (ready, merged, failed or interrupted; for an Architect run that dispatched nothing, plan published, idea filed, idea already filed or review failed), to the address given with the email flag or the default in the **User config**. Each sends one only when asked to, by the flag or by the User config; a Spec run's notification lists each **Ticket**'s outcome, and a Ticket's **Run** never sends one of its own. After an **Inherited failure** it carries, beside the cause, what the Run says on stderr: where the checks fail on the **Base branch**, and the offer of a **Base fix** if nobody decided against one. An Architect run's notification tells how its **Architecture review** ended, naming the plan or idea issue, and how the Spec run or Run it dispatched ended, which sends none of its own; a skipped one sends none. A **Pickup run** that took a **Ready issue** sends the one the Spec run or Run it dispatched would have sent, which sends none of its own; a skipped one sends none. Failing to send one never changes the outcome.
_Avoid_: completion email, alert

**User config**:
The per-machine settings file in the user's home folder that sets thirdshift's defaults for every **Run** started on that machine, such as the **Run notification** address, whether every Run is a **Merge run**, whether a Run may start a **Base fix**, whether a Run first brings the **Launch directory**'s checkout of the **Base branch** up to date with `origin`, and the **Harness**, with a **Model** and **Effort** for each Harness. With no User config, or one that says nothing about a setting, a Run does only what its command asks for.

**Credentials**:
The per-machine secrets file next to the **User config**, readable only by its owner, that holds the Resend API key a **Run notification** is sent with. The key in the environment wins over it; it is what lets a **Run** started from cron, `nohup` or an agent's shell find the key. Only commands that send email read it, so the User config holds no secret.
_Avoid_: secrets file, key file

**Setup**:
Writing the **User config** by answering a few questions, one per setting that matters most, with every other setting written out at its default so the file shows everything that can be changed. With **Run notifications** on, it also asks for the Resend API key and writes the **Credentials** when one is entered. Offered by the first **Run** on a machine with no User config, and run again at any time to change the answers.
_Avoid_: init, onboarding, configure

**Self-merge**:
The step at the end of a **Merge run** in which thirdshift itself merges the pull request into the **Base branch** with a merge commit, once it is open, ready for review, mergeable and green on the head commit it merges. No human reviews it first. It ends with the **Issue branch** deleted, the issue closed by thirdshift, unless it is already closed, and the issue's **Claim** removed.

**Policy refusal**:
A merge the **Self-merge** tried that failed, where the round of the Repair loop that followed found nothing to fix: the **Base branch** unchanged, no conflict, CI green or absent, the pull request mergeable. The cause is a repository setting or rule, such as merge commits disallowed or a review required. thirdshift never reads GitHub's error text to decide it. The **Merge run** is a **Failed run** that leaves the pull request ready for review.

**Declined CI fix**:
A CI-fix **Repair** that found nothing on the branch to fix, for example because the failure is not the branch's but was not recognised as an **Inherited failure**, since the **Base branch** commit had no finished result for that check: once the Repair ends and the **Base branch** is merged in again, the head of the **Issue branch** is still the commit whose CI just failed. That head gets its **Check re-run**; if CI is red again, or there can be no Check re-run, the **Run** is a **Failed run** with the cause `CI red on <short sha> and the Repair found nothing to fix on the branch`, rather than spending its remaining Repairs.

**Check re-run**:
The one time per head commit that thirdshift asks GitHub to re-run the branch's own failed checks and watches CI on that head again: when a CI-fix **Repair** has left the head unchanged, and before the **Run** fails as a **Declined CI fix**. It lets a check that failed for a reason that does not repeat, such as a flaky test, pass. What the Repair concluded is never read. It needs every one of those checks to be a GitHub Actions job, starts no Repair and counts against no budget, and never re-runs a check for being an **Inherited failure**.
_Avoid_: retry, re-trigger, CI re-run

**Inherited failure**:
A failed check on the head of the **Issue branch** that also failed, under the same check name, on the **Base branch** commit that head last merged in. It is not the branch's to fix: no **Repair** is started for it, and a **Run** whose only red checks are Inherited failures is a **Failed run** with the cause `CI red on <check>, which also fails on <base> at <short sha>; fix <base> first`. A check with no finished result on that Base branch commit is not one. When asked to, by a flag or the **User config**, the Run starts a **Base fix** before failing. A Run that fails on one with no Base fix taken links each check where it fails on the Base branch, after its cause, and, if neither a flag nor the User config decided against a Base fix, offers one: the command that starts the Run again with one allowed, and the User config setting that allows one for every Run.
_Avoid_: base-red check, flaky check, shared failure

**Base fix**:
A **Merge run** into a **Run**'s **Base branch** that thirdshift starts when that Run meets an **Inherited failure**, on an issue thirdshift writes naming the failing checks, if it was asked to by a flag or the **User config**. The Run waits for it, then merges the new Base branch in and watches CI again; one Base fix per Run, and a Run that finds an open Base fix issue for the same failure waits on that one instead. A Base fix sees no Inherited failures, starts no Base fix of its own, and sends no **Run notification**: the Run that started it reports it, and is a **Failed run** naming its issue if the Base fix fails.
_Avoid_: hotfix, base repair (a **Repair** works on the Issue branch)

**Failed run**:
A **Run** that ends, including by interruption, without an open pull request from its **Issue branch** that targets the **Base branch**, is mergeable, and has passing CI. For a **Merge run**, it is also a Run that ends without its pull request merged. Its work is still pushed so nothing is lost (or, if the push fails, its worktree and local **Issue branch** are kept), and its open pull request, if any, is converted back to a draft, unless the pull request is ready, mergeable and green and only the **Self-merge** could not happen.

**Issue branch**:
A branch a **Run** works on for one issue: `issue-<n>` (the first), then `issue-<n>-branch-<k>` for k ≥ 2. A number counts as used once its pull request is merged or closed, even if the branch itself was deleted.

**Continuation**:
A **Run** that picks up an existing **Issue branch**, one with no pull request or an open one, instead of starting fresh. When a pull request is open, its base is the **Base branch**.

**Repair**:
A follow-up agent session a **Run** starts after the pull request exists, or a **Spec run** starts on its **Spec PR** after the **Spec review**: to resolve a merge conflict with the **Base branch** or the **Issue branch** on `origin`, to fix failing CI checks, or, in a **Merge run**, to review and fix **Foreign commits**.

**Delivery**:
What a **Run**, or a **Spec run** for its **Spec PR**, does from its worktree once its work begins. It runs the opening session (the implement session, or the **Spec review**), pushes, marks the pull request ready, keeps it mergeable and green through the **Repair loop**, and, in a **Merge run**, does the **Self-merge**. A Delivery that fails takes the **Failed run** path.

**Repair loop**:
The rounds a **Delivery** goes through until the pull request's head is mergeable with the **Base branch** and its CI is green or absent. Each round merges the Base branch in (and, in a **Merge run**, any **Foreign commits**), pushes, watches CI, and starts a **Repair** for a conflict or for the branch's own red checks. Repairs and upstream moves have fixed budgets.

**Foreign commit**:
A commit that appears on the **Issue branch** during a **Run** and was made neither by one of that Run's sessions nor by thirdshift itself. Commits already on the branch when a **Continuation** starts are not Foreign commits. A **Merge run** merges one only after a **Repair** has reviewed it.

**Resume**:
A continuation of the same agent session, started once when that session ended its turn with background work still running, which was killed with it: thirdshift takes it as work the session was waiting on. Its **Session prompt** asks the agent to re-run that work in the foreground and finish. A Resume that ends the same way gets no second Resume and does not fail the **Run**: the work may have been abandoned rather than awaited, so thirdshift names it in a progress line and carries on, and the steps that follow decide the outcome. If the Run is then a **Failed run**, its cause names the killed work ahead of the step that failed. A Resume is not a **Repair** and does not count against the Repair cap.

**Session log**:
The full transcript of one agent session, one per session, a **Resume** and each **Repair** included. Kept with its repository's other logs, named for the session's kind and stamped with the local time its command started, so all of a command's Session logs sort together.
_Avoid_: transcript, session file

**Command**:
One invocation of thirdshift that does factory work: a **Run** or **Spec run** started on an **Issue URL**, an **Architect run**, or a **Pickup run**. A child Run, a **Ticket**'s Run or a **Base fix**, is part of the command that started it, not one of its own.
_Avoid_: invocation, job

**Command log**:
Everything one **Run**, **Spec run**, **Architect run** or **Pickup run** printed, kept in a file of its own with its repository's other logs, stamped like its **Session logs**. An Architect run's or Pickup run's covers the Spec run or Run it dispatched, and a Spec run's covers its **Tickets**' Runs, as a Run's covers its **Base fix**: none of those keeps one of its own. It is kept once the command starts work: a Pickup run or Architect run skipped before doing any work keeps none, and neither does a Run that fails before it starts work, such as on **Origin match**.
_Avoid_: cron log, output log, run log (a **Run** is one kind of command)

**Activity log**:
One running record per repository of what the factory did there: a line when a **Run**, **Spec run**, **Architect run** or **Pickup run** starts work, naming its **Command log**, and a line when it ends, with its outcome. A skipped Architect run or Pickup run writes a line only when its reason differs from the last one recorded for its kind, so a repository that sits idle shows one line, not one per pass. It is what thirdshift writes about a scheduled pass, in place of output to the scheduler.
_Avoid_: cron log, pass log, history
