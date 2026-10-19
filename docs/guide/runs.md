# Runs and delivery

## Workflows

A [Run](#what-a-run-does), one issue in and one pull request out, is the factory's first workflow. Five more build on it:

- **Merge run**, `thirdshift merge <Issue URL>`: a Run that also merges its own reviewed, green pull request (see [Usage](commands.md#usage)).
- **[Spec run](#spec-runs)**, `thirdshift <Issue URL>` on an issue with sub-issues: works through its **Tickets** in dependency order, each merged into a **Spec branch**, then leaves one **Spec PR** ready for review.
- **[Architect run](passes.md#architect-runs)**, `thirdshift architect`: reviews the codebase's architecture, publishes a plan for its top Strong recommendation and builds it, or leaves an **Architect idea** for the **Day shift**.
- **[Pickup run](passes.md#pickup-runs)**, `thirdshift pickup`: takes the lowest-numbered **Ready issue** and runs it.
- **[Security run](passes.md#security-runs)**, `thirdshift secure`: audits the **Base branch** and records each **Security finding** privately. It is report-only by default; with fixing allowed, it fixes one finding it reproduced.

Architect, Pickup and Security runs are **Passes**: they take no Issue URL, so your operating system's scheduler can start them. See [A Pickup run on a schedule](passes.md#a-pickup-run-on-a-schedule), [Weeding](passes.md#weeding) for Architect runs, and [Fencing](passes.md#fencing) for Security runs.

Every session runs on the **Harness** you choose: Claude Code by default, or Codex, Antigravity CLI, Grok Build, Muse Code or OpenCode. See [Harness, Model and Effort](configuration.md#harness-model-and-effort).
## What a Run does

From a clone of the issue's repository, `thirdshift <Issue URL>`:

1. Checks that the Run makes sense, before creating anything:
   - the **Origin match**: the **Issue URL** must belong to the same GitHub repository as the `origin` remote of the directory you run it from;
   - the issue is open;
   - the **Base branch** is well defined: HEAD is not detached, the branch exists on `origin`, and your local copy is not ahead of it, so unpushed commits can't leak into the pull request;
   - a local Issue branch, if you have one, points at the same commit as its copy on `origin`, so local-only commits are never destroyed.

   Any failed check stops the Run before any work happens.
2. Picks the **Issue branch** (`issue-<n>`, or `issue-<n>-branch-<k>` once earlier ones are finished) and the **Base branch**, normally the branch you have checked out. Then it makes the [Claim](#the-claim): it labels the issue `in-progress`, in place of `ready-for-agent` if it has that.
3. Creates a git worktree next to your clone, named `<repo>-<Issue branch>`, so your own checkout is never touched.
4. Runs a headless agent session in that worktree on the **Harness**, Claude Code unless you [choose another](configuration.md#harness-model-and-effort), with the **Factory skills** in [`skills/`](../../skills/) linked into its `.claude/skills/`, or `.agents/skills/` on the other Harnesses, as `thirdshift-<skill>` and kept out of git by a `.git/info/exclude` entry ([ADR-0012](../../docs/adr/0012-factory-skills-linked-into-the-worktree-codex-unsandboxed.md)), started with one of the **Session prompts** in [`prompts/`](../../prompts/). The agent implements the issue, reviews its work against the Base branch, addresses the **Standards findings** and **Spec findings** it agrees with, and opens a ready-for-review pull request that lists every **Unaddressed finding** with a reason and closes the issue. With a [Security review](passes.md#security-review) asked for, a second session then reviews the change for vulnerabilities.
5. Takes over deterministically: pushes the Issue branch, skipping your repo's git hooks since the session runs the tests and CI gates the pull request, checks through `gh` that the pull request exists, is open and targets the Base branch, and marks it ready for review (`gh pr ready`) if the agent left it as a draft.
6. Keeps the pull request mergeable and green: merges the Base branch in and watches CI, starting a **Repair** session for a merge conflict or failing checks, at most 5 per Run. A failed check that also failed, under the same name, on the Base branch commit the head last merged in is an **Inherited failure**, not the branch's to fix: the CI-fix Repair is given only the branch's own failures, with the Inherited failures listed as not to fix. If every failed check is an Inherited failure, no Repair starts: the Run merges the Base branch again if it has moved since, and otherwise ends as a [Failed run](#failed-runs) that says to fix the Base branch first, unless it was given `base-fix` or the [User config](configuration.md#user-config) sets `base.fix`, when it starts a [Base fix](#base-fix) first. A check that is pending, passed or missing on that Base branch commit is the branch's own, and thirdshift never triggers or waits for the Base branch's CI. If a CI-fix Repair leaves the head unchanged, with no commit of its own and no Base branch move to merge, thirdshift does a **Check re-run**, once for that head: it asks GitHub to re-run the branch's own failed checks (`gh run rerun <run-id> --failed`, once per workflow run), says so on stderr as `re-running the failed checks on <short sha>: <check>[, <check>…]`, and watches CI on that head again, from the new attempt on. A check that failed for a reason that does not repeat, such as a flaky test or a runner fault, then passes, and the Run goes on as for any green head. If CI is red again, the Run ends as a Failed run, with no second Repair or re-run for that head. It ends the same way, with nothing re-run, if one of those checks can't be re-run (a commit status, or a check run that is not a GitHub Actions job), since the head could not go green. When GitHub refuses a re-run, stderr shows the refusal and thirdshift reads all checks on that head again: a Repair may already have re-run them. Green continues the Run, pending checks are watched, and the branch's own checks still red fail as before, without another Repair or request for that head. Checks that cannot be read or disappear after known failures cannot establish green CI. Any earlier successful workflow request still waits for its new attempt before the whole head is assessed. What the Repair concluded is never read, Inherited failures are never re-run on their own account, and a Check re-run is no Repair, so it counts against no budget ([ADR-0010](../../docs/adr/0010-one-check-re-run-before-a-declined-ci-fix.md)). If the Base branch moves while CI runs, it merges it again and goes round, at most 5 times per Run. In a Merge run, each round also takes in **Foreign commits** first: see [Foreign commits in a Merge run](#foreign-commits-in-a-merge-run).
7. In a **Merge run** (`thirdshift merge <Issue URL>`), does the **Self-merge**: once the pull request is open, ready for review, mergeable and green, thirdshift merges it into the Base branch with a merge commit, on exactly the head commit whose CI it watched (`gh pr merge --merge --match-head-commit <sha>`). Just before each attempt, it reads the PR's `reviewDecision`: `REVIEW_REQUIRED` or `CHANGES_REQUESTED` ends the Run as **review required**, a policy refusal, without attempting the merge or taking another Repair round. An empty or `APPROVED` decision, or a failed read, keeps the existing merge behavior. It never uses GitHub's auto-merge ([ADR-0004](../../docs/adr/0004-self-merge-by-thirdshift-not-github-auto-merge.md)). A merge that fails goes back round step 6, within the same budgets, and is tried again on the new green head. If that round finds nothing to fix, the refusal is a **policy refusal**, such as merge commits being disallowed or a review being required. After the merge, thirdshift deletes the Issue branch on `origin`, and closes the issue if it is still open, with the comment `Closed by #<pr>, merged into <base> by a thirdshift Merge run.` GitHub closes it on its own only for a merge into the repository's default branch, and then thirdshift leaves it alone. Once the issue is closed, either way, thirdshift takes `in-progress` off it, [removing the Claim](#when-the-claim-ends).
8. Cleans up: removes the worktree, the local Issue branch and the temporary directory the Factory skills were written out to, whether the Run succeeded or not. The one exception is a **Failed run** whose work could not be pushed: see below.

### The Claim

The **Claim** is the mark that the factory has taken an issue, so the issue list shows what it is working on. When a Run or a [Spec run](#spec-runs) starts on an issue, thirdshift labels the issue `in-progress`, in place of `ready-for-agent` if it has that. An issue with no `ready-for-agent` label still gets `in-progress`, so the label means the same thing however the Run was started.

- It is made once every check of step 1 has passed and the branches are picked, and before the worktree is created. A Run stopped by a check changes no label.
- It is one request, which adds `in-progress`, removes `ready-for-agent` and keeps the issue's other labels, so the swap can't stop halfway. stderr says `labelling #<n> in-progress, in place of ready-for-agent`, or `labelling #<n> in-progress`.
- thirdshift first creates the `in-progress` label if the repository has none.
- It is made however the Run or Spec run was started: by you on an Issue URL, by an [Architect run](passes.md#architect-runs) for the plan it dispatches, or by a [Pickup run](passes.md#pickup-runs) for the Ready issue it took.
- A Ticket's Run in a Spec run makes none, since the Spec carries the Claim, and nor does a [Base fix](#base-fix): their issues' labels are left as they are.
- A [Continuation](#continuation) on an issue that is already `in-progress` changes nothing and makes no request. One on an issue that isn't Claimed yet Claims it.
- If the Claim can't be made, the Run stops there, before any work, with exit `1` and the cause: `could not make the Claim on #<n>: <gh's error>`.

#### When the Claim ends

How the Run or the Spec run ends decides what becomes of its Claim:

- **Released**, except for a failed Security fix, when it fails with nothing on `origin` to take over: no Issue branch for the issue there and no pull request from one, open, merged or closed, or for a Spec run, no Spec branch and no Spec PR. That is a session that fails before it changes anything, as on a usage limit or an expired login, a Ctrl-C before any work, or a worktree that can't be created, on an issue that was never started before. A retry that fails that way on an issue an earlier Run left an Issue branch or a pull request for keeps its Claim. The issue's labels go back as they were before the Claim: `in-progress` comes off, and `ready-for-agent` goes back if the issue had it, and only then. So an outage costs a retry and doesn't strand the issue. stderr says `releasing the Claim on #<n>: labelling it ready-for-agent, in place of in-progress`, or `releasing the Claim on #<n>: removing in-progress`. A Failed run whose push failed is released too, since nothing of it reached `origin`: its work is only in the worktree it kept, which stops the next Run on that machine at the pre-flight checks until you push or delete the local Issue branch.
- **Kept**, on any other ending that leaves the issue open: a pull request ready for review, or a [Failed run](#failed-runs) that pushed its Issue branch or left a pull request. The issue stays `in-progress` until you relabel it, so a failure waits for you.
- **Removed**, in a Merge run, once the Self-merge has left the issue closed, whether thirdshift closed it or GitHub did: `in-progress` comes off, after `removing in-progress from issue #<n>` on stderr.

A release undoes only what the Claim changed, in one request that keeps the issue's other labels, any added during the Run included. An issue that was already `in-progress` when the Run started stays `in-progress`, with `ready-for-agent` back if the Claim took it off (`releasing the Claim on #<n>: labelling it ready-for-agent again`), and one whose `in-progress` someone took off during the Run is left as it is. A Claim that can't be released, or can't be removed after a merge, doesn't change how the Run ends: stderr gets a `warning:` line naming the command to run by hand.

A process killed outright, with `kill -9` or by a power cut, releases nothing: the issue stays `in-progress` for you to relabel.

An issue closed some other way, as when you merge its pull request by hand, keeps `in-progress` until a [Pickup run](passes.md#pickup-runs) takes it off, in the [Sweep](passes.md#the-sweep).

### Status

The designed behaviour described in this guide is implemented. Known gaps and planned work are tracked in the [open issues](https://github.com/JacobStephens2/thirdshift/issues).
## Foreign commits in a Merge run

A Merge run merges only code an agent wrote or reviewed. If someone else pushes to the Issue branch during the Run, their **Foreign commits** are reviewed before they can be merged:

1. Each round of step 6 starts by fetching the Issue branch from `origin`. Any new commits there are merged into the Run's branch, by fast-forward or a merge commit, never a rebase, each logged on stderr as `merging new commit <sha> from origin/<branch>`. Foreign commits that arrive while CI runs on a green head send the Run round again rather than on to the merge. A merge that conflicts with the Run's own work gets a conflict Repair.
2. A **review Repair** then runs `/thirdshift-code-review` with the head thirdshift last knew as the Run's own as the fixed point. The agent fixes the findings it agrees with, adds the rest to the pull request body's "Unaddressed findings" section marked as coming from the Foreign commits, and pushes.
3. The round goes on as usual: the Base branch is merged in, CI watched, and the merge tried once the head is green.

Review Repairs are logged as `repair-<i>`, numbered with the other Repairs, count against the cap of 5 Repairs, and get a Resume like any session. A round that picks up Foreign commits counts against the same 5, like a Base branch move. A Run that spends either budget fails.

A Run without `merge` does none of this: it never fetches someone else's commits into its branch.
## Base fix

A Base branch whose CI is broken fails every Run on it with an Inherited failure. To have a Run fix the Base branch itself and carry on, give it `base-fix` (or `--base-fix`), before or after the URL:

```sh
thirdshift base-fix https://github.com/acme/widgets/issues/7
```

To allow it for every Run on a machine, set `base.fix` in the [User config](configuration.md#user-config); `no-base-fix` (or `--no-base-fix`), before or after the URL, then forbids it for one Run. Giving either flag twice, or `base-fix` together with `no-base-fix`, is an argument error.

When the Run's only red checks are Inherited failures and the Base branch has not moved since, a Run allowed to then starts a **Base fix**, at most one per Run ([ADR-0008](../../docs/adr/0008-inherited-failures-fail-the-run.md)):

1. thirdshift looks for an open issue labelled `base-fix`, other than the Run's own, whose title names the same Base branch and every one of those checks. If there is one, another Run's Base fix is already under way, and the Run waits on that one instead: see [One Base fix per Base branch](#one-base-fix-per-base-branch).
2. Otherwise, thirdshift opens an issue from a fixed template, with no agent session: titled `CI red on <base>: <check>[, <check>…]`, its body naming the checks with their URLs on the Base branch, the Base branch and its short sha, and the Run's pull request, labelled `base-fix` and `ready-for-agent`. A label the repository lacks is added to it first.
3. It starts a child `thirdshift` on that issue from the same Launch directory, as a Merge run into the Run's Base branch, whatever the Run's own goal. Its progress lines are relayed with a `#<n>: ` prefix, after `starting Base fix #<n> into <base>: <issue URL>` and `waiting on Base fix #<n>`. The Base fix sends no Run notification, makes no [Claim](#the-claim) on its issue, treats every red check as its own to fix rather than as an Inherited failure, and never starts a Base fix of its own.
4. Once the Base fix has merged, and its Self-merge has closed its issue, the Run merges the Base branch in again and watches CI, and goes on as usual: ready for review, or merged in a Merge run.

If the Base fix fails, the Run is a [Failed run](#failed-runs) with the cause `Base fix <issue URL> failed: <its cause>`. If the checks are still Inherited failures once the Base fix has merged, there is no second one, and the cause is `CI red on <check>, which also fails on <base> at <short sha>, even after Base fix <issue URL> merged; fix <base> first`. The Run's last progress lines include `Base fix: <issue URL> merged`, or `not merged`, and its own [Run notification](configuration.md#email) has the same `Base fix:` line.

On a Spec, `base-fix`, `no-base-fix` or the User config's `base.fix` holds for each Ticket's Run, whose Base fix goes into the Spec branch, its Base branch, and for the Spec PR, whose Base fix goes into the Base branch. Tickets running at once that meet the same Inherited failure share one Base fix: the first starts it, and the others wait on it. A Ticket's Run sends no Run notification, so the Spec run reports its Base fix: a Ticket that landed after one has `#<n> landed with <PR URL>, after Base fix <issue URL> merged`, or `closed` for one it waited on, as its line in the Tickets checklist and in the Spec run's Run notification, and one whose Base fix failed has the cause naming the issue.

### One Base fix per Base branch

A Run that finds an open Base fix issue for its Base branch and checks starts none of its own. After `waiting on Base fix #<n>, already open: <issue URL>`, it waits for that issue to close, as the Base fix's Self-merge leaves it, then merges the Base branch in again and watches CI. Waiting is the Run's one Base fix: if the checks are still Inherited failures once the issue has closed, the cause is `CI red on <check>, which also fails on <base> at <short sha>, even after Base fix <issue URL> closed; fix <base> first`, and its Run notification's `Base fix:` line says `closed`, or `not closed` if the Run ended first.

An issue that covers more checks than the Run's counts; one for another Base branch, or that leaves out one of the Run's checks, does not. The wait has no time limit: a Base fix issue from another clone or machine that nobody is working on has to be closed, or the Run interrupted, by hand.

Runs from one Launch directory, as a Spec run's Tickets are, look for the issue and write it one at a time, under a lock in the clone's git directory, so those that meet the same Inherited failure at once get one Base fix issue and one Base fix. There, a Run also knows whether a Base fix started from that Launch directory is still running:

- A Run waiting on one that fails, leaving its issue open, fails too, with the cause `Base fix <issue URL> ended with its issue still open`.
- A Run that finds the issue open after that Base fix has ended starts it again on the same issue, as its one Base fix, after `Base fix #<n> is open but no longer running; starting it again into <base>: <issue URL>`. It continues the Base fix's Issue branch, as any Run on an issue with one does: see [Continuation](#continuation).

Across clones and machines the look is only a best-effort lock, and two Runs that look at the same moment may each start a Base fix.

Without `base-fix` or `base.fix`, or with `no-base-fix`, an Inherited failure fails the Run as described in [What a Run does](#what-a-run-does), and the Run says [where the checks fail on the Base branch](#an-inherited-failure-links-the-base-branchs-checks), offering a Base fix if nobody decided against one.
## Continuation

Running thirdshift again on an issue picks up where the last Run stopped ([ADR-0002](../../docs/adr/0002-existing-issue-branch-means-continue.md)). It looks at the highest-numbered Issue branch:

- **No Issue branch yet**: a fresh Run creates `issue-<n>` from the Base branch.
- **The branch exists with no pull request, or an open one**: the Run is a **Continuation**. It checks out that branch, and the agent builds on its commits instead of starting over, then creates the pull request or updates the open one. With an open pull request, that pull request's base is the Base branch, whatever you have checked out.
- **The branch's pull request was merged or closed**: finished work is never reopened. The Run starts fresh on the next number, `issue-<n>-branch-2`, `-3`, and so on. A number counts as used even if GitHub deleted the branch after merging.

So you can retry a Failed run with the same command, or start an issue on one server and continue it on another.

A Run is not idempotent: re-running builds on whatever is already on the branch, including a Failed run's work-in-progress commit.
## Failed runs

A **Failed run** is one that ends, including by Ctrl-C or a closed terminal, without an open pull request from its Issue branch that targets the Base branch, is mergeable and has passing CI, or, for a Merge run, without that pull request merged. Causes include the session exiting non-zero, no pull request or one with the wrong base, running out of Repairs, CI red only on **Inherited failures** (`CI red on <check>[, <check>…], which also fails on <base> at <short sha>; fix <base> first`), a CI-fix Repair that finds nothing on the branch to fix (a **declined CI fix**: the head is unchanged after it, and its one Check re-run left CI red or could not happen), and a Base branch that keeps moving while CI runs, or in a Merge run, an Issue branch that keeps getting Foreign commits.

A Failed run:

1. Commits any uncommitted work as `thirdshift: failed run (<reason>)`, with a timestamp and the hostname, and pushes the Issue branch, so nothing is lost. If the branch has no changes against the Base branch, nothing is pushed.
2. Converts its open pull request, if any, back to a draft, so a pull request only claims to be ready when the factory stands behind it. The next successful Continuation marks it ready again. The exception is a Merge run's policy refusal: the pull request is ready, mergeable and green and only the Self-merge could not happen, so it stays ready for review, and no failure commit is pushed onto the head whose CI was watched. This includes **review required**: a review blocks the merge, so thirdshift skips the merge attempt and another Repair round. stderr and the Run notification say `PR <url> needs an approving review before it can merge; it is ready for review`, with `review required` as the notification outcome. The Run exits `1`, prints the PR URL on stdout, keeps its Claim and cleans up as usual. The same applies to a Spec run's Spec PR.
3. Cleans up as usual, prints the reason to stderr and exits non-zero. If the push failed, the worktree and local Issue branch are kept instead, and stderr names the branch, its head commit and the worktree path, so you can recover the work or push it by hand.
4. Releases its [Claim](#when-the-claim-ends), unless it is a failed Security fix, if it left nothing on `origin`: no Issue branch and no pull request. Otherwise the issue stays `in-progress`.

Merges, never rebases or force-pushes: a branch worked on from several servers never loses history.

### An Inherited failure links the Base branch's checks

A Run that fails on Inherited failures with no Base fix taken says more after its cause, on stderr and in its [Run notification](configuration.md#email): a `Base check:` line for each of those checks, with its URL on the Base branch commit, as a [Base fix](#base-fix) issue lists them, or the name alone for a check with no URL. If nobody decided against a Base fix, with neither `base-fix` nor `no-base-fix` given and no `base.fix = true` in the [User config](configuration.md#user-config), it also offers one: a `Retry with:` line with the command that starts the Run again with `base-fix` added, keeping the `merge`, `email` and `parallel` flags it was given, and an `Or set:` line naming `base.fix`:

```
thirdshift: 03:12:40 CI red on test, which also fails on main at 362b9ca; fix main first
thirdshift: 03:12:40 Base check: test: https://github.com/acme/widgets/actions/runs/1/job/2
thirdshift: 03:12:40 Retry with: thirdshift https://github.com/acme/widgets/issues/7 base-fix
thirdshift: 03:12:40 Or set: base.fix = true in ~/.thirdshift/config.toml, to allow a Base fix for every Run on this machine
thirdshift: 03:12:40 session log: ~/.thirdshift/logs/acme/widgets/commands/issue/7-….jsonl
thirdshift: 03:12:40 command log: ~/.thirdshift/logs/acme/widgets/commands/issue/7-….log
```

A Run given `no-base-fix` gets the `Base check:` lines and no offer. A Run that had its one Base fix gets neither: its `Base fix:` line already says what happened. The cause itself, and so the failure commit's message, is the same in every case, and no other cause adds a line.

In a [Spec run](#spec-runs), a Ticket's Run says this on its own stderr, relayed with its `#<n>: ` prefix, and the command it offers is the Spec run's, with `base-fix` added. The Spec run's summary and its Run notification show the Ticket's cause alone. After an [Architect run](passes.md#architect-runs), the command offered is `thirdshift <plan URL>` with the dispatched run's flags and `base-fix`, since another Architect run would start a new review rather than take that plan up again.
## Spec runs

A **Spec** is an issue with sub-issues, its **Tickets**. `thirdshift <Issue URL>` on a Spec is a **Spec run**: it works through the Tickets in the order their GitHub "blocked by" links allow, each Ticket's **Run** a **Merge run** into the **Spec branch**, then leaves one **Spec PR** from the Spec branch into the Base branch ready for review ([ADR-0006](../../docs/adr/0006-spec-runs-merge-tickets-into-a-spec-branch.md)). An issue with no sub-issues is an ordinary Run.

A Spec run makes the [Claim](#the-claim) on the Spec, as a Run does on its issue: the Spec is labelled `in-progress`, in place of `ready-for-agent`. Its Tickets' Runs change no label on their Tickets. The Claim [ends](#when-the-claim-ends) as a Run's does: it is released if the Spec run fails with no Spec branch on `origin` and no Spec PR, except for a failed Security fix, removed once a Self-merge of the Spec PR has left the Spec closed, and kept otherwise. The Spec branch is pushed before the first Ticket starts, so a Spec run that got as far as its Tickets keeps its Claim.

The Spec PR opens as a draft as soon as the first Ticket lands, titled from the Spec, with `Closes #<spec>` and a Tickets checklist: one line per Ticket, ticked once it is done, with its pull request, or saying it is running, failed, blocked or unready. thirdshift rewrites the checklist between its `<!-- thirdshift:tickets -->` markers as each Ticket starts and ends, leaving the rest of the body as it is. Once every Ticket is done, the **Spec review** rewrites the body, and thirdshift puts the checklist back, appending it if the markers are gone, before marking the Spec PR ready.

The Spec review session reviews the whole Spec branch against the Base branch and the Spec. Once it is marked ready, the Spec PR goes through the same step 6 as a Run's pull request, with the Spec as the issue and the Spec branch as the Issue branch: the Base branch is merged in (never rebased), CI watched, and a conflict or red CI handed to a Repair, within the same budgets, with the same Check re-run after a CI-fix Repair that leaves the head unchanged. Inherited failures are told apart the same way everywhere: the Spec PR's against the Base branch, and a Ticket's against the Spec branch, its Base branch, with the cause shown on the Ticket's line of the checklist.

Tickets always merge into the Spec branch, whatever the command or the User config says. `merge` (or `--merge`, or `merge.always = true` without `no-merge`) applies to the Spec PR alone: the Spec run ends with the Self-merge of the Spec PR into the Base branch, then deletes the Spec branch on `origin` and closes the Spec if the merge did not. Without it, the Spec run ends with the Spec PR ready for review. A Spec PR that fails from the Spec review on follows the [Failed run](#failed-runs) rules for its pull request, back to draft unless only the Self-merge could not happen (a policy refusal), and the Spec run exits `1`.

A Spec run takes every Ticket it can reach. A Ticket runs once it is open, has every blocker closed, and is not an **Unready Ticket**: an open Ticket labelled `ready-for-human`, `needs-info`, `wontfix` or `needs-triage`. An open Ticket with no triage label is taken. A Ticket with sub-issues of its own is never run either, and is reported as unready. A blocker outside the Spec counts once it is closed. The graph is read again from GitHub whenever a Ticket's Run ends, so removing a label, adding a Ticket or closing one by hand takes effect in the same Spec run.

A Ticket whose Run fails is not tried again in that Spec run, and stops only the Tickets it blocks; every other Ticket it can reach still runs. Nor do Unready Tickets, Tickets blocked by an open issue outside the Spec, or Tickets in a cycle of "blocked by" links run, nor any Ticket downstream of them.

Independent Tickets run at once, up to 3 by default. Whenever a Ticket's Run ends, the Spec run reads the graph from GitHub again and starts ready Tickets until the limit is reached. To change the limit for one Spec run, add `parallel <n>` (or `--parallel <n>`) before or after the URL; `spec.parallel` in the [User config](configuration.md#user-config) sets it for every Spec run on the machine:

```sh
thirdshift parallel 5 https://github.com/acme/widgets/issues/20   # up to 5 Tickets at once
thirdshift --parallel 1 https://github.com/acme/widgets/issues/20 # one at a time
```

`parallel` followed by `0`, a negative number or anything but a whole number, or given twice, is an argument error (exit `2`). `parallel` on an issue with no sub-issues stops the Run before any work, since there are no Tickets to run at once. The Tickets' Runs share the Launch directory: they create their worktrees there one at a time, and a git command that finds a lock file held by another waits and tries again.

When nothing is left to run and any Ticket is not done, the Spec run is a **Failed spec run**: it leaves the Spec PR a draft, its checklist showing what's missing, prints its URL on stdout (if any Ticket has landed, so there is one), exits `1`, and lists on stderr each Ticket that landed, with its pull request, and each one not done, with why:

```
thirdshift: 03:12:40 #21 failed: claude exited 1 (session log: ~/.thirdshift/logs/acme/widgets/commands/issue/21-….jsonl)
thirdshift: 03:12:40 #22 blocked by #21
thirdshift: 03:12:40 #23 landed with https://github.com/acme/widgets/pull/1
thirdshift: 03:12:40 #24 unready: labelled needs-info
thirdshift: 03:12:40 #25 blocked by #99 (outside the Spec)
thirdshift: 03:12:40 #26 in a cycle: #26 blocked by #27 blocked by #26
thirdshift: 03:12:40 #27 in a cycle: #27 blocked by #26 blocked by #27
thirdshift: 03:12:40 Tickets not done: #21, #22, #24, #25, #26, #27
```

Running the Spec again picks up where the last Spec run stopped. The Spec branch is on `origin`, so the Spec run continues it ([ADR-0002](../../docs/adr/0002-existing-issue-branch-means-continue.md)) and updates its draft Spec PR rather than opening another. Closed Tickets are done and not run again, and a Ticket that failed still has its Issue branch and draft pull request, so its Run is a [Continuation](#continuation) into the Spec branch. With every Ticket closed, the Spec run goes straight to the Spec review and the Spec PR. With every Ticket closed and no Spec branch, as when the Spec was done some other way, it stops with `every Ticket is closed and there is no Spec branch; nothing to do` and exits `1`, creating no branch or pull request and leaving the Spec open.
