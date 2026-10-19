# Commands

## Usage

```sh
cd ~/repos/widgets          # a clone of the issue's repository, on the Base branch
thirdshift https://github.com/acme/widgets/issues/7
```

You can use a positive decimal issue number anywhere a Run takes an Issue URL:

```sh
thirdshift 7
thirdshift merge 7
thirdshift --no-merge 7 harness codex
```

The number resolves against the Launch directory's `origin`, including from nested
directories in the clone. Supported origins are `https://github.com/owner/repo`,
`git@github.com:owner/repo` and `ssh://git@github.com/owner/repo`, with or without
`.git`. The Run uses the same Issue URL and Origin match as the full-URL form.
Zero, overflow and malformed numbers are rejected. A missing or non-GitHub
`origin`, or starting outside a clone, stops shorthand resolution before any
session or tracker change. All Run options work with either form.

A Run takes minutes to tens of minutes.

To have the Run merge its pull request instead of leaving it for your review, start a **Merge run**:

```sh
thirdshift merge https://github.com/acme/widgets/issues/7
```

`thirdshift --merge <Issue URL>` does the same, and either goes before or after the URL. The agent gets the same implement prompt either way; the differences are the Self-merge at the end and the review of [Foreign commits](runs.md#foreign-commits-in-a-merge-run). To make every Run on a machine a Merge run, set `merge.always` in the [User config](configuration.md#user-config); `no-merge` (or `--no-merge`), before or after the URL, then makes one Run end ready for review instead. Giving a flag twice, or `merge` together with `no-merge`, is an argument error.

A Merge run ends in one of four ways:

- **Merged**: exit `0`, the pull request's URL on stdout, and `PR <url> is merged` as stderr's last line.
- **Review required**: the PR's `reviewDecision` is `REVIEW_REQUIRED` or `CHANGES_REQUESTED` just before a Self-merge attempt. Exit `1`, the PR URL on stdout, and `PR <url> needs an approving review before it can merge; it is ready for review` on stderr and in the Run notification. The Run notification and Activity log summary name `review required`. The PR stays ready, the Claim is kept, and no merge attempt or further Repair round runs. An empty or `APPROVED` decision, or a failed read, leaves the existing merge behavior in place.
- **Policy refusal**: GitHub refused the merge and a round of merging the Base branch and watching CI found nothing left to fix. Exit `1`, the pull request's URL on stdout, and GitHub's error on stderr. The pull request stays open and ready for review, not a draft, for you to merge by hand or to change the repository's settings. thirdshift never reads GitHub's error text to decide this.
- **Any other failure**: a [Failed run](runs.md#failed-runs), as for a Run without `merge`.

To be emailed when the Run ends, add `--email` (or `email`), optionally followed by the address, before or after the URL and alongside `merge`:

```sh
thirdshift --email you@example.com https://github.com/acme/widgets/issues/7
```

It sends one [Run notification](configuration.md#email), whatever the outcome. To have every Run on a machine send one, set `email.always` in the [User config](configuration.md#user-config); `no-email` (or `--no-email`) then skips it for one Run.

- **stdout** carries only the pull request's URL: on success, and on a Failed run that leaves an open pull request, a draft or, after a policy refusal, one ready for review. The exit code tells the two apart, so script it as `url=$(thirdshift "$issue") && echo "ready: $url"`.
- **stderr** carries everything else: errors, cleanup problems, and progress lines while sessions run. Each line starts with the local time it was printed, as in `thirdshift: 12:14:49 pushing issue-7`, so a quiet terminal shows how long the Run has been on its last step. The first line of a Run, a Spec run or a Pass also names the date and the UTC offset, as in `thirdshift: 12:00:01 starting on https://github.com/acme/widgets/issues/7, 2026-10-03 -0400`, so a log that collects many commands, such as a crontab line's, reads as a dated list. Everything on stderr and stdout is kept in the command's [Command log](observing.md#logs) too. A top-level Run or Spec run prints its elapsed wall time near the end, as `Took: 1h 2m 3s`, before its final outcome or failure cause. This also covers graceful interruption, works without email, and adds no summary for child Runs or skipped Passes. A successful Run's last line names the pull request too: `PR <url> is ready for review`, or `PR <url> is merged` after a Merge run, followed only by a `warning:` line if a [Run notification](configuration.md#email) can't be sent.
- **Exit code** `0` means the Run ended with a pull request the factory stands behind, merged in a Merge run. Once the Self-merge has merged, the Run succeeds even if deleting the Issue branch on `origin`, closing the issue or removing `in-progress` from it then fails: the merge can't be undone, so each failed step is a `warning:` line on stderr naming the command to run by hand, and the Run still exits `0` with the URL on stdout. Ctrl-C likewise: before the merge it makes a Failed run, after it thirdshift finishes these steps and exits as merged. `2` means the command is one thirdshift can't use: the issue argument is missing or isn't a GitHub Issue URL or positive decimal number, there is an argument other than the issue and the Run flags, a Run flag is repeated or contradicts another, or `parallel` isn't followed by a whole number from 1 up; the error and the help text go to stderr, before any work. Failure to resolve a number against the clone's origin exits `1`. A [User config](configuration.md#user-config) thirdshift can't use exits `1`, also before any work. Any other failure exits `1`.

During a CI wait, `CI on <short sha>: 3 still running out of 4 checks listed so far` counts the checks GitHub currently lists, including commit statuses. Dependent jobs and matrix jobs may become visible later, so both the pending count and the listed total can rise as CI progresses. A change to either count prints a new progress line; identical repeated counts stay quiet. The listed total is not a prediction of the final workflow total. This reporting does not change which checks count as passing or when Delivery may merge.

CI check observations get at most three total attempts: the original read and two retries, with interruptible delays of one second then two seconds. Transport failures, HTTP 429 or 5xx responses, and truncated JSON can recover this way; known permanent errors and unclassified failures stop immediately with their diagnostics. This applies to the initial watch, pending-check polling, Base comparison and Check re-run reconciliation. Each retry reads the same repository and full commit again. Failed reads never count as passing or absent CI, never start a Repair, and never repeat a workflow re-run request. Progress lines retain each failure and delay; exhaustion reports all three attempts and their causes. Cancellation stops active reads and backoff without another request.

The other commands:

```sh
thirdshift architect [<focus>]      # review the Base branch's architecture, publish a plan and run it (see Architect runs)
thirdshift architect [<focus>] --plan-only   # publish the plan, mark it ready and stop there
thirdshift architect base <branch> [<focus>]   # do either with <branch> as the Base branch, from a clone on any branch
thirdshift pickup                   # take the lowest-numbered Ready issue in the repository and run it (see Pickup runs)
thirdshift pickup base <branch>     # do that with <branch> as the Base branch, from a clone on any branch
thirdshift secure                   # audit the Base branch and record findings privately (see Security runs)
thirdshift secure base <branch>     # do that with <branch> as the Base branch, from a clone on any branch
thirdshift email-test [<address>]   # send a test email through Resend (see Email)
thirdshift setup                    # choose your defaults and write the User config with every setting (see User config)
thirdshift update                   # update to the latest release (see Updating)
thirdshift version                  # print thirdshift <version>
thirdshift help                     # print a compact command and option reference
```

`help` lists commands and shared options, with a link to the [project README](../../README.md), which points to these guides for workflow details, settings, Harness setup, email, logs and scheduling.

`version` and `help` print to stdout and exit `0`. `update`, `setup` and `email-test` follow the Run's rule: stdout stays empty, messages go to stderr. `architect` follows it too: stdout carries the pull request's URL of the run it dispatches, or, in its place, the URL of the issue it ended on: its plan with `--plan-only`, or its idea when the review published no plan. A skipped Architect run prints nothing there when [another is still running](passes.md#one-at-a-time), the URL of each Architect plan that is [still open](passes.md#one-architect-plan-at-a-time), the URL of each [Architect idea waiting for triage](passes.md#an-architect-idea-waits-for-triage), and the URL of the [Ready issue that goes first](passes.md#a-ready-issue-goes-first). `pickup` follows it as well: stdout carries the pull request's URL of the run it dispatches, and nothing when the [Pickup run](passes.md#pickup-runs) is skipped. So does `secure`: stdout carries the pull request's URL of a fix it dispatches, and nothing otherwise.

Uncommitted changes in your clone are fine: the Run works in its own worktree from `origin`, so they are simply left out. Unpushed commits on the Base branch are not: push them first, or the Run stops.
