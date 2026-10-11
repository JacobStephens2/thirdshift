# Timing checklist for thirdshift reviews

A "no" is a finding unless the diff shows the risk cannot arise.

## Shared state across Commands

1. **Launch directory and shared `.git`.** Is every operation on the Launch directory's checkout, its `.git` or the origin fetch safe while other Runs, Ticket Runs and Passes use the same repository? If not, one Command's fetch or prune breaks another's Worktree or the user's checkout.
2. **Sampled refs.** Does the change sample `origin/<base>` (or any remote-tracking ref) once and carry that commit, instead of re-reading a ref another Worktree's fetch can move? If not, validation and action use different commits.
3. **Lock vs. GitHub state.** Does exclusion across machines rest on GitHub state (Claim, labels), with the per-machine lock taken before the skip decision and released on every path? If not, two Pickup runs take one Ready issue, or a leaked lock skips every later Pass.
4. **Check-then-act.** Where the change looks something up and then creates or edits it (Issue branch number, Base fix issue, Claim), does it tolerate another Command acting in between? If not, two Runs share one `issue-<n>`, Base fixes duplicate, or a Sweep strips a fresh Claim.
5. **Logs and files.** Are Command logs, Session logs and Activity log lines uniquely named or atomically appended when Commands start in the same second? If not, records interleave or overwrite.

## State that moves while thirdshift waits

6. **Reobserve after every wait.** After a session, CI watch, Base fix or Ticket drain, does the code re-read the captured pull request number and recheck open state, head, Issue branch and Base branch before marking ready, drafting or merging? If not, it acts on a pull request closed, retargeted or given Foreign commits meanwhile.
7. **CI bound to a head.** Is every CI verdict tied to one full head SHA and the current attempt, never a listing from before a push or Check re-run? If not, a stale green readies or Self-merges unwatched code.
8. **Unknown is not success.** Do unreadable, empty, missing or pending observations count as "not established" rather than green, mergeable or merged? If not, a GitHub hiccup ships unverified work.
9. **Base branch moves.** Do "unchanged" judgments (Policy refusal, Declined CI fix, a Security run's skip) compare commits actually observed, and send a moving Base branch through the upstream-move budget? If not, a race is misread as a policy or flaky failure.
10. **Bounded waits.** Is every wait bounded, interruptible and reported? If not, a cron-started Command hangs silently and holds the lock against every later Pass.

## Repeats: cron, Resume, Repair, retries

11. **Cron fires again.** If the same Pass starts again mid-run, or after this one crashed, does it decide from durable state, not process memory? If not, work doubles or Weeding and Fencing stall for good.
12. **Repeatable steps.** Can a Continuation, Resume, Repair, Spec run rerun or retry repeat this step without duplicating a pull request, comment, label, advisory or checklist line? If not, each restart leaves another copy for the Day shift.
13. **Allowances consumed first.** Are one-shot allowances (Check re-run per head, one Resume, one Base fix, Repair cap, upstream-move budget) consumed before the action and keyed so a retry or revisited head cannot refill them? If not, the Repair loop spends sessions and CI without end.
14. **Retried destructive steps.** Does each retry of a destructive Git or GitHub step recheck ownership before and after the attempt? If not, a retry deletes a Worktree, branch or label another Command now owns.

## Every exit path

15. **Salvage on all exits.** On an early return, `?`, panic, SIGINT/SIGTERM or Harness crash, do Failed run salvage, confirmed Self-merge completion and Claim ending still finish, without clearing the recorded interruption or starting new sessions, Tickets or Repairs? If not, work is lost, an issue stays `in-progress`, or an interrupted Command reports success.
16. **Children die with the Command.** Are Harness sessions, Ticket Runs and Base fixes inside the Command's signal and reaping scope? If not, an orphan keeps pushing after its Run has reported.
17. **Half-done transitions.** If the process dies between two steps (pushed but no PR, merged but branch not deleted, label set but no session), does the next Run finish it or name it for the Day shift? If not, the factory leaves state nobody sees.
