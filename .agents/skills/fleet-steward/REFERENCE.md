# Fleet reference

Consult this during a tick. This user's hosts, routing and decisions are in `~/.thirdshift/fleet/fleet.md`; where the two differ, that file wins.

## Failure catalogue

Read the Command log tail first (`~/.thirdshift/logs/<owner>/<repo>/commands/<kind>/<stamp>.log` on the host that ran it). Its `Stopped at:`, `Preserved:` and `Recovery:` lines say where the run stopped and how to resume it.

| Symptom in the log | Class | Action |
|---|---|---|
| Rate limit, HTTP 5xx, `503 UNAVAILABLE`, timeout, host overload, a harness plan limit already routed elsewhere | transient | Retry: remove `agent-failed` **and** re-add `ready-for-agent` (the Claim removed it), then dismiss the row. |
| `This content was flagged for possible cybersecurity risk` (codex implementing a `security-finding` or `security-fix` Ticket) | refusal | Clear `agent-failed`, then rerun the issue on a host whose other harness is logged in: `thirdshift <issue URL> harness <other>`, with the repo's usual Pass words. Dismiss the original row. |
| `Security review refused: Codex's cybersecurity safeguard` | refusal | Point that host's `[security] harness` at another logged-in harness, with a backup. Then run the logged Recovery command. |
| `Security review incomplete: … omitted a valid final line` | check the trace | Read the Session trace's last `result`. If a valid `Security review: {…}` line came before a stray last line (seen with agy), run the Recovery command. If the session says why it stopped (missing dependencies, say), retry once, then hand it to the user. |
| `Security review left N unaddressed introduced finding(s)` | real | Read the finding in the PR body. It is usually a product decision, so give the user the options. Once they decide, post the decision on the issue and run the Recovery command. |
| `the Security fix Ticket includes private write-up text` | guard | thirdshift stopped advisory text from reaching a public Ticket. Nothing leaked; the next Security run retries. Note it, and treat a repeat on one harness as a pattern. |
| `the Security finding was triaged during its reproduction` | benign race | A Triage Pass labelled the finding mid-run. Dismiss the row. |
| `push … non-fast-forward`, `PR is a draft`, `merged, not open` | collision (#2095 shape) | A Fix Pass on one host and a Pickup Claim on another worked the same branch. Check that origin holds the work, clear the labels if nothing was lost, and note the occurrence. |
| `adopted head … has no recorded agent review` | held merge | Run `thirdshift review <PR>`, then `thirdshift fix <PR> merge`. |
| `CI-fix found nothing to change on main` repeatedly while main stays red | outside the repo | The fix needs a server, secret or setting. Find the failing step and hand the infrastructure change to the user. Propose pausing Fix crons only if they keep merging churn. |
| `Base branch … hasn't changed since the last completed Security audit` after a failed audit | stale skip | A failed reproduction still counts as a completed audit, so the findings wait until the base branch changes or the user triages them. Tell the user. |
| Skipped at the Failure limit | stuck repo | `pickup.failure_limit` open `agent-failed` issues halt both Pickup and Triage on that repo. Clear the transient ones; report the real ones. |

**Silent** is decided by log growth only (#2247). Before calling a run hung, check its process tree and CPU on the host: Scoring and long test commands read as Silent while they work.

A Console row marked slower than usual with a `0–0 min` usual time has no history behind it, so it isn't actually slow.

## Commands and gotchas

- **Background runs over ssh**: export the PATH the host's crontab uses (`$HOME/.local/bin:$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin`, plus any toolchain directory it adds). Then use `ssh -n -f host '… cd <launch dir> && nohup thirdshift … > /tmp/<name>.out 2>&1 < /dev/null &'`. Without `-f` and full redirection, the ssh session hangs.
- **Launch directory**: a run started from the Console or by hand may come from a checkout other than `~/repos/<repo>`. The Command log's `Run from the Launch directory …` line names it.
- **Dismissing**: `thirdshift dismiss <kind>-<stamp>` on the host that ran it, for example `pickup-20261115T171301-0500` or `secure-20261116T032701-0500`. An issue reference or the full command id can answer "row is not in Needs you".
- **Quoting**: quote `~` in remote commands (`'~/.local/bin/codex …'`) so it expands on the remote host. Use `ssh -n` inside loops and heredocs so ssh leaves stdin alone.
- **Edit TOML** with `sed` using a `|` delimiter, since `#` comments break a `#` delimiter. Print the edited lines, then parse the file (Python 3.11+ `tomllib`) to confirm it is still valid.
- **A harness is usable** on a host when a one-line headless prompt answers, such as `agy -p "Reply with the single word ok."`. Check that before routing work to it.
- **Triage readiness** is host-wide (`[triage] allow_ready_for_agent`) or per cron line (`thirdshift triage base <b> --allow-ready-for-agent`). Use the cron-line form when a host triages repos with different rules.
- **Console Plan usage "CLI not on PATH"** means the `thirdshift-console.service` PATH lacks the harness's directory. Add a systemd drop-in, `thirdshift-console.service.d/path.conf`, that sets the cron PATH.
- **Scoring**: a Grader must differ in Model from the Run it grades. To keep Scoring off a harness near its limit, set its `harness` and `fallback_harness` to the same other harness, with a different `fallback_model`.

## Quota

Each host's Passes wait at a ~500-call floor until the hourly reset, so a brief skip near the end of the hour is the gate working. A floor every hour has a cause. Measure it before changing anything:

- **Call-heavy Passes**: a Fix or Review Pass sweeping a repo with many open PRs can cost hundreds of calls per pass.
- **Polling overhead**: every cron firing costs calls even when it finds nothing to do. Count firings per hour from each host's crontab. If quota still floors in an hour when no runs started, the polling is the cause.

The fix is spacing out cron schedules, which is the user's call: propose slowing idle repos and keeping busy ones frequent.

## Disk

On a tight host, these are regenerable and safe to clear:

- `cargo clean` in thirdshift clones;
- `docker builder prune --filter until=168h`.

These are the user's to remove: `/tmp`, Docker volumes and images, web roots, and other services' data. Moving logs or harness homes to a bigger volume is also the user's call; record the result in `fleet.md`.
