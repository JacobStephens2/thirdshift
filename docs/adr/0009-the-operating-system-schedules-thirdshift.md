# thirdshift never schedules itself: the operating system's scheduler starts each pass

An **Architect run** and a **Pickup run** find their own work, so they are meant to run with nobody typing the command. thirdshift still has no daemon, no `watch` command and no `--every`: each is one pass, started by the operating system's scheduler from a crontab line per repository, and each is the same command a person would type. What makes a pass safe to start from a timer lives in the command, not in a scheduled mode: the Base branch named on the command line, one unattended run per repository at a time on a machine, and a skipped outcome that exits `0` when there is nothing to do.

## Considered Options

- **A long-running `thirdshift watch`** that polls GitHub and starts Runs itself. Rejected:
  - It has to be kept alive: a service unit, restarts, lingering for a logged-out user. cron already survives reboots.
  - It keeps running the binary it was started with after `thirdshift update`, where the next pass simply runs the new one.
  - A daemon that hangs is silent. Each pass has an exit code and a line in the cron log.
  - It buys no capacity. Running more than one issue at once is a matter of how many passes the lock lets through, and a pass is the primitive a daemon would call anyway.
- **A GitHub Action or webhook fired by the label.** Rejected: thirdshift runs on the user's own machine, with their `claude` and `gh` logins, and a webhook needs a server GitHub can reach.

## Consequences

- A **Ready issue** waits up to the scheduler's interval before it is taken, and throughput is set by how often the crontab line fires.
- An issue is taken at most once across passes by state on GitHub, the **Claim**, since no process outlives a pass to remember it. The lock only covers one machine.
- Several repositories means several crontab lines, at hours the user staggers. thirdshift keeps no list of repositories.
- A frequent pass must be quiet when it does nothing: a skipped Pickup run or Architect run sends no **Run notification**, so either can be started every few minutes. A skip left only its line in the scheduler's log; since ADR 0011 it leaves one in the repository's **Activity log** instead, and only when its reason changes.
