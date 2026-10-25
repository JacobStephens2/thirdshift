# thirdshift

Read this manual [on thirdshift.app](https://thirdshift.app/docs/) or return to the [brief README](../README.md).

A factory that turns a GitHub issue into a ready-for-review pull request, or on request a merged one, by running unattended agent sessions against it. The humans are the day shift; the agents work the third shift, overnight, on any server, while you do something else.

Quality over quantity: a pull request marked ready for review is one the factory stands behind. When it can't stand behind the work, the **Ticket run** is a **Failed ticket run**: the work is pushed so nothing is lost, and the pull request goes back to a draft.

Terms in **bold** are defined in [`GLOSSARY.md`](../GLOSSARY.md).

## Workflows

A [Ticket run](#what-a-ticket-run-does), one issue in and one pull request out, is the factory's first workflow. It can leave its pull request for review or Self-merge it when asked with `thirdshift merge <Issue URL>` (see [Usage](#usage)). Four more workflows build on it:

- **[Spec run](#spec-runs)**, `thirdshift <Issue URL>` on an issue with sub-issues: works through its **Tickets** in dependency order, each merged into a **Spec branch**, then leaves one **Spec PR** ready for review.
- **[Architect run](#architect-runs)**, `thirdshift architect`: reviews the codebase's architecture, publishes a plan for its top Strong recommendation and builds it, or leaves an **Architect idea** for the **Day shift**.
- **[Pickup run](#pickup-runs)**, `thirdshift pickup`: takes the lowest-numbered **Ready issue** by default, the highest-numbered with `pickup.order = "highest-first"` or `highest-first` on the command, and runs it.
- **[Security run](#security-runs)**, `thirdshift secure`: audits the **Base branch** and records each **Security finding** privately. It is report-only by default; with fixing allowed, it fixes one finding it reproduced.

Architect, Pickup and Security runs are **Passes**: they take no Issue URL, so your operating system's scheduler can start them. See [A Pickup run on a schedule](#a-pickup-run-on-a-schedule), [Architect runs every few minutes](#architect-runs-every-few-minutes) for Architect runs, and [Security runs on a schedule, with fixing allowed](#security-runs-on-a-schedule-with-fixing-allowed) for Security runs.

Every session runs on the **Harness** you choose: Claude Code by default, or Codex, Antigravity CLI, Grok Build, Muse Code or OpenCode. See [Harness, Model and Effort](#harness-model-and-effort).

## What a Ticket run does

<a name="what-a-run-does" id="what-a-run-does"></a>

From a clone of the issue's repository, `thirdshift <Issue URL>`:

1. Checks that the Ticket run makes sense, before creating anything:
   - the **Origin match**: the **Issue URL** must belong to the same GitHub repository as the `origin` remote of the directory you run it from, and a mismatch says to start it from a clone of the issue's repository instead;
   - the issue is open;
   - the **Base branch** is well defined: HEAD is not detached, the branch exists on `origin`, and your local copy is not ahead of it, so unpushed commits can't leak into the pull request;
   - a local Issue branch, if you have one, points at the same commit as its copy on `origin`, so local-only commits are never destroyed.

   Any failed check stops the Ticket run before any work happens.
2. Picks the **Issue branch** (`issue-<n>`, or `issue-<n>-branch-<k>` once earlier ones are finished) and the **Base branch**, normally the branch you have checked out. Then it makes the [Claim](#the-claim): it labels the issue `in-progress`, in place of `ready-for-agent` if it has that.
3. Creates a git worktree next to your clone, named `<repo>-<Issue branch>`, so your own checkout is never touched.
4. Runs a headless agent session in that worktree on the **Harness**, Claude Code unless you [choose another](#harness-model-and-effort), with the **Factory skills** in [`skills/`](../skills) linked into its `.claude/skills/`, or `.agents/skills/` on the other Harnesses, as `thirdshift-<skill>` and kept out of git by a `.git/info/exclude` entry ([ADR-0012](adr/0012-factory-skills-linked-into-the-worktree-codex-unsandboxed.md)), started with one of the **Session prompts** in [`prompts/`](../prompts). The agent implements the issue, reviews its work against the Base branch, addresses the **Standards findings** and **Spec findings** it agrees with, and opens a ready-for-review pull request that lists every **Unaddressed finding** with a reason and closes the issue. With a [Security review](#security-review) asked for, a second session then reviews the change for vulnerabilities.
5. Takes over deterministically: pushes the Issue branch, skipping your repo's git hooks since the session runs the tests and CI gates the pull request, checks through `gh` that the pull request exists, is open and targets the Base branch, and marks it ready for review (`gh pr ready`) if the agent left it as a draft.
6. Keeps the pull request mergeable and green: merges the Base branch in and watches CI, starting a **Repair** session for a merge conflict or failing checks, at most 5 per Ticket run. A failed check that also failed, under the same name, on the Base branch commit the head last merged in is an **Inherited failure**, not the branch's to fix: the CI-fix Repair is given only the branch's own failures, with the Inherited failures listed as not to fix. If every failed check is an Inherited failure, no Repair starts: the Ticket run merges the Base branch again if it has moved since, and otherwise ends as a [Failed ticket run](#failed-ticket-runs) that says to fix the Base branch first, unless it was given `base-fix` or the [User config](#user-config) sets `base.fix`, when it starts a [Base fix](#base-fix) first. A check that is pending, passed or missing on that Base branch commit is the branch's own, and thirdshift never triggers or waits for the Base branch's CI. If a CI-fix Repair leaves the head unchanged, with no commit of its own and no Base branch move to merge, thirdshift does a **Check re-run**, once for that head: it asks GitHub to re-run the branch's own failed checks (`gh run rerun <run-id> --failed`, once per workflow run), says so on stderr as `re-running the failed checks on <short sha>: <check>[, <check>…]`, and watches CI on that head again, from the new attempt on. A check that failed for a reason that does not repeat, such as a flaky test or a runner fault, then passes, and the Ticket run goes on as for any green head. If CI is red again, the Ticket run ends as a Failed ticket run, with no second Repair or re-run for that head. It ends the same way, with nothing re-run, if one of those checks can't be re-run (a commit status, or a check run that is not a GitHub Actions job), since the head could not go green. When GitHub refuses a re-run, stderr shows the refusal and thirdshift reads all checks on that head again: a Repair may already have re-run them. Green continues the Ticket run, pending checks are watched, and the branch's own checks still red fail as before, without another Repair or request for that head. Checks that cannot be read or disappear after known failures cannot establish green CI. Any earlier successful workflow request still waits for its new attempt before the whole head is assessed. What the Repair concluded is never read, Inherited failures are never re-run on their own account, and a Check re-run is no Repair, so it counts against no budget ([ADR-0010](adr/0010-one-check-re-run-before-a-declined-ci-fix.md)). Before any Repair, a head whose own failed checks are all **Abandoned jobs**, GitHub Actions jobs GitHub ended as failed with no failed step and a step not completed, as when a runner is lost, has them re-run once, saying `<check> lost its runner on <short sha>; re-running it`, and CI watched again the same way: green goes on, and red again, or a refused or unseen re-run, gets the CI-fix Repair as before. This too is no Repair and counts against no budget ([ADR-0020](adr/0020-an-abandoned-job-is-re-run-before-any-repair.md)). If the Base branch moves while CI runs, it merges it again and goes round, at most 5 times per Ticket run. In a Ticket run that Self-merges, each round also takes in **Foreign commits** first: see [Foreign commits in a Ticket run that Self-merges](#foreign-commits-in-a-ticket-run-that-self-merges).
7. In a **Ticket run** that Self-merges (`thirdshift merge <Issue URL>`), does the **Self-merge**: once the pull request is open, ready for review, mergeable and green, thirdshift merges it into the Base branch with a merge commit, on exactly the head commit whose CI it watched (`gh pr merge --merge --match-head-commit <sha>`). Just before each attempt, it reads the PR's `reviewDecision`: `REVIEW_REQUIRED` or `CHANGES_REQUESTED` ends the Ticket run as **review required**, a policy refusal, without attempting the merge or taking another Repair round. An empty or `APPROVED` decision, or a failed read, keeps the existing merge behavior. It never uses GitHub's auto-merge ([ADR-0004](adr/0004-self-merge-by-thirdshift-not-github-auto-merge.md)). A merge that fails goes back round step 6, within the same budgets, and is tried again on the new green head. If that round finds nothing to fix, the refusal is a **policy refusal**, such as merge commits being disallowed or a review being required. After the merge, thirdshift deletes the Issue branch on `origin`, and closes the issue if it is still open, with the comment `Closed by #<pr>, merged into <base> by a thirdshift Ticket run with Self-merge.` GitHub closes it on its own only for a merge into the repository's default branch, and then thirdshift leaves it alone. Once the issue is closed, either way, thirdshift takes `in-progress` off it, [removing the Claim](#when-the-claim-ends).
8. Cleans up: removes the worktree, the local Issue branch and the temporary directory the Factory skills were written out to, whether the Ticket run succeeded or not. The one exception is a **Failed ticket run** whose work could not be pushed: see below.

### Worktree ownership and cleanup

A factory-owned checkout, on an **Issue branch** for a **Ticket run** or detached for an **Architecture review**.

Acquisition accounts for partial local effects: failed-acquisition recovery rechecks clean-only, instance-specific authority before and after every destructive attempt, including retries, advances only on verified transitions, and retains and names remaining work or uncertain partial effects. Failed-acquisition retention survives later Architecture review attempts.

Successful acquisition carries checkout and registration identity through its lifetime; ordinary synchronization, observations and Failed ticket run preservation share acquired-instance checks before and after every Git subprocess attempt, including retries, acting only on that acquired checkout and selected Issue branch and refusing changed or uncertain ownership. Failed ticket run preservation retains work when ownership has changed or is uncertain and finishes after Command interruption without clearing it; ordinary synchronization remains interruptible. Confirmed Self-merge branch deletion uses the captured Launch repository independently of checkout validity.

Cleanup checks stage-specific disposal authority before and after every destructive Git attempt, including retries: acquired identity and the captured cleanup head for checkout removal, removed-path absence and an unused expected head for local branch removal, then ref absence and the exact original local subsection for branch configuration removal. Only successful, verified transitions advance disposal; partial failures retain and name every remaining or uncertain resource. Live cleanup finishes after Command interruption without clearing it; stale disposal stays interruptible.

Stale Architecture review scratch is disposable only with evidence of successful detached acquisition and unchanged ownership; unmarked or uncertain checkouts are retained.

See [ADR-0014](adr/0014-successful-review-acquisition-authorizes-stale-disposal.md) for the ownership evidence that authorizes stale review disposal.

### Delivery and pull request identity

What a **Ticket run**, or a **Spec run** for its **Spec PR**, does from its worktree once its work begins. It runs the opening session (the implement session, or the **Spec review**), pushes, marks the pull request ready, keeps it mergeable and green through the **Repair loop**, and, in a **Ticket run** that Self-merges, does the **Self-merge**.

For a Spec PR, Delivery ends Ticket accounting's capture before the Spec review, allowing that session to replace the PR. The first successful observation after the opening session, or during Failed ticket run completion after an opening failure, captures one pull request number and URL in the issue's repository; all later observations and transitions use that identity. Readiness and every mergeability poll validate its expected **Issue branch** and **Base branch**, open state and readiness where required. A missing, closed or retargeted captured PR fails Delivery; a replacement from the same branch is never substituted.

A Delivery that fails takes the **Failed ticket run** path, drafting that same captured open PR after preserving work. Spec PR checklist restoration uses that Delivery capture before readiness and for final accounting after failure.

If a Delivery gate observes that its pull request has already merged into the expected Base branch, and its merged head includes all local work (the exact local head, with no uncommitted changes), the Ticket run ends as merged, even when it originally asked only for ready for review. Delivery stops further publication, PR edits and Self-merge work. A merged PR that leaves local work out still fails and retains that work; a merged state alone never establishes completion.

### Check re-run allowance

A **Check re-run** is allowed once per full head commit across one **Delivery**. The allowance is consumed before the request and stays consumed through green CI, **Inherited failures**, a **Base fix**, upstream moves, later Repair rounds and refused **Self-merge** reconciliation, even after visiting another head and returning.

Later watches still obtain fresh CI facts. If the branch's own checks are red on a head whose allowance has been consumed, the Ticket run ends as a **Declined CI fix** without another CI-fix Repair or Check re-run. A different head has its own allowance, and a separately started **Continuation** begins a new Delivery with fresh allowances. Inherited-only failures keep their usual handling.

The trigger, eligible GitHub Actions checks, refusal handling and CI watching are described in step 6 above and [ADR-0010](adr/0010-one-check-re-run-before-a-declined-ci-fix.md). A Check re-run starts no Repair, counts against no Repair or upstream-move budget, and never re-runs a check for being an Inherited failure.

### The Claim

The **Claim** is the mark that the factory has taken an issue, so the issue list shows what it is working on. When a Ticket run or a [Spec run](#spec-runs) starts on an issue, thirdshift labels the issue `in-progress`, in place of `ready-for-agent` if it has that. An issue with no `ready-for-agent` label still gets `in-progress`, so the label means the same thing however the Ticket run was started.

- It is made once every check of step 1 has passed and the branches are picked, and before the worktree is created. A Ticket run stopped by a check changes no label.
- It is one request, which adds `in-progress`, removes `ready-for-agent` and keeps the issue's other labels, so the swap can't stop halfway. stderr says `labelling #<n> in-progress, in place of ready-for-agent`, or `labelling #<n> in-progress`.
- thirdshift first creates the `in-progress` label if the repository has none.
- It is made however the Ticket run or Spec run was started: by you on an Issue URL, by an [Architect run](#architect-runs) for the plan it dispatches, or by a [Pickup run](#pickup-runs) for the Ready issue it took.
- A Ticket run in a Spec run makes none, since the Spec carries the Claim, and nor does a [Base fix](#base-fix): their issues' labels are left as they are.
- A [Continuation](#continuation) on an issue that is already `in-progress` changes nothing and makes no request. One on an issue that isn't Claimed yet Claims it.
- If the Claim can't be made, the Ticket run stops there, before any work, with exit `1` and the cause: `could not make the Claim on #<n>: <gh's error>`.

#### When the Claim ends

How the Ticket run or the Spec run ends decides what becomes of its Claim:

- **Released**, except for a failed Security fix, when it fails with nothing on `origin` to take over: no Issue branch for the issue there and no pull request from one, open, merged or closed, or for a Spec run, no Spec branch and no Spec PR. That is a session that fails before it changes anything, as on a usage limit or an expired login, a Ctrl-C before any work, or a worktree that can't be created, on an issue that was never started before. A retry that fails that way on an issue an earlier Ticket run left an Issue branch or a pull request for keeps its Claim. The issue's labels go back as they were before the Claim: `in-progress` comes off, and `ready-for-agent` goes back if the issue had it, and only then. So an outage costs a retry and doesn't strand the issue. stderr says `releasing the Claim on #<n>: labelling it ready-for-agent, in place of in-progress`, or `releasing the Claim on #<n>: removing in-progress`. A Failed ticket run whose push failed is released too, since nothing of it reached `origin`: its work is only in the worktree it kept, which stops the next Ticket run on that machine at the pre-flight checks until you push or delete the local Issue branch.
- **Kept**, on any other ending that leaves the issue open: a pull request ready for review, or a [Failed ticket run](#failed-ticket-runs) that pushed its Issue branch or left a pull request. The issue stays `in-progress` until you relabel it, so a failure waits for you.
- **Removed**, in a Ticket run that Self-merges, once the Self-merge has left the issue closed, whether thirdshift closed it or GitHub did: `in-progress` comes off, after `removing in-progress from issue #<n>` on stderr.

A release undoes only what the Claim changed, in one request that keeps the issue's other labels, any added during the Ticket run included. An issue that was already `in-progress` when the Ticket run started stays `in-progress`, with `ready-for-agent` back if the Claim took it off (`releasing the Claim on #<n>: labelling it ready-for-agent again`), and one whose `in-progress` someone took off during the Ticket run is left as it is. A Claim that can't be released, or can't be removed after a merge, doesn't change how the Ticket run ends: stderr gets a `warning:` line naming the command to run by hand.

A process killed outright, with `kill -9` or by a power cut, releases nothing: the issue stays `in-progress` for you to relabel.

An issue closed some other way, as when you merge its pull request by hand, keeps `in-progress` until a [Pickup run](#pickup-runs) takes it off, in the [Sweep](#the-sweep).

### Status

The designed behaviour described in this manual is implemented. Known gaps and planned work are tracked in the [open issues](https://github.com/JacobStephens2/thirdshift/issues).

## Machine status

`thirdshift status` names this host, counts its active runs, such as `2 active runs`, and shows its **Inbox** across every repository in `logs.dir`. The count takes each Command with live work anywhere in it once, whatever its group: a Spec run with two parallel Tickets is one, and a Pass with the Spec run or Ticket run it dispatched, their Tickets and any Base fix, is one. A live child still counts when its outer record has ended or died; ended, died, ready and skipped work counts none. The Inbox groups come in this order:

- **Needs you:** live Silent Ticket runs, then Slow Ticket runs, then ended problems newest first: Died, Failed, Triage (a Stop without requeue), or a Security finding waiting on a decision. An ended problem leaves when a later Command on the same issue starts on this machine, or when GitHub shows its issue closed or its pull request merged. A Triage row also leaves once its issue loses `needs-triage`; Resolved counts from the first read showing the label gone. Architect and Security runs use the issue and pull request of the Spec run or Ticket run they dispatched. A failed Ticket run whose pull request closed unmerged stays while its issue is open. Live Silent and Slow rows stay until their Commands end, whatever GitHub shows. A row GitHub can speak for no longer ages out; rows with no issue or pull request, no Command record, no read yet or nothing GitHub can find still age out after `inbox.days` (7 by default), straight to History. Reopening an issue brings back a row still in Done or Resolved. Dismissals hold across GitHub changes; Undo uses the latest kept facts. Failures with the same nonempty cause collapse into one item with a count; unknown causes stay separate.
- **Ready for review:** Commands that ended ready, with a pull request. One leaves when a later Command on the same issue starts on this machine, or when GitHub shows its pull request merged or closed; a pull request GitHub still shows as open keeps its row however old it is. A row GitHub can't speak for still leaves after `inbox.days`: one with no pull request, one with no Command record, one not read yet, or one whose pull request GitHub can't resolve. A merged pull request takes its row to Resolved for `inbox.days` from the merge time (a closed one from the close time); a pull request reopened while its row is in Done or Resolved brings the row back. From History on, a row keeps what GitHub last showed and its issue and pull request aren't read again. The Console reads ready rows' pull requests and Needs you rows' issues and pull requests in a batched query in the background once a minute, even with no pages open, and keeps the answer in one file on the machine. Changed answers wake open Inbox pages; unchanged reads leave pages idle. `status` uses the same answer while it is under 5 minutes old, and otherwise reads GitHub itself. Neither timing is a User config setting. The reads never write to GitHub. `status --json` gives each row what GitHub last showed and when, and how old the Inbox's answer is.
- **Landing:** live Ticket runs at CI watch or Self-merge. A Command already in Needs you has a reference here instead of a second row.
- **Working:** every other live Command.
- **Done:** Commands that ended in the last day with nothing waiting.
- **Resolved:** older completions with nothing waiting, back to `inbox.days` after they ended, after the later Command started for a row one took up, after GitHub shows its issue closed, its pull request merged (or closed for a ready row), or its triage label removed, or after the dismissal for a dismissed row. A row that ages out of Needs you goes straight to History. Any row in Needs you or Ready for review, live ones included, leaves on a [Dismissal](#dismissing-a-row).

When GitHub cannot answer, each row keeps the group its last facts give it; a row never read goes by age. Once the last good read is more than 5 minutes old, `status` and the Console warn with when GitHub last answered and why the latest read failed. Before the first good read, the warning says GitHub has not answered yet. A successful read clears the failure warning. `status --json` carries `github_last_answered_at` (an RFC 3339 time, or null before the first answer) and `github_failure` (the latest read failure, or null after success), alongside `github_answer_age_seconds`.

Each row with a kept answer also carries `github`: `subjects` maps its own or dispatched issue and pull request URLs to their `facts` and RFC 3339 `answered_at`. An unresolved URL has null facts; an older cache without an observation time has null `answered_at`. Finding rows include `github.findings` with per-URL waiting flags, the first observed `triaged_at`, and `answered_at`, without private write-ups. History freezes these observation times with its facts. The existing `github_state` and `github_at` fields still describe the row's own pull request and its merge or close event.

The Inbox leaves the last of GitHub's quota to the factory. Both the Console and `status` skip their GitHub reads while the last observation showed fewer than 500 GraphQL points or 500 REST requests remaining, until that quota's reset time. The warning says which quota is low, how much remains and when it resets, even while the kept answer is fresh. Neither the reserve nor the 5-minute warning threshold is a User config setting.

A row a later Command took up says so wherever it goes: `status` prints `left needs you: taken up by a later Command started at <time>` (or `left ready for review: …`), `status --json` carries `left_from`, `left_reason` and `left_at`, and the Console shows the same line on the row. A row GitHub moved out of Ready for review says `left ready for review: pull request merged at <time>` (or `pull request closed at <time>`); a Needs you row says `left needs you: issue closed at <time>`, `pull request merged at <time>` or `needs-triage removed at <time>`. `status` batches these rows' issues and pull requests in the same read. A dismissed row says so the same way: `status` prints `dismissed by <who> from the CLI at <time>`, `status --json` carries `dismissal` with `who`, `dismissed_at`, `source`, `kind` and all covered `kinds`, and the Console shows the same line on the row. A Command that never waited still goes by when it ended.

`--all` adds **History** for everything older or without a known ending. Each row read from a Command record has a state word: its Needs you kind (`Silent`, `Slow`, `Died`, `Failed`, `Triage` or `Finding`), `Stopping` while a live Command given a Stop is still ending, `Ready` in Ready for review, `Landing` in Landing, and otherwise `live`, `ended` or `died`. The Console shows the same word. Rows read from Activity logs or Session traces show their start, end and outcome instead; only the Console gives them a word, such as `Ready`. After the word come its issue and title when present, Harness, Model, Effort, current stage and its age. Architect runs, Pickup runs and Security runs nest the Ticket runs or Spec runs they dispatch beneath themselves, each Spec's Tickets and their Ticket runs, and each Ticket run's Base fix. Every Ticket is listed: running, merged into the Spec branch, failed, waiting on named blockers, or not started. A Pass with no issue is named by its kind and Command log stamp, such as `architect-20261018T120000-0400` or `secure-20261018T120000-0400`. A Command or child Ticket run with no outcome whose process has gone shows as `died at <stage>`; its start time bounds how long it stays in Needs you when no ending is known. Process start identifiers distinguish a reused process id on Linux and macOS. Records remain beside their Command logs, as `*.command.json`, with child records named for their issue as `*.issue-<n>.child.json`. Each process writes only its own files, replacing them atomically. A record write failure warns once per recording process and does not change its outcome.

History uses each source only for Commands missing from the preceding sources: Command records, then Activity log start/end lines with their Command logs, then Session traces in the current `<logs.dir>/<owner>/<repo>/commands/<kind>/` layout. Activity entries have start and end times and outcomes, with no stages. Session entries group logs by issue and exact Command owner; they have no known end or outcome and appear only with `--all`. A Spec run or Pass's Sessions share that Command's stamp and owner token. Legacy tokenless logs retain their existing grouping. The flat layout from before [ADR 0011](adr/0011-thirdshift-keeps-a-per-repository-activity-log.md) is ignored.

For each repository and kind of Pass whose latest Activity log line is a skip, status also shows that Pass as idle with its reason, even when the skip is older than another kind's activity. A later start or end for that kind clears its idle entry. For example:

```text
Architect run idle on acme/widgets: Ready issue #7 "Make widgets" goes first: https://github.com/acme/widgets/issues/7
```

`thirdshift status --json` is the versioned API for agents; the [Console](adr/0016-the-console-is-a-long-running-reader.md) shows the same Inbox. It runs outside a repository too. Version 1 has this shape:

```json
{
  "version": 1,
  "hostname": "factory",
  "active_runs": 1,
  "commands": [{
    "version": 2,
    "kind": "Run",
    "repository": "acme/widgets",
    "name": "acme/widgets#7",
    "issue": {"number": 7, "url": "https://github.com/acme/widgets/issues/7", "title": "Make widgets"},
    "hostname": "factory",
    "process_id": 1234,
    "process_start": "<OS process start identifier>",
    "harness": "claude",
    "model": "opus",
    "effort": "high",
    "started_at": "2026-10-18T12:00:00-04:00",
    "ended_at": null,
    "stages": [{"name": "implement", "began_at": "2026-10-18T12:00:01-04:00", "session_log": "/logs/acme/widgets/commands/issue/7-20261018T120000-0400~ownerA-implement.logs.jsonl"}],
    "session_log": "/logs/acme/widgets/commands/issue/7-20261018T120000-0400~ownerA-implement.logs.jsonl",
    "issue_branch": "issue-7",
    "pull_request": null,
    "outcome": null,
    "cause": null,
    "source": "command_record",
    "command_log": "/logs/acme/widgets/commands/issue/7-20261018T120000-0400~ownerA.log",
    "dispatched": [],
    "state": "live",
    "group": "working",
    "needs_you_kind": null,
    "landing": false,
    "stopping": false,
    "count": 1,
    "stage": {"name": "implement", "began_at": "2026-10-18T12:00:01-04:00", "age_seconds": 52},
    "parent_record": null,
    "parent_dispatch": null,
    "tickets": [],
    "children": []
  }],
  "idle_passes": [{
    "repository": "acme/widgets",
    "kind": "Pickup run",
    "reason": "no Ready issue on acme/widgets"
  }]
}
```

Times are RFC 3339; ages are nonnegative seconds. `model` and `effort` are null when left to the Harness. `issue` is null for a Pass with no issue; `session_log`, `issue_branch`, `pull_request`, `outcome`, `cause`, `ended_at`, `parent_record` and `parent_dispatch` are nullable. Command kinds are `Run`, `Spec run`, `Architect run`, `Pickup run`, `Security run` and `Base fix`. Text output and the Console display the stored `Run` kind as **Ticket run**; Command records, the Activity log and JSON retain `Run`, including when reading older records ([ADR 0022](adr/0022-ticket-runs-and-plain-schedules.md)). `name` identifies an issue or a Pass's kind and stamp. `dispatched` is an array of the Ticket runs or Spec runs a Pass dispatched, with the same fields as its parent (and an empty `dispatched` array); it shares the Pass's process identity and Command log. It retains its own outcome, end time, cause, Issue branch and pull request. The retained Command record has the same fields, including dispatched work, without the computed `state`, `stage`, `source`, `command_log` and `children` or empty leaf `dispatched` arrays. Older records without an end time use their last modification time. `active_runs` is the count above, computed from the same observations as `commands`, whatever `--all` asks; the Console shows the same total. `idle_passes` contains one entry per repository and kind whose latest Activity log line is a skip; it is independent of `commands`, and never creates a Command record or log for a skipped Pass.

Each top-level `commands` entry adds the computed `group` (`needs you`, `ready for review`, `landing`, `working`, `done`, `resolved` or `history`), nullable `needs_you_kind` (`silent`, `slow`, `died`, `failed`, `triage`, `finding`), `landing` (also true for a Needs you item referenced from Landing), `stopping` (true while a live Command given a [Stop](#stopping-a-command) is still ending), and `count`. A row a later Command took up also carries `left_from` (the waiting group it left), `left_reason` (`taken up by a later Command`) and `left_at` (that Command's start); rows still waiting, that never waited, or that aged out omit all three. With `group` and `needs_you_kind`, `stopping` gives the row's state word shown in text output and the Console; `state` stays the Command's own. A dismissed row also carries `dismissal` with `who`, `dismissed_at`, `source` (`CLI` or `Console`), the current covered `kind`, and all covered `kinds` in dismissal order. While a live row has no problem, `kind` is the last kind dismissed. Rows waiting on an uncovered kind, and rows never dismissed, omit `dismissal`. A collapsed failure has a `collapsed` array containing the other Commands, each with its own group and kind. These fields are observations, absent from durable records and `show`; child Ticket runs remain within their Command. The durable `security_findings_waiting` flag defaults to false for older records.

`state` is `live`, `died`, `ended`, or `unknown` when historical evidence has no ending. `stage` is null before the first known stage. Readers skip unknown stage names. Known stages are `implement`, `Resume`, `Security review`, `pull request`, `CI watch`, `Repair`, `Self-merge`, `cleanup`, `Tickets`, `Spec review`, `Architecture review`, `Security audit`, `Security reproduction`, `Security fix publishing`, `Security close check`, `dispatched Run` and `dispatched Spec run`. A Pass retains its own stages while its dispatched work records its stages separately. A Pickup run or Security run enters `dispatched Run` or `dispatched Spec run` when it dispatches, so its row reads, for example, `dispatched Spec run · 19 hr 46 min`, timed from the dispatch, and `died at dispatched Spec run` if its process goes. A Pickup run's `issue.title` is the one its Ready issue was listed with; older records have null.

`children` recursively contains the immediate child Ticket runs in separate processes, with the same fields as their parent. A Pickup's `dispatched` contains its Ticket run or Spec run. A Spec's `tickets` is its complete roster in issue-number order; each entry has `issue` (`number`, `url`, nullable `title`), `state` (`running`, `merged`, `failed`, `waiting`, or `not started`) and `blockers` (the named open blocking issues, with the same issue fields). Match a Ticket's `issue.url` to the Ticket run in that Spec's `children` for its stage, age, liveness and Base fix descendants. Tickets without a Ticket run remain in the roster. Completed child Ticket runs stay in the tree while its Command is listed. A killed child's `state: "died"` remains independent of the Spec's Ticket accounting. `parent_record` links each child to its parent's exact file; `parent_dispatch`, when present, names the dispatched issue's URL within that file. A top-level Command has null for both. Records from before these fields existed read as having no children or Tickets.

For example, this tree excerpt omits the common Ticket run fields shown above:

```json
{
  "kind": "Pickup run",
  "dispatched": [{
    "kind": "Spec run",
    "stage": {"name": "Tickets", "began_at": "2026-10-18T12:00:01-04:00", "age_seconds": 52},
    "tickets": [
      {"issue": {"number": 21, "url": "https://github.com/acme/widgets/issues/21", "title": null}, "state": "running", "blockers": []},
      {"issue": {"number": 22, "url": "https://github.com/acme/widgets/issues/22", "title": null}, "state": "waiting", "blockers": [{"number": 21, "url": "https://github.com/acme/widgets/issues/21", "title": null}]},
      {"issue": {"number": 23, "url": "https://github.com/acme/widgets/issues/23", "title": null}, "state": "merged", "blockers": []},
      {"issue": {"number": 24, "url": "https://github.com/acme/widgets/issues/24", "title": null}, "state": "not started", "blockers": []}
    ],
    "children": [{
      "kind": "Run",
      "issue": {"number": 21, "url": "https://github.com/acme/widgets/issues/21", "title": "Make widgets"},
      "state": "live",
      "stage": {"name": "implement", "began_at": "2026-10-18T12:00:01-04:00", "age_seconds": 52},
      "children": []
    }]
  }]
}
```
Every entry has `source`: `command_record`, `activity_log` or `session_log`. The latter two carry `repository`, `kind`, nullable `issue` (number, URL, nullable title), `harness`, `model`, `effort`, `started_at`, nullable `ended_at` and `outcome`, nullable `command_log`, `session_logs` (paths), `state`, `stages: []` and `stage: null`. They omit record-only fields such as process identity and branch information. Activity times use the UTC offset in the Command log's stamp; their outcome is the Activity log's full ending text. A Session entry has null Harness, Model, Effort, end time and outcome, and lists its grouped Session traces. Both flags can be combined as `status --all --json`.

## Show and read one Command

Name a Command by `widgets#7`, `acme/widgets#7`, or its full Issue URL. The owner is needed only when repositories on this machine share the name; an ambiguous short name is refused with the matching repositories. When several Commands worked on the issue, the live one wins, otherwise the latest start wins. A Ticket's issue names its Ticket run within the Spec run. A Pass with no issue is named by kind and stamp, such as `architect-20261018T120000-0400`, as listed by `status`.

```sh
thirdshift show 'widgets#7'
thirdshift show https://github.com/acme/widgets/issues/7 --json
thirdshift logs 'widgets#7'
thirdshift logs 'widgets#7' --session implement
thirdshift logs 'widgets#7' --session repair --follow
```

`show` includes the Ticket runs, stage history with start times and Usual ranges, outcomes, and paths to Command logs and Session traces. Each live Ticket run also gets its latest event, such as `Bash: cargo test`, from the last 64 KiB of its current Session trace. It uses the Harness's stream interpretation, including Claude and Codex, and ignores malformed or unknown lines. Missing or unreadable Session traces leave the event unavailable.

`status` and `show` describe live work as **Silent** when neither its Session trace nor its Command log has grown for `inbox.silent_minutes` (10 by default). Silence starts no earlier than the current stage. A dispatch stage is never Silent or Slow and has no Usual range: the dispatched Ticket run or Spec run carries its own. At three times the threshold, the words escalate: `silent 45 min · past 30`. A fresh line in either log keeps the Ticket run from being Silent. Missing logs use the stage's start time.

A stage's **Usual range** is its 25th–75th percentile time over the repository's last 30 Command records on this machine. Completed stages supply times, with the next stage's start or the Command's end marking completion; an unfinished stage supplies none. The range appears once there are at least three stage times. Quartiles interpolate between sorted times. A live Ticket run past the top of the range is **Slow**: `slower than usual (12–17 min), still working`. At twice the top it also shows the multiple, such as `2× usual` or `3× usual`. Silent takes precedence over Slow.

Both JSON views add `health` to recorded Commands: `{"state":"ok"}`, `{"state":"slow","step":1,"usual_range":{"lower_seconds":720.0,"upper_seconds":1020.0},"multiple":1}`, or `{"state":"silent","step":2,"seconds":2700,"threshold_seconds":600}`. Step 1 is the first threshold; step 2 is the escalation. Ended or dead Commands have null health. The `usual_ranges` map gives each stage's current range by name, for example `{"implement":{"lower_seconds":720.0,"upper_seconds":1020.0}}`; a stage is omitted until enough times exist. Health and this map are reader observations.

At completion, each Ticket run, Base fix and Spec run writes `run_metrics` into its own Command record (version 2). Ticket runs retain their own metrics. The field holds `outcome`, `repairs` (the number of Repair stages), `ci_rounds` (CI watch stages), `total_minutes` from launch to completion, and a `stages` array with each stage's `name`, `seconds`, `minutes`, `usual_range` and `position`. Position is `below`, `within`, `above` or `too_few_runs`; the last has a null range. The saved ranges use the same repository history as health, excluding the record being finished. They remain fixed as later Ticket runs finish. `show --json` and Console details include these retained metrics, reconstruct available metrics for past completed Ticket runs, and join confirmed Bug origins at read time, just as `metrics` does. Old records are never rewritten; signals they never recorded remain missing.

`unaddressed_findings` holds separate `standards`, `spec` and `security` counts from the PR body's Unaddressed findings section after Delivery's last Repair. Each list entry under an axis counts once; nested evidence does not. The Security review's deterministic outcome is included without counting its detailed entries twice, even if publishing its summary fails. A body without the section records three zeros; no PR or an older record without this signal shows `null`. If the body cannot be read, a known Security review count is retained with null Standards and Spec counts; with neither source available, the whole signal is null. These counts stay fixed when the Day shift later edits the PR.

### Run metrics and weekly trends

`thirdshift metrics <command> [--json]` reads one Ticket run's metrics, or a Spec run together with its Ticket runs and Base fixes. It accepts the same Command names as `show`. Saved completion metrics stay fixed. For older Command records, metrics are computed when read, without writing back: outcome, Repairs, CI rounds and the stage times the record holds, plus signals from converted Session traces. This also fills Session-trace signals in saved metrics that predate them. Missing completion times, review counts and saved Usual ranges remain missing. Live Ticket runs have no completion metrics yet.

`thirdshift metrics [--json]` reads this machine's Command records across repositories, including child Ticket runs exactly once. It groups Ticket runs by their start week (Monday in UTC), repository, Harness, requested Model and Effort, and kind of Ticket run. Session-stage rows add the kind of session and Harness version from the Session trace's resource, and use that session's own retained Harness, requested Model and Effort when available, so a Security review can differ from its Command. Without retained session setup, ordinary sessions inherit the Command's choices; a Security review's setup remains missing because it may have used another Harness. Ticket run totals have null `session_kind` and `harness_version`, so totals are never repeated for every session or version. Records without completion metrics contribute missing values. It neither reads another machine nor contacts GitHub.

When metrics are read, `confirmed_bug_origins` counts the confirmed Bug origins against the Ticket run whose pull request introduced each bug. The reader uses the Scoring data under `~/.thirdshift/evals/scoring/`, which covers both hosts, and matches only this machine's Ticket runs. A retained local identity selects that exact record path and Command name under the matching pull request; an unavailable or mismatched identity counts for nothing, with no fallback to another Continuation. Absent local attribution and cross-host attribution select the latest local Command, using the same matching and chronological ordering as Scoring (described below). A Continuation sharing the pull request never multiplies the count. A confirmation made after the Ticket run ended still counts in that Ticket run's start week. The latest Scoring window's decision for each bug wins: rejected and unsettled origins count for nothing, and a correction counts against the settled pull request. Counts are computed without contacting GitHub or rewriting Command records; weekly Ticket run totals summarize them separately from other signals.

At completion a Ticket run sums its Session-trace signals by session kind in `session_signals`, with observed and missing session counts. Each originating Session stage also retains its signals and Harness version; later stages referencing that same Session do not count it again. Tokens stay split by observed Model as input (including cache), output, cache read and cache write; an unreported Model is null, never inferred from the requested Model. Cost is in USD only when reported by the Harness, without a price table. Tool errors, permission denials, killed work and repeated tool calls remain separate counters. A repeated call uses the same tool and JSON input as an earlier call in that same session; object key order does not matter. Late permission metadata amends the original call. Missing measurements are null, including cost that a Harness does not report. A missing trace contributes a missing session rather than zero counters. Weekly Session rows summarize each signal independently, with tokens kept by observed Model.

Token counts and cost use the same [Session usage rules as Experiments](#experiments).
Partial authoritative totals refine only reported fields, retaining known input,
cache counts and cost when omitted. Saved completion metrics remain unchanged;
these rules also apply when reconstructing historical Session signals.

Each session's shell commands are also checked against one code-owned pattern list: force pushes, explicit shared-branch history rewrites, and recursive deletes outside the Worktree. Matches appear as `destructive_commands` entries with `pattern` and the original `command` in the Session stage's signals and the Ticket run's per-kind summary. They are advisory and never block or fail a Ticket run. Weekly Session rows summarize the number of flags. Older saved signals without this field show missing evidence. A historical trace without a recorded Worktree directory also shows missing evidence when no matches can be established; known matches are retained. The check matches literal commands; it does not execute shell expansions or resolve filesystem symlinks.

```sh
thirdshift metrics 'widgets#7' --json
thirdshift metrics --from 2026-10-05 --through 2026-10-19 --repo acme/widgets --json
```

`--from` and `--through` select inclusive Monday week-start dates (`YYYY-MM-DD`); omit either for an open end. Repeat `--repo owner/repository` to select several repositories. These filters apply only to trends. Without filters, all recorded weeks and repositories are included.

The version 1 JSON has `version`, `hostname` and either `command` or `weeks`. A Command has its identity, `metrics` and `children`. Each weekly row identifies its group and includes `runs`, separate numeric signal summaries (`observed`, `missing`, `sum`, `mean`), outcome counts, Unaddressed findings by Standards, Spec and Security, and stage summaries. A summary with no observations has null `sum` and `mean`; zero is reserved for an observed zero. Stage summaries keep times, Usual-range bounds, position counts and Standards and Spec review counts separate. Text shows the same signals, spelling null as `missing`. No view combines signals into a score.

`show --json` is a versioned API, usable outside a repository. Version 1 wraps the selected Command in this shape:

```json
{
  "version": 1,
  "hostname": "factory",
  "command": {
    "version": 2,
    "kind": "Run",
    "repository": "acme/widgets",
    "name": "acme/widgets#7",
    "state": "live",
    "latest_event": "Bash: cargo test",
    "stages": [{"name": "implement", "began_at": "2026-10-18T12:00:01-04:00", "session_log": "/logs/acme/widgets/commands/issue/7-20261018T120000-0400~ownerA-implement.logs.jsonl"}],
    "command_log": "/logs/acme/widgets/commands/issue/7-20261018T120000-0400~ownerA.log",
    "session_log": "/logs/acme/widgets/commands/issue/7-20261018T120000-0400~ownerA-implement.logs.jsonl",
    "session_logs": ["/logs/acme/widgets/commands/issue/7-20261018T120000-0400~ownerA-implement.logs.jsonl"],
    "children": [],
    "dispatched": []
  }
}
```

The Command excerpt omits the common fields documented for `status --json`; all those fields are retained, including process identity, issue, Harness, Model, Effort and nullable outcome. `children` and `dispatched` retain the same recursive shape. `session_logs` lists the selected Ticket run's Session trace paths, including original sessions and Resumes that share a stage. `latest_event` is optional, present only when a live Ticket run has a readable event, including in child Ticket runs. History without a Command record uses the same Activity-log or Session-trace shape as `status --all --json`, with Session traces discovered by issue/kind and exact Command owner, without invented stages or liveness.

`logs` writes the Command log verbatim. `--session <kind>` writes that kind's latest Session trace as raw JSON lines; `implement` and `spec-review` include a later Resume, and `repair` selects the latest numbered Repair. A missing log or session kind gives an error. `--follow` first prints the selected log, then flushes appended bytes until the captured outer Command process exits, including output written after its outcome was recorded. A Ticket shares its Command's Command log but selects its own Session traces. The selected Command and Session trace stay fixed while following. Exact-file following (`logs --file <id> --follow`) and the Console also drain through writer exit; an exact Session trace becomes complete when its owning record names a later Session. Retained files without a captured writer are finite. Each exact-file or Console read first checks that the selected file is still in the log inventory: removing it, or replacing its owner, repository or kind folder or the file itself with a symlink, ends the read with an error. Following by Command is unaffected. The CLI emits raw bytes; the Console holds an unfinished live line until its newline arrives or the selected file is complete.

## Install

```sh
curl -LsSf https://github.com/JacobStephens2/thirdshift/releases/latest/download/thirdshift-installer.sh | sh
```

The shorter `curl -fsSL https://thirdshift.app/install.sh | sh`, shown on [thirdshift.app](https://thirdshift.app), redirects to the same installer. The shell installer and the binary both come from this repository's [GitHub Releases](https://github.com/JacobStephens2/thirdshift/releases). It puts `thirdshift` in `~/.local/bin`, where Claude Code's installer puts `claude`, and needs no Rust toolchain. thirdshift is a single binary with the Factory skills compiled in, so it runs without a checkout of this repository ([ADR-0001](adr/0001-rust-binary-with-embedded-skills.md), [ADR-0003](adr/0003-static-binaries-through-github-releases.md)).

The installer ends by pointing you to Setup. Run `thirdshift setup` to choose your defaults, or just start a Ticket run and thirdshift will offer it (see [User config](#user-config)).

If you use Rust, you can install it from crates.io instead:

```sh
cargo install thirdshift
```

### Console installation

To open this machine's Inbox locally, run:

```sh
thirdshift console --no-sign-in
```

Open the localhost address it prints. The Console shows the same Inbox as `thirdshift status`, with live stages, logs and Session traces. Ended rows show their recorded finish date and time in UTC and an explicit relative age, for example **Ended Oct 23, 2026 at 23:03:17 UTC · 52 min ago**. This is time since finish, not run duration. Nested Ticket runs and History use each Command's own finish time; missing or invalid finish times say **End time unknown**. The finish information also works without JavaScript. The Console never starts work; start Commands from the CLI or the operating system's scheduler.

#### Console on a server

Set the machine's address, fixed localhost port and sign-in allowlist in `~/.thirdshift/config.toml`:

```toml
[console]
address = "https://console.example.com" # a DNS name, with or without https://; no port or path
port = 8711
allowed_emails = ["you@example.com"]
```

Point that DNS name at the machine, then run these commands as the owner:

```sh
thirdshift console password --email you@example.com
thirdshift console install --output install-console.sh
# Read the script, then install it:
sudo sh install-console.sh
```

`console install` only prints a script; `--output <file>` saves it to that file instead, replacing any earlier copy, and writes nothing if the settings can't make a script. On Linux it prefers a running Caddy or Apache service; otherwise it checks for `caddy`, `apache2ctl` or `httpd` on PATH. For either proxy, the script first checks the Console's address with the system resolver (`getent hosts`). If it doesn't resolve, it stops before writing files or changing services and asks you to add an A or AAAA record pointing at this server, then run the script again. It checks resolution only, not whether the address points at this machine. The script writes `thirdshift-console.service`, running the installed binary as you with your HOME and automatic restarts. Only the script needs root.

Install `curl` before running the script. After enabling and restarting the Console, it waits for the service to be active and answer at `127.0.0.1:<console.port>` before configuring or restarting the proxy. It tries up to 15 times, with one-second request timeouts and one-second pauses (about 30 seconds). A sign-in response counts as answering. If startup fails or the checks run out, the script exits nonzero and prints the unit's last 50 journal lines. If `getenforce` reports `Enforcing`, it also warns that SELinux can block the Console and links to [#1078](https://github.com/JacobStephens2/thirdshift/issues/1078); it does not change SELinux labels or policy.

For Caddy it writes `/etc/caddy/sites/thirdshift-console.caddy`, imports the sites folder from `/etc/caddy/Caddyfile` if needed, and disables proxy buffering with `flush_interval -1`. Caddy handles HTTPS. It **restarts**, rather than reloads, Caddy to preserve HTTP/3 on Android. For Apache it writes the port-80 site under `/etc/apache2/sites-available` or `/etc/httpd/conf.d`, preserves Host, forwards the request protocol and disables gzip on the Console's event streams. Install certbot with its Apache plugin first; the script runs `certbot --apache -d <name>`. Debian's module/site helpers are included. On an `httpd` machine, enable `proxy`, `proxy_http`, `headers`, `ssl` and `setenvif` and permit proxy connections in the host's SELinux policy first.

With neither proxy, the script installs a localhost-only service with `--no-sign-in` and prints an SSH tunnel command. No address or accounts are needed for that mode. Run the tunnel on your computer and open the printed `http://localhost:<port>` address. On other operating systems, run `thirdshift console --no-sign-in` directly and use the same tunnel; only the Linux installer assumes systemd.

#### nginx

For nginx, use the generated service but remove `--no-sign-in` from its `ExecStart` before installing it. Configure the allowlist and password above. Obtain an HTTPS certificate for your DNS name, then add a site like this, adjusting the name, certificate paths and port:

```nginx
server {
    listen 443 ssl;
    server_name console.example.com;
    ssl_certificate /etc/letsencrypt/live/console.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/console.example.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:8711;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_set_header Connection "";
        proxy_buffering off;
        proxy_cache off;
        gzip off;
        proxy_read_timeout 1h;
    }
}
```

Run `sudo nginx -t`, then `sudo systemctl restart nginx` on a systemd host. The Console still binds only to 127.0.0.1; nginx terminates HTTPS and passes live updates without buffering.

### Updating

```sh
thirdshift update
```

It replaces the installed binary with the latest stable GitHub Release, or says it is already on it. Messages go to stderr and stdout stays empty. It exits `0` when it updated or was already up to date, and `1` on any failure, such as no network. Updating is safe while a Ticket run or a Spec run is using the old binary: it keeps running it. On Linux so do the child Ticket runs it starts afterwards, a Ticket run or a Base fix, which it starts from the binary it is running rather than from the install path, so they run its version even once that path holds the new one or nothing at all. On macOS a child Ticket run is started from the install path, so one started after the update runs the new binary. A Ticket run never checks for updates or updates itself.

`thirdshift update` only replaces a copy put in place by the shell installer (the `curl` command above), which leaves an install receipt in `~/.config/thirdshift/`. It refuses to touch any other copy and lists the command that updates each other kind of install: `cargo install thirdshift` for a crates.io install, pulling and reinstalling for a build from source (see [Building from source](#building-from-source)), and the `curl` command for a copy placed by hand.

Keep a single copy: with copies in more than one directory on your `PATH` (say `~/.local/bin` and `~/.cargo/bin`), you can end up running a stale one without noticing. `type -a thirdshift` lists every copy on your `PATH`.

## Supported platforms

- **Rocky Linux** 9.8 and later, x86_64.
- **Ubuntu** 24.04.3 and later, x86_64.
- **WSL2**, latest release, with Ubuntu 24.04 or later, x86_64. Keep your repositories on the Linux filesystem (e.g. `~/repos`), not under `/mnt/c`, where git is slow and file permissions misbehave. WSL1 is not supported.
- **macOS** Tahoe 26.5.2 and later, Apple Silicon.

One statically linked Linux binary covers the first three, so it doesn't depend on the host's glibc; macOS gets a native build.

## Prerequisites

The user account that runs thirdshift needs:

- The selected **Harness** installed and authenticated: **`claude`** (Claude Code, the default), **`codex`** (Codex CLI), **`agy`** (Google Antigravity CLI), **`grok`** (Grok Build), **`muse`** (Muse Code), or **`opencode`** (OpenCode, with a configured Model provider). See [Harness, Model and Effort](#harness-model-and-effort) for each CLI's flags and quirks.
- **`gh`** (GitHub CLI), logged in.
- **`git`** with a global `user.name` and `user.email`, and credentials that can push to the repository (`gh auth setup-git` makes git use `gh`'s login). The agents commit as this identity; without it, an agent may borrow the author of the last commit.
- **`node`** (Node.js) on `PATH`, for a [Security run](#security-runs) only: the Security audit's report validators run on it.

### Unattended sessions

Sessions run headless with the full permissions of that user account, including `sudo` if the user has it, and nobody is there to approve anything. On Claude Code they run in auto mode (`claude -p --permission-mode auto`): its classifier checks each action and may block ones it judges risky, such as destructive commands or actions outside the task. A blocked action the agent can't work around can end the Ticket run as a Failed ticket run. The other Harnesses run with approvals and their sandbox turned off ([ADR-0013](adr/0013-every-harness-runs-unattended-and-fully-trusted.md)), so no approval step or sandbox stands between an action and the account.

Every factory session on Claude Code, Repairs, reviews and Resumes included, also passes `--settings '{"attribution":{"commit":"","pr":""}}'`. These per-session settings override the user's and repository's attribution settings, hiding Claude Code's commit trailer and pull request attribution. The pull request keeps thirdshift's `Built with` line as its only attribution; nothing replaces the commit trailer. Claude Code run by hand keeps its own settings. The release-summary invocation, which runs with tools disabled, keeps its existing arguments.

`claude -p` exits as soon as the agent ends its turn, killing any background task it started. So every prompt tells the agent to run long commands, such as tests, in the foreground, and to stop any background task it no longer needs, with the `TaskStop` tool, before it ends its turn. If a session still ends with background work running, thirdshift takes it as work the session was waiting on and gives it a **Resume**: it continues that same session once (`claude -p --resume`), asking the agent to re-run the work in the foreground and finish. If the Resume ends the same way, the work may have been abandoned, such as a hung test run the agent could not stop: thirdshift says which task was killed and carries on, so the pull request check, CI and the rest decide how the Ticket run ends. If the Ticket run then fails, its cause names the killed task ahead of what failed.

### What sessions leave behind

Cleanup removes only thirdshift's own worktree, local Issue branch and the temporary directory it wrote the Factory skills out to. The `.git/info/exclude` entry that keeps the linked skills out of git, `/.claude/skills/thirdshift-*`, or `/.agents/skills/thirdshift-*` on the other Harnesses, is added once and left in place, as every worktree of the repository shares the file. Anything else an agent does as your user stays. For example, one Ticket run downloaded a JDK to `~/.local/jdk/` because the server had no Java, installed Playwright in `/tmp/pw`, and left a pull request body draft in `/tmp/`. This is by design, but worth knowing:

- **Install the target repository's toolchains up front** (language runtimes, package managers, test tools), so the agents don't download their own on every Ticket run.
- **For UI work, install the headless browser dependencies**, e.g. `libgbm1` on Ubuntu server, which headless Chromium needs. Without it, agents can't take screenshots to check a UI change and fall back to reading the code.

### Usage and cost

When `claude` is logged in with a claude.ai subscription, Ticket runs use the plan's usage allowance, not API billing. The `total_cost_usd` in the session traces is the API-price equivalent, not a charge. An unattended Ticket run that hits the plan's usage limit stalls or fails partway through.

## Usage

```sh
cd ~/repos/widgets          # a clone of the issue's repository, on the Base branch
thirdshift https://github.com/acme/widgets/issues/7
```

You can use a positive decimal issue number anywhere a Ticket run takes an Issue URL:

```sh
thirdshift 7
thirdshift merge 7
thirdshift --no-merge 7 harness codex
```

The number resolves against the Launch directory's `origin`, including from nested
directories in the clone. Supported origins are `https://github.com/owner/repo`,
`git@github.com:owner/repo` and `ssh://git@github.com/owner/repo`, with or without
`.git`. The Ticket run uses the same Issue URL and Origin match as the full-URL form.
Zero, overflow and malformed numbers are rejected. A missing or non-GitHub
`origin`, or starting outside a clone, stops shorthand resolution before any
session or tracker change. All Ticket run options work with either form.

A Ticket run takes minutes to tens of minutes.

To have the Ticket run merge its pull request instead of leaving it for your review, ask the Ticket run to **Self-merge**:

```sh
thirdshift merge https://github.com/acme/widgets/issues/7
```

`thirdshift --merge <Issue URL>` does the same, and either goes before or after the URL. The agent gets the same implement prompt either way; the differences are the Self-merge at the end and the review of [Foreign commits](#foreign-commits-in-a-ticket-run-that-self-merges). To have every Ticket run on a machine Self-merge, set `merge.always` in the [User config](#user-config); `no-merge` (or `--no-merge`), before or after the URL, then makes one Ticket run end ready for review instead. Giving a flag twice, or `merge` together with `no-merge`, is an argument error.

A Ticket run that Self-merges ends in one of four ways:

- **Merged**: exit `0`, the pull request's URL on stdout, and `PR <url> is merged` as stderr's last line.
- **Review required**: the PR's `reviewDecision` is `REVIEW_REQUIRED` or `CHANGES_REQUESTED` just before a Self-merge attempt. Exit `1`, the PR URL on stdout, and `PR <url> needs an approving review before it can merge; it is ready for review` on stderr and in the Run notification. The Run notification and Activity log summary name `review required`. The PR stays ready, the Claim is kept, and no merge attempt or further Repair round runs. An empty or `APPROVED` decision, or a failed read, leaves the existing merge behavior in place.
- **Policy refusal**: GitHub refused the merge and a round of merging the Base branch and watching CI found nothing left to fix. Exit `1`, the pull request's URL on stdout, and GitHub's error on stderr. The pull request stays open and ready for review, not a draft, for you to merge by hand or to change the repository's settings. thirdshift never reads GitHub's error text to decide this.
- **Any other failure**: a [Failed ticket run](#failed-ticket-runs), as for a Ticket run without `merge`.

To be emailed when the Ticket run ends, add `--email` (or `email`), optionally followed by the address, before or after the URL and alongside `merge`:

```sh
thirdshift --email you@example.com https://github.com/acme/widgets/issues/7
```

It sends one [Run notification](#run-notifications), whatever the outcome. To have every Ticket run on a machine send one, set `email.always` in the [User config](#user-config); `no-email` (or `--no-email`) then skips it for one Ticket run.

- **stdout** carries only the pull request's URL: on success, and on a Failed ticket run that leaves an open pull request, a draft or, after a policy refusal, one ready for review. The exit code tells the two apart, so script it as `url=$(thirdshift "$issue") && echo "ready: $url"`.
- **stderr** carries everything else: errors, cleanup problems, and progress lines while sessions run. Each line starts with the local time it was printed, as in `thirdshift: 12:14:49 pushing issue-7`, so a quiet terminal shows how long the Ticket run has been on its last step. The first line of a Ticket run, a Spec run or a Pass also names the date and the UTC offset, as in `thirdshift: 12:00:01 starting on https://github.com/acme/widgets/issues/7, 2026-10-03 -0400`, so a log that collects many commands, such as a crontab line's, reads as a dated list. Everything on stderr and stdout is kept in the command's [Command log](#logs) too. A top-level Ticket run or Spec run prints its elapsed wall time near the end, as `Took: 1h 2m 3s`, before its final outcome or failure cause. This also covers graceful interruption, works without email, and adds no summary for child Ticket runs or skipped Passes. A successful Ticket run's last line names the pull request too: `PR <url> is ready for review`, or `PR <url> is merged` after a Ticket run that Self-merges, followed only by a `warning:` line if a [Run notification](#run-notifications) can't be sent.
- **Exit code** `0` means the Ticket run ended with a pull request the factory stands behind, merged in a Ticket run that Self-merges. Once the Self-merge has merged, the Ticket run succeeds even if deleting the Issue branch on `origin`, closing the issue or removing `in-progress` from it then fails: the merge can't be undone, so each failed step is a `warning:` line on stderr naming the command to run by hand, and the Ticket run still exits `0` with the URL on stdout. Ctrl-C likewise: before the merge it makes a Failed ticket run, after it thirdshift finishes these steps and exits as merged. `2` means the command is one thirdshift can't use: the issue argument is missing or isn't a GitHub Issue URL or positive decimal number, there is an argument other than the issue and the Ticket run flags, a Ticket run flag is repeated or contradicts another, or `parallel` isn't followed by a whole number from 1 up; the error and the help text go to stderr, before any work. Failure to resolve a number against the clone's origin exits `1`. A [User config](#user-config) thirdshift can't use exits `1`, also before any work. Any other failure exits `1`.

During a CI wait, `CI on <short sha>: 3 still running out of 4 checks listed so far` counts the checks GitHub currently lists, including commit statuses. Dependent jobs and matrix jobs may become visible later, so both the pending count and the listed total can rise as CI progresses. A change to either count prints a new progress line; identical repeated counts stay quiet. The listed total is not a prediction of the final workflow total. This reporting does not change which checks count as passing or when Delivery may merge.

CI check observations get at most three total attempts: the original read and two retries, with interruptible delays of one second then two seconds. Transport failures, HTTP 429 or 5xx responses, and truncated JSON can recover this way; known permanent errors and unclassified failures stop immediately with their diagnostics. This applies to the initial watch, pending-check polling, Base comparison and Check re-run reconciliation. Each retry reads the same repository and full commit again. Failed reads never count as passing or absent CI, never start a Repair, and never repeat a workflow re-run request. Progress lines retain each failure and delay; exhaustion reports all three attempts and their causes. Cancellation stops active reads and backoff without another request.

Reads of `origin` by git, `git ls-remote` and `git fetch`, get the same three attempts with the same interruptible delays when git's error says the connection failed in transport: a dropped HTTP/2 stream or connection (`RPC failed; curl <n>`, `early EOF`, `the remote end hung up unexpectedly`, `Connection reset`), a timeout, a name lookup, a TLS read, or HTTP 429 or 5xx. Each retry prints a progress line with the command, its git error and the attempt, as in `git ls-remote --heads origin refs/heads/issue-7 refs/heads/issue-7-branch-* failed: error: RPC failed; curl 16 Error in the HTTP2 framing layer (attempt 1/3); retrying in 1s`. Authentication failures, a repository not found, a missing ref and any error not recognised fail at once, and a read that fails all three times fails with its last error. `git push` is never retried this way.

The other commands:

```sh
thirdshift architect [<focus>]      # review the Base branch's architecture, publish a plan and run it (see Architect runs)
thirdshift architect [<focus>] --plan-only   # publish the plan, mark it ready and stop there
thirdshift architect base <branch> [<focus>]   # do either with <branch> as the Base branch, from a clone on any branch
thirdshift pickup                   # take the lowest-numbered Ready issue in the repository and run it (see Pickup runs)
thirdshift pickup highest-first   # take the highest-numbered Ready issue instead, for that one pass
thirdshift pickup base <branch>     # do that with <branch> as the Base branch, from a clone on any branch
thirdshift secure                   # audit the Base branch and record findings privately (see Security runs)
thirdshift secure base <branch>     # do that with <branch> as the Base branch, from a clone on any branch
thirdshift score                    # sample merged factory pull requests (see Scoring runs)
thirdshift scores [<pull request>] [--json] # show Scores and calibration
thirdshift scores <pull request> --mine [--json] # enter your five Scores
thirdshift bug-origins [<issue>] [--json]  # list Bug origins or show one
thirdshift bug-origins <issue> --confirm   # confirm the proposed pull request
thirdshift bug-origins <issue> --correct <pull request> # confirm a different pull request
thirdshift bug-origins <issue> --none      # reject the origin
thirdshift email-test [<address>]   # send a test email through Resend (see Email)
thirdshift setup                    # choose your defaults and write the User config with every setting (see User config)
thirdshift evals list               # list private Eval suites and approved/proposed Eval tasks
thirdshift evals approve <suite> <task> [--json]  # approve a proposed Eval task in its suite
thirdshift evals import-replays     # import #429's private manifests and grading files once
thirdshift evals touched <path>...  # select suites from their declared source dependencies
thirdshift evals prepare <suite> <task> <destination>  # prepare a throwaway repository at its base
thirdshift evals verify <repository> <base>  # refuse history beyond the base commit
thirdshift evals prepare-arms <repository> <baseline-commit> <baseline-harness> <candidate-commit> <candidate-harness> <record.json>  # cache builds and record fixed Harness paths
thirdshift experiment <candidate> [--baseline <ref>] [--suite <name>] [--json]  # run or resume an Experiment
thirdshift experiment --release <commit> [--json]  # use the last release tag and touched suites
thirdshift experiments [<id>] [--json]  # list Experiments or show one
thirdshift experiments <id> --session <session-id> [--json]  # read a Trial's retained session from its inventory
thirdshift recheck <id> <trial> <pass|fail> --reason <text> [--wrong-finding <true|false>] [--json]
thirdshift update                   # update to the latest release (see Updating)
thirdshift version                  # print thirdshift <version>
thirdshift help                     # print a compact command and option reference
```

`help` lists commands and shared options, with a link to this manual for workflow details, settings, Harness setup, email, logs and scheduling.

### Eval tasks

Eval suites live privately under `~/.thirdshift/evals/suites/<suite>/`. Their
manifests declare Factory skill, Session prompt and Harness, Model and Effort
default dependencies, plus Trials per Arm. `evals touched` selects suites from
those declarations. `evals list` includes proposed Eval tasks, while `evals
prepare` refuses them until approved. `evals approve <suite> <task> [--json]`
approves one in its suite without changing its inputs or starting work. The
Console offers the same action under Proposed Eval tasks on the Experiments
list and, for its suites, on an Experiment page. Approval affects future
Experiments; existing Experiments keep their fixed Eval tasks when resumed.
Preparation keeps only base history, applies
the change under review without its commit objects, and gives Spec-review Eval
tasks a local bare origin. These commands write JSON and start no agent sessions.
`evals prepare-arms` prepares both Arms before Trials: it caches release builds
by commit under `~/.thirdshift/evals/builds/` and records their commits, binaries,
and each Harness's real path and version in a new JSON file. A failed build or
version lookup stops preparation. Trials use the recorded Harness path so an
installation's symlink update cannot change the Harness between Arms.
The [Eval task format](https://github.com/JacobStephens2/thirdshift/blob/main/docs/eval-tasks.md)
documents the manifests with made-up data and the Arm record.

On the host holding #429's material, `thirdshift evals import-replays` reads
the private replay manifests, cached original issue bodies and grading index
under `~/.thirdshift/replays/` into the code-review and Spec-review suites.
Rerunning adds missing Eval tasks and completes missing Trial inputs in both suites,
preserving local edits. The format document also describes the
grading index to prepare on that host; private material stays outside the repository.

### Experiments

From a local thirdshift clone, `thirdshift experiment <candidate>` compares a
branch or commit with its merge-base with `main`. Use `--baseline <ref>` to name
another Baseline. Suites default to those touched by the change, using their
declarations; repeat `--suite <name>` to select them explicitly, including for a
Harness, Model or Effort comparison with the same build in both Arms.
Before building either Arm, an Experiment names every selected approved Eval
task missing a `[trial]` table. This check also precedes the release Baseline probe.

`thirdshift experiment --release <commit>` uses the last reachable `v<version>`
tag as its Baseline and selects suites touched since that tag. `--baseline`
and `--suite` do not apply in this mode. When no suite is touched, or the last
release's build has no Trial flag, it prints a note and succeeds without an
Experiment. With `--json`, these cases return `release_skip` (`no_suites` or
`baseline_without_trial`) and `reason`; otherwise the ordinary Experiment
result and resume rules apply.

Each Arm accepts `--baseline-harness`, `--baseline-model`, `--baseline-effort`
or the corresponding `--candidate-*` flag, followed by a value. Unspecified
choices follow the User config, then the Harness defaults. Both builds must
support the [versioned Trial protocol](https://github.com/JacobStephens2/thirdshift/blob/main/docs/trial.md).
Both Code review and Spec review Eval tasks run. Code review Trials use the
production code-review Factory skill on the historical diff and retain both
axis reports without fixing or publishing anything. Spec review Trials keep
the production Spec review prompt. They have no PR service: `gh` calls fail
inside the sandbox and only their reports are graded.

The Bar is saved before the first Trial: margin `-0.10`, credible level `0.95`,
prior `1`, seed `0`, `20000` draws, each suite's Trials per Eval task, an
`1800`-second per-Trial timeout, and up to two fresh retries for Void Trials.
Override these with `--margin`, `--credible-level`, `--prior`, `--seed`,
`--draws`, `--trials-per-task`, `--timeout` (seconds), or `--void-retries`.
`--void-retries` accepts zero, one or two. A sandbox that cannot start, a
provider connection or login failure, a provider server error before work, a
plan-limit stop, or a host kill or Trial-only memory cap makes a Trial void.
Timeouts, context-window overflow, refusals and agent failures count as failed
Trials. Void Trials retry fresh up to the Bar's limit and remain void if
exhausted. A task still void in either Arm drops out of both Arms.

Usage/rate limits pause scheduling until the same Experiment is started again.
Claude's warning and rejection events also pause it immediately, as do Codex quota reports at
85% usage in either window. A Trial cut short by a warning is void; a Trial
already running on the other worker may finish, but neither worker starts
another Trial or retry. Ordinary Runs do not stop at these warnings.
The Experiment watches each Trial with the current detectors, including when
an Arm's build predates warning detection.

Two workers take alternating Baseline and Candidate Trials, each in a fresh
prepared repository. Trial artifacts and one result per finished Trial live
privately in `~/.thirdshift/evals/experiments/<id>/`. Trials run in a bwrap
sandbox with a fresh home and provider-only network. Code Graders then score each Trial:
a changed repository makes it void, a review with no reports fails, and the
Eval task's mechanical checks must pass. Scores retain each check and its reason.
Then a fresh model Grader scores one blind packet per Eval task against its
written success criteria. Each packet holds every Trial's review reports with
shuffled neutral labels and identifying Harness, Model, Effort and Arm lines
removed. Graders see report text, never Trial paths or raw session streams.
When an installed Harness has a configured Model different from both Arms,
that Model grades; otherwise the User config's default choice grades. The
Grader choice and executable are fixed before Trials start. Model Graders run
in the same sandbox, seeing only their packet workspace. Code Graders run
offline with their report packet and prepared repository. Neither can read
Trial logs, Arm mappings or the Experiment's other files. An unavailable
model Grader pauses grading without publishing Scores; resume reuses Trials.
`THIRDSHIFT_TRIAL_NO_SANDBOX=1` selects the pass-through launcher for tests
and emergency use, for both Trials and Graders.

Each Trial and each Grader runs in a scope of its own, with the `[trial]` caps, and raises its own `oom_score_adj` to the maximum, so it gives way before the factory under system-wide memory pressure. A Trial its cap kills dies alone and is void with `memory_cap`, from the scope's `memory.events` or journal evidence. An agent's own out-of-memory error stays failed. Each code check has its own Grader scope and the Trial timeout. A Grader killed by its cap stops the Experiment with a diagnostic so you can raise the cap and retry.

Each Trial retains the model's pass/fail, reason, wrong-finding flag and
borderline mark beside its mechanical checks. A mechanical failure still fails
the Trial. Wrong-finding flags enter the paired Verdict; the Verdict also shows
the number of borderline Trials for rechecking, without changing the Bar.
Packets, Grader session output and validated Scores stay in the private
Experiment directory. Grader sessions retain OTLP log records and spans, model
evidence and stderr there, including after interruption or timeout. A failed,
incomplete or malformed Grader response pauses the Experiment; resuming retries
grading without rerunning finished Trials,
and reuses already validated Scores.

The Verdict stays `waiting_on_rechecks` until you have rechecked every borderline
Score and every decisive Score. A Score is decisive when flipping its pass/fail
or wrong-finding flag changes the Experiment's pass/fail/inconclusive decision
under the fixed Bar. Scores from a task dropped as void are excluded.
`thirdshift experiments <id> --json` lists these under `pending_rechecks`, with
each Trial's `id`, suite, task, Arm, Trial number and borderline/decisive flags.
The text view lists the same IDs. `verdict.provisional_verdict` and suite
intervals show the calculation while `verdict.verdict` waits.

Read the Trial's reports and record your decision:

```sh
thirdshift recheck <experiment-id> <trial-id> pass --reason 'The report meets the written criteria.'
thirdshift recheck <experiment-id> <trial-id> fail --reason 'This finding is wrong.' --wrong-finding true
```

Your latest recheck supplies the pass/fail used in the Verdict; the optional
`--wrong-finding true|false` corrects that flag too. Omission retains its last
value. Each Trial's `score` keeps the original Grader's decision, checks and
model evidence, with your timestamped decisions and reasons in `score.rechecks`.
Rechecking again appends to that history. The command atomically saves the
history and recalculated Verdict, then prints the updated Experiment, with
`--json` available. A changed Score can make another Score decisive, so consult
the updated pending list. Trials and Graders never rerun for a recheck, and the
Bar stays fixed. An Experiment being run or rechecked refuses concurrent writes.

Each suite gets a paired Verdict using its recorded Trials per Eval task (one
or three). Any failed suite fails the Experiment; otherwise any inconclusive
suite makes it inconclusive. More than a tenth of all Eval tasks dropped as void
also makes it inconclusive. Results include the suite intervals, the Bar and
each Arm's minutes across all attempts, including Void Trials and timeouts.
The code-review suite runs three Trials per Arm per Eval task by default; the
Spec-review suite runs one. For three-Trial suites, `experiments --json` reports
`pass_cubed`: the share of retained Eval tasks where all three Trials passed,
separately for each Arm.
Provider usage/rate limits pause scheduling until the same Experiment is
started again, consuming the same fresh-retry budget across resumes. Completed
attempts count toward minutes and per-Arm Void totals even while paused or
after resume. Stop preserves completed attempts and the remaining budget.
Recovery applies the saved Bar's Void cause list as well as its retry count,
including older saved lists. Pending history that is already terminal publishes
without another attempt after a publication failure. A newly completed limit
or interrupted Void checkpoints before pausing, even on the last attempt;
resuming exhausted history finishes without replaying its old pause.

The same Arms and suite names identify the same Experiment. Running it again
skips finished Trials and re-runs interrupted ones; a completed Experiment only
reports its saved results. A changed Bar is refused after the first Trial.
`thirdshift experiments --json` lists them and `thirdshift experiments <id>
--json` shows one. The starting command's `--json` ends with one JSON line
containing the Verdict and suite intervals alongside the Trial results and Arm
minutes. Both views also show each Arm's tokens by observed Model and its cost
in USD where the Harness reports one. JSON keeps these under `usage.baseline`
and `usage.candidate`, with `models` and `cost_usd`; missing measurements are
`null`, and tokens without a reported Model are grouped under `unknown`.
Input tokens include cache reads and writes, which are also shown separately.
Experiments and Run metrics share Session usage accounting. Partial Model totals
replace only reported fields: a call with 11 input tokens and 5 cache reads,
followed by a total reporting only 3 output tokens, keeps 16 inclusive input
tokens and 5 cache reads. A raw input total of 20 instead yields 25 inclusive
input tokens. A single matching Model can also be refined by Session totals;
whole-Session tokens spanning several Models are not distributed or added again.
Turn totals supply unknown-Model tokens only when call, Model and Session token
evidence is absent. Reported Session cost wins, including zero; otherwise each
Model's reported cost replaces its call-cost sum, retaining costs the total omits.
Totals include all attempts and Resume sessions; Grader usage is not charged
to either Arm. The text view shows usage beside each Arm's minutes.
Both views show Void counts and causes per Arm, and report repeated
Trial-only memory-cap stops as a finding against that Arm.

The Console's **Experiments** link opens `/experiments`, grouped into Running,
Needs you and Done, newest first within each. Open an Experiment for its live
Trial grid, per-suite Verdict intervals, Arm comparison and folded Bar. Select
a Trial or Eval task to open its Peek, including Grader reasons, rechecks,
Void causes and links to retained Session logs or Session traces. Both pages
pair with `experiments --json`; its view includes task names, active Trial start
times and the `sessions` inventory. Read one with
`thirdshift experiments <id> --session <session-id>` (add `--json` for exactly
`id`, `trace` and UTF-8 `contents`). The CLI and Console share the Experiment's
confined Session evidence reader. Reading one Session validates the saved
Experiment and selects its evidence without calculating accounting, reading
Trial metadata or upgrading a legacy Verdict. Healthy evidence remains readable
when unrelated Trial metadata or usage spans are corrupt. Session IDs stay the
same when raw logs become Session traces; converted log records take precedence.
Pending rechecks offer a
copyable CLI command and a filter for their Eval tasks; the Console records no
recheck itself. The page polls every two seconds, including after completion
so rechecks recorded in the CLI appear. Older records show unknown refs,
versions or end times where those facts were not retained.

Each invocation that does work keeps a Command record, Command log and Activity
log lines under `<logs.dir>/JacobStephens2/thirdshift/commands/experiment/`
(the Activity log is at the repository level). The record names both Arms and
the Bar and moves through Arm builds, Eval tasks and Trials. It appears in the
Inbox: a pass goes in Done, and a fail, inconclusive Verdict or waiting rechecks
go in Needs you. Rechecking a Score updates the latest invocation's Command
outcome and Inbox group alongside the saved Verdict, keeping its completion time.
Use the name from `status` with `thirdshift stop <command>` to Stop it.
The individual Trials keep no factory Command records.
Trial and model Grader Session traces and code Grader spans join the invocation's
Command Trace. Export it with `thirdshift logs <command> --export <dir>`; the
private Trial and Grader artifacts stay under the Experiment's Eval folder.
On resume, each invocation's Trace keeps the Trial attempts and Grader sessions
it started, while Arm totals cover the whole Experiment.
Older Arms with Session traces are joined after completion; Arms predating
Session traces contribute minutes only.

`version` and `help` print to stdout and exit `0`. `update`, `setup` and `email-test` follow the Ticket run's rule: stdout stays empty, messages go to stderr. `architect` follows it too: stdout carries the pull request's URL of the run it dispatches, or, in its place, the URL of the issue it ended on: its plan with `--plan-only`, or its idea when the review published no plan. A skipped Architect run prints nothing there when [another is still running](#one-at-a-time), the URL of each Architect plan that is [still open](#one-architect-plan-at-a-time), the URL of each [Architect idea waiting for triage](#an-architect-idea-waits-for-triage), and the URL of the [Ready issue, or issue still settling, that goes first](#a-ready-issue-goes-first). `pickup` follows it as well: stdout carries the pull request's URL of the run it dispatches, and nothing when the [Pickup run](#pickup-runs) is skipped. So does `secure`: stdout carries the pull request's URL of a fix it dispatches, and nothing otherwise.

Uncommitted changes in your clone are fine: the Ticket run works in its own worktree from `origin`, so they are simply left out. Unpushed commits on the Base branch are not: push them first, or the Ticket run stops.

### Host resources

`thirdshift resources` (or `thirdshift resources --json`) reports this host's available and total RAM in MB (1,000,000 bytes), logical CPU count, one-minute load average, configured limits and whether another session may start. The Console's **Resources** page shows the same snapshot; reload to update it.

Set `[resources]` in `~/.thirdshift/config.toml` to delay new sessions when this machine is busy:

```toml
[resources]
min_available_memory_mb = 2048
max_load_per_cpu_percent = 100
```

Both settings default to `0`, disabling that limit. Values are nonnegative whole numbers; `max_load_per_cpu_percent` is at most 4,294,967,295. A load limit of 100 permits a one-minute load of 8 on eight logical CPUs. Load includes runnable work and, on Linux, tasks waiting for IO; it is not instantaneous CPU usage.

Before every session, including a Resume, Repair, Security review or a preflight Model test call, thirdshift checks the host. If available RAM is below the minimum or load per CPU exceeds the maximum, it reports why and waits, rechecking every ten seconds. Running sessions continue. Limits are read again on every check, so editing the config can release a waiting Command. Stop or Ctrl-C interrupts the wait through normal cleanup. A failed resource read with limits enabled fails the Command rather than starting a session without a measurement.

These are host snapshots, not resource reservations or per-session budgets: simultaneous Commands may both pass a check, and container quotas are not measured. `spec.parallel` still sets the upper bound on parallel Ticket runs. Limits apply to sessions; they do not prevent worktree preparation or Claims.

### User config

A **User config** at `~/.thirdshift/config.toml` sets this machine's defaults for every Ticket run. It is optional: with no file, or one that says nothing about a setting, a Ticket run does only what its command asks for.

```toml
[merge]
always = true   # every Ticket run Self-merges, without the merge word

[base]
fix = true      # every Ticket run may start a Base fix, without the base-fix word

[launch]
pull = true     # every Ticket run first fast-forwards your checkout of the Base branch

[email]
always = true                                 # every Ticket run sends a Run notification, without the email word
to = "you@example.com"                        # where email goes when the command names no address
from = "thirdshift@your-verified-domain.com"  # the sender; onboarding@resend.dev if unset

[logs]
dir = "~/elsewhere/logs"   # the root of the logs, instead of ~/.thirdshift/logs
waiting_line_minutes = 5  # repeat unchanged CI watch and Self-merge waiting lines

[otlp]
endpoint = "http://localhost:4318/v1/traces"   # also send ended spans to a local viewer

[activity]
quiet_skips = true   # a skipped Pass prints nothing

[spec]
parallel = 2   # how many Tickets a Spec run runs at once, instead of 3

[pickup]
limit = 5   # how many open issues labelled in-progress stop a Pickup run taking another, instead of 3
wait_minutes = 60   # minutes since the latest shaping event before an issue is Ready, instead of 5; 0 disables waiting
order = "highest-first"   # take the highest-numbered Ready issue, instead of the lowest-numbered

[security]
review = true   # review each Ticket run's change for Security findings
harness = "claude"   # the Harness for Security runs and Security reviews; blank or missing follows the Command's

[harness]
default = "claude"   # the Harness every session runs on; claude unless set

[harness.claude]
model = "opus"   # the Model Claude Code's sessions run on; blank for its own default
effort = "high"  # how hard that Model reasons; blank for Claude Code's own default

[harness.codex]
model = ""
effort = ""

[harness.agy]
model = "gemini-3.8-flash"
effort = "medium"
```

`otlp.endpoint` is optional and blank by default. Set the full OTLP/HTTP traces URL (usually ending in `/v1/traces`) to receive each Session's ended spans live as OTLP JSON. Delivery is best effort: sends run off the Session's stream reader, each request times out after 150 ms, and a Session waits at most 200 ms for pending sends on exit. A full queue drops spans; local Session traces are still kept. Security and Scoring runs export none of their sessions, including child Ticket runs. Sandbox launcher environment values are redacted from retained Session arguments.

`thirdshift setup` writes this file for you, listing every setting at its default so the file itself shows what can be changed:

```toml
[merge]
always = false   # every Ticket run Self-merges, without the merge word; default false

[base]
fix = false      # every Ticket run may start a Base fix, without the base-fix word; default false

[launch]
pull = false     # every Ticket run first fast-forwards your checkout of the Base branch; default false

[email]
always = false                  # every Ticket run sends a Run notification, without the email word; default false
unless_merged = false           # every Ticket run sends a Run notification unless it merged, without the email-unless-merged word; default false
# to = "you@example.com"        # where email goes when the command names no address; no default
from = "onboarding@resend.dev"  # the sender; default onboarding@resend.dev, which only delivers to your Resend account's address

[logs]
dir = "~/.thirdshift/logs"   # the root of the logs, each repository's in <owner>/<repo>/; default ~/.thirdshift/logs
waiting_line_minutes = 5   # repeat unchanged CI watch and Self-merge waiting lines; default 5 minutes

[otlp]
endpoint = ""   # optional OTLP/HTTP traces URL, including /v1/traces; default blank (off); Security and Scoring runs never export

[inbox]
days = 7             # days to keep Resolved and rows GitHub cannot speak for; default 7
silent_minutes = 10   # minutes without growth in either log before a Ticket run is Silent; default 10

[activity]
quiet_skips = false   # a skipped Pass prints nothing, leaving only its Activity log line; default false

[spec]
parallel = 3   # how many Tickets a Spec run runs at once; default 3

[pickup]
limit = 3   # how many open issues labelled in-progress stop a Pickup run taking another; default 3
wait_minutes = 5   # minutes since the latest shaping event before an issue is Ready; 0 disables waiting; default 5
order = "lowest-first"   # which Ready issue a Pickup run takes, lowest-first or highest-first; default lowest-first

[harness]
default = "claude"   # the Harness every Ticket run's sessions run on, claude, codex, agy, grok, muse or opencode; default claude

[harness.claude]
model = ""    # the Model Claude Code's sessions run on; default blank, for Claude Code's own
effort = ""   # how hard that Model reasons; default blank, for Claude Code's own

[harness.codex]
model = ""    # the Model Codex's sessions run on; default blank, for Codex's own
effort = ""   # how hard that Model reasons; default blank, for Codex's own

[harness.agy]
model = ""    # the Model Antigravity CLI's sessions run on; default blank, for agy's own
effort = ""   # how hard that Model reasons; default blank, for agy's own

[harness.grok]
model = ""    # the Model Grok Build's sessions run on; default blank, for Grok Build's own
effort = ""   # how hard that Model reasons; default blank, for Grok Build's own

[harness.muse]
model = ""    # the Model Muse Code's sessions run on; default blank, for Muse Code's own
effort = ""   # how hard that Model reasons; default blank, for Muse Code's own

[harness.opencode]
model = ""    # the Model OpenCode's sessions run on, provider/model; default blank, for OpenCode's own
effort = ""   # the Model's variant, passed as #effort; default blank, for OpenCode's own

[security]
fix = false   # Security runs may fix reproduced findings; default false
close = false   # Security runs may close a finding their fix merged, when fixing is allowed; default false
review = false   # Ticket runs review their change for Security findings; default false
harness = ""   # the Harness for Security runs, unless the command names one, and for every Security review; default blank; blank or missing leaves Security runs on harness.default, or Claude Code if that is missing, and Security reviews on their Command's Harness

[trial]
memory_high = "1G"   # throttle each Trial and Grader scope past this; default 1G
memory_max = "2G"   # kill each Trial and Grader scope past this, voiding the Trial; default 2G
memory_swap_max = "1G"   # swap each Trial and Grader scope may use; default 1G
```

Every key holds its real value, so a Ticket run reading it does exactly what it does with no file. `email.to` has no default, so `setup` suggests one: the public email of your GitHub profile (from `gh api user`), else your global git `user.email`, unless that is a `@users.noreply.github.com` address, which can't receive mail. With neither, `email.to` is the only line written commented out, as above. `setup` never asks `gh` for more scopes, so a private GitHub email is not read, and a Ticket run never looks the suggestion up: `--email` with no address and no `email.to` still stops the Ticket run. From a terminal (stdin and stderr both terminals), `setup` first asks, on stderr:

1. The Harness for every Ticket run's sessions (`harness.default`), listing only those installed on `PATH`. An unavailable Harness or unknown name is refused and asked again. The default is the configured Harness when installed, else the first installed in `claude`, `codex`, `agy`, `grok`, `muse`, `opencode` order. Then the Model (`harness.<name>.model`) and the Effort (`harness.<name>.effort`) for the Harness chosen only, each defaulting to the User config's, where Enter on none keeps the Harness's own default and `-` clears one. They're checked as a Ticket run checks them (see [Harness, Model and Effort](#harness-model-and-effort)). For Claude, a named Model gets the test call; if Claude refuses it, Setup shows what Claude said and asks the Model and Effort again. For Codex, Setup lists the Models `codex debug models` offers, by slug and display name, then the Efforts the Model chosen supports; each answer is matched regardless of case and written as Codex names it, so `GPT-6.1-Sol` and `Max` are written as `gpt-6.1-sol` and `max`, and a Model or Effort the catalog doesn't have is asked again, with the valid choices. For agy, Setup lists `agy models` and its supported Efforts, checks each answer without a model turn, and writes the canonical names regardless of case. For Grok Build, Setup lists `grok models` and the chosen Model's Efforts from its refreshed cache, and checks each answer against them. Muse proposes `muse-spark-1.3` and checks the answers against its cache, or with a minimal call; a refusal asks for the Model and Effort again. OpenCode proposes no Model and checks the Model and Effort with a minimal standalone call, asking again on a refusal. The other Harnesses' sections are left as they are. With no Harness installed here, Setup says so, asks none of this, and leaves the harness settings as they are.
2. `Every Ticket run Self-merges?` (`merge.always`)
3. `Every Ticket run may start a Base fix when the Base branch's CI is red?` (`base.fix`), whatever the Self-merge answer. Pressing Enter keeps the current `base.fix` value.
4. `Every Ticket run first fast-forwards your checkout of the Base branch?` (`launch.pull`)
5. Security runs may fix reproduced findings? (`security.fix`), off by default.
6. When fixing is on, Security runs may close a finding their fix merged? (`security.close`), off by default.
7. `Ticket runs review their change for Security findings?` (`security.review`), off by default.
8. Run notifications? (`email.always`). If yes, the address (`email.to`), asked again until it has an `@`, then the sender (`email.from`).
9. With notifications on, the Resend API key, with input hidden. With none saved it asks `Resend API key (input hidden, Enter to skip):`; with one in the [Credentials](#email), `Resend API key (input hidden, Enter keeps the saved one):`, and a new one replaces it. Surrounding spaces are trimmed, and anything that doesn't start with `re_` is asked again. A key you give is saved in the Credentials, `~/.thirdshift/credentials.toml`, created with mode 600 (and `~/.thirdshift` with it) or edited in place, keeping its comments and anything else in it and changing only `resend.key`; `setup` then prints `wrote the Credentials <path>`. The key is never printed, nor written to the User config. Skipping writes no Credentials and says how to add a key later: rerun `thirdshift setup`, or set `RESEND_API_KEY`. With `RESEND_API_KEY` set and not empty, which wins over the Credentials, nothing is asked, and it says the key comes from `RESEND_API_KEY`. With a key found or given, it offers to send a test email (default No), as `thirdshift email-test` does, once the files are written.
10. A Claude setup token, made separately with `claude setup-token`, with input hidden, whether notifications are on or off. Enter skips it, or keeps the saved token; a new one replaces `claude.setup_token` in the Credentials, keeping comments and the Resend key. Saving Credentials sets mode 600. The token is never printed or written to the User config. It is reserved for Trials; a Trial receives it as `CLAUDE_CODE_OAUTH_TOKEN`. Ordinary Runs keep using the Harness's existing login.

When the saved `security.review` or `security.fix` is on, Setup also prints a Security sandbox probe result. On Linux it starts a real `bwrap` sandbox and tries a connection to a temporary host-loopback listener (no internet), opening the saved User config for writing without changing its contents, and writing a scratch file with a throwaway HOME. A failed check names the capability and prints operator-run commands for installing bubblewrap and granting `/usr/bin/bwrap` user namespaces through AppArmor on Ubuntu; Setup runs none of those commands and still saves the User config. The sandbox has a three-second deadline. With both settings off, no probe runs. On macOS the probe reports that it is unavailable. Passing checks these capabilities only: Security sessions must still enforce all the audit skill's controls, including credential isolation and resource limits.

Pressing Enter takes the default shown, which is the file's current value, or else the setting's default, and for the address the suggested email above. `logs.dir`, `activity.quiet_skips`, `spec.parallel`, `pickup.limit`, `pickup.wait_minutes`, `pickup.order` and `security.harness` are not asked about. Setup adds a missing `security.harness` at its default, blank, with a comment explaining it; a configured Harness is kept. The answers are written like everything else below: in place, keeping your comments. Ctrl-C, SIGTERM or SIGHUP during the questions or a Model/Effort check exits `1` with `interrupted` and writes nothing: neither the User config nor the Credentials. Ordinary and hidden questions stop without another line of input, and hidden input restores the saved terminal settings, including echo. With notifications off, the Resend key is not asked about and stays as it was, so `--email` on a single Ticket run still works. The Claude setup token is offered independently. Credentials a Ticket run would refuse (see [Email](#email)) are refused before any question, exit `1`, and not touched. With no terminal, as from cron or `thirdshift setup </dev/null`, `setup` asks nothing and never writes the Credentials. Either way it prints the file's path on stderr and exits `0` with stdout empty. Over a User config that is already there, `setup` edits it in place: its comments and key order stay, as do the values it didn't ask about, and each key it lacks is added at its default with its comment, so afterwards the file lists every setting this version knows. If `[security]` falls between Harness sections, Setup moves it after them, keeping its values and comments. One that already lists every setting, down to the commented-out `email.to` line, with Security outside the Harness area is left byte for byte as it was. A key added to an inline table, such as `launch = { pull = true }`, gets no comment, since TOML has no place for one there. One a Ticket run would refuse is refused the same way, exit `1`, and not touched. Any argument after `setup` is an argument error (exit `2`).

The first Ticket run on a machine with no User config, started from a terminal, offers Setup before any work, on stderr: `No User config at <path>. Set your defaults now? [Y/n]`. Yes (or Enter) asks the questions above, writes the file, and the Ticket run carries on using your answers; a flag in the command, such as `--no-merge`, `--email` or `--no-email`, still wins over them. No writes every setting at its default, as `setup` with no terminal does, asks nothing about a key, writes no Credentials, says that `thirdshift setup` changes it, and the Ticket run carries on; later Ticket runs find the file and don't offer again. If the file can't be written, stderr gets a `warning:` line and the Ticket run carries on with the defaults. Ctrl-C, SIGTERM or SIGHUP during the offer, the questions or a Model/Effort check exits `1` with `interrupted`, writes nothing and ends the command before any work, with no Run notification. A command thirdshift can't parse exits `2` before any offer. A Ticket run with no terminal, as from cron, CI, `nohup` or an agent's shell, offers nothing, writes nothing, and runs on the defaults, so a later Ticket run from a terminal still gets the offer.

With `merge.always = true`, `thirdshift <Issue URL>` is a Ticket run that Self-merges, and `thirdshift --no-merge <Issue URL>` (or `no-merge`, before or after the URL) leaves that one Ticket run's pull request ready for review.

With `base.fix = true`, every Ticket run may start a [Base fix](#base-fix), as if given `base-fix`, and `thirdshift --no-base-fix <Issue URL>` (or `no-base-fix`, before or after the URL) forbids it for that one Ticket run.

With `launch.pull = true`, every Ticket run brings the Base branch checked out in the directory you start it from (the **Launch directory**) up to date with `origin`, so you no longer `git pull` by hand before each Ticket run. It happens after the pre-flight checks pass and before the worktree is created, as `git merge --ff-only origin/<Base branch>`: fast-forward only, never a merge commit or a rebase, always from `origin`, whatever the branch's upstream or your `pull.*` settings. A progress line on stderr says when it updates the branch; an already up-to-date branch is left quietly as it is. It is skipped when the checked-out branch isn't the Base branch, as in a Continuation whose open pull request targets another base, or on a detached HEAD. If the update can't happen, for example because uncommitted changes are in the way, stderr gets a `warning:` line with git's error and the command to run by hand, your changes are left as they were, and the Ticket run carries on with the same outcome and exit code. The setting only affects your checkout: the Ticket run's worktree starts from `origin/<Base branch>` either way.

With `email.always = true`, every Ticket run sends a [Run notification](#run-notifications) to `email.to`, as if given `--email`, and `thirdshift --no-email <Issue URL>` (or `no-email`, before or after the URL) sends none for that one Ticket run. With `email.unless_merged = true`, with or without `email.always`, every Ticket run sends one only when it did not end merged, as if given `--email-unless-merged`; `--email` on the command line still emails that one Ticket run whatever its outcome, and `--no-email` sends none. Answering no to Setup's Run notifications question turns both off.

`spec.parallel` sets how many Tickets a [Spec run](#spec-runs) runs at once, by default 3. It must be a whole number from 1 up; `parallel <n>` on the command line wins over it for one Spec run.

`pickup.limit` sets the [Claim limit](#the-claim-limit): how many open issues labelled `in-progress` stop a [Pickup run](#pickup-runs) from taking another, by default 3. It must be a whole number from 1 up. There is no command-line flag for it.

`trial.memory_high`, `trial.memory_max` and `trial.memory_swap_max` cap each Trial and each Grader in its own scope, from `systemd-run --user --scope`. Past `memory_high` the scope throttles. Past `memory_max` its processes are killed together and the Trial is void. `memory_swap_max` bounds its swap. They default to `1G`, `2G` and `1G`, chosen with headroom above #1009's largest observed Harness resident size of about 480 MiB. That snapshot had no builds running; a Trial's footprint beside two building Runs remains unmeasured. Each limit must be a byte count systemd accepts, such as `512M`, `2G` or `infinity`. There is no command-line flag for them. The Experiment suite overrides them with `THIRDSHIFT_TRIAL_MEMORY_HIGH`, `THIRDSHIFT_TRIAL_MEMORY_MAX` and `THIRDSHIFT_TRIAL_MEMORY_SWAP_MAX` for a small test cap. Scopes need your user manager: run `loginctl enable-linger <user>` once, so they work while you are logged out. A logged-out launcher discovers the user's runtime directory if its environment omits it. Without a reachable manager the Experiment stops before dispatch and names the linger step.

`pickup.order` sets which end of the issue numbers a [Pickup run](#pickup-runs) takes its Ready issue from: `lowest-first`, the default, or `highest-first`. Anything else, or anything but a quoted string, stops the pass before any work, naming the file and the key, as a Ticket run would refuse it. `lowest-first` and `highest-first` on the command line win over it for one pass.

`logs.dir` sets the root of the [logs](#logs), with each repository's in its `<owner>/<repo>/` folder, created if missing. It must be an absolute path, `~` or a path starting with `~/`, where `~` stands for `$HOME`. A relative path stops the Ticket run before any work, since the directory a Ticket run is launched from is no base for a setting that holds for every Ticket run.

`logs.waiting_line_minutes` sets how often CI watch and Self-merge repeat a waiting line even when nothing changes, in the terminal and the Command log. It defaults to 5 and must be a whole number of minutes from 1 up. CI watch still writes a line whenever the running or total check count changes; Self-merge writes waiting lines while GitHub calculates mergeability.

`inbox.days` defaults to 7 and must be a positive whole number of days. It bounds ended problems GitHub can't speak for in Needs you, and older completions in Resolved: Resolved runs to `inbox.days` after a Command ended, after the later Command started for a row one took up, after a ready row's pull request merged or closed on GitHub, or after the dismissal for a dismissed row. Done always covers one day. A ready Command whose pull request GitHub shows as open stays in Ready for review however old it is; only a row GitHub can't speak for still ages out of Ready for review after `inbox.days`. Security runs retain whether findings were waiting at completion, without their private write-ups; older records lacking this fact do not infer it from the audit's outcome.

`inbox.silent_minutes` defaults to 10 and must be a whole number of minutes from 1 up. If `logs.waiting_line_minutes` is greater than or equal to it, `status` warns on stderr and waiting stages use half the silent time instead. A one-minute silent threshold therefore uses waiting lines every 30 seconds.

`[harness]` chooses the **Harness**, the agent CLI every session of a command runs on, and for each Harness the **Model** its sessions run on and their **Effort**, how hard that Model reasons. See [Harness, Model and Effort](#harness-model-and-effort).

A Ticket run reads the file before any work. One that isn't valid TOML, or that has a key or section thirdshift doesn't know, such as `alway` for `always`, or a value of the wrong type, such as anything but `true` or `false` for `always`, or `0` for `spec.parallel` or `pickup.limit`, stops the Ticket run with an error naming the file and the offending key, so a typo can't silently leave a setting off. `email-test` and `setup` read it the same way. `update`, `version` and `help` never read it, so a broken User config can't block them, and they never offer Setup; nor does `email-test`.

### Harness, Model and Effort

The available Harnesses are Claude Code (`claude`), Codex (`codex`), Antigravity CLI (`agy`), Grok Build (`grok`), Muse Code (`muse`) and OpenCode (`opencode`).

Every session of a command, its implement session, each Repair and Resume, a Spec review, an Architecture review, each Ticket run and a Base fix, runs on one **Harness**, with one **Model** and one **Effort**. The one exception is a [Security review](#security-review): with `security.harness` set in the User config, it runs on that Harness ([ADR-0021](adr/0021-a-command-may-run-its-security-review-on-another-harness.md)). `harness <name>`, `model <name>` and `effort <level>` (or `--harness`, `--model` and `--effort`) choose them for one command, before or after the Issue URL, and on `architect` (but not with `--plan-only`) and `pickup` too:

```sh
thirdshift https://github.com/<owner>/<repo>/issues/<n> model claude-opus-5-5 effort high
```

For each of the three, the command wins, then the [User config](#user-config), then the default. The Harness's default is `claude`; the Model's and the Effort's are nothing, which leaves them to the Harness, so with no flag and no `[harness]` section sessions run exactly as `claude -p` would. A Model and Effort set in the User config come from the chosen Harness's own section, so `harness claude` over a `codex` default takes `[harness.claude]`, while `model` and `effort` on the command apply to whichever Harness is chosen. Names are passed to Claude Code as given: `--model` and `--effort` when set. Grok Build takes `-m <model>` and `--reasoning-effort <effort>`. Codex takes them as it names them, which the check below settles: `-m <model>` and `-c model_reasoning_effort="<effort>"` when set.

With `harness codex`, every session runs `codex exec --json --dangerously-bypass-approvals-and-sandbox` in the Ticket run's worktree, with stdin set to null, and with `-c project_doc_fallback_filenames=["CLAUDE.md"]`, so Codex reads `CLAUDE.md` where a repository has no `AGENTS.md`. Codex's sandbox is off: on a stock Ubuntu machine it can't start, and commits and pushes from a worktree write outside it, so the worktree is the only boundary, as with Claude's auto mode ([ADR-0012](adr/0012-factory-skills-linked-into-the-worktree-codex-unsandboxed.md)). The Factory skills are linked into the worktree's `.agents/skills/`, kept out of git with `/.agents/skills/thirdshift-*` in `.git/info/exclude`, and each prompt's first line loads its skill as `$thirdshift-<skill>`. `codex exec` kills what the agent left running as it exits, so a session whose turn ends with a command or a sub-agent call still running (started, never completed) gets a Resume, as on Claude: `codex exec … resume <session id>`, with the Model, Effort, bypass flag and `CLAUDE.md` fallback given again, as Codex keeps none of them. An interrupt sends a Codex session SIGINT, the only signal Codex stops cleanly on, before the SIGTERM and SIGKILL every session gets, each 10 seconds apart. Progress lines come from Codex's `item.*` events, the session-ended line gives its token totals, as Codex reports no turns or cost, and the final message is the last `agent_message`. A session whose turn fails (`turn.failed`), or that exits non-zero, fails the Ticket run with Codex's error in the cause; an `error` event Codex retries does not. Each Codex session adds a `trust_level = "trusted"` entry for the repository to `~/.codex/config.toml`, and your `~/.claude/` instructions and hooks don't reach it.

With `harness agy`, every session runs Google **Antigravity CLI** as `agy -p --dangerously-skip-permissions --output-format stream-json`, with `--model` and `--effort` when set, stdin null, and `AGY_CLI_DISABLE_AUTO_UPDATE=true` (agy ignores `=1`). The sandbox is opt-in and is never enabled, so unattended sessions can write the shared `.git` ([ADR-0013](adr/0013-every-harness-runs-unattended-and-fully-trusted.md)). Factory skills are linked into `.agents/skills/`, excluded from git, and loaded by the `/thirdshift-<skill>` first line. agy reads project `AGENTS.md`, `GEMINI.md` and `.agents/rules/`, and user instructions under `~/.gemini/`; it does not read `~/.claude/`. When root `CLAUDE.md` exists and neither `AGENTS.md` nor `GEMINI.md` does, thirdshift adds a `GEMINI.md` link to `CLAUDE.md` and excludes it through `.git/info/exclude`; existing instruction files always win.

agy's `init` supplies the conversation id and `step_update` supplies tool progress. The final `result` supplies the final message, success/error status, error and cumulative usage (including on Resume). `ERROR` or a nonzero exit fails the session with agy's error. A Resume uses `--conversation <id>` with the same flags, Model, Effort and environment; an interrupt sends SIGTERM, then SIGKILL after the grace period. Its start-up can be slow and variable: the research measured 8–95 seconds. No speed assumption or backend retry is made.

Before any work on agy, a named Model or Effort is checked against the free `agy models` catalog, which lists IDs, labels and effort-suffixed variants. IDs, display labels and bare aliases are matched regardless of case, as are Efforts. `model Gemini-3.8-Flash effort Medium` becomes `--model gemini-3.8-flash --effort medium`; `model gemini-3.8-flash-medium` already carries its Effort. A bare alias requires an Effort; a Model or Effort the catalog lacks fails naming valid choices. This check uses null stdin and disables self-update too, without a model turn.

With `harness grok`, every session runs `grok -p <prompt> --always-approve --sandbox off --output-format streaming-messages-json` with stdin set to null and with `GROK_DISABLE_AUTOUPDATER=1` and `GROK_FOLDER_TRUST=0`. The approval flag and disabled sandbox are always explicit, overriding the CLI’s own config and `GROK_SANDBOX`. No repository needs a one-time `grok --trust`: the trust environment override allows its project instructions and skills to load in the Ticket run's worktree. The Factory skills are linked into `.agents/skills/`, excluded from git, and the prompt loads its skill as `/thirdshift-<skill>` on its first line. A Resume repeats the flags and environment with `-r <session id>`. Grok documents no stream notification proving that background work was killed at exit, so thirdshift does not infer an automatic Resume from its tool activity. Grok's Messages stream shares Claude's event shapes, with Grok's tool names and errors; it supplies progress, the session id, final message, turns and cost, or token usage when cost is unavailable. Grok starts shell commands in their own process groups, which thirdshift's tree-stop also stops; the CLI gets SIGTERM before SIGKILL.

Grok reads both `AGENTS.md` and `CLAUDE.md` when both exist, along with `AGENT.md`, `CLAUDE.local.md`, `.claude/CLAUDE.md` and project rules under `.grok`, `.claude` and `.cursor`. It also reads `~/.grok/rules`, `~/.claude/CLAUDE.md` and `~/.claude/rules`; thirdshift writes none of these user-level files. Grok Build uses `~/.grok` for sign-in and sessions (or `GROK_HOME` when set).

Interrupting a Command stops every session's process tree on every Harness, including commands that started their own process group or session. The same owner supervises all Model/Effort checks, from Command startup and terminal Setup: interruption stops the check and its owned commands, even after the CLI exits while its captured output pipes are still open. A cancelled Command check ends before any Claim, worktree or Command log. thirdshift snapshots the descendants with `ps` on Linux and macOS, sends each process group the Harness's stop signals with a 10-second grace period each, then takes another snapshot before SIGKILL to catch commands started in the meantime. If the CLI exits on an earlier signal, thirdshift kills the remaining commands immediately without another grace period.

Before any work, where an Origin match failure would stop it, and so before the Claim, the worktree and any Command log, a command checks what it chose: the Harness's CLI must be on `PATH`, else it fails naming the CLI and the setting that chose it; and on Claude a named Model gets a minimal test call, `claude -p --model <model>` with the Effort, if any, which must succeed, else the command fails with what Claude said. No test call is made when no Model is named. Claude Code and Codex first check sign-in with the non-billed `claude auth status` and `codex login status`; a signed-out status fails before any Model test call. For other Harnesses without a non-billed sign-in check, sign-in is left to the session; the Security review startup fallback covers those failures. On Codex a named Model and Effort are checked against `codex debug models`, which costs no tokens: each matches regardless of case, a Model by its slug or its display name, and becomes the name Codex takes, so `model GPT-6.1-Sol effort Max` reaches Codex as `gpt-6.1-sol` and `max`. An unknown Model, or an Effort that Model doesn't support (with no Model, one no Model supports), fails naming the valid choices. On Grok Build, `grok models` lists the available Models without a model call and refreshes `models_cache.json` under `GROK_HOME` or `~/.grok`; thirdshift reads that cache for each Model's supported Efforts. Both are matched regardless of case and passed as Grok names them. With no Model named, a named Effort is checked against Grok's current default Model. Unknown Models and unsupported Efforts fail with the valid choices, and an unreadable catalog fails before work. The check has the same null stdin and fixed environment as sessions. What the command records is what the check settled on. A Pass makes these checks only once it has decided not to skip, and a Spec run makes them once for all its Tickets.

With `harness muse`, every session and Resume runs `muse exec --json --yolo`, with stdin null and `MUSE_NO_AUTO_UPDATE=1`. `--yolo` disables approvals and the sandbox and trusts the worktree ([ADR-0013](adr/0013-every-harness-runs-unattended-and-fully-trusted.md)). Muse reads the project's `AGENTS.md`, falling back to `CLAUDE.md`, and also reads `~/.claude/CLAUDE.md`. The Factory skills are linked into `.agents/skills/` and kept out of git. The prompt's first line tells the model to load its `thirdshift-<skill>` with its skill tool. If the stream shows no `read_skill` load of that skill, thirdshift writes a warning to the progress lines and Command log; it neither fails nor retries the session.

Muse's named Model is checked against its cached catalog under `~/.local/share/muse/model-catalog/` when present, matching its id or display label regardless of case. Without that cache, thirdshift makes a minimal test call with the same flags and environment. Effort is checked locally before any call: `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` or `ultra`. Model and Effort reach Muse as `--model` and `--reasoning-effort`. Setup proposes `muse-spark-1.3`; leaving the Model unset uses Muse's own default, `muse-spark-1.3-contributor`, which allows content to be used for product improvement.

Muse's session id comes from `stream.id`, and a Resume uses `--session-id <id>` with every flag and environment override again. Progress lines name session starts, tools and errors. After exit, thirdshift reads the last reply and token totals from Muse's own `sessions/<year>/<month>/<day>/<id>/session.jsonl`. Architecture reviews see that last reply alone, rather than Muse's terminal event's joined replies. A missing or unreadable log falls back to the stream text with no usage. `run.terminal.failed` or a non-zero exit fails the Ticket run with Muse's error. An interrupt sends SIGTERM, then SIGKILL after ten seconds if needed.

With `harness opencode`, every session, Resume and preflight check runs `opencode run --standalone --format json --auto`, with the prompt on stdin and `OPENCODE_DISABLE_AUTOUPDATE=1`. Standalone starts a private server in the session's process tree and adds about 2–3 seconds per session; it keeps the work and its commands under thirdshift's interruption handling rather than in the shared service ([ADR-0013](adr/0013-every-harness-runs-unattended-and-fully-trusted.md)). OpenCode reads project `AGENTS.md` and `~/.config/opencode/AGENTS.md`; it never reads `CLAUDE.md` or `~/.claude/CLAUDE.md` itself. When root `CLAUDE.md` exists and `AGENTS.md` does not, thirdshift links `AGENTS.md` to it and excludes that link from git. Factory skills are linked into `.agents/skills/`, and the prompt tells the model to load its skill with the skill tool. A missing successful `skill` load produces a warning without failing or retrying the session.

OpenCode takes the Model id from your provider config, as `<provider>/<model>`, including MiMo-V2.6-Pro when your provider routes it. Model and Effort are passed together as `-m <provider>/<model>#<effort>`; an explicit Effort replaces a variant already in the Model id, and needs a Model set too. Before any work, thirdshift makes a minimal standalone test call with the same stdin, flags and environment. `provider.no-route` identifies a bad Model or Effort without a model turn; a successful check uses a model call because OpenCode has no free catalog without its service. Setup checks its answers the same way, proposing no default Model.

The session id comes from `sessionID` in the stream, and a Resume uses `-s <id>` with every flag and environment override again. OpenCode creates a new session for an unknown id, so thirdshift only resumes an id it read from the stream. Progress comes from `step_start`, `tool_use`, `step_finish` and `error`. After exit, thirdshift reads `opencode session export --standalone <id>`: an exported `failed` outcome fails the Ticket run even with exit 0, the final message is the last assistant message's last text part, and usage and cost are summed over assistant messages to exclude the automatic title call. Input totals include cache reads and writes, output totals include reasoning, and each subtotal is shown. If export fails or is unreadable, the last stream `text` supplies the final message and no usage is shown. A stream error or nonzero exit also fails the Ticket run with OpenCode's error. An interrupt sends SIGTERM through the whole process tree, then SIGKILL after the grace period if needed.

The Command log's opening lines (`sessions run on claude · claude-opus-5-5 · high`), the Activity log's start line, the pull request's body and the Run notification each name the Harness, Model and Effort, `default model` or `default effort` where the Harness's own is used. In the pull request's body, the Spec PR's included, thirdshift writes the line itself once the opening session has ended, such as `Built with claude 2.1.11 · claude-opus-5-5 · high`, replacing any it wrote there before. The Harness version comes from the same detection as its Session trace: stream metadata when available, otherwise its version command, shared across the Command's sessions and child Runs. An unavailable version is `unknown`. Older Built-with lines without a version still read.

If a Repair or Base fix reports a different Harness version, thirdshift refreshes the open pull request's line before Self-merge so the Run notification matches it.

### Email

thirdshift sends email itself, with one HTTPS request to [Resend](https://resend.com)'s API, so it needs no mail server on the machine and works where SMTP ports are blocked ([ADR 0005](adr/0005-run-notifications-through-resend.md)). It needs a Resend account and an API key:

- **The API key** comes from the **`RESEND_API_KEY`** environment variable when it is set and not empty, and otherwise from the **Credentials**, `~/.thirdshift/credentials.toml`, a file only you should be able to read (mode 600):

  ```toml
  [resend]
  key = "re_..."
  ```

  `thirdshift setup` asks for the key, hidden, and saves it there for you. The environment variable wins, so a CI secret or an `export` overrides the saved key for one command. The Credentials are what let a Ticket run started from cron, `nohup` or an agent's shell find the key, since those read no shell profile. thirdshift never reads the key from the User config, so that file holds no secret.
- **`email.to`** in the [User config](#user-config) is the address email goes to when the command gives none.
- **`email.from`** is the sender. Without it, email comes from **`onboarding@resend.dev`**, Resend's shared sender, which only delivers to the address of your own Resend account. To send to any other address, set `email.from` to an address on a domain you have verified with Resend.

To check the setup without starting a Ticket run:

```sh
thirdshift email-test you@example.com   # or just `thirdshift email-test`, to send to email.to
```

It sends one test email, whose subject marks it as a test and whose body names the host, the time and the sender. Before sending, it checks that it has an address (the argument, else `email.to`) and a key (`RESEND_API_KEY`, else the Credentials); if either is missing, it exits `1` naming what's missing and sends nothing. With no key anywhere, it lists every way to give one:

```
no Resend API key. Either:
  - run `thirdshift setup`, or
  - add it to /home/you/.thirdshift/credentials.toml (mode 600):
        [resend]
        key = "re_..."
  - or set RESEND_API_KEY in the environment the Ticket run starts from
    (a crontab line, CI secret, or a shell profile the Ticket run's shell reads)
```

Setup validates the Credentials before asking questions on a terminal. For email, `email-test`, the Console and a Run or a Pass that asks for a notification read them only when `RESEND_API_KEY` is unset or empty. A missing file just means no key from it. The file accepts quoted `resend.key`, `claude.setup_token` and `opencode.api_key` values. Invalid TOML, unknown sections or keys (such as `kye`), and values of other types stop the command with exit `1`, naming the file and the offending key, and nothing is sent. An OpenCode Trial's provider key goes there by hand until `setup` asks for it, as `api_key` under `[opencode]`; without it, an OpenCode Trial fails before its session starts. A Grok Trial signs its own Trials home in once, with `GROK_HOME=~/.thirdshift/trials/grok grok login`; without that login, a Grok Trial fails before its session starts. One that others can read is still used, with a `warning:` line on stderr saying to `chmod 600` it. Nothing is sent to check the key itself. When Resend accepts the email, it prints `accepted by Resend; check your inbox` and exits `0`; that is all it can verify, so check that the email arrives. When Resend refuses it, for example for a bad key or a sender it won't send from, it prints Resend's error text word for word, with where the key came from, and exits `1`. It gives up after 30 seconds without an answer.

#### Run notifications

`--email` (or `email`) asks a Ticket run for a **Run notification**: one email, sent when the Ticket run ends, whatever the outcome. The word after the flag is the address only if it contains `@` and doesn't start with `https://`, so the Issue URL is never taken for it; otherwise the email goes to `email.to`. With `email.always = true` in the [User config](#user-config), a Ticket run asks for one without the flag, and `--no-email` (or `no-email`) skips it for that Ticket run; an address after `--email` still wins over `email.to`. Giving a flag twice, or any two of `--email`, `--email-unless-merged` and `--no-email`, is an argument error.

`--email-unless-merged` (or `email-unless-merged`), optionally followed by an address as `--email` is, asks for a Run notification that is emailed only when the command did not end `merged`, whatever else it ended as. A merged one is still saved beside the [Command log](#logs), with nothing sent, unless it is a [Security run](#security-runs) whose fix left its finding open or whose audit left findings waiting for the Day shift: that one is emailed. It makes the same checks as `--email` before any work.

A Ticket run that asks for a notification, by either flag or by `email.always` or `email.unless_merged`, makes the same checks as `email-test` before any other work: an address is known, and a key is found, in `RESEND_API_KEY` or else the Credentials. If either fails, or the Credentials are broken, the Ticket run stops, exits `1` naming what's wrong (with no key, the message above listing every way to give one), and sends nothing. A Ticket run that asks for no notification never reads the Credentials, so a broken file can't stop it, and `update`, `version` and `help` never read it either. Once they pass, every way the Ticket run ends sends exactly one notification, after its outcome is final and its cleanup done: ready for review, merged, a [Failed ticket run](#failed-ticket-runs) (including a later preflight failure such as an origin mismatch), or interrupted by Ctrl-C, SIGTERM or a closed terminal.

- **Subject**: `[thirdshift] <owner>/<repo>#<n> <issue title>: <outcome>`, where the outcome is `ready for review`, `merged`, `failed` or `interrupted`. The title is left out if it can't be read from GitHub.
- **Body**, plain text: a `Result:` line with the same outcome as the subject, so it is readable even when a long issue title hides the end of the subject; immediately followed by one `Issue:` line with the full original Issue URL, including after failure or interruption with no pull request, and a `Title:` line with the issue title when it was read; then the pull request URL (if any), the failure cause (if failed), after an Inherited failure with no Base fix taken its [`Base check:`, `Retry with:` and `Or set:` lines](#an-inherited-failure-links-the-base-branchs-checks), the session trace path (if any), the same one stderr gives, the [Command log](#logs) path as a `Command log:` line, a `Sent by:      thirdshift` line, the hostname and how long the Ticket run took. The body tells everything the subject does, so copying the body alone is enough.

The email's `Built with` line carries the same Harness version as the pull request's line, names the command's configured Harness, Model and Effort and is marked `(configured)`. A `Session models` section lists sessions whose Harness was launched, including failed or interrupted sessions, Resumes and child Ticket runs belonging to that Command. Models evidenced by the Harness's output or retained records are marked `(reported)`, including model switches and delegated Claude answers. When the Harness supplies no model evidence, the requested Model is labelled `(requested; actual model not reported)`, or its default is labelled `Harness default (actual model not reported)`. A listed session proves the Harness was launched; an unconfirmed request does not prove that Model executed. Effort is the session's configured Effort; it does not describe a delegated answer's Effort. Attribution records (`*.models.json`) are kept beside the Session traces; a failure to write or read them is a warning and never changes the Ticket run's outcome.

A [Spec run](#spec-runs) sends at most one notification for the whole Spec, under the same rules, with its checks made once before any Ticket starts. Its subject names the Spec, its `Issue:` line links the parent Spec rather than a Ticket or pull request, its outcome is the Spec run's, and its body, after what a Ticket run's holds (the Spec PR, if any), lists each Ticket's outcome, one line per Ticket as in the summary on stderr, such as `#21 landed with https://github.com/acme/widgets/pull/1` or `#22 blocked by #21`. The Ticket runs inside it never send a notification of their own, whatever the User config says.

An [Architect run](#architect-runs) takes the same flags and the same `email.always` and `email.unless_merged`, and sends [one notification](#one-run-notification) covering its review and the run it dispatched.

A [Pickup run](#pickup-runs) takes the same flags and the same `email.always` and `email.unless_merged` too. One that took an issue sends [one notification](#one-run-notification-for-the-issue-taken), the one the run it dispatched would have sent by hand, and a skipped one sends none.

A notification that can't be sent is a `warning:` line on stderr with Resend's error. It never changes the Ticket run's outcome, stdout or exit code.

### Stopping a Command

`thirdshift stop widgets#7` stops a live Ticket run and waits for its cleanup. It accepts
the same Command names as `show`: `owner/repo#number`, an Issue URL, or a Pass's
`kind-stamp` name. It works from any directory and never prompts. Ended and died
Commands are refused; Stop sends SIGTERM only to the recorded process, with no
forced kill.

Stopping a Spec run ends its Tickets' runs too, saving their unfinished work.
Only the Spec's issue receives Stop labels; its Tickets' labels stay as they
were. A Stop on a Pickup run applies to the issue it took, whether a Ticket run or a
Spec run. A Pass still without an issue, such as an Architect run in its
Architecture review or a Security run in its audit, ends without changing
labels. Stop refuses a Ticket's issue and names the Spec run to stop instead.

By default, Stop leaves the issue for the Day shift: it adds `needs-triage` and
does not put `ready-for-agent` back. It removes `in-progress` when nothing reached
`origin`, and keeps it when an Issue branch or pull request remains. Add
`--requeue` to release the Claim and put `ready-for-agent` back even after a push;
the next Ticket run continues that Issue branch. Ordinary interruption without a Stop
keeps its usual Claim rules.

The Stop request is kept beside the Command record with the Unix user who asked
and whether to requeue. The Command writes `<subject> stopped from the CLI by
<who>` to its Activity log before its `ended:` line. Stop prints `still ending`
each minute while cleanup runs, then the outcome and any Issue branch or pull
request left behind. Until then, `thirdshift status` and the Console's Inbox show
the Command as `Stopping`.

### Dismissing a row

A dismissed row's Resolved retention runs from its dismissal. A later GitHub settlement or Command does not restart that clock. Taking the dismissal back restores the other Inbox rules.

`thirdshift dismiss widgets#7` gives a row in Needs you or Ready for review a
**Dismissal**: the row leaves that group, and nothing else changes. It accepts
the same Command names as `show`: `owner/repo#number`, an Issue URL, a Pass's
`kind-stamp` name, or a record's path. Any row in either group can be dismissed,
live ones included: a dismissed live row moves to Working or Landing with its
health line still showing, and its Ticket runs keep running. An ended one goes to
Done, Resolved or History by its age, and Resolved holds it for `inbox.days`
after the dismissal. Dismissing a collapsed failure dismisses every Command it
stands for, and a later failure with that cause starts a row of one. Add
`--undo` to take a dismissal back wherever the row now is. A dismissal holds
until the row turns into a kind its dismissal has not covered: Slow becoming
Silent or Silent becoming Slow brings it back, as does a live Command dying,
failing or ending ready for review. A dismissed Security failure comes back as a
Finding while its Command record says findings wait. Dismissing again adds that
kind to the dismissal. Each kind counts once for the whole row: a Silent Ticket
run that logs and falls Silent again stays dismissed, as does a Spec run when a
second Ticket goes Silent. Its row in Working still says it was dismissed.

Dismissing a row outside Needs you and Ready for review is refused, as is a row
with no Command record and `--undo` on a row that isn't dismissed. The dismissal
is kept beside the Command record with who asked, when, whether from the CLI or
the Console, which of the record's rows it covers, and every kind it covered. The
Command, its issue, labels, Claim and pull request stay as they are, and nothing
is written to GitHub. The Activity log records each dismissal and each one taken
back, naming the Unix user and the CLI. `dismiss --json` reports what it did.

In the Console, any signed-in account can click **Dismiss** in a waiting row's
Command details, beside Stop when available. One click dismisses, with no
confirmation page. A dismissed row offers **Undo** there. While either request
is pending, Command details shows “Dismissing…” or “Taking back dismissal…”
and disables the button, including through live updates. A failed request shows
the error and lets you retry. Each button shows its CLI form beside it. Both
posts require a session and a valid CSRF token, open
pages update together, and the Activity log names the account's email and the
Console.

### Logs

Export a Command's whole **Trace** for another tool with:

```sh
thirdshift logs acme/widgets#7 --export ./trace
```

This writes `spans.jsonl` and `logs.jsonl` in the directory, creating it if
needed. Both use OpenTelemetry's OTLP JSON Lines format: one export request
per line, with spans and log records in separate files. The spans include the
Command, all its child Ticket runs and their stages, using the Command record's times,
plus every retained Session trace's spans. The log records come from every
Session trace in that Command. All share one trace ID, and every span's parent
is included. Naming a child Ticket run exports its whole owning Command too.

The export stays on the machine; nothing is sent anywhere. Existing export
files are never overwritten. A Command record is required. Missing files, malformed
requests or missing parent spans fail the export before the output files are
published. Wait for a live Command to finish before exporting its complete
Trace; unfinished record intervals retain an unset end time rather than an
invented completion time.

Everything thirdshift logs goes under `~/.thirdshift/logs/`, or the `logs.dir` set in the [User config](#user-config), in a folder per repository, `<owner>/<repo>/`, named for the GitHub repository rather than your checkout, so a fork and its upstream never share one and every checkout or worktree of a repository logs to the same place. thirdshift creates each folder as it needs it, so a repository's first pass needs nothing made by hand:

```
~/.thirdshift/logs/<owner>/<repo>/
├── activity.log              the Activity log
├── audits/                   each Security audit's report, with its evidence
├── security-reviews/         each Security review's private report of old findings
└── commands/
    ├── architect/            <stamp>~<token>.log and Session traces
    ├── pickup/               <n>-<stamp>~<token>.log and Session traces
    ├── secure/               <stamp>~<token>.log and Session traces
    └── issue/                <n>-<stamp>~<token>.log and Session traces
```

The folder already names the repository, so no file name repeats it. `audits/` and `security-reviews/` hold vulnerability evidence, so keep the logs private (see [Security runs](#security-runs)). A **Session trace** records one session's events and timed work in two OTLP JSON Lines files: `.logs.jsonl` holds a log record for every Harness event as it arrives, including its raw output; `.spans.jsonl` holds the Session, model-call and tool-call spans as they end. Sessions write these files, Model evidence and any review reports; they no longer write the raw Session log. All of a Command's Session traces share its **Trace** identity, including child Ticket runs:

```
~/.thirdshift/logs/<owner>/<repo>/commands/issue/<n>-<stamp>~<token>-implement.logs.jsonl
~/.thirdshift/logs/<owner>/<repo>/commands/issue/<n>-<stamp>~<token>-implement.spans.jsonl
~/.thirdshift/logs/<owner>/<repo>/commands/issue/<n>-<stamp>~<token>-repair-<i>.logs.jsonl
~/.thirdshift/logs/<owner>/<repo>/commands/architect/architect-<stamp>~<token>-architecture-review.logs.jsonl
```

Session traces share the owning command's folder, stamp and owner token: a Pickup run's dispatched Ticket run logs in `commands/pickup/`, an Architect run's in `commands/architect/`, and a Security run's in `commands/secure/`. Tickets and nested Base fixes inherit that same ownership too.

When a Command that produces a [Run notification](#run-notifications) ends, its subject and body are saved beside its Command log as `<Command log stem>.notification.txt`, whether or not email is enabled. For example, `7-20261003T120000-0400~aB3cD4eF5gH6iJ7k.log` has `7-20261003T120000-0400~aB3cD4eF5gH6iJ7k.notification.txt` beside it. The file starts with `Subject: <subject>`, then a blank line and the exact plain-text email body. A Pickup run writes one for the issue it took, and a Spec run writes one covering its Tickets; dispatched Ticket runs, Ticket runs inside a Spec run and Base fixes write none of their own. A skipped Pass writes none. A failed write produces a `warning:` line and leaves the outcome unchanged. These files are not included in `thirdshift logs` or the Console's log inventory.

A Resume is logged as its session's kind plus `-resume`, e.g. `implement-resume.logs.jsonl`.

A **Command log** is everything one command printed, stderr and stdout in the order printed, while the terminal still shows all of it. Its folder is the command typed: `thirdshift <Issue URL>`, a Ticket run or a Spec run, in `issue/`, `thirdshift pickup` in `pickup/`, named for the issue it took, and `thirdshift architect` in `architect/`. A Spec run's covers its Tickets' runs, a Ticket run's covers its [Base fix](#base-fix), and a Pass's covers the Spec run or Ticket run it dispatched: none of those keeps one of its own. So `ls -t ~/.thirdshift/logs/acme/widgets/commands/pickup/*.log | head` lists the recent passes on acme/widgets that took an issue. A command keeps its Command log once it starts work: a Pass skipped before doing any work keeps none, nor does a Ticket run that fails before it starts work, as on the [Origin match](#what-a-ticket-run-does), and `setup`, `email-test`, `update`, `version` and `help` never keep one. Lines printed before the Command log's name is known, such as a Pickup run's lines on the issues it [passed over](#why-an-issue-was-passed-over) before it took one, are written first, and once it is created a progress line says `logging this command to <path>`. A Command log that can't be written produces one `warning:` line on stderr. A failure confined to the Command log leaves the outcome, stdout and exit code unchanged; if the shared folder also prevents writing a Session trace, the Ticket run fails before starting that session.

The stamp is the local time the command started, with its UTC offset, as in `20261003T120000-0400`, in the machine's time zone, or `TZ`'s if set, so it agrees with `date` and `ls -l`. A command's Command log and all of its Session traces, its Tickets' runs' and its Base fix's included, share that one stamp, so they sort together and each can be found from the other. When a Ticket run fails, stderr ends with the path of the Session trace of the session its cause came from, the place to start looking, then that of its Command log. A failed implement session, Spec review or Repair names its own log, as does a [Security review](#security-review) whose hold is reported only after later Repairs. A cause that came from no session, such as red CI with the Repairs used up, a Declined CI fix, the upstream-move budget, a Harness check or a git or GitHub error, names the most recent Session trace. A Spec run whose Tickets are not done names the Session trace of the one Ticket whose Ticket run failed, and none if several did, as no single session caused it; each failed Ticket's line gives its own. The Command log's closing `session trace:` line and the [Run notification](#run-notifications) give the same path.

Command logs and Session traces insert `~<token>` immediately after the stamp. The random token gives each Command its own storage even when start times agree; its Command record uses the exact Command-log stem followed by `.command.json`. Model sidecars (`.models.json`), review reports (`.standards` and `.spec`) and review summaries (`.review-summary.json`) use the Session stem before `.logs.jsonl`, for example `7-<stamp>~<token>-implement.models.json`. The Command record names the log-records file; Silent watches that file grow, and the Waterfall reads the trace spans. Human Command names stay the same, and start stamps remain display and grouping facts. A matching stamp alone does not attach a tokenless Session to a new Command: a record reference or exact Model evidence must establish that ownership. Converted traces and older Command records remain readable without renaming; a record naming the former raw file resolves to its log-records file. Readers no longer accept the old Session-log format, and the one-time `logs --convert` and `--remove-converted` commands have been retired. A failed Command-log writer retains its allocated owner; if allocation itself failed, a Session fails with that cause instead of choosing a shared timestamp-only filename.

The code review writes `summary.json` beside its reports with finding counts by Standards and Spec, including findings later fixed or declined. At Ticket run completion, the review's originating Session stage keeps these in its Run metrics as `review_findings`, with `standards` and `spec` counts. Missing or invalid summaries are `null`, and an unavailable axis is `null` rather than zero. A Resume updates that same review's summary; another review keeps its own counts. `show --json` reads these retained counts without parsing report prose. A release changing this Factory skill needs an Experiment once #977 exists.

The **Activity log**, `activity.log`, is a short running record of what the factory did on the repository ([ADR-0011](adr/0011-thirdshift-keeps-a-per-repository-activity-log.md)). A Ticket run, a Spec run or a Pass writes a line when it starts work, naming its Command log, and one when it ends, with its outcome; a Ticket run you type by hand writes them too, and the run a Pass dispatched, a Spec run's Tickets and a Base fix write none of their own. A skipped Pass writes a line only when its reason differs from the last line of its own kind, so a repository that sits idle shows one line, not one per pass, and the first skip after a pass that did work always shows: that is when the repository went idle. Every line starts with the local date and time:

```
2026-10-03 02:00:01 Pickup run skipped: no Ready issue on acme/widgets
2026-10-03 09:30:01 Pickup run #41 started: commands/pickup/41-20261003T093001-0400~aB3cD4eF5gH6iJ7k.log
2026-10-03 09:52:17 Pickup run #41 ended: PR https://github.com/acme/widgets/pull/42 is merged
2026-10-03 10:00:01 Pickup run skipped: no Ready issue on acme/widgets
```

Each line is appended whole, so passes that run at once on one repository never garble it. One that can't be written is one `warning:` line on stderr, and the command carries on as for a Command log. thirdshift never rotates it: collapsed skips keep it small. Existing logs, including Session traces in `<owner>/<repo>/sessions/` and logs from before the per-repository layout, are not moved.

With `quiet_skips = true` in the `[activity]` section of the [User config](#user-config), a skipped Pass prints nothing on stdout or stderr, its dated first line included, and leaves only its Activity log line. A pass that does work, or fails, prints as ever, so a scheduler's log file catches only what went wrong. Without it, a skipped pass prints as it always has, for a pass you type by hand.

## Foreign commits in a Ticket run that Self-merges

<a name="foreign-commits-in-a-merge-run" id="foreign-commits-in-a-merge-run"></a>

A Ticket run that Self-merges merges only code an agent wrote or reviewed. If someone else pushes to the Issue branch during the Ticket run, their **Foreign commits** are reviewed before they can be merged:

1. Each round of step 6 starts by fetching the Issue branch from `origin`. Any new commits there are merged into the Ticket run's branch, by fast-forward or a merge commit, never a rebase, each logged on stderr as `merging new commit <sha> from origin/<branch>`. Foreign commits that arrive while CI runs on a green head send the Ticket run round again rather than on to the merge. A merge that conflicts with the Ticket run's own work gets a conflict Repair.
2. A **review Repair** then runs `/thirdshift-code-review` with the head thirdshift last knew as the Ticket run's own as the fixed point. The agent fixes the findings it agrees with, adds the rest to the pull request body's "Unaddressed findings" section marked as coming from the Foreign commits, and pushes.
3. The round goes on as usual: the Base branch is merged in, CI watched, and the merge tried once the head is green.

Review Repairs are logged as `repair-<i>`, numbered with the other Repairs, count against the cap of 5 Repairs, and get a Resume like any session. A round that picks up Foreign commits counts against the same 5, like a Base branch move. A Ticket run that spends either budget fails.

A Ticket run without `merge` does none of this: it never fetches someone else's commits into its branch.

## Base fix

A Base branch whose CI is broken fails every Ticket run on it with an Inherited failure. To have a Ticket run fix the Base branch itself and carry on, give it `base-fix` (or `--base-fix`), before or after the URL:

```sh
thirdshift base-fix https://github.com/acme/widgets/issues/7
```

To allow it for every Ticket run on a machine, set `base.fix` in the [User config](#user-config); `no-base-fix` (or `--no-base-fix`), before or after the URL, then forbids it for one Ticket run. Giving either flag twice, or `base-fix` together with `no-base-fix`, is an argument error.

When the Ticket run's only red checks are Inherited failures and the Base branch has not moved since, a Ticket run allowed to then starts a **Base fix**, at most one per Ticket run ([ADR-0008](adr/0008-inherited-failures-fail-the-run.md)):

1. thirdshift looks for an open issue labelled `base-fix`, other than the Ticket run's own, whose title names the same Base branch and every one of those checks. If there is one, another Ticket run's Base fix is already under way, and the Ticket run waits on that one instead: see [One Base fix per Base branch](#one-base-fix-per-base-branch).
2. Otherwise, thirdshift opens an issue from a fixed template, with no agent session: titled `CI red on <base>: <check>[, <check>…]`, its body naming the checks with their URLs on the Base branch, the Base branch and its short sha, and the Ticket run's pull request, labelled `base-fix` and `ready-for-agent`. A label the repository lacks is added to it first.
3. It starts a child `thirdshift` on that issue from the same Launch directory, as a Ticket run that Self-merges into the Ticket run's Base branch, whatever the Ticket run's own goal. Its progress lines are relayed with a `#<n>: ` prefix, after `starting Base fix #<n> into <base>: <issue URL>` and `waiting on Base fix #<n>`. The Base fix sends no Run notification, makes no [Claim](#the-claim) on its issue, treats every red check as its own to fix rather than as an Inherited failure, and never starts a Base fix of its own.
4. Once the Base fix has merged, and its Self-merge has closed its issue, the Ticket run merges the Base branch in again and watches CI, and goes on as usual: ready for review, or merged in a Ticket run that Self-merges.

If the Base fix fails, the Ticket run is a [Failed ticket run](#failed-ticket-runs) with the cause `Base fix <issue URL> failed: <its cause>`. If the checks are still Inherited failures once the Base fix has merged, there is no second one, and the cause is `CI red on <check>, which also fails on <base> at <short sha>, even after Base fix <issue URL> merged; fix <base> first`. The Ticket run's last progress lines include `Base fix: <issue URL> merged`, or `not merged`, and its own [Run notification](#run-notifications) has the same `Base fix:` line.

On a Spec, `base-fix`, `no-base-fix` or the User config's `base.fix` holds for each Ticket run, whose Base fix goes into the Spec branch, its Base branch, and for the Spec PR, whose Base fix goes into the Base branch. Tickets running at once that meet the same Inherited failure share one Base fix: the first starts it, and the others wait on it. A Ticket run sends no Run notification, so the Spec run reports its Base fix: a Ticket that landed after one has `#<n> landed with <PR URL>, after Base fix <issue URL> merged`, or `closed` for one it waited on, as its line in the Tickets checklist and in the Spec run's Run notification, and one whose Base fix failed has the cause naming the issue.

### One Base fix per Base branch

A Ticket run that finds an open Base fix issue for its Base branch and checks starts none of its own. After `waiting on Base fix #<n>, already open: <issue URL>`, it waits for that issue to close, as the Base fix's Self-merge leaves it, then merges the Base branch in again and watches CI. Waiting is the Ticket run's one Base fix: if the checks are still Inherited failures once the issue has closed, the cause is `CI red on <check>, which also fails on <base> at <short sha>, even after Base fix <issue URL> closed; fix <base> first`, and its Run notification's `Base fix:` line says `closed`, or `not closed` if the Ticket run ended first.

An issue that covers more checks than the Ticket run's counts; one for another Base branch, or that leaves out one of the Ticket run's checks, does not. The wait has no time limit: a Base fix issue from another clone or machine that nobody is working on has to be closed, or the Ticket run interrupted, by hand.

Ticket runs from one Launch directory, as a Spec run's Tickets are, look for the issue and write it one at a time, under a lock in the clone's git directory, so those that meet the same Inherited failure at once get one Base fix issue and one Base fix. There, a Ticket run also knows whether a Base fix started from that Launch directory is still running:

- A Ticket run waiting on one that fails, leaving its issue open, fails too, with the cause `Base fix <issue URL> ended with its issue still open`.
- A Ticket run that finds the issue open after that Base fix has ended starts it again on the same issue, as its one Base fix, after `Base fix #<n> is open but no longer running; starting it again into <base>: <issue URL>`. It continues the Base fix's Issue branch, as any Ticket run on an issue with one does: see [Continuation](#continuation).

Across clones and machines the look is only a best-effort lock, and two Ticket runs that look at the same moment may each start a Base fix.

Without `base-fix` or `base.fix`, or with `no-base-fix`, an Inherited failure fails the Ticket run as described in [What a Ticket run does](#what-a-ticket-run-does), and the Ticket run says [where the checks fail on the Base branch](#an-inherited-failure-links-the-base-branchs-checks), offering a Base fix if nobody decided against one.

## Continuation

Running thirdshift again on an issue picks up where the last Ticket run stopped ([ADR-0002](adr/0002-existing-issue-branch-means-continue.md)). It looks at the highest-numbered Issue branch:

- **No Issue branch yet**: a fresh Ticket run creates `issue-<n>` from the Base branch.
- **The branch exists with no pull request, or an open one**: the Ticket run is a **Continuation**. It checks out that branch, and the agent builds on its commits instead of starting over, then creates the pull request or updates the open one. With an open pull request, that pull request's base is the Base branch, whatever you have checked out.
- **The branch's pull request was merged or closed**: finished work is never reopened. The Ticket run starts fresh on the next number, `issue-<n>-branch-2`, `-3`, and so on. A number counts as used even if GitHub deleted the branch after merging.

So you can retry a Failed ticket run with the same command, or start an issue on one server and continue it on another.

A Ticket run is not idempotent: re-running builds on whatever is already on the branch, including a Failed ticket run's work-in-progress commit.

## Failed ticket runs

<a name="failed-runs" id="failed-runs"></a>

A **Failed ticket run** is one that ends, including by Ctrl-C or a closed terminal, without an open pull request from its Issue branch that targets the Base branch, is mergeable and has passing CI, or, for a Ticket run that Self-merges, without that pull request merged. Causes include the session exiting non-zero, no pull request or one with the wrong base, running out of Repairs, CI red only on **Inherited failures** (`CI red on <check>[, <check>…], which also fails on <base> at <short sha>; fix <base> first`), a CI-fix Repair that finds nothing on the branch to fix (a **declined CI fix**: the head is unchanged after it, and its one Check re-run left CI red or could not happen), and a Base branch that keeps moving while CI runs, or in a Ticket run that Self-merges, an Issue branch that keeps getting Foreign commits.

A Failed ticket run:

1. Commits any uncommitted work as `thirdshift: failed ticket run (<reason>)`, with a timestamp and the hostname, and pushes the Issue branch, so nothing is lost. If the branch has no changes against the Base branch, nothing is pushed.
2. Converts its open pull request, if any, back to a draft, so a pull request only claims to be ready when the factory stands behind it. The next successful Continuation marks it ready again. The exception is a policy refusal of a Ticket run asked to merge: the pull request is ready, mergeable and green and only the Self-merge could not happen, so it stays ready for review, and no failure commit is pushed onto the head whose CI was watched. This includes **review required**: a review blocks the merge, so thirdshift skips the merge attempt and another Repair round. stderr and the Run notification say `PR <url> needs an approving review before it can merge; it is ready for review`, with `review required` as the notification outcome. The Ticket run exits `1`, prints the PR URL on stdout, keeps its Claim and cleans up as usual. The same applies to a Spec run's Spec PR.
3. Cleans up as usual, prints the reason to stderr and exits non-zero. If the push failed, the worktree and local Issue branch are kept instead, and stderr names the branch, its head commit and the worktree path, so you can recover the work or push it by hand.
4. Releases its [Claim](#when-the-claim-ends), unless it is a failed Security fix, if it left nothing on `origin`: no Issue branch and no pull request. Otherwise the issue stays `in-progress`.

Merges, never rebases or force-pushes: a branch worked on from several servers never loses history.

### An Inherited failure links the Base branch's checks

A Ticket run that fails on Inherited failures with no Base fix taken says more after its cause, on stderr and in its [Run notification](#run-notifications): a `Base check:` line for each of those checks, with its URL on the Base branch commit, as a [Base fix](#base-fix) issue lists them, or the name alone for a check with no URL. If nobody decided against a Base fix, with neither `base-fix` nor `no-base-fix` given and no `base.fix = true` in the [User config](#user-config), it also offers one: a `Retry with:` line with the command that starts the Ticket run again with `base-fix` added, keeping the `merge`, `email` and `parallel` flags it was given, and an `Or set:` line naming `base.fix`:

```
thirdshift: 03:12:40 CI red on test, which also fails on main at 362b9ca; fix main first
thirdshift: 03:12:40 Base check: test: https://github.com/acme/widgets/actions/runs/1/job/2
thirdshift: 03:12:40 Retry with: thirdshift https://github.com/acme/widgets/issues/7 base-fix
thirdshift: 03:12:40 Or set: base.fix = true in ~/.thirdshift/config.toml, to allow a Base fix for every Ticket run on this machine
thirdshift: 03:12:40 session trace: ~/.thirdshift/logs/acme/widgets/commands/issue/7-….logs.jsonl
thirdshift: 03:12:40 command log: ~/.thirdshift/logs/acme/widgets/commands/issue/7-….log
```

A Ticket run given `no-base-fix` gets the `Base check:` lines and no offer. A Ticket run that had its one Base fix gets neither: its `Base fix:` line already says what happened. The cause itself, and so the failure commit's message, is the same in every case, and no other cause adds a line.

In a [Spec run](#spec-runs), a Ticket run says this on its own stderr, relayed with its `#<n>: ` prefix, and the command it offers is the Spec run's, with `base-fix` added. The Spec run's summary and its Run notification show the Ticket's cause alone. After an [Architect run](#architect-runs), the command offered is `thirdshift <plan URL>` with the dispatched run's flags and `base-fix`, since another Architect run would start a new review rather than take that plan up again.

## Spec runs

A **Spec** is an issue with sub-issues, its **Tickets**. `thirdshift <Issue URL>` on a Spec is a **Spec run**: it works through the Tickets in the order their GitHub "blocked by" links allow, each **Ticket run** Self-merging into the **Spec branch**, then leaves one **Spec PR** from the Spec branch into the Base branch ready for review ([ADR-0006](adr/0006-spec-runs-merge-tickets-into-a-spec-branch.md)). An issue with no sub-issues is an ordinary Ticket run.

A Spec run makes the [Claim](#the-claim) on the Spec, as a Ticket run does on its issue: the Spec is labelled `in-progress`, in place of `ready-for-agent`. Its Tickets' runs change no label on their Tickets. The Claim [ends](#when-the-claim-ends) as a Ticket run's does: it is released if the Spec run fails with no Spec branch on `origin` and no Spec PR, except for a failed Security fix, removed once a Self-merge of the Spec PR has left the Spec closed, and kept otherwise. The Spec branch is pushed before the first Ticket starts, so a Spec run that got as far as its Tickets keeps its Claim.

The Spec PR opens as a draft as soon as the first Ticket lands, titled from the Spec, with `Closes #<spec>` and a Tickets checklist: one line per Ticket, ticked once it is done, with its pull request, or saying it is running, failed, blocked or unready. thirdshift rewrites the checklist between its `<!-- thirdshift:tickets -->` markers as each Ticket starts and ends, leaving the rest of the body as it is. Once every Ticket is done, the **Spec review** rewrites the body, and thirdshift puts the checklist back, appending it if the markers are gone, before marking the Spec PR ready.

The Spec review session reviews the whole Spec branch against the Base branch and the Spec. Once it is marked ready, the Spec PR goes through the same step 6 as a Ticket run's pull request, with the Spec as the issue and the Spec branch as the Issue branch: the Base branch is merged in (never rebased), CI watched, and a conflict or red CI handed to a Repair, within the same budgets, with the same Check re-run after a CI-fix Repair that leaves the head unchanged. Inherited failures are told apart the same way everywhere: the Spec PR's against the Base branch, and a Ticket's against the Spec branch, its Base branch, with the cause shown on the Ticket's line of the checklist.

Tickets always merge into the Spec branch, whatever the command or the User config says. `merge` (or `--merge`, or `merge.always = true` without `no-merge`) applies to the Spec PR alone: the Spec run ends with the Self-merge of the Spec PR into the Base branch, then deletes the Spec branch on `origin` and closes the Spec if the merge did not. Without it, the Spec run ends with the Spec PR ready for review. A Spec PR that fails from the Spec review on follows the [Failed ticket run](#failed-ticket-runs) rules for its pull request, back to draft unless only the Self-merge could not happen (a policy refusal), and the Spec run exits `1`.

A Spec run takes every Ticket it can reach. A Ticket runs once it is open, has every blocker closed, and is not an **Unready Ticket**: an open Ticket labelled `ready-for-human`, `needs-info`, `wontfix` or `needs-triage`. An open Ticket with no triage label is taken. A Ticket with sub-issues of its own is never run either, and is reported as unready. A blocker outside the Spec counts once it is closed. The graph is read again from GitHub whenever a Ticket run ends, so removing a label, adding a Ticket or closing one by hand takes effect in the same Spec run.

A Ticket whose Ticket run fails is not tried again in that Spec run, and stops only the Tickets it blocks; every other Ticket it can reach still runs. Nor do Unready Tickets, Tickets blocked by an open issue outside the Spec, or Tickets in a cycle of "blocked by" links run, nor any Ticket downstream of them.

Independent Tickets run at once, up to 3 by default. Whenever a Ticket run ends, the Spec run reads the graph from GitHub again and starts ready Tickets until the limit is reached. To change the limit for one Spec run, add `parallel <n>` (or `--parallel <n>`) before or after the URL; `spec.parallel` in the [User config](#user-config) sets it for every Spec run on the machine:

```sh
thirdshift parallel 5 https://github.com/acme/widgets/issues/20   # up to 5 Tickets at once
thirdshift --parallel 1 https://github.com/acme/widgets/issues/20 # one at a time
```

`parallel` followed by `0`, a negative number or anything but a whole number, or given twice, is an argument error (exit `2`). `parallel` on an issue with no sub-issues stops the Ticket run before any work, since there are no Tickets to run at once. The Tickets' runs share the Launch directory: they create their worktrees there one at a time, and a git command that finds a lock file held by another waits and tries again.

When nothing is left to run and any Ticket is not done, the Spec run is a **Failed spec run**: it leaves the Spec PR a draft, its checklist showing what's missing, prints its URL on stdout (if any Ticket has landed, so there is one), exits `1`, and lists on stderr each Ticket that landed, with its pull request, and each one not done, with why:

```
thirdshift: 03:12:40 #21 failed: claude exited 1 (session trace: ~/.thirdshift/logs/acme/widgets/commands/issue/21-….logs.jsonl)
thirdshift: 03:12:40 #22 blocked by #21
thirdshift: 03:12:40 #23 landed with https://github.com/acme/widgets/pull/1
thirdshift: 03:12:40 #24 unready: labelled needs-info
thirdshift: 03:12:40 #25 blocked by #99 (outside the Spec)
thirdshift: 03:12:40 #26 in a cycle: #26 blocked by #27 blocked by #26
thirdshift: 03:12:40 #27 in a cycle: #27 blocked by #26 blocked by #27
thirdshift: 03:12:40 Tickets not done: #21, #22, #24, #25, #26, #27
```

Running the Spec again picks up where the last Spec run stopped. The Spec branch is on `origin`, so the Spec run continues it ([ADR-0002](adr/0002-existing-issue-branch-means-continue.md)) and updates its draft Spec PR rather than opening another. Closed Tickets are done and not run again, and a Ticket that failed still has its Issue branch and draft pull request, so its Ticket run is a [Continuation](#continuation) into the Spec branch. With every Ticket closed, the Spec run goes straight to the Spec review and the Spec PR. With every Ticket closed and no Spec branch, as when the Spec was done some other way, it stops with `every Ticket is closed and there is no Spec branch; nothing to do` and exits `1`, creating no branch or pull request and leaving the Spec open.

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

`merge`, `no-merge`, `base-fix`, `no-base-fix`, `parallel <n>`, `harness <name>`, `model <name>` and `effort <level>` (or the same with dashes) are for the run the plan is dispatched as, and mean what they do for `thirdshift <Issue URL>`, so none of them is ever read as the focus. The [Harness, Model and Effort](#harness-model-and-effort) are the Architecture review's too. `--plan-only` stops the Architect run once the plan is marked ready, and dispatches nothing. `email` or `email-unless-merged`, optionally followed by an address, and `no-email` (or the same with dashes) are for the Architect run's own [Run notification](#one-run-notification), with or without `--plan-only`. The word after `email` or `email-unless-merged` is the address only if it contains `@`, so a focus without one is never taken for it.

`base <branch>` (or `--base <branch>`) names the Architect run's **Base branch**, so the branch your clone has checked out stops mattering: start it from a clone on another branch, on a detached HEAD, or with uncommitted changes. The Architecture review's worktree starts at the head of `<branch>` on `origin`, and the run the plan is dispatched as takes `<branch>` as its Base branch too: a Ticket run's **Issue branch**, or a Spec run's **Spec branch**, is branched off it, and the pull request or **Spec PR** targets it. It goes before or after the focus and the other flags, `--plan-only` included, and the word after it is always the branch, never the focus. Without `base`, the Base branch is the branch checked out. `base` is a flag of a Pass only: `thirdshift <Issue URL>` doesn't take it. It is what lets an Architect run be started [on a schedule](#on-a-schedule).

A second focus, a repeated flag, `merge` with `no-merge`, `base-fix` with `no-base-fix`, any two of `email`, `email-unless-merged` and `no-email`, `parallel` without a whole number from 1 up, `base` with no branch after it, any other argument starting with a dash, or an empty focus is an argument error (exit `2`). So is `merge`, `no-merge`, `base-fix`, `no-base-fix` or `parallel` with `--plan-only`, since nothing is dispatched for them to apply to.

An Architect run:

1. Makes the checks a Ticket run makes that don't need an issue, before creating anything: `origin` is a GitHub repository, git has a `user.name` and `user.email`, HEAD is not detached, and the Base branch exists on `origin` with your local copy not ahead of it. With `base <branch>`, a detached HEAD is fine, since the checkout no longer picks the Base branch, and the other two checks are made on `<branch>`: `base branch <branch> does not exist on origin; push it first with: git push --set-upstream origin 'refs/heads/<branch>:refs/heads/<branch>'` when that branch exists locally (the command quotes the actual refspec safely), or `local <branch> is <n> commit(s) ahead of origin/<branch>; push them first`. If `<branch>` is absent both locally and on origin, the error instead says to create or obtain it first, or select a valid Base branch; no push command is suggested. Preflight makes no push, checkout or branch creation. There is no **Origin match**, since there is no Issue URL: the repository is the one `origin` names. If another Pass on the repository is still running, it is [skipped](#one-at-a-time) here, and if not, it is skipped when an earlier [Architect plan is still open](#one-architect-plan-at-a-time), and if not, when an [Architect idea waits for triage](#an-architect-idea-waits-for-triage), and if not, when the repository has a [Ready issue or an issue that would be Ready once settled](#a-ready-issue-goes-first). With `launch.pull = true` in the [User config](#user-config), it then fast-forwards your checkout of the Base branch, as a Ticket run does, and so only when the Base branch is the branch checked out: with `base <branch>` from a clone on another branch, your checkout is left alone.
2. Creates a git worktree next to your clone, named `<repo>-architect`, detached at the head of the Base branch on `origin`, with no **Issue branch**. Your checkout, its uncommitted changes and its untracked files are never touched or scanned.
3. Runs the **Architecture review** there: a headless session on the Harness, with the **Factory skills** loaded, started with the [Architecture review prompt](../prompts/architecture-review.md). It looks for deepening opportunities, skips any an open issue already covers, and takes the top recommendation. If it is Strong, the review publishes it as the plan, labelled `needs-triage`: a **Spec** with **Tickets**, or a single Ticket when one session is enough. It may edit files in the worktree to check an idea, but commits and pushes nothing. It ends its final message with one line naming the plan: `Architecture review plan: <Issue URL>`. thirdshift reads only that line. With [no Strong candidate](#no-strong-candidate) the line names another issue, which thirdshift labels an **Architect idea**, and steps 5 to 7 are skipped.
4. Removes the worktree once the session ends, whatever the outcome. The temporary directory the Factory skills were written out to goes as the Architect run ends.
5. Checks the plan: the issue is in this repository, is open, was created after the Architect run started, and carries no other label that says it is not agent work (`ready-for-human`, `needs-info` or `wontfix`).
6. Marks the plan ready, in one request: `needs-triage` is swapped for `ready-for-agent`, the plan is labelled `architect-plan`, the mark a later Architect run finds an open **Architect plan** by, and its other labels are kept. thirdshift adds that label, never the agent, and first creates it if the repository has none. A Spec's Tickets are left as the review labelled them, without `architect-plan`.
7. Dispatches the plan, unless given `--plan-only`, exactly as `thirdshift <plan URL>` would from the same clone on the Architect run's Base branch: a [Spec run](#spec-runs) when the plan has sub-issues, a Ticket run otherwise. Its Base branch is the Architect run's, so with `base <branch>` it is `<branch>`, not the branch checked out. `merge` and `no-merge` apply to the Spec PR or the Ticket run's pull request, `parallel <n>` to the Spec run, and `base-fix` and `no-base-fix` to whether that run may start a [Base fix](#base-fix), as if given to that command, and the [User config](#user-config) sets what they leave unsaid: `merge.always`, `spec.parallel`, `base.fix`, `launch.pull` and `logs.dir`. The Architecture review itself never watches CI, so a Base fix can only happen in the dispatched run. It makes the [Claim](#the-claim) as that command would too: once its checks pass, the plan's `ready-for-agent` is swapped for `in-progress`, and the plan keeps `architect-plan`. The one difference is that it sends no [Run notification](#run-notifications) of its own, whatever `email.always` says: the Architect run sends [the one](#one-run-notification).

The dispatched run's ending is the Architect run's: its exit code, its pull request's URL alone on stdout, and its last line on stderr, `PR <url> is ready for review` or `PR <url> is merged`. If it fails, the Architect run fails as that Failed ticket run or Failed spec run does, with the cause, the session trace and the Command log on stderr and the pull request's URL on stdout if it left one. The plan stays open and `architect-plan`, and `in-progress` unless the run left nothing on `origin` and so [released its Claim](#when-the-claim-ends), for `thirdshift <plan URL>` to take up again: no later Architect run [retries it](#one-architect-plan-at-a-time). `parallel <n>` on a plan that is a single Ticket fails the same way as it does for `thirdshift <Issue URL>` on an issue that isn't a Spec: `parallel is only for a Spec, and #<n> has no sub-issues`, exit `1`, before any implementing and before the Claim, with the plan left `ready-for-agent`, to run without it.

With `--plan-only`, it instead exits `0` with the plan's URL alone on stdout, and `plan <url> is ready for an agent` as stderr's last line. Read or edit the plan, then run it with `thirdshift <Issue URL>`. The plan is labelled `architect-plan` here too, so until it is finished or closed the next Architect run is [skipped](#one-architect-plan-at-a-time).

Progress lines on stderr say when the review starts, which plan it reported, when the labels are swapped, and when the plan is dispatched: `dispatching the plan <url>, as thirdshift <url> would`. The dispatched run's own progress lines follow. With no Strong candidate, the last line says which issue the Architect run ended on instead.

A review session that fails or is interrupted, a final message without one of the lines the prompt asks for, or a plan that fails a check ends the Architect run as a failure: exit `1`, nothing on stdout, the cause on stderr and then the paths of the session trace and the Command log. Nothing is dispatched and no label is changed, so a plan the review did publish stays `needs-triage`, without `architect-plan`, for you to finish or close, and doesn't skip the next Architect run. A review that finds no deepening opportunity at all has no issue to name, so it ends without one of those lines and the Architect run fails this way too; its session trace says what it looked at.

### One Run notification

An Architect run asked for a [Run notification](#run-notifications), by `email` or by `email.always = true` in the [User config](#user-config) without `no-email`, sends exactly one, whatever its outcome, with or without `--plan-only`, unless it is skipped. A skipped Architect run, [however](#one-at-a-time) [it](#one-architect-plan-at-a-time) [is](#an-architect-idea-waits-for-triage) [skipped](#a-ready-issue-goes-first), sends none, even when asked, so a schedule can start one every few minutes without flooding your inbox: its line is in the log its scheduler keeps. It makes a Ticket run's checks before any other work all the same, an address and a Resend API key, and stops with exit `1` if either is missing, so a run that would have been skipped fails on them too. The email goes after the outcome is final and printed, and a failed send is only a `warning:` line on stderr: it changes neither the exit code nor stdout.

- **Subject**: `[thirdshift] <owner>/<repo> Architect run: <outcome>`. With a dispatched run, the outcome is that run's: `ready for review`, `merged`, `failed` or `interrupted`. Without one, it is the review's: `plan published` (with `--plan-only`), `idea filed`, `idea already filed`, `review failed` (also for a plan that fails a check) or `interrupted`. The repository is left out if `origin` doesn't name one on GitHub.
- **Body**, plain text: a `Result:` line with the same outcome as the subject; a `Command:      Architect run` line and a `Repository:` line naming the repository, when known; a `Review:` line saying how the Architecture review ended, with the URL of the plan or idea issue it named (`plan published: <url>`, `idea filed: <url>`, `idea already filed: <url>`, `failed` or `interrupted`); a `Description:` line with that issue's title, when known, so you can see what the review proposed without opening the link; when the plan was dispatched, a `Dispatched:` line with that run's outcome; then what a Ticket run's notification holds, for the dispatched run or else the failed review: the pull request URL (if any), the failure cause (if failed), with the [lines after it](#an-inherited-failure-links-the-base-branchs-checks) of a dispatched run that failed on an Inherited failure, a `Base fix:` line for a dispatched run that started or waited on a [Base fix](#base-fix), the session trace path (if any), the `Command log:` path (if any), the `Sent by:` line, the hostname and how long the Architect run took. After a dispatched Spec run, it ends with a line per Ticket, as a Spec run's notification does.

The body also includes a `Strength:` line for a completed Architecture review: `Strong`, `Worth exploring` or `Speculative`. A published plan is always `Strong`; for an idea, including one already filed, the rating comes from this review's final message. If the review omits the rating or reports an unrecognized value, the line says `not reported`. Failed or interrupted reviews have no strength line.

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

Any triage decision releases the pause: take `needs-triage` off, whatever replaces it (`ready-for-agent`, `ready-for-human`, `needs-info`, `wontfix` or nothing), or close the issue. The issue keeps `architect-idea`, which only counts alongside `needs-triage`. An idea triaged `ready-for-agent` then goes first under the [Ready issue check](#a-ready-issue-goes-first), even while it is settling, if it meets every other Ready issue condition, so the next Architect run is skipped for it in turn, until a Pickup run, or you, takes it.

### A Ready issue goes first

Work a human shaped goes ahead of the factory's own: while the repository has a **Ready issue**, or an issue that meets every Ready issue condition except settling, every Architect run on it is skipped, before any review. For a Spec, that includes having a Startable Ticket. A [Pickup run](#pickup-runs) still waits until the issue has settled before taking it. The check comes last, after the [lock](#one-at-a-time), the [open Architect plan](#one-architect-plan-at-a-time) check and the [waiting Architect idea](#an-architect-idea-waits-for-triage) check, so with an open plan or a waiting idea as well, the skip reports that instead. It is only the search: an Architect run makes no [Sweep](#the-sweep) and holds no [Claim limit](#the-claim-limit), which are the Pickup run's.

- **stdout** carries the URL of the lowest-numbered issue that is Ready or would be Ready once settled, whether it has settled or not.
- **stderr** carries [a line on each issue labelled `ready-for-agent`](#why-an-issue-was-passed-over) passed over before it, then one progress line naming it: `Ready issue #<n> "<title>" goes first: <Issue URL>`, or, while it is settling, `Ready issue #<n> "<title>" goes first once it settles: <Issue URL>`.
- **Exit code** `0`: skipped is not a failure. No agent session starts, and nothing else is done.

An issue passed over for any reason other than settling never skips an Architect run: one that carries a Claim, has an open blocker or an untrusted edit, is a Ticket inside a Spec, is a Spec with no Startable Ticket, or is a Base fix's issue, among the rest of the rule. Those get their lines on stderr all the same, and the Architect run goes on. With `pickup.wait_minutes = 0`, there is no settling wait and the check is unchanged. Security runs still check only for settled Ready issues.

The rule doesn't ask whether anything will take the Ready issue. A `ready-for-agent` issue that you mean to run by hand, on a repository with no Pickup line in its crontab, keeps every Architect run on it from starting until you run it, which makes the Claim, or take the label off.

### One at a time

A **Pass** is a command started with no Issue URL that decides before any work whether it is skipped. Only one Pass per repository runs at a time on a machine. A Pass started while another on the same repository is still running, the Spec run or Ticket run it dispatched included, is **skipped**: it prints `another Pass is already running on <owner>/<repo>` as a progress line on stderr, prints nothing on stdout, and exits `0`. Skipped is not a failure. A skipped Pass does nothing else: no launch pull, no worktree, no agent session, and no issue filed or labelled. This holds for every Pass, however it was started, so two never build one repository at once.

The repository is the one `origin` names, so two clones of one repository count as the same, and Passes on different repositories run at the same time. A Ticket run or a Spec run started on an **Issue URL** is never skipped this way, and never makes a Pass skip.

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

The check comes after the lock so that the plan of an Architect run that is still running, labelled already while its Spec run or Ticket run goes on, is reported as [already running](#one-at-a-time), not as an unfinished plan. A skipped run sends no [Run notification](#one-run-notification), even when asked: its stdout carries the open Architect plans' URLs, and its stderr line their commands.

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
- **The log file** takes everything the command prints. A failure before the [Run notification](#one-run-notification)'s checks have passed, such as a broken [User config](#user-config), a missing Resend key or a bad argument, shows up only there: no email is sent for it. Use one fixed file for every line, in a folder that already exists, such as `~/.thirdshift/logs/`: the shell opens it before thirdshift starts, so a line whose folder is missing never starts the command, and nothing reports it. What each pass did on its repository is in that repository's [Activity log](#logs), in folders thirdshift creates itself, and with `activity.quiet_skips = true` the log file holds no skips at all.
- **The Harness's CLI and `gh` must already be logged in** for the user the schedule runs as, with git able to push, as the [Prerequisites](#prerequisites) say. Without a terminal an Architect run behaves as it does from one, except that it never offers Setup.

The command's flags and the User config decide what a scheduled Architect run does, as for a hand-typed one: with `merge.always` it merges the Architect plan's pull request, and without it the pull request is left for review. These are the cautious variants, whatever the User config says:

```
0 2 * * * cd ~/repos/thirdshift && thirdshift architect base main no-merge >> ~/.thirdshift/logs/cron.log 2>&1
0 2 * * * cd ~/repos/thirdshift && thirdshift architect base main --plan-only >> ~/.thirdshift/logs/cron.log 2>&1
```

The first leaves the pull request for you to review, and the second stops at the Architect plan, for you to read and run with `thirdshift <Issue URL>`.

A scheduled Architect run is skipped when another Pass on the repository is [still running](#one-at-a-time), the Spec run or Ticket run it dispatched included, when an [Architect plan is still open](#one-architect-plan-at-a-time), when an [Architect idea waits for triage](#an-architect-idea-waits-for-triage), or when the repository has a [Ready issue](#a-ready-issue-goes-first), so a `ready-for-agent` issue with no Pickup line to take it holds off every night until you run it: no new refactor is planned until last night's Architect plan is closed or loses its `architect-plan` label. Merging its pull request closes it; if its run failed, pick it up with `thirdshift <plan URL>` first. Closing the pull request alone leaves the Architect plan open. A skip exits `0`, so the scheduler sees no failure, and the repository's [Activity log](#logs) has the line that says why, as does the log file unless `activity.quiet_skips` is set. With Run notifications on, by `email.always = true` in the User config or `email` on the line, an Architect run that did work sends one email, and a skipped one sends none, so a crontab line can fire every few minutes, as for [Architect runs every few minutes](#architect-runs-every-few-minutes), without flooding your inbox. A night without an email means it was skipped, or failed before the notification's checks passed: look in the log file.

Any scheduler that runs the command works; a systemd user timer needs lingering on (`loginctl enable-linger`) to fire while you are logged out.

### Architect runs every few minutes

<a name="weeding" id="weeding"></a>

Start **Architect runs every few minutes** to clear what features leave behind in a codebase as they land, with nobody starting each pass, so each one begins as soon as the last one's Architect plan is merged. thirdshift still never schedules itself ([ADR-0009](adr/0009-the-operating-system-schedules-thirdshift.md)): it is the line from [On a schedule](#on-a-schedule), fired every five minutes instead of nightly, with `merge.always = true` in the [User config](#user-config) so each Architect plan's pull request is merged without waiting for you:

```
PATH=/home/you/.local/bin:/home/you/.cargo/bin:/usr/local/bin:/usr/bin:/bin
*/5 * * * * cd ~/repos/thirdshift && thirdshift architect base main >> ~/.thirdshift/logs/cron.log 2>&1
```

The `PATH` line, the `cd` and `base main` and the logins are as in [On a schedule](#on-a-schedule). Set `activity.quiet_skips = true` in the User config too, so the skips leave the log file alone: each is in the repository's [Activity log](#logs), once per change of reason.

Most passes are skipped, and that is what paces frequent Architect runs. A pass is skipped while:

- **a run is still going**: a Pass on the repository, the Spec run or Ticket run it dispatched included ([One at a time](#one-at-a-time));
- **an Architect plan is open**: one whose pull request waits for review, one whose run failed, or one published with `--plan-only` and not yet built ([One Architect plan at a time](#one-architect-plan-at-a-time));
- **an Architect idea waits for triage**, since the factory has run out of Strong ideas ([An Architect idea waits for triage](#an-architect-idea-waits-for-triage));
- **the repository has a Ready issue**, so work a human shaped goes first ([A Ready issue goes first](#a-ready-issue-goes-first)).

A skip exits `0` and sends no email, even with Run notifications on, so the scheduler sees no failure and your inbox only hears of passes that did work. The repository's [Activity log](#logs) has the line that says why, once for each change of reason. An Architect plan whose run failed stays open, so frequent Architect runs pause for the **Day shift** until you pick it up with `thirdshift <plan URL>`, close it, or take its `architect-plan` label off; an Architect idea pauses it until you triage the idea.

On a repository that also has a [Pickup line](#a-pickup-run-on-a-schedule), put `--plan-only` on the frequent Architect run line instead: the Architect run publishes the Architect plan and marks it ready, and a Pickup run builds it as an ordinary Ready issue, under the [Claim limit](#the-claim-limit), and merges it with `merge.always`, as in the [whole crontab](#a-pickup-run-on-a-schedule) below.

## Security runs

`thirdshift secure` starts a **Security run**, report-only by default: it audits the **Base branch** at its head on `origin` and records each **Security finding** privately as a draft repository security advisory. Start it from a clone on the Base branch, or name that branch from any checkout:

```sh
thirdshift secure
thirdshift secure base main harness claude
thirdshift secure base main harness codex model gpt-6.1-sol effort high
```

The command takes the Harness, Model and Effort words and the shared Pass words (`merge`, `no-merge`, `base-fix`, `no-base-fix`, `parallel`, `email`, `email-unless-merged`, `no-email`, `security-fix`, `no-security-fix`, `security-close` and `no-security-close`, with or without dashes). After recording findings, it tries to reproduce each untriaged one in a fresh throwaway worktree at the recorded audited commit, and keeps the outcome, severity when reproduced, notes and test in the private record. It preserves findings already triaged by the Day shift. It never publishes an advisory.

Fixing is off by default. `security-fix` on the command or `fix = true` under `[security]` allows one fix; `no-security-fix` overrides the setting. Both words are accepted on every command that starts Ticket runs and passed to those Ticket runs. With fixing allowed, a Security run takes the most severe reproduced finding still awaiting a fix before auditing, with ties in private-record order. After an audit reproduces findings, it goes on to the most severe one.

Closing is off by default too. `security-close` on the command or `close = true` under `[security]` lets a Security run close the finding its fix merged, naming the merged pull request; `no-security-close` overrides the setting. The words take effect only when fixing is also allowed. After a dispatched fix ends merged, the run re-runs the private record's proof-of-concept test against the merged Base branch in the reproduction sandbox. When the test now passes, the run names the merged pull request in the private record and closes it: a draft advisory through its description and state, a finding issue with a comment naming the pull request before it is closed. A fix left ready for review closes nothing, and a test that still fails, or cannot run, leaves the record open with a private note of what happened; a failed close never changes the fix's outcome. A closed record is never picked up as waiting or as the next fix.

A publishing session reads the private record and follows the reproduction's fix-size decision: a single **Ticket**, or a **Spec** with **Tickets** through `thirdshift-to-spec` and `thirdshift-to-tickets`. Every new issue is terse, saying only what the fix changes and linking the private record. thirdshift checks all the issues, swaps the top issue's `needs-triage` for `ready-for-agent`, adds `security-fix` to every fix issue (creating the label when missing), records the fix link privately and dispatches its **Ticket run** or **Spec run**. The Security run ends as that run ends, including a **Ticket run** that Self-merges when asked by the command or the User config.

On a private repository, the finding's own issue is the fix's Ticket or Spec, keeping its write-up and proof-of-concept. A one-session fix needs no publishing session or second issue: thirdshift marks the finding's issue ready and dispatches it. For a bigger fix, the publishing session adds terse Tickets as that issue's native sub-issues, with their blocking links.

A failed fix keeps its **Claim**, even if it pushed nothing, and stays open for the **Day shift**. No later Security run or Pickup run retries it. Security runs pause while that fix's issue is open; closing the issue lets them go on.

`email` or `email-unless-merged`, optionally followed by an address, or `email.always` or `email.unless_merged` in the User config asks for one **Run notification** when the Security run ends, whether the audit succeeded, failed or was interrupted. `no-email` overrides the default. The notification says how the audit ended and lists each finding it recorded or matched to an existing private record: its severity when known, title and private link. It includes no finding write-up, trace or evidence, since it passes through Resend. When the Security run dispatches a fix, the notification lists that finding under `Addressed by the pull request:`, or under `Fix attempted, not fixed:` when the fix failed or was interrupted, and its audit's other findings under `Recorded, waiting for the Day shift:` while their records wait for triage, else under `Also recorded by the audit:`. Findings left waiting put the Security run in the Inbox's Needs you; the fixed one does not. Its `Audit:` line then says how the audit ended, or `not run` when it fixed an earlier finding without auditing; the `Result:` and `Pull request:` lines tell how the fix ended. When a fix merged, it also says which finding was closed by which pull request, or that the finding was left open and why. Detailed failure causes stay in the local logs; the notification gives the audit's status and log paths. A recording failure still lists the records reached before it failed. When a run leaves a reproduced finding unfixed and neither the command nor the User config decided against fixing, its notification and stderr offer both `thirdshift secure security-fix` and `fix = true` under `[security]`. With `no-security-fix`, or the setting turned off, it offers nothing. A skipped Security run sends none. The usual address and Resend API key checks run before the skip checks or any work; a failed send is a warning and never changes the run's outcome.

If a Security run fails before its audit starts, its Run notification says `Result: pre-audit failed` and `Audit: not started`. A Base branch missing both locally and on `origin` gives `Cause: Base branch not found`. A branch present only locally, a detached HEAD with no Base branch named, or local Base branch commits absent from `origin` gets a fixed Base branch Cause too. GitHub failures keep their fixed dependency or rate-limit Cause; other failures say `pre-audit checks failed`. Raw diagnostics stay on the machine; the notification says which host and start time to check on stderr.

A Security run chooses its Harness from the command's `harness` word, then `[security] harness` in the User config, then `harness.default`, then Claude Code. When `security.harness` is blank or missing, it uses `harness.default`; when `harness.default` is also missing, it uses Claude Code. Model and Effort come from the chosen Harness's own `[harness.<name>]` section, with command words taking precedence. For example, `harness claude` overrides `[security] harness = "codex"` and uses `[harness.claude]`.

Codex security sessions and their Resumes set `agents.max_concurrent_threads_per_session=8` and ask for fresh sub-agents with `fork_turns: "none"`, so the skill's verifiers stay independent.

A Security run checks its gates in order: another **Pass** on the repository running on the machine, a **Ready issue**, a failed Security fix whose issue is still open, then an unchanged **Base branch** since the last completed Security audit. A **Security finding** waiting for the **Day shift** is not a gate: a draft advisory with no severity, an open finding issue with `needs-triage`, or a reproduced finding while fixing is disabled holds off no audit. When an audit finds such a finding again and its reproduction has completed, reproduced with a severity or not reproduced, thirdshift keeps that outcome and starts no reproduction session for it; the Run notification lists it with its known severity. A finding whose reproduction never ran, failed, was refused or was interrupted is reproduced again, and a finding the Day shift graded keeps its grade. The audited commit comes from the skill's own `run-metadata.json` under the audit root, with no separate state file. A skip exits `0`, starts no session, keeps no Command log and sends no Run notification; its reason goes into the repository's **Activity log** only when it changes, with the usual `activity.quiet_skips` behavior. Before starting work, thirdshift checks the chosen Harness and that **Node.js** is on `PATH`, since the embedded skill's report validators require it.

The Security audit runs in a throwaway worktree detached at origin's Base branch head, leaving the Launch directory and local work untouched, including when `launch.pull` is set. Its Session prompt runs `thirdshift-security-audit` in full audit mode with the `quick` profile, auditing the whole repository except vendored and third-party code. It names a `SECURITY.md` or conventional threat-model document when present, reads compatible earlier runs and ends `incomplete` instead of asking for input.

Each audit keeps a new output directory under `<logs.dir>/<owner>/<repo>/audits/`, outside the worktree. The audit artifacts and private advisory descriptions contain evidence; keep the logs directory private. thirdshift runs both embedded validators before recording anything. A missing final line, an incomplete session or an invalid report fails the run and names its Session trace. Command logs and Session traces are in `commands/secure/`; Session traces are named `secure-<stamp>~<token>-security-audit.logs.jsonl`; the Activity log records the start and ending as for other Passes.

A finding's title becomes the advisory summary. Its description preserves its write-up, trace, evidence and validation plan, with the fingerprint and audited commit. thirdshift claims no severity, CWE or affected version range. It uses the package in the repository's manifest, or the `other` ecosystem when none is identified. Matching fingerprints in any advisory state are kept rather than recorded again. Rejected candidates stay in the local report and produce no advisory. Draft advisories require GitHub repository security manager or administrator access. On a private repository whose advisory endpoint is unavailable, the private record is an issue labelled `security-finding` and `needs-triage`. It is never replaced by a public finding issue.

### Security review

`security-review`, or `review = true` under `[security]`, enables one guidance review after a Ticket run's opening session or a Spec run's Spec review, before Delivery pushes and enters the Repair loop. A Spec run reviews every Ticket against its Spec branch and the whole Spec branch against its Base branch once. `no-security-review` overrides the setting. Both words, with or without dashes, are accepted on commands that start Ticket runs and follow dispatched Ticket runs and Base fixes. Setup asks about the setting; it is off by default.

The review runs on `[security] harness` when that is set, with that Harness's own `[harness.<name>]` Model and Effort, wherever it runs: in a Ticket run, each Ticket, on a Spec run's Spec PR or in a Base fix. The command's `harness`, `model` and `effort` words choose the Harness of every other session, and don't change the review's; there is no word for the review's Harness. With `security.harness` blank or missing, the review runs on the Command's Harness. Every other session, Repairs and Resumes after the review included, stays on the Command's Harness.

Before any session, a command checks its review's Harness with the same PATH, Model, Effort and sign-in checks as its own. If a non-default review Harness can't be used, the review falls back to `harness.default`, or Claude Code when blank, with that Harness's own configured Model and Effort. A single `warning:` line names both Harnesses and the reason. If the review Harness is already the default, or the fallback also fails its check, the command fails at preflight. A Spec run checks before any Ticket starts and hands the settled choice to all Tickets and the Spec PR. With Security review off, no review Harness is checked. A Pass checks it only if it may dispatch a Ticket run: a Security run that only audits, or `architect --plan-only`, does not.

If a non-default review Harness passes preflight but returns a Harness error before any turns or tokens, thirdshift warns in the same way and retries the review once on the default Harness. A failure on the default, a model refusal or a failure after work starts is not retried. Each attempt keeps its own Session trace and `.models.json`; the retry uses `security-review-fallback`, also selected by `logs --session security-review`. Those records, the Command log and the Run notification name the Harness each attempt actually ran on; the `Built with` line still names the Command's. A Continuation reads `security.harness` again and applies these checks again.

The session reads the relevant attack-class guidance and supporting code without delegated auditors or the full audit workflow. It fixes only findings reproduced by a failing proof-of-concept test, keeps that test and commits the fix. Unaddressed introduced findings appear under Security in the PR's Unaddressed findings. A Ticket run or Spec run asked to Self-merge leaves the PR ready for review and fails with the findings named if any remain, or if the review was refused or incomplete. A Ticket run or Spec run not asked to merge records the outcome and continues. An old finding, whose proof-of-concept also fails at the pinned merge base, is reported in a private file under the repository's logs (`security-reviews/`). thirdshift records it as a draft advisory, or a `security-finding`/`needs-triage` issue on a private repository, reusing existing fingerprints and preserving the Day shift's grades. New records include scored reproduction and fix-size metadata so a later Security run can take the fix when allowed. Its details stay out of the PR and it does not hold Self-merge; fixing belongs to a separate Security run. The separate Security audit retains its full audit workflow.

### Security runs on a schedule, with fixing allowed

<a name="fencing" id="fencing"></a>

Start **Security runs on a schedule, with fixing allowed** to find and fix vulnerabilities with nobody starting each pass. thirdshift never schedules itself ([ADR-0009](adr/0009-the-operating-system-schedules-thirdshift.md)). This Linux crontab entry starts one every five minutes; use your own paths, with `claude` and `gh` already logged in and Git able to push:

```cron
PATH=/home/you/.local/bin:/usr/local/bin:/usr/bin:/bin
*/5 * * * * cd /home/you/repos/widgets && thirdshift secure base main harness claude security-fix >> /home/you/thirdshift-secure.log 2>&1
```

Each pass yields to a **Ready issue**, so work the **Day shift** shaped goes first. It is skipped while another Pass on the repository is running, while a fix has failed and its issue stays open, or while the Base branch is unchanged and no fix is waiting. A failed fix pauses scheduled Security runs for the Day shift until its issue is closed; it is never retried by a later Security run or Pickup run. Findings waiting for triage hold off no audit, and a finding found again is not reproduced again once its reproduction has completed. With fixing allowed, a reproduced finding is fixed before the next audit.

A skip exits `0` and sends no Run notification. Its reason appears in the **Activity log** once per change of reason. Set `activity.quiet_skips = true` in the [User config](#user-config) to keep those skips out of the scheduler's log. `merge` on the command, or `merge.always = true` in the User config, lets each fix's Ticket run merge its pull request; otherwise it stays ready for review.

## Pickup runs

`thirdshift pickup` starts a **Pickup run**: one pass, with no **Issue URL**, that takes the lowest-numbered **Ready issue** in the repository and runs it, so that an issue you have already marked ready needs no command typed for it. Start it from a clone of the repository, on the **Base branch**, or name the Base branch with `base <branch>`:

```sh
cd ~/repos/widgets
thirdshift pickup                      # take the lowest-numbered Ready issue and run it
thirdshift pickup merge parallel 2     # merge its pull request; two Tickets at once if it is a Spec
thirdshift pickup base main no-merge   # run it on main, whatever branch the clone is on, and leave its pull request for review
thirdshift pickup highest-first        # take the highest-numbered Ready issue instead, for that one pass
```

Which end of the issue numbers the pass takes is its order: `lowest-first` by default, `highest-first` with `pickup.order = "highest-first"` in the [User config](#user-config), or for one pass with `lowest-first` or `highest-first` on the command, which wins over the User config. Two schedules working through the same queue from opposite ends, one with each order, collide less often; the reservation below still protects either choice.

A Ready issue is an open issue that:

- is labelled `ready-for-agent`: an issue without the label is never taken;
- has none of the labels that make an **Unready Ticket**, `ready-for-human`, `needs-info`, `wontfix` and `needs-triage`, so a contradictory label errs on the side of not running;
- carries no [Claim](#the-claim): it is not labelled `in-progress`;
- is not a sub-issue. A sub-issue is a **Ticket** of a **Spec**, and is never run on its own, whatever the Spec's labels: it is reached through its Spec, when the Spec is itself a Ready issue, so its work always goes through the **Spec branch**. Labelling one Ticket never promotes its Spec, either;
- is not labelled `base-fix` or `security-fix`: the Ticket run that opened a [Base fix](#base-fix)'s issue, or the Security run that dispatched a fix, owns it. A failed Security fix is still passed over if GitHub prevented its Claim from being made;
- has no open blocker, by GitHub's "blocked by" links, never the text of its body. Once every blocker is closed, it can be taken;
- either was never started (no **Issue branch** on `origin` and no pull request from one, open, merged or closed), or you handed it back by applying `ready-for-agent` at or after its last [Claim](#the-claim) ended. For started work, the full label history must show an earlier `in-progress` application and its removal, with `ready-for-agent` applied at that removal's timestamp or later. A swap made in one edit counts in either listed order; a new Claim applied at the same timestamp leaves the issue Claimed. Work never Claimed, or without that hand-back, stays with you. An eligible branch with no PR or an open PR follows the existing [Continuation](#continuation) rule; merged or closed PRs follow ADR 0002 and start a fresh numbered branch. This is deliberate hand-back, not automatic retries;
- is not a **Spec** whose **Tickets** are all closed. With nothing started on it, such a Spec was done some other way, and the [Spec run](#spec-runs) it would be dispatched as stops with nothing to do, so a Pickup run passes it over rather than take it on every pass. Waiting never makes it a Ready issue, so this comes before the next condition, however lately the Spec was labelled: close it, or take its `ready-for-agent` off. A Spec with at least one open Ticket is taken if it meets the conditions below;
- is settled: by default, five minutes have passed since the latest of `ready-for-agent` being applied to it, a sub-issue being added to it or removed, and a "blocked by" link being added to it or removed. So a Spec is not taken while its Tickets are still being attached. These are read from the issue's timeline, since adding a sub-issue does not change the issue's update time, and `pickup.wait_minutes` in the [User config](#user-config) sets the window in whole minutes from 0 up, within the supported duration range; 0 disables waiting. An Architect run also waits for an issue that meets every other Ready issue condition while it is settling. A later pass takes the issue once it has settled;
- if it is a **Spec**, has a **Startable Ticket**: an open Ticket in the Spec's repository, with no sub-issues, no open blocker and none of the labels that make an Unready Ticket. That is the rule a [Spec run](#spec-runs) picks its next Ticket by, as if nothing were running yet, so a Spec whose open Tickets all wait on you, such as one whose last open Ticket is `needs-info`, is passed over rather than Claimed for a Spec run that could start nothing and would fail. This applies to a Spec handed back with `ready-for-agent` too. A Spec with one Startable Ticket is taken, and the Spec run handles its other Tickets as usual. Once a Ticket becomes startable, because its Unready label comes off or its blocker closes, a later pass takes the Spec. Its Tickets are read only for a Spec that meets every other condition.

A Pickup run:

1. Makes the checks an [Architect run](#architect-runs) makes, before anything else but the [Run notification](#one-run-notification-for-the-issue-taken)'s own: `origin` is a GitHub repository, git has a `user.name` and `user.email`, HEAD is not detached unless `base <branch>` names the Base branch, and the Base branch exists on `origin` with your local copy not ahead of it. A check that fails stops the pass with exit `1`, before any label is read or changed.
2. Tries the Pass lock, and is [skipped](#one-at-a-time) if another Pass on the repository is still running on the machine.
3. Takes `in-progress` off the repository's closed issues: the [Sweep](#the-sweep).
4. Counts the repository's open issues labelled `in-progress`, and is skipped if the repository is at its [Claim limit](#the-claim-limit).
5. Lists the repository's open issues labelled `ready-for-agent`, in its order (lowest number first by default, highest number first with `highest-first`), and takes the first that is a Ready issue, saying so on stderr: `taking Ready issue #<n> "<title>", as thirdshift <Issue URL> would`. Each one it passes over on the way gets [a line saying why](#why-an-issue-was-passed-over). One issue a pass: the rest wait for the next.
6. Atomically reserves the selected issue on `origin`, then rechecks that it is still Ready and no Claim has been made since selection. If another Pickup run holds the reservation, the issue ceased to be Ready, or an intervening Ticket run has already taken it and returned it to Ready, skips with exit `0`, no Claim, no session and no notification. This also protects two hosts using the same GitHub identity, with settling disabled or clocks out of sync.
7. Dispatches it exactly as `thirdshift <Issue URL>` would from the same clone on the Pickup run's Base branch: a [Spec run](#spec-runs) when the issue has sub-issues, a Ticket run otherwise. That run makes the Claim, so the issue's `ready-for-agent` is swapped for `in-progress` and no later pass takes it again. The reservation lasts through the dispatched run's ending, including failed-work preservation and Claim ending, then is released with an exact ownership check, even after interruption.

Every Pickup host must run a version that honors reservations. The reservation branch is `thirdshift/pickup/<issue-number>`; direct Issue URL commands and other Pass kinds keep their existing behavior. A crash or an uncertain remote failure can leave a reservation behind. Before releasing one, confirm its host's Command has stopped. Inspect `git ls-remote origin refs/heads/thirdshift/pickup/7`, fetch that ref, and read its commit message for the launch path, process and start time. Then delete it with the observed SHA as an exact lease: `git push --force-with-lease=refs/heads/thirdshift/pickup/7:<observed-sha> origin :refs/heads/thirdshift/pickup/7`. Never delete a live owner's reservation or remove one on a timer. Repository permissions must allow creating and deleting these branches. See [ADR 0017](adr/0017-pickup-reserves-issues-on-origin.md).

`merge`, `no-merge`, `base-fix`, `no-base-fix`, `parallel <n>`, `harness <name>`, `model <name>` and `effort <level>`, with or without dashes, are for the dispatched run, and mean what they do for `thirdshift <Issue URL>`, as do `email` or `email-unless-merged`, optionally followed by an address, and `no-email` for the Pickup run's [Run notification](#one-run-notification-for-the-issue-taken). The [User config](#user-config) sets what they leave unsaid: `merge.always`, `base.fix`, `spec.parallel`, `email.always`, `email.unless_merged`, `launch.pull` and `logs.dir`. A User config a Ticket run would refuse stops the pass the same way, before any check. `parallel <n>` applies when the Ready issue is a Spec and is ignored, with no error, when it isn't, unlike on an Issue URL: the command can't know which it will take. `base <branch>` (or `--base <branch>`) names the Base branch as it does for [`architect`](#architect-runs), and the dispatched run takes it: its Issue branch or Spec branch is branched off `<branch>`, and its pull request targets it. With `base <branch>`, a Pickup run started [on a schedule](#a-pickup-run-on-a-schedule) runs from the clone you work in, whatever branch you left it on.

`lowest-first` and `highest-first`, with or without dashes, are for the Pickup run itself: they set its order for that one pass, winning over `pickup.order`. `secure` and `thirdshift <Issue URL>` take neither.

`pickup` is a command only as the first argument, and takes nothing but those flags, each at most once: a focus, `--plan-only`, an Issue URL, a repeated or contradictory flag, or any other argument is an argument error (exit `2`).

The dispatched run's ending is the Pickup run's: its exit code, its pull request's URL alone on stdout, and its last line on stderr, `PR <url> is ready for review` or `PR <url> is merged`. If it fails, the Pickup run fails as that [Failed ticket run](#failed-ticket-runs) or Failed spec run does. After an [Inherited failure](#an-inherited-failure-links-the-base-branchs-checks), the command it offers is `thirdshift <Issue URL>` with the dispatched run's flags and `base-fix`, since another Pickup run needs you to remove its Claim and then apply `ready-for-agent` before it can take started work again.

A pass is **skipped** when the lock is held, when the repository is at its Claim limit, when the repository has no Ready issue, or when the selected issue is reserved or stale: it exits `0` with stdout empty and a reason on stderr. A reserved or stale selection says `#<n> is reserved by another Pickup run or is no longer Ready`, with the reservation ref to inspect if it stays reserved. Skipped is not a failure. A skipped pass starts no agent session, makes no Claim, and sends no Run notification. One skipped for the lock changes no label at all; the others have made the Sweep, whose lines come before the reason.

### The Claim limit

A Pickup run takes nothing while the repository is at its **Claim limit**: as many open issues carry a [Claim](#the-claim), that is, are labelled `in-progress`, as the limit, which is 3 unless you set it. It keeps a broken Base branch from failing every Ready issue in turn, one a pass, and pull requests from piling up unreviewed: once that many issues wait on you, the factory waits too.

- The count is every open issue labelled `in-progress` in the repository, whoever started it: a Ticket run you started by hand counts as one a Pickup run dispatched does. Closed issues don't count.
- `pickup.limit` in the [User config](#user-config) sets the limit, a whole number from 1 up, by default 3. Raise it on a machine where you review quickly; `limit = 1` takes one issue at a time. There is no command-line flag for it.
- At or over the limit, the pass is skipped: exit `0`, stdout empty, and the reason on stderr names every holding issue, lowest number first, along with the limit, such as `at the Claim limit on acme/widgets: #924, #936 and #953 are labelled in-progress, pickup.limit is 3`. No Ready issue is looked for, and no session is started.
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
thirdshift: 03:00:07 #34 not settled: labelled ready-for-agent less than 5 minutes ago
thirdshift: 03:00:07 #35 not settled: a sub-issue added or removed less than 5 minutes ago
thirdshift: 03:00:08 #36 not settled: a "blocked by" link added or removed less than 5 minutes ago
thirdshift: 03:00:09 #38 has no Ticket that can start: #39 labelled needs-info; #40 blocked by #39; #41 blocked by #12 (outside the Spec)
thirdshift: 03:00:09 no Ready issue on acme/widgets
```

A line with `blocked by` names every open blocker. A Ticket's line names its Spec: the Ticket runs when its Spec does. A Ticket whose Spec is a Ready issue gets no line: its Spec is taken, by this pass or a later one. A started issue without a recorded hand-back gets the `started` line. Even with a hand-back, a Spec with every Ticket closed is passed over with `every Ticket is closed`, and one with no Startable Ticket with `has no Ticket that can start`, naming each open Ticket and why it can't start: the Unready label it has, the open issues blocking it, `has sub-issues` or `outside the Spec's repository`. When a later issue is a Ready issue, the lines on the earlier ones are printed and it is taken:

```
thirdshift: 03:00:01 Pickup run starting, 2026-10-03 -0400
thirdshift: 03:00:02 #18 labelled needs-info
thirdshift: 03:00:03 #20 blocked by #17
thirdshift: 03:00:04 taking Ready issue #22 "Sharpen the widgets", as thirdshift https://github.com/acme/widgets/issues/22 would
thirdshift: 03:00:04 logging this command to ~/.thirdshift/logs/acme/widgets/commands/pickup/22-20261003T030001-0400~aB3cD4eF5gH6iJ7k.log
```

### One Run notification for the issue taken

A Pickup run asked for a [Run notification](#run-notifications), by `email` or by `email.always = true` in the [User config](#user-config) without `no-email`, sends exactly one when it took a Ready issue (asked by `email-unless-merged` or `email.unless_merged`, only when the dispatched run did not merge): the notification the Spec run or Ticket run it dispatched would have sent started by hand. An address after `email` wins over `email.to`, as for a Ticket run.

- **Subject**: that run's, `[thirdshift] <owner>/<repo>#<n> <issue title>: <outcome>`, naming the issue taken, with the outcome `ready for review`, `merged`, `failed` or `interrupted`.
- **Body**: that run's too: what a Ticket run's notification holds, with `Issue:` immediately after `Result:` linking the issue it took, then its `Title:` (the parent Spec when it dispatched a Spec run), and after a Spec run a line per Ticket. The time it gives is the whole pass's, the search for the Ready issue included.

The email goes after the outcome is final and printed, and a failed send is only a `warning:` line on stderr: it changes neither the exit code nor stdout. The dispatched run sends no notification of its own, whatever the User config says, as with an [Architect run](#one-run-notification), nor do a Spec run's Ticket runs.

A skipped pass sends no notification, even when one was asked for: a pass every half hour would otherwise send dozens a day. A skipped Architect run sends none either.

The notification's checks, an address and a Resend API key, are made before any other work on every pass, before the checks in step 1, the lock and any label read or changed. A broken setup therefore stops the pass with exit `1`, naming what is missing, even a pass that would have been skipped, so it shows in the scheduler's log on the first pass, not only once an issue is ready. A pass that asks for no notification makes none of these checks.

### A Pickup run on a schedule

thirdshift has no scheduler of its own ([ADR-0009](adr/0009-the-operating-system-schedules-thirdshift.md)): the operating system's scheduler runs the ordinary command, one pass each time. This crontab entry, written for a Linux machine with cron, starts a pass every half hour. Add it with `crontab -e`, with your own paths in place of `/home/you` and `~/repos/widgets`:

```
PATH=/home/you/.local/bin:/home/you/.cargo/bin:/usr/local/bin:/usr/bin:/bin
*/30 * * * * cd ~/repos/widgets && thirdshift pickup base main >> ~/.thirdshift/logs/cron.log 2>&1
```

The `PATH` line, the `cd` and `base main`, the log file's directory, the logins the Harness's CLI and `gh` need, and schedulers other than cron are as for an Architect run: see its [On a schedule](#on-a-schedule). What differs for a Pickup run:

- **The interval** sets how soon a Ready issue is taken and how much work is started: a pass takes one issue, so `*/30` starts at most one every half hour. A pass lasts as long as the run it dispatched, and the passes that fire meanwhile are skipped, so the schedule builds one issue of a repository at a time on a machine, and the first pass after it ends takes the next, unless the repository is by then at its [Claim limit](#the-claim-limit): each pull request left for review, and each failure left for you, holds a Claim, and at 3 of them, or the `pickup.limit` you set, the passes take nothing until you have dealt with one. An issue you have just labelled also waits five minutes by default, or the `pickup.wait_minutes` you set, until it is settled. A Ticket run you start by hand on an Issue URL is outside all this: it neither waits for a pass nor makes one skip.
- **One line per repository.** thirdshift keeps no list of repositories. Each Pass on one repository skips while another is still running, the Spec run or Ticket run it dispatched included, so both lines can go in one crontab and two Passes never build that repository at once. Passes on different repositories do run at the same time: give their lines different minutes, such as `15,45`, if they would compete for the machine.
- **A pass is skipped** when the [lock is held](#one-at-a-time), when the repository has no Ready issue, and when it is at its Claim limit. A skip exits `0`, so the scheduler sees no failure, and sends no email, even with Run notifications on. The repository's [Activity log](#logs) records a skip when its reason changes. Unless `activity.quiet_skips` is set, the log file holds each skip's line too, and before the line of a pass that found no Ready issue, as before the line of one that took an issue, [why each issue was passed over](#why-an-issue-was-passed-over). A pass skipped for the lock or the Claim limit looks at no issue, so it has no such lines.
- **A Run notification**, by `email.always = true` or `email.unless_merged = true` in the [User config](#user-config), or `email` or `email-unless-merged` on the line, is sent [only for an issue taken](#one-run-notification-for-the-issue-taken). So a day without email doesn't say the passes are running: a pass that fails before it takes an issue, on a broken User config, a missing Resend key or a failed check, shows up only in the log file.

A whole crontab with both frequent Architect runs and a Pickup run on one repository looks like this, with `activity.quiet_skips = true` in the User config, every line sending its output to the one log file, which catches only what went wrong, so `tail ~/.thirdshift/logs/acme/widgets/activity.log` shows what the factory has been doing on the repository lately:

```
# cron reads no shell profile: list every directory that holds thirdshift, claude, gh, git and the build tools
PATH=/home/you/.local/bin:/home/you/.cargo/bin:/usr/local/bin:/usr/bin:/bin

# Architect runs every five minutes: an Architect run publishes each Architect plan, and a Pickup run builds it
*/5 * * * * cd ~/repos/widgets && sleep 20 && thirdshift architect base main --plan-only >> ~/.thirdshift/logs/cron.log 2>&1

# Pickup runs, every half hour
*/30 * * * * cd ~/repos/widgets && thirdshift pickup base main >> ~/.thirdshift/logs/cron.log 2>&1
```

For one Architect run a night instead of every few minutes, put the nightly line from [On a schedule](#on-a-schedule) in place of the frequent Architect run line: a repository needs one or the other, since both lines start Architect runs at different intervals. Adding a repository is adding its lines. Add the lines with `crontab -e` rather than by joining files, and end each with a newline: a line run together with the comment after it, such as `... 2>&1# Edit this file`, is a shell syntax error, so cron fires it but the pass never starts and nothing reaches its log.

The command's flags and the User config decide what a pass does with the issue it takes, as for a hand-typed `thirdshift <Issue URL>`: with `merge.always` it merges the pull request, and without it the pull request is left for review. This is the cautious variant, whatever the User config says:

```
*/30 * * * * cd ~/repos/widgets && thirdshift pickup base main no-merge >> ~/.thirdshift/logs/cron.log 2>&1
```

An issue whose run failed [keeps its Claim](#when-the-claim-ends) and waits for the **Day shift**: it stays `in-progress`, with the Issue branch and any draft pull request the run left, no later pass takes it, and it counts towards the Claim limit until it is closed or you take the label off. To send it round again by hand, run `thirdshift <Issue URL>` from the clone, with the Base branch checked out, since that command takes no `base <branch>`: it picks up where the failed ticket run stopped, as a [Continuation](#continuation) does. To hand it back to Pickup instead, remove `in-progress` and apply `ready-for-agent`, either in the same edit or afterward: after settling, a pass can continue it if every other Ready issue gate still permits it. Applying the ready label strictly before the last Claim ends does not hand it back. For an ordinary Ticket run, the one failure a later pass does retry is one that left nothing on `origin`, such as a usage limit or an expired login: its Claim is released, so the issue is a Ready issue again once it has settled. A failed Security fix is always left for the Day shift, even with nothing on `origin`.

When shaping issues with `to-spec` and `to-tickets` from [mattpocock/skills](https://github.com/mattpocock/skills), keep a **Spec** labelled `needs-triage` until every **Ticket** is published, attached as a sub-issue and linked to its blockers; then replace it with `ready-for-agent`. A Spec with no Tickets yet looks exactly like a standalone Ticket, so a pass would start a plain Ticket run on it. This repository's Day shift follows the publication procedure in [`docs/agents/triage-labels.md`](agents/triage-labels.md), required by [`AGENTS.md`](../AGENTS.md), which overrides the imported skills' defaults and survives their updates. The settling window (five minutes by default) is only a second line of defence: it covers Tickets attached within minutes of the label, not a Spec left labelled and without Tickets for longer.

## Scoring runs

`thirdshift score` starts a Scoring run from any directory. It searches the
configured GitHub owners for merged pull requests whose bodies contain the
`<!-- thirdshift:built-with -->` marker, excluding work whose issue carries
`security-fix`. Its first window looks back seven days; each later window starts
at the previous saved window's end and ends at this Command's start. Boundaries
are inclusive at the start and exclusive at the end, so work is sampled once.

Configure the owners before the first run. Setup writes these defaults and
preserves existing values; it does not ask Scoring questions:

```toml
[scoring]
owners = []                 # GitHub users or organizations; configure before score
sample_size = 10            # maximum weekly sample size (historical 20 are additional)
harness = "claude"          # Grader's Harness
model = ""                  # set an explicit Model before grading a sample
fallback_harness = "codex"   # second Grader, when the first matches the Run's Model
fallback_model = ""         # set an explicit independent Model for the second pair
```

The sample is drawn randomly across the Harness and Model pairs in the Built-with
lines. Each saved window records its seed and sampled pull requests, including
their repository, merge date, Harness, Model, Effort and version when present.
Samples are private under `~/.thirdshift/evals/scoring/`. `thirdshift scores`
lists them by ISO week; `thirdshift scores --json` returns the saved windows and
their samples. Each new window's `packets` directory holds one numbered folder
per sampled pull request: `packet.json` and `repository/`. The JSON holds the
issue title and body from its edit history at merge time, the merged diff, the
pull request title and body, and CI check names and conclusions. Built-with and
Claude Code's Generated-with lines are removed from this text for every Harness.
The repository contains the merge commit's files with Git metadata and
attribution lines removed, so
neither commit messages nor later history reach the Grader. Unavailable
historical issue text fails the Command rather than substituting a later
revision.

A model Grader scores each packet in the Experiment sandbox, with the repository
read-only. It gives five independent pass/fail Scores with reasons: Correct,
Scoped, Tested, Honest and Standards. Honest checks only the pull request's
claims against its diff and CI. Scores are retained on each sampled pull request,
with the Grader's Harness and Model; local work also links to its Run's Command
record. Both `scores` formats withhold model Scores until all five human Scores
are saved for that pull request. Revealed Scores include each criterion's
calibration: `calibrated`, `matches` and `compared` in JSON.

Scoring samples, Bug-origin proposals and settlements, and read-time Run metrics
share retained pull-request attribution. Matching ignores ASCII letter case and
trailing slashes. Latest means the greatest parsed RFC 3339 instant, including
UTC offsets; ties use greatest lexical record path, then greatest Command name.
An unparseable start sorts before every valid time, with the same path/name ties.
Selection follows the captured Command ownership tree, including dispatched work,
nested Ticket runs and Base fixes. It does not admit orphaned child records.

The first Scoring run also draws 20 past pull requests across Harness and Model
pairs, or all eligible work if fewer than 20 exist. They are marked `for_you`
and graded too. They can predate the weekly window; overlaps with that week's
draw are graded once. Later runs mark two from each ISO week's samples for you
(or as many as available); reruns in the same week do not add another two.

Use `thirdshift scores <pull request> --mine [--json]` with a sampled pull request URL,
or a number that identifies only one sampled repository's pull request. It asks
for pass or fail and a nonempty reason for Correct, Scoped, Tested, Honest and
Standards. The model's Scores stay hidden in ordinary views and during entry.
All five of yours are saved together before the model's appear beside them;
`--json` returns that saved result as JSON. EOF or interruption
leaves any previous Scores unchanged. Only one Scoring run or human scoring
session can write at once. `thirdshift scores <pull request> [--json]` shows
that pull request; omit it to list samples by week.

Each criterion is calibrated when at least 18 of the last 20 distinct pull
requests you and the model both scored agree on pass/fail. Fewer than 20 paired
pull requests means uncalibrated. The rolling order is when you completed your
Scores; reasons are retained but not compared. Every criterion stands on its
own, so one that stays uncalibrated does not hold up the others.

Set an explicit `scoring.model`: a blank default cannot establish that the
Grader differs from the Run's Model. When the first Model matches the Built-with
line (ignoring case), the second Harness/Model pair is used; if it also matches
or is blank, grading fails without starting that Grader.

A Claude plan-limit warning or rejection, or a Codex quota window at 85%, stops
grading immediately. Completed Scores are checkpointed individually. The next
`score` command scores the retained remainder before drawing a new window;
completed pull requests are never graded again. Other Harnesses have no verified
warning protocol yet. Grader sessions use bwrap and socat on Linux; if isolation
cannot start, grading fails and the sample remains pending. As for Experiments,
`THIRDSHIFT_TRIAL_NO_SANDBOX=1` selects the pass-through launcher for fake Harness
tests or an explicit emergency override.

Every closed `bug` issue also gets a proposed Bug origin, whoever fixed it,
except issues labelled `security-fix`. The first run that writes proposals
looks through all closed bugs, including those older than the sample window;
later runs use the previous proposal window's end. Existing saved samples
without proposals do not skip this backfill.

A short session on the configured Scoring Harness and Model uses the Eval
sandbox and memory caps to inspect the fix and only its earlier Git history.
It names an introducing commit only when that history establishes one;
otherwise it records no commit and explains why. A closure without a linked
fix records no commit. GitHub's commit-to-PR associations map a named commit
to its merged pull requests, then to a local Run's Command record when present
or the PR's Built-with line. Human work can have no Run. Multiple historical
PR associations are retained without choosing between them.

The saved window's `bug_origins` list contains these proposals and their
evidence; `thirdshift scores --json` includes it. Session artifacts stay in
the window's private `packets/origins/` directory. Proposals count for nothing
until the Day shift settles them. A plan-limit warning stops Scoring
before another proposal starts and advances no window; retry `thirdshift score`
to analyze that window again. A session or search failure also advances no window.

`thirdshift bug-origins [<issue>] [--json]` lists origins and their state:
`proposed`, `confirmed` or `rejected`. Use an issue URL, or a number when it
identifies exactly one bug across the retained repositories. A settlement
changes the latest proposal for that bug and retains its original evidence.
`--confirm` needs exactly one proposed pull request; `--correct <pull request>`
confirms a different merged pull request in the bug's repository, given its
URL or number. Corrections have `state: "confirmed"` and `settlement.corrected:
true`. `--none` rejects it. Each action also accepts `--json`. Unknown or
ambiguous origins and incomplete GitHub evidence fail without settling anything.
Settlement uses the same machine-wide writer lock as Scoring and human Scores.

A confirmed origin retains its chosen pull request and Run, when known. Each
local Run's `thirdshift show <command> --json` includes
`command.run_metrics.confirmed_bug_origins`, derived from the latest decisions;
rejected and unsettled origins contribute zero. Correcting an origin moves
its attribution to the chosen Run, without rewriting old Command records.
The saved local record path and Command name remain authoritative: if that pair
is missing or belongs to a different pull request, metrics count no match rather
than substituting another Continuation. Only absent or cross-host attribution
uses latest local selection.

Confirmation also creates a private `proposed` Eval task in the `code-review`
suite, creating the suite with its usual dependencies when absent. Its local
repository pins the PR head and its historical merge-base, separately from
the merge tree and later fix. The target's parent at merge time excludes
upstream changes already merged into the PR branch; preserved linear heads
use the first PR commit's parent. It carries the originating issue at merge
time, or the PR body when no issue was linked, and a draft catch criterion
based on the proposal's evidence. Review and
refine that criterion before `thirdshift evals approve code-review <task>`.
Until approval, preparation and Experiments exclude it. Repeated confirmation
keeps the same Eval task, including edits and approval. Rejecting or correcting
an already confirmed origin withdraws its old unapproved proposal, including
prior occurrences of a reopened and reclosed bug; a task the
Day shift independently approved stays in the suite. Security fixes cannot be
selected as origins. Confirmation checks the bug's current labels and refuses
`security-fix`, including a proposal saved before that label. All these files
stay in private Eval storage on this host.

The Command record and Command log live under
`<logs.dir>/JacobStephens2/thirdshift/commands/scoring/`, with start and end lines
in that repository's Activity log. The Command appears in the Inbox, can be
Stopped with `thirdshift stop <command>`, and a failure lands in Needs you.
Another Scoring run on the machine is skipped without recording a Command.
A failed or interrupted search advances no window. Pull-request searches
exceeding GitHub's 1,000-result limit fail visibly rather than saving an incomplete
window. Closed-bug searches split their time ranges to cover larger backfills;
more than 1,000 closures in one second fail visibly without advancing the window.

Your operating system can schedule it weekly, for example:

```cron
0 3 * * 1 /usr/local/bin/thirdshift score >> /tmp/thirdshift-cron.log 2>&1
```

## Building from source

This Markdown is the source for the site's manual. After changing it, run `cd site && npm ci && npm run docs` from a repository checkout and commit `site/docs/index.html` in the same pull request. `cd site && npm test` checks that the generated page is current and readable in a browser; the deployed site needs no build step.

Building needs the Rust toolchain. From a clone of this repository:

```sh
cargo install --path .
```

`cargo install` puts the binary in `~/.cargo/bin`. To update it, pull and reinstall:

```sh
git checkout main && git pull
cargo install --path .
```

If the shell installer also put a copy in `~/.local/bin`, typing `thirdshift` runs whichever copy comes first on your `PATH`, and the installer's copy often does. To see every copy, in the order the shell tries them:

```sh
type -a thirdshift
```

To run the build from source, give its path, as in `~/.cargo/bin/thirdshift <Issue URL>`, or put `~/.cargo/bin` first on your `PATH`, for example in `~/.bashrc`:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
```

`thirdshift version` can't tell the two apart: a build from `main` reports the last released version until the next release bumps it.

Editing a skill in `skills/` has no effect until you rebuild and reinstall ([ADR-0001](adr/0001-rust-binary-with-embedded-skills.md)).

The [Prompts and skills page](https://thirdshift.app/prompts/) is generated from the prompts, the `claude` and `codex` arguments and the skills, and each Session prompt is also generated as a Markdown file in [`prompts/`](../prompts), with a [`README.md`](../prompts/README.md) that lists them with the `claude` and `codex` command lines. `cargo test` fails until they are regenerated after a change to any of them. Regenerate the page and `prompts/` with `UPDATE_PROMPTS=1 cargo test prompts_page`, which also deletes any file in `prompts/` that it doesn't generate.

The integration tests swap in a fake `gh`, `claude`, `codex`, `agy`, `grok`, `muse` and `opencode`, from [`tests/fakes.rs`](../tests/fakes.rs). The first test that needs them builds them with `rustc` (the one on `PATH`, or `$RUSTC`), so the suite needs nothing else on `PATH` beyond `git` and `bash`.

### The Rust version

[`rust-toolchain.toml`](../rust-toolchain.toml) pins the exact Rust version this repository builds with, and the `rustfmt` and `clippy` components CI runs. `rustup` reads it, so `cargo` in a checkout, CI, the release builds and the crates.io publish all use that version, and `rustup` installs it the first time it is needed. No workflow names a version of its own. The file is not in the crates.io package, so `cargo install thirdshift` builds with whatever Rust the machine has.

A newer Rust arrives as a pull request that changes `channel` in that file, so it runs through CI before `main` builds with it. Dependabot opens that pull request within a week of a stable release ([`.github/dependabot.yml`](../.github/dependabot.yml)). To move the pin by hand, set `channel` to the new version in full, patch number included, run the three checks CI runs, and open a pull request with the change and whatever the new compiler's lints asked for:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Releasing

Before pushing anything, the release script runs `thirdshift experiment
--release <commit>` using this checkout's runner and private Eval suites.
It evaluates the content commit beneath the version-only bump, so reruns use
the same Experiment even before a bump branch exists. It pins that commit in a
local Git ref before starting, so a merge to `main` during interruption does
not change the Candidate on restart. The pin is removed after the branch push,
when the pushed bump retains the selection. A pass carries on;
a fail or inconclusive Verdict shows each suite's accuracy and wrong-finding
intervals and asks `release anyway? [y/N]`, including without `--review`.
Only `y` or `yes` continues; EOF or any other answer stops. Pending borderline
and decisive Scores are listed with their recheck commands and polled until
you have rechecked them. No touched suite or a last release with no Trial flag
prints a note and carries on. An interrupted invocation resumes its saved
Trials. A pushed bump branch is checked again before opening its PR, and an
open or merged bump PR before merging or tagging. An already pushed tag only
resumes the release workflow. Python 3 is needed to read the Experiment results.

If `main` gains different content after the Candidate was selected, the script
refuses to push or merge that release under its old Experiment; choose a fresh
release version to measure current `main`. It also checks the merged tree before
tagging, so a concurrent merge cannot publish unmeasured content.

From a clone of this repo, signed in to `gh` and the User config's default Harness, run the release script with the new version:

```sh
scripts/release.sh 0.4.0
```

It works from `origin/main` in a temporary worktree, so your checked-out branch and uncommitted changes don't matter and aren't touched. It opens a pull request from `release-<version>` that bumps the version in `Cargo.toml` and `Cargo.lock` and nothing else. Its body is a summary of the release, then the version diff. The User config’s default Harness, with its configured Model and Effort (Claude Code when no default is set), writes the summary from the version diff and the title, number and body of each pull request merged since the last tag, following the prompt in [`scripts/release-summary.md`](../scripts/release-summary.md). The script uses `cargo run` in this checkout to share thirdshift’s config parser and Harness adapters, and runs the summary in a temporary directory without Factory skills. Because pull request titles and bodies are contributor-editable, no Harness gets its session permission bypass, and each runs with as few tools as its CLI allows. Claude runs with no tools. Codex runs read-only and sandboxed, with its shell, MCP, web, app, plugin, browser and hook tools off. Grok gets only its to-do tool. OpenCode refuses every tool call. Muse has no shell, writes or web, and one model step. agy denies every tool that needs a permission, though it can still read files. A summary that tries a refused tool usually comes back empty, so the script falls back as below. If the config is invalid, the chosen Harness fails, or it returns no summary, the script warns and uses GitHub's generated notes as the summary instead, with a note saying so. It writes the summary before it pushes anything. With `--review` (`scripts/release.sh --review 0.4.0`), it then prints the summary and asks `[y]es / [e]dit / [n]o`: `y` carries on, `e` opens the summary in `$EDITOR` and carries on with what you save, and `n` stops with no branch, pull request or tag pushed. Without `--review`, only a failed or inconclusive Experiment prompts. It waits for the pull request's checks, merges it with a merge commit, then tags that merge commit `v<version>` and pushes the tag. Pushing the tag starts the release workflow (below), and the script then waits for that workflow's run on the merge commit, printing the run's URL so you can watch it. CI's run on `main` for the same commit isn't waited for. Once the release workflow run succeeds, the GitHub Release is published with its final body and the crate is on crates.io, and the script prints the GitHub Release's URL and exits `0`. It prints a line on stderr for each step, stamped with the local time as `HH:MM:SS` as thirdshift's own progress lines are, so a long wait can be told from a stalled one. If the checks fail, it stops before merging and leaves the pull request open. If the release workflow run fails, it exits `1`, naming the failed jobs and the run's URL, and leaves the tag in place: the tag is already public, so rerun the failed jobs with `gh run rerun --failed <run id>` rather than cutting a new version. Neither wait has an overall timeout.

Before it pushes anything, it refuses a release that can't be cut and says why: a version that isn't plain `X.Y.Z` (no leading `v`), a version that isn't higher than the one in `Cargo.toml` on `origin/main`, a `v<version>` tag that already exists locally or on origin, or a CI run on main's current tip that ended without success. It prints a progress line and waits for that tip's CI run to appear and finish, with no overall timeout. While waiting, it fetches `main` again: if the tip moves, it names the old and new commits and waits for the new tip's CI, checking that the requested version is still higher. Once CI succeeds, it bumps the version from that commit and starts the release automatically; if CI ends without success, it names the commit and conclusion and stops without pushing anything.

If it's interrupted, run it again with the same version: it picks up the existing branch or pull request and carries on from the first step not yet done, without the checks above, which the first run passed. With `--review`, it asks about a new summary before opening a pull request for a branch already pushed. Once the tag is on the merge of the pull request, it says the release is already tagged and picks up from the release workflow run: it waits for a run still going, prints the GitHub Release's URL and exits `0` for one that succeeded, and exits `1` as above for one that failed, reporting the run's latest attempt, so a rerun of its failed jobs counts. So you can stop it with Ctrl-C while it waits and run it again to carry on waiting. A `v<version>` tag anywhere else on origin is still refused.

The tag starts the release workflow, generated by [`dist`](https://github.com/axodotdev/cargo-dist) from `dist-workspace.toml`, with the event policy applied by `python3 scripts/generate-release.py` (`--check` verifies freshness). It builds the Linux and macOS binaries and the installer, checks that the Linux binary starts on Rocky Linux 9 and in WSL2 with Ubuntu 24.04, and only then publishes the GitHub Release. It then publishes the same version to crates.io, skipping it if it is already there, and finally replaces the Release's body with the bump pull request's summary, then notes generated from the merged pull requests, then the install instructions. [`scripts/release-notes.sh`](../scripts/release-notes.sh) builds that body; for a tag pushed by hand, with no bump pull request summary, it is the generated notes and the install instructions alone.

## Credits and license

thirdshift is released under the [MIT License](../LICENSE).

Most Factory skills in `skills/` are adapted from Matt Pocock's [mattpocock/skills](https://github.com/mattpocock/skills), changed to run headless with no human in the loop. They are used under the MIT License; its copyright notice is kept in [`skills/LICENSE`](../skills/LICENSE) and is embedded in the binary and written out with the skills on every Ticket run. The `thirdshift-pr` skill also reproduces part of Dex Horthy's `show-me` skill; see [`skills/thirdshift-pr/CREDITS.md`](../skills/thirdshift-pr/CREDITS.md).

The `thirdshift-security-audit` Factory skill is copied from Cloudflare's [security-audit-skill](https://github.com/cloudflare/security-audit-skill) at commit `c1c8a8c`, with only its frontmatter name changed. It is used under the MIT License; Cloudflare's copyright notice and licence are kept in [`skills/thirdshift-security-audit/LICENSE`](../skills/thirdshift-security-audit/LICENSE), with the upstream repository and full commit in [`CREDITS.md`](../skills/thirdshift-security-audit/CREDITS.md). Both are embedded and written out with the skill.
