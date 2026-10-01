# A head a CI-fix Repair left unchanged gets one Check re-run before the Run fails

A check that fails for a reason that does not repeat, such as a flaky test or a runner or network fault, used to fail the Run: the CI-fix Repair rightly committed nothing, the head was still the commit whose CI had failed, and the Run ended as a **Declined CI fix** although the same commit would pass (#262, where the Failed-run marker commit pushed next went green on the same code). Now, at the point where the Run would fail as a Declined CI fix, thirdshift does a **Check re-run** first: it asks GitHub to re-run the failed jobs of each GitHub Actions workflow run that the branch's own failed checks belong to, once per workflow run, and watches CI on that head again. Green, the Run carries on as for any green head. Red again on a check of the branch's own, the Run is a Failed run with the Declined CI fix cause, with no second Repair and no second re-run for that head.

The trigger is the head being unchanged after a CI-fix Repair, and nothing else. What the Repair concluded, and the "CI notes" it may have written in the pull request body, are never read: this is a deterministic check, as in ADR 0008.

## Considered Options

- **Re-run before any Repair.** Rejected: it adds a full CI cycle to every real failure, which is most of them, to save a Repair session on the rare flaky one.
- **Re-run only when the Repair calls the failure flaky.** Rejected: it leaves the outcome to the agent's judgement and to thirdshift reading its prose, where the unchanged head is a fact thirdshift already checks.
- **Re-run each failed job on its own** (`POST …/actions/jobs/<id>/rerun`) rather than a workflow run's failed jobs. Rejected: one request per check, where failed checks usually share a workflow run, for the sake of the case below.
- **More than one re-run per head.** Rejected: a check that fails twice on the same commit is not one a Run should wait out, and each re-run costs a CI cycle.

## Consequences

- A Check re-run needs every one of the branch's own failed checks to be a GitHub Actions job, told by the check run's details URL naming the workflow run. With a commit status or another app's check run among them, the head could not go green, so nothing is re-run and the Run fails as before. A re-run GitHub refuses, for example for a missing permission, fails the Run the same way, with the Declined CI fix cause and the refusal in a progress line.
- The watch after the re-run must not read the attempt before it. A re-run job gets a new check run, and GitHub lists only the latest check run of a name, so thirdshift waits up to the grace period for the failed check runs' ids to leave the list, then watches as usual. If they never leave, the Run fails as a Declined CI fix rather than trust what it sees.
- Inherited failures are never re-run on their own account and are handled as before, also after a Check re-run. An Inherited failure that shares a workflow run with one of the branch's own failed checks is re-run along with it, since the request is per workflow run.
- A Check re-run starts no Repair and counts against neither the Repair cap nor the upstream-move budget. A later head, from a Repair that committed or the Base branch merged in again, may get its own.
