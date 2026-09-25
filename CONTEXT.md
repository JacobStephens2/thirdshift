# thirdshift

A factory that turns a GitHub issue into a ready-for-review pull request by running unattended agent sessions against it: the humans are the day shift, the agents work the third shift. Quality over quantity: a pull request marked ready for review is one the factory stands behind.

## Language

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

**Failed run**:
A **Run** that ends, including by interruption, without an open pull request from its **Issue branch** that targets the **Base branch**, is mergeable, and has passing CI. Its work is still pushed so nothing is lost (or, if the push fails, its worktree and local **Issue branch** are kept), and its open pull request, if any, is converted back to a draft.

**Issue branch**:
A branch a **Run** works on for one issue: `issue-<n>` (the first), then `issue-<n>-branch-<k>` for k ≥ 2. A number counts as used once its pull request is merged or closed, even if the branch itself was deleted.

**Continuation**:
A **Run** that picks up an existing **Issue branch**, one with no pull request or an open one, instead of starting fresh. When a pull request is open, its base is the **Base branch**.

**Repair**:
A follow-up agent session a **Run** starts after the pull request exists, either to resolve a merge conflict with the **Base branch** or to fix failing CI checks.
