# An Abandoned job is re-run before any Repair

A GitHub Actions job whose runner GitHub lost ends as `failure` with a step still in progress and none failed. Its code never ran to completion, so a CI-fix **Repair** given it has nothing to fix: one did nothing for 81 s and 420,084 input tokens on Spec PR #842, spending one of the Delivery's 5 Repairs (#908). Now such a job is an **Abandoned job**. When every one of the branch's own failed checks on a head is one, thirdshift asks GitHub to re-run the failed jobs of their workflow runs, with no Repair, and watches CI on that head again, once per head for the complete Delivery. Green, the Run carries on as for any green head; red, it carries on as for any red CI.

What makes a job abandoned is read from the jobs API's job and step fields alone: the job's conclusion is `failure`, no step's is `failure`, and at least one step is not completed. Annotation text such as "The hosted runner lost communication with the server" and job logs are never read, as with ADR 0008 and ADR 0010.

## Considered Options

- **Re-run before any Repair**, the option ADR 0010 rejected. Still rejected: that adds a CI cycle to every real failure to save a Repair on the rare flaky one, and whether a failure is flaky is a judgement. An Abandoned job is not a judgement. GitHub reports that the code never finished, so there is nothing for a Repair to read, and the extra CI cycle is spent only where a Repair could do nothing.
- **Fold it into the Check re-run.** Rejected: the Check re-run follows a Repair that left the head unchanged, so it cannot save that Repair. The two stay separate, each with its own allowance per head, and a head may get both, in that order.
- **Read the annotation or the log for the runner-lost message.** Rejected: it is prose GitHub may reword, where the step states are fields.
- **Re-run abandoned jobs mixed with ordinary failures.** Rejected: the ordinary failures need a CI-fix Repair anyway, and the Repair's new head gets fresh CI.

## Consequences

- Steps are read once per watch that ends red, and only when every one of the branch's own failed checks is a GitHub Actions job concluded `failure`, stopping at the first that is not abandoned, since only all of them being abandoned changes anything. Inherited failures, commit statuses and other apps' check runs are never read or re-run for this. A job whose steps cannot be read is not abandoned: the Run says so and carries on as for any red CI, rather than failing where it would have started a Repair.
- The request and the watch after it are the Check re-run's: one request per workflow run, and the watch waits up to the grace period for the old check run ids to leave the listing before it trusts what it sees. If a request is refused, or no new attempt appears in time, the red CI first watched is handled as before, with a CI-fix Repair.
- A head whose re-run came to an Abandoned job again, or any later watch of that head, gets a CI-fix Repair as before, and after it, if the head is unchanged, its Check re-run. A head whose Check re-run has been consumed gets no Abandoned job re-run.
- The re-run starts no Repair and counts against neither the Repair cap nor the upstream-move budget. The Command log names it: `<check> lost its runner on <short sha>; re-running it`.
