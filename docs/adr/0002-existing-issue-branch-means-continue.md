# An existing issue branch means continue, not error

When `origin/issue-<n>` already exists with no PR or an open PR, a **Run** checks out that branch and continues the work, and updates the open PR if there is one. It does not refuse to run and does not start over. This lets work on an issue move between servers and environments, and lets a **Failed run** (whose work is pushed whenever origin accepts it) be picked up where it stopped. If the branch's PR is merged or closed, the Run starts a fresh numbered branch (`issue-<n>-branch-2`, `-3`, …) instead, so finished work is never reopened.

## Consequences

- A Run is not idempotent: re-running an issue builds on whatever is already on its branch, including a failed run's work-in-progress commit.
- The prompt differs between a fresh start and a continuation, so the agent knows to build on existing commits.
- In a Continuation with an open PR, the PR's base, not the branch checked out where the Run was launched, is the Base branch. So continuing from another server works whatever that server has checked out.
- Branch numbers are decided from both the remote branches and the PR history by head branch name, because GitHub often deletes a branch when its PR merges. Without the PR history, a deleted `issue-<n>` would be reused.
