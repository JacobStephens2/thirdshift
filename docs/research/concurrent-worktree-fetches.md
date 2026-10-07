# Concurrent Worktree changes and fetches (#509)

The error is from a fetch's **local** connectivity check. The fake origin's
advertised refs are not the only refs Git reads: `check_connected` runs
`git rev-list --objects --stdin --not --all --quiet --alternate-refs` in the
fetching repository. `--all` includes other Worktree heads. See Git's
[connectivity check](https://github.com/git/git/blob/v2.55.0/connected.c) and
[revision handling](https://github.com/git/git/blob/v2.55.0/revision.c).

## Evidence

A Linux probe used a bare origin containing only `main` and a launch clone
with one commit. The origin stayed unchanged throughout. One thread repeatedly
added a sibling Worktree on `issue-22`, removed it, and deleted the local
`issue-22` branch; the other repeatedly fetched only `main`.

- Git 2.43.0: 11 failures in 1,000 fetches, reporting
  `fatal: bad object worktrees/sibling/HEAD` followed by
  `origin.git did not send all necessary objects`.
- Git 2.55.0, built from its upstream tag: the exact CI error,
  `fatal: bad object refs/heads/issue-22`, on fetch 106 with syscall tracing.
- A targeted 2.43.0 trace of the failing fetch's `rev-list` child showed it
  reading an all-zero sibling `HEAD`. Git 2.43.0's
  [Worktree creation](https://github.com/git/git/blob/v2.43.0/builtin/worktree.c)
  temporarily writes that value before attaching the head.
- Branch-only churn and cleanup-only probes each passed 1,000 fetches. Those
  negative results do not establish that deletion alone is safe, or identify
  the precise Git 2.55.0 instruction that produced its bad reference. The
  controlled evidence establishes the unsafe overlap with local Worktree
  changes; the original CI logs establish its overlap with cleanup.

Neither probe changed the origin during fetching, so the failure does not
require a non-atomic origin branch deletion. GitHub was not exercised. Since
this connectivity check runs locally, an atomic GitHub ref deletion would
not protect it from local Worktree changes; that is an inference from the
Git code and the unchanged-origin reproduction.

## Fix and regression checks

`Git::fetch`, Worktree acquisition (including failure recovery), and owned
Worktree disposal share `thirdshift-worktree-refs.lock` in the common Git
directory. Acquisition and disposal take it after the existing ownership
lock. Fetch takes only the refs lock, so acquisition's own fetch cannot
recursively acquire a lock it already holds. Ticket sessions still run
concurrently; only these Git operations wait for each other.

The creation regression pauses a real `Worktree::create_fresh`, holds a
registered sibling `HEAD` at the known all-zero transitional value, and
starts a sibling `Worktree::merge_base_branch`. Before the fix, its fetch
fails with the connectivity error. After the fix, it waits for the valid
head to be restored and succeeds. The disposal regression pauses real
scope-exit cleanup and verifies that the sibling fetch waits until the
Worktree and branch have been removed. Both release their gates and join
their workers before asserting results.

No new fetch retry or ignored failure is introduced. Separate real-Git
checks preserve errors for a missing required ref, a missing required
object, and an unavailable origin. The original Spec run scenarios keep
their Base fix sharing and three-simultaneous-Tickets assertions.
