# thirdshift keeps a per-repository Activity log, and skipped passes leave nothing for the scheduler

With one `logs.dir` serving several repositories, the scheduler's log was the only place a pass showed up per repository, and getting it there took a crontab redirect into a folder per repository. The shell opens a redirect before `thirdshift` starts, so a repository whose folder did not exist yet never ran: its crontab line failed silently, every minute (cascade and keeplore, #339). The log it produced, once it worked, was two lines per pass, nearly all of them skips.

Now thirdshift keeps the record itself. Every log sits under the repository it belongs to, `<logs.dir>/<owner>/<repo>/`, in folders thirdshift creates as it needs them, and each repository has an **Activity log**: a line when a command starts work and one when it ends, and a line for a skipped Architect run or Pickup run only when its reason differs from that kind's last line. With `activity.quiet_skips` set in the User config, a skipped pass prints nothing at all, so a crontab line redirects to one fixed file, which then catches only what failed before thirdshift knew the repository. Adding a repository is adding one crontab line.

This replaces ADR 0009's consequence that a skip leaves only its line in the scheduler's log.

## Considered Options

- **Keep the per-repository redirect, and `mkdir -p` in each crontab line.** Rejected: every crontab line carries setup that thirdshift should own, forgetting it fails silently, and thirdshift can't trim noise in a file it does not write.
- **Write nothing for a skipped pass.** Rejected: it loses the record of when, and why, a repository went idle.
- **One line for every skip.** Rejected: about 1,440 lines a day per kind per repository, which buries the passes that did work.
- **Leave skips off the terminal whenever stderr isn't one.** Rejected: guessing from the terminal surprises anyone piping the output. A User config key keeps the crontab line the same command a person would type.

## Consequences

- The repository in the path is the GitHub repository, `<owner>/<repo>`, not the name of the local checkout, so log file names drop it: `commands/pickup/41-<stamp>.log`, `sessions/41-<stamp>-implement.jsonl`.
- Whether the scheduler is still firing is no longer a line per pass: an idle repository's Activity log ends on one skip line.
- Logs written before this, under `<logs.dir>/sessions/` and `<logs.dir>/commands/`, are not moved by thirdshift.
