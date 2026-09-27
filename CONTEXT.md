# thirdshift

A factory that turns a GitHub issue into a ready-for-review pull request, or on request a merged one, by running unattended agent sessions against it: the humans are the day shift, the agents work the third shift. Quality over quantity: a pull request marked ready for review, or merged by the factory, is one the factory stands behind.

## Language

**Day shift**:
The human half of the workflow: shaping work into issues the factory can take on (grilling, specs, tickets), and reviewing the pull requests it delivers, except those a **Merge run** merges itself.
_Avoid_: planning phase

**Issue URL**:
The GitHub issue link the factory is given to implement, e.g. `https://github.com/<owner>/<repo>/issues/<n>`.
_Avoid_: ticket, spec link

**Origin match**:
The rule that the **Issue URL** must belong to the same GitHub repository as the `origin` remote of the directory the factory is started from. A mismatch stops the run before any work happens.

**Base branch**:
The branch the work branches off and the pull request targets. Normally the branch checked out in the directory the factory is started from; in a **Continuation** with an open pull request, that pull request's base instead.

**Factory skills**:
The skills in `skills/`, loaded into the session as the `thirdshift` plugin: adapted copies of Matt Pocock's skills, designed to run headless with no human in the loop.
_Avoid_: the skills (ambiguous with `.claude/skills/`)

**Standards finding**:
A finding from the Standards axis of a code review: the change breaks a documented coding standard or shows a baseline code smell.

**Spec finding**:
A finding from the Spec axis of a code review: the change is missing, gets wrong, or goes beyond what the issue asked for.

**Unaddressed finding**:
A **Standards finding** or **Spec finding** the agent chose not to fix. It is listed in the pull request body so a human can decide on it.

**Run**:
One invocation of the factory on an **Issue URL**, from launch to cleanup.

**Merge run**:
A **Run** asked to end with its pull request merged rather than left for review. It does everything a **Run** does, then a **Self-merge**.
_Avoid_: auto-merge (GitHub's own feature, which thirdshift does not use)

**Self-merge**:
The step at the end of a **Merge run** in which thirdshift itself merges the pull request into the **Base branch** with a merge commit, once it is open, ready for review, mergeable and green on the head commit it merges. No human reviews it first. It ends with the **Issue branch** deleted and the issue closed, by thirdshift if the merge did not close it.

**Policy refusal**:
A merge the **Self-merge** tried that failed, where the round of the Repair loop that followed found nothing to fix: the **Base branch** unchanged, no conflict, CI green or absent, the pull request mergeable. The cause is a repository setting or rule, such as merge commits disallowed or a review required. thirdshift never reads GitHub's error text to decide it. The **Merge run** is a **Failed run** that leaves the pull request ready for review.

**Declined CI fix**:
A CI-fix **Repair** that leaves the head of the **Issue branch** the commit whose CI just failed, having found nothing on the branch to fix, for example because the check also fails on the **Base branch**. Going round again would only watch the same red CI, so the **Run** is a **Failed run** with the cause `CI red on <short sha> and the Repair found nothing to fix on the branch`, rather than spending its remaining Repairs.

**Failed run**:
A **Run** that ends, including by interruption, without an open pull request from its **Issue branch** that targets the **Base branch**, is mergeable, and has passing CI. For a **Merge run**, it is also a Run that ends without its pull request merged. Its work is still pushed so nothing is lost (or, if the push fails, its worktree and local **Issue branch** are kept), and its open pull request, if any, is converted back to a draft, unless the pull request is ready, mergeable and green and only the **Self-merge** could not happen.

**Issue branch**:
A branch a **Run** works on for one issue: `issue-<n>` (the first), then `issue-<n>-branch-<k>` for k ≥ 2. A number counts as used once its pull request is merged or closed, even if the branch itself was deleted.

**Continuation**:
A **Run** that picks up an existing **Issue branch**, one with no pull request or an open one, instead of starting fresh. When a pull request is open, its base is the **Base branch**.

**Repair**:
A follow-up agent session a **Run** starts after the pull request exists: to resolve a merge conflict with the **Base branch** or the **Issue branch** on `origin`, to fix failing CI checks, or, in a **Merge run**, to review and fix **Foreign commits**.

**Foreign commit**:
A commit that appears on the **Issue branch** during a **Run** and was made neither by one of that Run's sessions nor by thirdshift itself. Commits already on the branch when a **Continuation** starts are not Foreign commits. A **Merge run** merges one only after a **Repair** has reviewed it.

**Resume**:
A continuation of the same agent session, started once when that session ended its turn while waiting on background work, which was killed with it. It asks the agent to re-run that work in the foreground and finish. A Resume is not a **Repair** and does not count against the Repair cap.
