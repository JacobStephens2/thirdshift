# A Merge run merges the pull request itself, not through GitHub auto-merge

A **Merge run** ends with thirdshift itself running `gh pr merge --merge --match-head-commit <sha>` on the head commit whose CI it watched go green, not with `gh pr merge --auto`. The merge happens inside the loop that already keeps the pull request mergeable and green, so a merge that fails because the Base branch moved into a conflict, CI failed, or a **Foreign commit** arrived goes back into that loop: a **Repair** fixes it, or reviews the Foreign commit, and the merge is tried again. Only code an agent wrote or reviewed gets merged, and it merges only once thirdshift has seen it pass.

## Considered Options

- **GitHub auto-merge (`gh pr merge --auto`)**. Rejected:
  - It needs the repository's "Allow auto-merge" setting and branch protection or a ruleset that requires reviews or status checks. GitHub offers it "only on pull requests that cannot be merged immediately". thirdshift runs on any repository its user can push to, most of which have neither, including this one.
  - It waits only for *required* checks. Checks that aren't marked required, and repositories with no required checks, are ignored. thirdshift watches every check on the head commit.
  - Once enabled, GitHub merges whatever head passes the required checks, including commits pushed after thirdshift last looked. Nobody reviews them. thirdshift merges only the commit it watched, and has a Foreign commit reviewed first.
  - The merge would happen after the Run ends. The Run couldn't report the outcome through its exit code, repair a conflict or red CI that appears at merge time, or close the issue and delete the branch after the merge.
- **The agent merges from inside a session**. Rejected: whether the work ships would be the agent's decision about its own work, not a deterministic check.

## Consequences

- A merge that fails with nothing left to fix (no new commits, Base branch unchanged, mergeable, green) is a policy refusal, such as a disallowed merge method or a required review. The Run fails, but it leaves the pull request ready for review, not a draft. thirdshift never parses GitHub's error text to tell policy refusals apart from other failures.
- A repository that requires reviews can't be used with a Merge run; that is the repository owner's choice to have a human look first.
