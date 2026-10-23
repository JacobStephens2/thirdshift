# thirdshift

A factory that turns a GitHub issue into a ready-for-review pull request, or on request a merged one, by running unattended agent sessions against it: the humans are the day shift, the agents work the third shift. Quality over quantity: a pull request marked ready for review, or merged by the factory, is one the factory stands behind.

## Language

### Issues and triage

**Day shift**:
The human half of the workflow: shaping work into issues the factory can take on (grilling, specs, tickets), and reviewing the pull requests it delivers, except those a **Merge run** merges itself.
_Avoid_: planning phase

**Issue URL**:
The GitHub issue link the factory is given to implement, e.g. `https://github.com/<owner>/<repo>/issues/<n>`.
_Avoid_: ticket, spec link

**Spec**:
An issue describing a multi-session piece of work, what is being built rather than how each session does its share, made of **Tickets**: its GitHub sub-issues. See [Spec](https://www.aihero.dev/ai-coding-dictionary/spec) in Matt Pocock's AI Coding Dictionary.
_Avoid_: PRD, parent issue

**Ticket**:
An issue scoping one session of work, standing alone or as a sub-issue of a **Spec**. A Ticket in a Spec can block or be blocked by sibling Tickets; the order of work falls out of those GitHub "blocked by" links. See [Ticket](https://www.aihero.dev/ai-coding-dictionary/ticket) in Matt Pocock's AI Coding Dictionary.
_Avoid_: task, sub-task

**Ready issue**:
An open issue a **Pickup run** may take: labelled `ready-for-agent`, written by a **Trusted account**, with its most recent `ready-for-agent` labelling made by a Trusted account and no edit to its title or body since then by an account that is not one, with no label that makes an **Unready Ticket** and no **Claim**, not a sub-issue, not a **Base fix**'s or a **Security run**'s fix issue, with no open blocker, either never started (no **Issue branch** and no pull request) or handed back by the **Day shift** applying `ready-for-agent` after the last **Claim** ended, as recorded in its label history, not a **Spec** whose **Tickets** are all closed, left untouched long enough that whoever is shaping it has finished, and, if it is a Spec, with a **Startable Ticket**. On a Spec, the label says its Tickets are published. A `ready-for-agent` Ticket inside a Spec is not one: it is reached through its Spec, when the Spec is itself a Ready issue.
_Avoid_: queued issue, backlog item

**Trusted account**:
The repository's owner, a member of the organization that owns it, or a collaborator on it: an issue author's `authorAssociation` of `OWNER`, `MEMBER` or `COLLABORATOR`, or equivalent repository membership for an actor with no association field. The rule is fixed, with no **User config** setting. A **Spec run** stops before starting the Run of a **Ticket** whose author is not a Trusted account, or whose title or body an account that is not one edited after the later of the Ticket's and its Spec's last `ready-for-agent` labellings by a Trusted account. A Ticket needs no `ready-for-agent` labelling of its own: the labelling that counts is its Spec's, checked when a **Pickup run** takes the Spec. Before each session of a **Run**, a **Resume** included, its issue is checked again: a title or body edit by an account that is not a Trusted account, made since a few minutes before the Run started, fails the Run instead of starting the session. Each session that works on an issue gets the issue from thirdshift, written into its worktree again before the session starts: its title, its body, and only the comments by Trusted accounts, each other comment replaced by a line saying it was left out. The **Session prompt** and **Factory skills** tell the session to work from that and not to fetch the issue's comments itself.

**Unready Ticket**:
An open **Ticket** labelled with a triage role that says it is not agent work: `ready-for-human`, `needs-info`, `wontfix` or `needs-triage`. A **Spec run** never starts a **Run** for one, nor for a Ticket it blocks. An open Ticket with no triage label is taken. A Ticket with sub-issues of its own is treated as one too.

**Startable Ticket**:
An open **Ticket** in its **Spec**'s repository, with no sub-issues of its own, no open blocker, and not an **Unready Ticket**: one a **Spec run** could start if no Ticket were running. A Spec run starts only these, and a **Spec** with none is not a **Ready issue**.

**Claim**:
The mark that the factory has taken an issue: thirdshift labels the issue `in-progress`, in place of `ready-for-agent` if it has that, when a **Run** or a **Spec run** starts on it, whether started on its **Issue URL** or by an **Architect run**, a **Pickup run** or a **Security run**. A **Ticket**'s Run in a Spec run and a **Base fix** make none. Ending a Claim finishes even after Command interruption. A failed Security fix keeps its Claim even with nothing on `origin`, so it stays with the Day shift. For other Runs, a Claim is released, `ready-for-agent` put back, only when the Run ends with nothing on `origin` to take over and was not ended by a **Stop**: no **Issue branch** and no pull request. Otherwise it stays while the issue is open, whether its pull request is ready for review or the Run failed, until the **Day shift** relabels it, and thirdshift takes the label off once the issue is closed.
_Avoid_: lock (a lock is per machine and ends with its process), assignment

**Claim limit**:
The number of open issues carrying a **Claim** at which a **Pickup run** takes no more, set in the **User config**. It keeps a broken **Base branch** from failing every **Ready issue** in turn, and pull requests from piling up unreviewed.
_Avoid_: WIP limit, concurrency (it counts issues waiting on the **Day shift**, not Runs in flight)

**Sweep**:
A **Pickup run**'s removal of `in-progress` from every closed issue in the repository that still has it, such as one whose pull request was merged by hand, with the issue's other labels kept. Each Pickup run that is not skipped for the lock makes one, before it holds the repository against its **Claim limit**. A Sweep that fails is a warning, never a failure.
_Avoid_: cleanup, garbage collection

### Repository and branches

**Launch directory**:
The directory a **Run** is started from. It must pass **Origin match**, and its checked-out branch is normally the **Base branch**. Base branch validation and optional pulling share one sampled origin commit; **Worktree** acquisition samples independently. A Run works in its own worktree, never in the Launch directory.
_Avoid_: originating workspace, launch repo

**Origin match**:
The rule that the **Issue URL** must belong to the same GitHub repository as the `origin` remote of the **Launch directory**. A mismatch stops the run before any work happens.

**Base branch**:
The branch the work branches off and the pull request targets. Normally the branch checked out in the **Launch directory**; for an **Architect run** or a **Pickup run** whose command names a branch, that branch, whatever is checked out; for a **Ticket**'s **Run** in a **Spec run**, the **Spec branch**; in a **Continuation** with an open pull request, that pull request's base instead.

**Worktree**:
A [factory-owned checkout](docs/guide.md#worktree-ownership-and-cleanup), on an **Issue branch** for a **Run** or detached for an **Architecture review**. It is retained whenever ownership is uncertain or the Run's work could not be preserved elsewhere.

**Issue branch**:
A branch a **Run** works on for one issue: `issue-<n>` (the first), then `issue-<n>-branch-<k>` for k ≥ 2. A number counts as used once its pull request is merged or closed, even if the branch itself was deleted.

### Agent sessions

**Factory skills**:
The skills in `skills/`, each named `thirdshift-<skill>` and placed in each **Run**'s worktree for its **Harness** to find: adapted copies of Matt Pocock's skills, designed to run headless with no human in the loop.
_Avoid_: the skills (ambiguous with `.claude/skills/`)

**Session prompt**:
The message thirdshift starts or resumes an agent session with. It carries the **Run**'s facts (issue, **Base branch**, **Issue branch**, pull request) and names the **Factory skills** to use. Written by thirdshift, not by a human.
_Avoid_: instructions, system prompt

**Harness**:
The headless agent CLI that runs every agent session of a **Command**, named as its CLI is: Claude Code (`claude`), Codex (`codex`), Antigravity CLI (`agy`), Grok Build (`grok`), Muse Code (`muse`) or OpenCode (`opencode`). One per Command, its child Runs and every **Resume** and **Repair** included, except the **Security review**, which runs on the User config's `security.harness` when that is set, falling back to the default Harness when it can't start. Chosen by the command, else, for a **Security run**, the User config's `security.harness`, else the **User config**'s default Harness, else Claude Code.
_Avoid_: agent, provider, backend

**Model**:
The model the **Harness** runs its sessions on, such as Opus 5.5 or GPT-6.1-Sol. One per **Command**, chosen by the command, else the **User config**'s choice for that Harness, else the Harness's own default.

**Effort**:
How hard the **Model** reasons in each session, such as `high` or `max`. One per **Command**, chosen the way the Model is.
_Avoid_: reasoning level, thinking budget

**Resume**:
A continuation of the same agent session, started once when that session ended its turn with background work still running, which was killed with it: thirdshift takes it as work the session was waiting on. Its **Session prompt** asks the agent to re-run that work in the foreground and finish. A Resume that ends the same way gets no second Resume and does not fail the **Run**: the work may have been abandoned rather than awaited, so thirdshift names it in a progress line and carries on, and the steps that follow decide the outcome. If the Run is then a **Failed run**, its cause names the killed work ahead of the step that failed. A Resume is not a **Repair** and does not count against the Repair cap.

### Runs

**Command**:
One invocation of thirdshift that does factory work: a **Run** or **Spec run** started on an **Issue URL**, an **Architect run**, a **Pickup run**, a **Security run**, a **Scoring run**, or an **Experiment**. A child Run, a **Ticket**'s Run or a **Base fix**, is part of the command that started it, not one of its own. Interruption stops its ordinary CLI work and prevents more from starting, while Failed run salvage, confirmed Self-merge completion, Claim ending and Spec accounting still finish; finishing never clears the recorded interruption.
_Avoid_: invocation, job

**Run**:
One invocation of the factory on a single issue that is not a **Spec**, from launch to cleanup, ending in one pull request. Started directly on an **Issue URL**, by a **Spec run** for one of its **Tickets**, by an **Architect run** for the Ticket its **Architecture review** published, by a **Pickup run** for the **Ready issue** it took, or by a **Security run** for a **Security finding** it fixes.

**Spec run**:
One invocation of the factory on a **Spec**'s **Issue URL**: it works through the Spec's **Tickets** in dependency order, starting a **Run** for each Ticket once the Tickets blocking it are done, several at once when the graph allows.
_Avoid_: batch run, spec implementation

**Architect run**:
One invocation of the factory with no **Issue URL**: an **Architecture review** of the **Base branch**, then, unless asked to stop there, a **Spec run** or a **Run** on the **Architect plan** it published. One pass per invocation. It is skipped, doing nothing, when another **Pass** on the same repository is still running on the machine, its Spec run or Run included, when an earlier Architect plan is still open, when an **Architect idea** is waiting for triage, or when the repository has a **Ready issue** or an issue that meets every Ready issue condition except settling, including a **Spec** having a **Startable Ticket**, so that work a human shaped goes first. Skipped is not a failure, and sends no **Run notification**, so that it can be started every few minutes and so follow the last one as soon as its plan is merged.
_Avoid_: improve run, architecture run

**Pickup run**:
One invocation of the factory with no **Issue URL**: it takes the lowest-numbered **Ready issue** in the repository and starts a **Spec run** or a **Run** on it, as the command on that issue's URL would. One issue per invocation, and thirdshift never schedules itself: the operating system's scheduler starts each one. Before dispatch, it reserves the issue across machines and rechecks that it is Ready with no intervening **Claim** since selection. The reservation lasts through the dispatched run's ending, then is released only by its owner; after a crash it waits for the **Day shift** to confirm the owner has stopped and release it. It is skipped, doing nothing, when another **Pass** on the same repository is still running on the machine, its Spec run or Run included, when there is no Ready issue, when the repository is at its **Claim limit**, or when the selected issue is reserved or its selection is stale. Skipped is not a failure, and sends no **Run notification**.
_Avoid_: watch, daemon, queue run, poll

**Security run**:
One invocation of the factory with no **Issue URL**: a **Security audit** of the **Base branch**, then, for each **Security finding**, a session that tries to reproduce it with a proof-of-concept test in a throwaway worktree, and scores the severity of each one it reproduces. When a flag or the **User config** allows it, each reproduced finding is fixed by a **Run**, or by a **Spec run** when one session isn't enough, on issues that say only what the fix changes. Every other finding waits for the **Day shift**, and one nothing reproduces is never fixed unattended. It is skipped, like an **Architect run**, when another **Pass** on the same repository is still running on the machine, when the repository has a **Ready issue**, when one of its findings still waits for the Day shift, when a fix it dispatched failed and is still open, or when the Base branch hasn't changed since the last Security audit that finished. A failed fix is left for the Day shift; no later Security run or **Pickup run** retries it.
_Avoid_: Secure run (it reads as a run that is secure), audit run, security pass

**Pass**:
An **Architect run**, a **Pickup run** or a **Security run**: a **Command** started with no **Issue URL**, which decides before any work whether it is skipped.
_Avoid_: tick, cycle

**Merge run**:
A **Run** asked to end with its pull request merged rather than left for review, by the command it was started with or by the **User config**. It does everything a **Run** does, then a **Self-merge**. A **Ticket**'s Run in a **Spec run** is always a Merge run into the **Spec branch**; the command's ask applies to the **Spec PR**.
_Avoid_: auto-merge (GitHub's own feature, which thirdshift does not use)

**Weeding**:
The factory clearing the detritus that features leave in a repository's codebase as they land, with no human starting it: **Architect runs** started often enough that each follows the last as soon as its **Architect plan** is merged. It yields to work a human shaped, and pauses for the **Day shift** whenever a plan fails or an **Architect idea** waits for triage.
_Avoid_: upkeep, continuous architect, daemon, loop

**Fencing**:
The factory finding and fixing vulnerabilities in a repository's codebase with no human starting it: **Security runs** started as often as the operating system's scheduler is set to start them, with fixing allowed. Like **Weeding**, it yields to work a human shaped, and pauses for the **Day shift** whenever a fix fails or a **Security finding** waits for triage.
_Avoid_: patrol, continuous security, security loop

### Architecture

**Architecture review**:
The agent session that opens an **Architect run**: it scans the **Base branch** for deepening opportunities, skips any already covered by an open issue, and publishes the top recommendation as the **Architect plan**, but only when that recommendation is Strong. Otherwise it files the top recommendation as an **Architect idea** for the **Day shift**, or names the open issue that already covers it, and nothing is implemented. It changes nothing in the repository.
_Avoid_: planning session

**Architect plan**:
The **Spec**, or the standalone **Ticket** when one session is enough, that an **Architecture review** published: one or the other, never a third kind of issue. thirdshift labels it `architect-plan` once the review has ended cleanly, and it stays open until its work is merged or someone closes it. A failed **Run** or **Spec run** on it leaves it open for the **Day shift** to take over; no later **Architect run** retries it, and a **Pickup run** takes it again only if the Day shift hands it back as a **Ready issue**.
_Avoid_: plan, plan doc

**Architect idea**:
The issue an **Architecture review** ends on when its top recommendation is not Strong: the one it filed, or the open issue that already covered it. thirdshift labels it `architect-idea` and `needs-triage`, the second put back on an issue that already covered the idea and had been triaged, since the factory again takes it for the best next move. While any Architect idea is open and still labelled `needs-triage`, every **Architect run** on the repository is skipped: the factory has run out of Strong ideas, and waits for the **Day shift** to triage one, whatever the decision.
_Avoid_: weak plan, suggestion

### Spec coordination

**Spec branch**:
The **Issue branch** of the **Spec** in a **Spec run**, branched off the **Base branch**. Each **Ticket**'s **Run** is a **Merge run** into it, so the Spec's work gathers there before it reaches the Base branch.
_Avoid_: integration branch, feature branch

**Spec PR**:
The pull request from the **Spec branch** into the **Base branch** that closes the **Spec**. It is left ready for review once every **Ticket** is done, or merged by a **Self-merge** when the **Spec run** was asked to merge; a draft listing what is missing otherwise. It opens as a draft once the first Ticket lands, with the **Tickets checklist** in its body. Ticket accounting captures the resumed or created PR by number in the Spec's repository. Every draft or checklist operation reobserves that number and checks its open state, Spec branch, Base branch and repository ownership; an invalid or missing PR is never replaced during accounting. A landing must pass those checks before the Spec review can start. Delivery explicitly ends this accounting capture before the Spec review and captures its own PR afterward.

**Tickets checklist**:
The part of the **Spec PR**'s body, between fixed markers, that thirdshift writes: one line per **Ticket**, ticked once it is done, with its pull request, or saying whether it is not started, running, failed, interrupted, blocked, in a cycle or unready. It is rewritten as each Ticket starts and ends, and put back after the **Spec review** rewrites the body.

**Spec review**:
The agent session a **Spec run** starts once every **Ticket** is done: it reviews the whole **Spec branch** against the **Base branch** and the **Spec**, fixes what it agrees with, and writes the **Spec PR**'s description, listing the rest as **Unaddressed findings**.

**Failed spec run**:
A **Spec run** that ends, including by interruption, with any of its **Tickets** not done, or with its **Spec PR** not ready, mergeable and green (or, when asked to merge, not merged). It still takes every Ticket it can reach, and a failed Ticket stops only the Tickets it blocks. After Command interruption it starts no new Ticket or Spec review, drains its active Tickets, and finishes publishing their final Tickets checklist and outcome lines.

### Reviews and findings

**Standards finding**:
A finding from the Standards axis of a code review: the change breaks a documented coding standard or shows a baseline code smell.

**Spec finding**:
A finding from the Spec axis of a code review: the change is missing, gets wrong, or goes beyond what the issue asked for.

**Security finding**:
A vulnerability a **Security review** or a **Security audit** found. One the **Run**'s own change introduced, left unaddressed, holds a **Self-merge** into the **Base branch**. One already on the Base branch is never written up in public before it is fixed: it is recorded privately for the **Day shift**, as a draft security advisory on a public repository or a labelled issue on a private one. Once recorded, it waits for the Day shift, unless it is reproduced and fixing is allowed, until the Day shift triages it: by closing its draft advisory, publishing it or giving it a severity, or by taking `needs-triage` off its issue. That decision also settles whether it was real.
_Avoid_: vulnerability report, security issue

**Unaddressed finding**:
A **Standards finding** or **Spec finding** the agent chose not to fix, listed in the pull request body with what refutes it: a line of code, a plan decision, an ADR or a run that exercises the finding. When a Spec or the Day shift must decide, the entry instead links the `needs-triage` issue filed for that decision, or the open issue already covering it. A **Security finding** the change itself introduced, which the **Security review** left unfixed, is listed there too, and holds a **Self-merge** into the **Base branch**.

**Security audit**:
The agent session that opens a **Security run**: it audits the **Base branch** for vulnerabilities, and records each **Security finding** privately for the **Day shift**. It changes nothing in the repository, and writes nothing public about a vulnerability.
_Avoid_: security scan, security review (a **Security review** reviews one Run's change)

**Security review**:
When a flag or the **User config** asks for one, the agent session a **Run** starts after its opening session, or a **Spec run** starts on its **Spec PR** after the **Spec review**: it reviews the branch's change against the **Base branch** for vulnerabilities, and fixes each **Security finding** it can show with a failing test. Each **Ticket**'s Run and **Base fix** gets one when its Command asked for one. It runs on the User config's `security.harness` when that is set, whatever **Harness** the Command chose, and on the Command's Harness otherwise. When that Harness can't start, it falls back to the User config's default Harness, or Claude Code when none is set. A Security review that a model refused, or that ended early, holds a **Self-merge** as an unaddressed Security finding does.
_Avoid_: security axis, security audit (a **Security audit** covers the whole Base branch)

### Delivery and recovery

**Delivery**:
The [workflow](docs/guide.md#delivery-and-pull-request-identity) that takes a **Run**'s work, or a **Spec run**'s **Spec PR**, from its opening session to a ready-for-review or merged pull request. It includes publication, the **Repair loop** and, in a **Merge run**, the **Self-merge**.

**Self-merge**:
The step at the end of a **Merge run** in which thirdshift itself merges Delivery's captured pull request by number in its repository, with a merge commit at the exact head whose CI it watched. A fresh observation validates its expected **Issue branch** and **Base branch**, open state, readiness and mergeability immediately before the request. GitHub guards the requested head, but offers no atomic expected-base guard against a retarget between that observation and the server merge. No human reviews it first. A failed or interrupted request counts as merged only when a completion observation of the same number confirms the requested head and expected branches. Completion then continues even after Command interruption. It ends with the **Issue branch** deleted, the issue closed by thirdshift, unless it is already closed, and the issue's **Claim** removed.

**Repair loop**:
The rounds a **Delivery** goes through until the pull request's head is mergeable with the **Base branch** and its CI is green or absent. Each round merges the Base branch in (and, in a **Merge run**, any **Foreign commits**), pushes, watches CI, and starts a **Repair** for a conflict or for the branch's own red checks. When the Base branch moved while green or absent CI ran, a Merge run goes round again to merge it in. A Delivery that leaves the pull request ready for review stops on that head if GitHub reports the pull request mergeable, and the move is not counted; otherwise it goes round again. Repairs and upstream moves have fixed budgets.

**Repair**:
A follow-up agent session a **Run** starts after the pull request exists, or a **Spec run** starts on its **Spec PR** after the **Spec review**: to resolve a merge conflict with the **Base branch** or the **Issue branch** on `origin`, to fix failing CI checks, or, in a **Merge run**, to review and fix **Foreign commits**.

**Foreign commit**:
A commit that appears on the **Issue branch** during a **Run** and was made neither by one of that Run's sessions nor by thirdshift itself. Commits already on the branch when a **Continuation** starts are not Foreign commits. A **Merge run** merges one only after a **Repair** has reviewed its resulting tree, unless that tree is identical to the Run's own head before reconciliation and is already covered by its existing review. A tree-identical batch still counts as an upstream move, but uses no Review Repair.

**Policy refusal**:
A **Self-merge** blocked by a required review, or a merge it tried that failed where the round of the Repair loop that followed found nothing to fix: the **Base branch** unchanged, no conflict, CI green or absent, the pull request mergeable. The cause is a repository setting or rule, such as merge commits disallowed or a review required. thirdshift never reads GitHub's error text to decide it. A blocking review ends the Run as **review required** without a merge attempt or another Repair round. The **Merge run** is a **Failed run** that leaves the pull request ready for review.

**Declined CI fix**:
A CI-fix **Repair** that found nothing on the branch to fix, for example because the failure is not the branch's but was not recognised as an **Inherited failure**, since the **Base branch** commit had no finished result for that check: once the Repair ends and the **Base branch** is merged in again, the head of the **Issue branch** is still the commit whose CI just failed. That head gets its one **Check re-run** for the **Delivery**; if its own checks are red again, or they cannot be re-run, the **Run** is a **Failed run** with the cause `CI red on <short sha> and the Repair found nothing to fix on the branch`, rather than spending its remaining Repairs. Own checks found red on that head in any later round, including after a refused **Self-merge**, fail with the same cause without another CI-fix Repair or Check re-run. Inherited-only failures keep their usual handling, and a different head may receive its own CI-fix Repair and Check re-run.

**CI watch**:
Observing CI on a full **Issue branch** head against its selected **Base branch** commit, including eligible **Abandoned job** re-runs and **Check re-runs**, with allowances remembered across one **Delivery**. CI watch reports CI outcomes or a **Declined CI fix** and starts neither **Repairs** nor **Base fixes**.

**Check re-run**:
The [one re-run](docs/guide.md#check-re-run-allowance) of a branch's own failed checks allowed per head commit within a **Delivery**, after a CI-fix **Repair** leaves that head unchanged and before a **Declined CI fix**. It gives a transient CI failure one chance to pass without another Repair. It is separate from the re-run of **Abandoned jobs**, which comes before any Repair.
_Avoid_: retry, re-trigger, CI re-run

**Abandoned job**:
A GitHub Actions job among the branch's own failed checks that GitHub ended as a failure while none of its steps failed and at least one had not completed, as when its runner is lost: its code never ran to completion. It is told from the job's and its steps' fields alone, never annotation or log text. When every one of the branch's own failed checks on a head is one, thirdshift re-runs them with no **Repair**, once per full head commit across one **Delivery**, and watches CI on that head again; red again, or a re-run refused or never seen, the head gets a CI-fix Repair as for any red CI. It comes before, and is separate from, that head's **Check re-run**, which a head may also get after the Repair, and like it counts against no budget. Mixed with ordinary failures of the branch's own, it is handed to the CI-fix Repair with them.
_Avoid_: lost runner, infrastructure failure

**Inherited failure**:
A failed check on the head of the **Issue branch** that also failed, under the same check name, on the **Base branch** commit that head last merged in. It is not the branch's to fix: no **Repair** is started for it, and a **Run** whose only red checks are Inherited failures is a **Failed run** with the cause `CI red on <check>, which also fails on <base> at <short sha>; fix <base> first`. A check with no finished result on that Base branch commit is not one. When asked to, by a flag or the **User config**, the Run starts a **Base fix** before failing. A Run that fails on one with no Base fix taken links each check where it fails on the Base branch, after its cause, and, if neither a flag nor the User config decided against a Base fix, offers one: the command that starts the Run again with one allowed, and the User config setting that allows one for every Run.
_Avoid_: base-red check, flaky check, shared failure

**Base fix**:
A **Merge run** into a **Run**'s **Base branch** that thirdshift starts when that Run meets an **Inherited failure**, on an issue thirdshift writes naming the failing checks, if it was asked to by a flag or the **User config**. The Run waits for it, then merges the new Base branch in and watches CI again; one Base fix per Run, and a Run that finds an open Base fix issue for the same failure waits on that one instead. A Base fix sees no Inherited failures, starts no Base fix of its own, and sends no **Run notification**: the Run that started it reports it, and is a **Failed run** naming its issue if the Base fix fails.
_Avoid_: hotfix, base repair (a **Repair** works on the Issue branch)

**Failed run**:
A **Run** that ends, including by interruption, without an open pull request from its **Issue branch** that targets the **Base branch**, is mergeable, and has passing CI, or a confirmed completed merge as described under **Delivery**. For a **Merge run**, it is also a Run that ends without its pull request merged. Even after Command interruption, any unfinished merge is aborted and its work is still pushed so nothing is lost (or, if the push fails, its worktree and local **Issue branch** are kept), and its open pull request, if any, is converted back to a draft through the same finishing path, unless the pull request is ready, mergeable and green and only the **Self-merge** could not happen. It ends with its **Recovery**.

**Recovery**:
The lines a **Failed run** ends with, saying where it stopped, what work was preserved and where, and the command that continues or retries it; built only from factory-owned facts, never agent output.

**Continuation**:
A **Run** that picks up an existing **Issue branch**, one with no pull request or an open one, instead of starting fresh. When a pull request is open, its base is the **Base branch**.

### Configuration

**User config**:
The per-machine settings file in the user's home folder that sets thirdshift's defaults for every **Run** started on that machine, such as the **Run notification** address, whether every Run is a **Merge run**, whether a Run may start a **Base fix**, whether a Run first brings the **Launch directory**'s checkout of the **Base branch** up to date with `origin`, and the **Harness**, with a **Model** and **Effort** for each Harness. With no User config, or one that says nothing about a setting, a Run does only what its command asks for.

**Credentials**:
The per-machine secrets file next to the **User config**, readable only by its owner, that holds the Resend API key a **Run notification** is sent with, and the **Console**'s sign-in emails. The key in the environment wins over it; it is what lets a **Run** started from cron, `nohup` or an agent's shell find the key. Only commands that send email read it, so the User config holds no secret.
_Avoid_: secrets file, key file

**Setup**:
Writing the **User config** by answering a few questions, one per setting that matters most, with every other setting written out at its default so the file shows everything that can be changed. With **Run notifications** on, it also asks for the Resend API key and writes the **Credentials** when one is entered. Offered by the first **Run** on a machine with no User config, and run again at any time to change the answers.
_Avoid_: init, onboarding, configure

### Logs and records

**Session trace**:
The events and timed work of one agent session, one per session, a **Resume** and each **Repair** included. Each belongs to its owning **Command**'s **Trace**. Kept with its repository's other logs, named for the session's kind and stamped with the local time its command started, so all of a command's Session traces sort together. The start stamp is a display and grouping fact; distinct Commands have distinct storage ownership even when their stamps agree. Model evidence is kept beside each Session trace and belongs to the exact owning **Command**, including across child Runs. It survives an interrupted stream and is replaced by completed retained evidence only when a report is available, even when that report has no observed Models; an absent report leaves the stream evidence intact. Requested **Model** and **Effort** choices are recorded separately from observed Models. A session that ran the code review has its reviewers' reports kept beside its Session trace.
_Avoid_: Session log, transcript, session file

**Trace**:
A **Command**'s whole tree of work, its child **Runs** and all of their **Session traces** included. Their shared identity keeps the tree together across sessions.

**Command log**:
Everything one **Run**, **Spec run**, **Architect run**, **Pickup run** or **Security run** printed, kept in a file of its own with its repository's other logs, stamped like its **Session traces**. The start stamp is for display and grouping; Commands with the same start time still have distinct storage ownership for their Command logs, Session traces and **Command records**. An Architect run's, Pickup run's or Security run's covers the Spec runs or Runs it dispatched, and a Spec run's covers its **Tickets**' Runs, as a Run's covers its **Base fix**: none of those keeps one of its own. It is kept once the command starts work: a Pickup run, Architect run or Security run skipped before doing any work keeps none, and neither does a Run that fails before it starts work, such as on **Origin match**.
_Avoid_: cron log, output log, run log (a **Run** is one kind of command)

**Activity log**:
One running record per repository of what the factory did there: a line when a **Run**, **Spec run**, **Architect run**, **Pickup run** or **Security run** starts work, naming its **Command log**, a line when it is given a **Stop** or a **Dismissal**, or a Dismissal is taken back, naming who asked and whether from the CLI or the **Console**, and a line when it ends, with its outcome. A skipped Architect run, Pickup run or Security run writes a line only when its reason differs from the last one recorded for its kind, so a repository that sits idle shows one line, not one per pass. It is what thirdshift writes about a scheduled pass, in place of output to the scheduler.
_Avoid_: cron log, pass log, history

**Command record**:
What a **Command** keeps on its machine about where it is: its process, its issue if it has one, each child **Run** with the stages it has been through and when each began, and, once it ends, its outcome. Kept beside its **Command log** after the Command ends, so the **Console** and the CLI can show live and past Commands alike. A record whose process is gone, with no outcome, belongs to a Command that died at its last stage.
_Avoid_: live record, state file, status file

**Run notification**:
A message thirdshift sends when a **Run**, a **Spec run**, an **Architect run** or a **Security run** ends, whatever its outcome (ready, merged, failed or interrupted; for an Architect run that dispatched nothing, plan published, idea filed, idea already filed or review failed), to the address given with the email flag or the default in the **User config**. Its body tells everything its subject does and names the owning **Command**, so the body alone carries it. Each sends one only when asked to, by the flag or by the User config; a Spec run's notification lists each **Ticket**'s outcome, and a Ticket's **Run** never sends one of its own. After an **Inherited failure** it carries, beside the cause, what the Run says on stderr: where the checks fail on the **Base branch**, and the offer of a **Base fix** if nobody decided against one. An Architect run's notification tells how its **Architecture review** ended, naming the plan or idea issue, and how the Spec run or Run it dispatched ended, which sends none of its own; a skipped one sends none. A **Pickup run** that took a **Ready issue** sends one naming the Pickup run as its Command and the Spec run or Run it dispatched, which sends none of its own; a skipped one sends none. A **Security run**'s notification lists each **Security finding**'s severity, title and private link, never its write-up, since the message passes through a third party. When a Run leaves a reproduced finding unfixed and nobody decided against fixing, it offers both the command and the User config setting that allow fixing; a skipped one sends none. Its Session models section collects Model evidence kept beside each **Session trace**, belonging to the exact owning **Command** across child Runs. Stream evidence survives interruption and is replaced by completed retained evidence only when a report is available; a present report with no observed Models is authoritative. Requested **Model** and **Effort** choices remain distinct from observed Models, with requested/default fallback wording when none were reported. Failing to collect evidence or send one never changes the outcome.
_Avoid_: completion email, alert

### Console

**Console**:
The web page a machine serves that shows every **Command** started on it, opening on its **Inbox**, with each one's child **Runs**, their stages and their logs, and from which a live Command can be stopped and an Inbox row given a **Dismissal**. Each machine serves its own, behind a sign-in. It does nothing the CLI can't do: every view and action it offers is also a command.
_Avoid_: dashboard, floor, UI, web UI

**Stop**:
Ending a live **Command** on request, from the CLI or the **Console**. It is an interruption, and the Command cleans up as after any other, but its issue is left for the **Day shift**: labelled `needs-triage`, with `ready-for-agent` not put back, unless the Stop asks for the issue to go back in line. Stop waits for the Command to end, then reports its outcome and any branch or pull request it left. The **Activity log** records who asked for it. A Stop of a **Spec run** stops its **Tickets**' Runs with it.
_Avoid_: kill, cancel, abort

**Console account**:
An email address allowed by the **User config** to sign in to a machine's **Console**, with its password. Created, and its password reset, by proving the address with a code sent to it, or from the CLI. Taking an address off the allowlist ends its sign-ins at once; the account is kept, so adding it back restores it.
_Avoid_: user, login

**Console log**:
One running record per machine of what happened to its **Console accounts**: sign-ins, failed sign-ins, codes sent, passwords set and sign-outs, each naming the account. What people did to **Commands** goes in the **Activity log** instead, and what they only looked at is recorded nowhere.
_Avoid_: audit log, access log

**Inbox**:
The **Console**'s first page, and `thirdshift status`'s order: a machine's **Commands** grouped by what each asks of the **Day shift**. **Needs you** comes first. Ready for review holds Commands that ended ready, with a pull request waiting, until GitHub shows it merged or closed, or a later Command on the same issue starts on the machine. Like Needs you, it keeps a row until it is dealt with, not until it is old: age clears only a row GitHub can't speak for. Landing holds live Runs at CI watch or **Self-merge**. Working holds every other live Command. Done holds Commands that ended in the last day, and Resolved holds older ones with nothing waiting, until the period the **User config** sets has passed since they ended or stopped waiting. Commands older still are only in the history. A **Dismissal** takes a row out of Needs you or Ready for review, and a row that leaves either one says why.
_Avoid_: dashboard, feed, queue

**Needs you**:
The **Inbox**'s first group: what won't come right unless the **Day shift** acts. Live problems come first, **Silent** then **Slow** Runs, and then the ended ones: a Command that died, one that failed, a **Stop** that left its issue `needs-triage`, and a **Security finding** waiting on a decision. An ended one leaves once GitHub shows it settled: its issue closed or its pull request merged, or those of the Run it dispatched, or, for a Stop, `needs-triage` taken off, or, for findings, none still waiting. It also leaves when a later Command on the same issue starts on the machine. Age clears only one GitHub can't speak for, such as a Command with no issue, once it is older than the period the **User config** sets. Any of them, live or ended, leaves on a **Dismissal**.
_Avoid_: alerts, attention, triage queue

**Silent**:
A live **Run** whose **Session trace** and **Command log** have both stopped growing for longer than the silent time the **User config** sets, 10 minutes unless set: it may be stuck. Stages that wait on something else, such as CI watch, write a line at intervals while they wait, so silence means the same thing in every stage. A stall is silence, not a long stage: a Run that is past its **Usual range** but still logging is **Slow**.
_Avoid_: stalled, stuck, hung

**Slow**:
A live **Run** whose stage has gone past the top of that stage's **Usual range** while its logs keep growing. It is still working; it only needs a look, unlike a **Silent** one.
_Avoid_: late, overdue, stalled

**Usual range**:
How long a stage usually takes in a repository: the middle half of that stage's times over the repository's recent **Command records** on the machine, shown only once there are enough of them. It is what makes a Run **Slow**.
_Avoid_: average, baseline, estimate

**Dismissal**:
A person's judgment, from the CLI or the **Console**, that a row in **Needs you** or Ready for review asks nothing more of the **Day shift**. The row leaves that group, with every **Command** it stands for, and nothing else changes: a live Command keeps running, and its issue, its labels and its pull request stay as they are. It holds until the row turns into a kind it wasn't when dismissed, such as a **Slow** Run that then dies, or until someone takes it back. The **Activity log** records who made it, and who took it back.
_Avoid_: acknowledgement, archive, snooze, mute, hide

**Theme**:
How the **Console** looks, its colours, fonts, shapes, effects and motion, over a layout that every Theme shares. thirdshift comes with some, and a user can make their own. Choosing one changes how the page looks, never what it shows.
_Avoid_: skin, style, variant

### Evals

**Eval**:
A test of how well thirdshift's agents do their work: offline, on an **Eval suite** in an **Experiment**, or online, on a sample of merged work in a **Scoring run**.
_Avoid_: evaluation, benchmark

**Eval suite**:
The **Eval tasks** that test one kind of session, such as the code review or the **Spec review**.
_Avoid_: dataset, golden dataset, test set

**Eval task**:
One past situation in an **Eval suite**: a repository as it once was, the kind of session to run on it, and what success means there, such as a bug the review must catch.
_Avoid_: task, case, test case, example, sample

**Arm**:
One setup that an **Experiment** compares: a build of thirdshift, with its **Factory skills** and **Session prompts**, and a **Harness**, **Model** and **Effort**. The **Baseline arm** is the current setup, and the **Candidate arm** holds the change being weighed.
_Avoid_: variant, provider, configuration

**Trial**:
One attempt at an **Eval task** by one **Arm**, in an agent session of its own, within an **Experiment**. A Trial is not a **Run**, and writes no **Command record** of its own.
_Avoid_: run, attempt, sample

**Void Trial**:
A **Trial** that failed for a reason outside its **Arm**, under rules fixed with the **Bar**, such as a sandbox that didn't start or a plan-limit stop. It is re-run, and kept out of the **Verdict**.
_Avoid_: errored Trial, infrastructure failure

**Grader**:
What judges a **Trial** against its **Eval task**'s success criteria, or a **Run** in a **Scoring run**'s sample against the criteria merged work is held to: a check in code, an agent session, or the **Day shift**.
_Avoid_: judge, evaluator, scorer

**Score**:
What a **Grader** decides about one **Trial** against its **Eval task**'s success criteria, or about a sampled **Run** on one criterion: whether it passed, and why.
_Avoid_: grade, rating, feedback

**Experiment**:
The **Command** that compares a **Baseline arm** with a **Candidate arm** on the **Eval suites** a change touches, against a **Bar** fixed before it starts, and ends in a **Verdict**. It shows whether the change makes the agents better or worse before it reaches production.
_Avoid_: replay, eval run, benchmark run, A/B test

**Bar**:
The rules an **Experiment**'s **Verdict** follows, fixed before it starts: the margin, the credible level, the prior, the seed and draw count, the **Trials** per **Eval task**, and the rules for **Void Trials**.
_Avoid_: threshold, gate, acceptance criteria

**Verdict**:
What an **Experiment** ends in under its **Bar**: pass, fail or inconclusive.
_Avoid_: result, outcome

**Scoring run**:
The weekly **Command** that evaluates production work. A **Grader** gives **Scores** to a sample of the week's merged factory pull requests, whichever machine made them, and a session proposes a **Bug origin** for each bug fixed that week.
_Avoid_: grading run, tasting, production monitoring

**Run metrics**:
What a **Run**'s **Command record** and **Session traces** show about how it went, such as its outcome, **Repairs**, CI rounds, **Unaddressed findings**, stage times, tokens and tool errors. Each is kept apart, never combined into one number.
_Avoid_: Run score, KPI

**Bug origin**:
The pull request that introduced a bug, and the **Run** that made it if a Run did, as a session proposed it and the **Day shift** confirmed it.
_Avoid_: culprit, blame
