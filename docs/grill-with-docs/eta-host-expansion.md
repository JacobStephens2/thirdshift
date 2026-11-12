# Grilling: Running thirdshift on more employer hosts

Issue: none (an operations decision; no thirdshift code change came out of it)
Research: a probe of each candidate host over the private network, 2026-11-12 (below)

Jacob wanted more of his employer's (ETA's) cloud VMs to run thirdshift, for more throughput and better use of idle compute. There were six candidates. This copy names them by role and leaves out host names, addresses, accounts and security details, since this repository is public.

| Role in this file | What the VM is for |
|---|---|
| **preview host** | Runs the app's per-branch preview environments in Docker |
| **monitoring host** | Observability for the whole fleet (metrics, logs, traces) |
| **staging host** | Staging copy of production |
| **replay host** | A small single-purpose web service |
| **DB replica host** | A replica of the production database |
| **management host** | Docker services for internal tooling |

Hosts that already ran thirdshift: three older ETA VMs (called *dev*, *orchestrate* and *test* here) and the new shared factory host from #1979. The two ETA repositories in play are called the **app repo** (the largest `ready-for-agent` queue) and the **infra repo** (infrastructure as code).

## Round 1 (answered)

**Q1.1 - What is the actual limit on throughput?** The hosts already scheduled were mostly idle: one-minute load was about 1.2 of 8 vCPU, 2 of 8 and 0.5 of 2. factory had just come up with 8 vCPU / 32 GB. The queues were small: the app repo had 16 `ready-for-agent` issues and 13 open PRs, the infra repo 7 and 5, and thirdshift 10 and 3. The options:
- (a) Empty the app and infra queues faster by running more Pickup Passes at once on more machines.
- (b) Use idle compute for other kinds of work (Experiments, more Architect, Security or Review).
- (c) Something else is the limit (shared Claude/Codex usage limits, or Day shift review time), so more machines won't help.

*Recommended:* (a), with an eye on (c). Every machine draws on the same subscriptions, and 13 open PRs suggest review could be next.

*Answer:* agree

**Q1.2 - Leave the DB replica host out?** Its database already uses most of the memory. An agent build that hit the OOM killer could stop replication.

*Answer:* agree (left out)

**Q1.3 - Leave the replay host out?** It has 2 vCPU and 3 GB, so one build would use all of it.

*Answer:* agree (left out)

**Q1.4 - Leave the monitoring host out?** If agent load made it miss data, the gaps would land while the other hosts are under load, which is when that data is needed most.

*Answer:* agree (left out)

**Q1.5 - Leave the staging host out?** It wasn't on the private network yet, so it couldn't be inspected.

*Answer:* Jacob added it to the private network. Reconsider it. (See Q2.1.)

**Q1.6 - Add the preview host?** 8 vCPU / 15 GB, load about 1, plenty of disk, and thirdshift and Claude Code already installed. It was the only candidate with real headroom.

*Answer:* agree

**Q1.7 - Hold the management host until it has more disk?** It had low load, but its root disk was 88% full, and worktrees and build output would fill it quickly. The options: (a) free or add disk first, (b) add it now for light Passes only, (c) leave it out.

*Answer:* Expand the disk by attaching a cloud block-storage volume. (See Q2.2.)

**Q1.8 - Measure factory first?** factory had just roughly doubled Pickup capacity on the app repo. The options: (a) add the preview host now and decide on the rest after a few days of run data, (b) add every accepted host now.

*Answer:* agree (a)

## Round 2 (answered)

**Q2.1 - Leave the staging host out after all?** Once it was reachable, it turned out to have 1 vCPU and 1 GB. One agent session alone would use all of that.

*Answer:* agree (left out)

**Q2.2 - How should the management host get more disk?** The options: (a) attach a 100 GB block-storage volume and keep repos and worktrees on it, (b) resize the VM's disk (needs a power-off and can't be undone), (c) prune Docker (about 42 GB reclaimable, but some images may belong to other people's services).

*Answer:* agree (a). *Done:* the volume is attached, mounted at boot, and holds `~/repos`. thirdshift puts worktrees next to the repos, so they land there too.

**Q2.3 - Which repos and Passes on the preview host?** The options: (a) Pickup only, for the app and infra repos, (b) the app repo only, (c) Pickup and Fix for both.

*Answer:* agree (a), and add thirdshift repo work there too. (See Q3.1 for the infra repo.)

**Q2.4 - Can I prepare the preview host?** It needed a `gh` login, clones, and an update from thirdshift 0.20.0 to 0.30.0.

*Answer:* agree

**Q2.5 - Prune the preview host's Docker build cache?** About 109 GB was reclaimable.

*Answer:* do it now. *Done:* `docker builder prune -f` reclaimed 109.4 GB. Images, containers and volumes weren't touched.

## Round 3 (answered)

**Q3.1 - Keep infra-repo Pickup on its current host only?** Infra-repo sessions rely on gitignored state and fleet access that exist only on *orchestrate*. A clean clone elsewhere would either fail or need secrets copied across.

*Answer:* agree (stays on *orchestrate*). The preview host runs app-repo and thirdshift Pickup.

**Q3.2 - App-repo toolchain on the preview host?** The app repo's tests need PHP 8.4 and Composer, and writable checkouts run only database-free checks. Neither the preview host nor factory had PHP, so factory's app-repo Pickups couldn't run tests.

*Answer:* agree: install PHP 8.4 and Composer on both, with no database configuration. *Done:* `vendor/bin/phpunit` is present on both.

**Q3.3 - thirdshift-repo toolchain on the preview host?** That repo needs Rust and Python 3.11 or later, and the system Python was 3.9.

*Answer:* agree: rustup and uv's Python 3.12 in the home directory, with Python 3.12 on the PATH of the thirdshift cron line only.

**Q3.4 - What cadences?** The other hosts' Pickup slots were at minutes 1 (every 5), 3 and 7 (every 10).

*Answer:* agree: app-repo Pickup at `8-59/10` and thirdshift Pickup at `9-59/10`, both without `merge`.

**Q3.5 - Leave the management host waiting on the Q1.8 measurement?**

*Answer:* agree

## Round 4 (answered)

**Q4.1 - Which harness on the preview host?** The other ETA hosts default to `codex`.

*Answer:* agree: `claude`, so the preview host draws on a different subscription instead of splitting the same Codex quota.

**Q4.2 - Base the preview host's config on factory's?** The preview containers already use about 5 of its 15 GB.

*Answer:* agree: copy factory's User config and Credentials, but raise `min_available_memory_mb` from 3000 to 5000 so the previews keep headroom. (A side check confirmed `thirdshift email-test` delivers through Resend from factory and from the preview host.)

**Q4.3 - When and how to judge the measurement?**

*Answer:* agree: on 2026-11-16, compare Runs started per host, Pickups that found nothing ready, usage-limit failures and the `ready-for-agent` counts. If the preview host's Pickups mostly find nothing ready, stop there. If the queues stay above about 5 and Runs succeed, set up the management host.

## Settled by earlier answers

- The DB replica, replay, monitoring and staging hosts stay out.
- The preview host runs Pickup only (app repo and thirdshift) on the `claude` harness. Fix, Review, Triage, Architect and Security stay where they are.
- Infra-repo work stays on the host that holds its state.
- The management host has disk ready but waits for the 2026-11-16 measurement.

## Findings worth keeping

- **Pairing a Console behind someone else's reverse proxy.** The preview host's ports 80 and 443 belong to the preview environments' proxy, so `console install` (Caddy or Apache) didn't fit. The Console runs on localhost behind an HTTPS proxy on another port, reachable only on the private network, and is paired into the Day shift's Fleet as a live entry.
- **SELinux and `thirdshift update`.** On an SELinux-enforcing host, an updated binary keeps the temp-file label it was downloaded with (`user_tmp_t`), so a systemd unit whose `ExecStart` runs it directly fails with `203/EXEC` "Permission denied". Starting it via `/bin/sh -c "exec …"` works. Filed as #2131.
- **A fresh `gh` login can stop working within minutes** (cause unknown; a second login held). Check `gh auth status` from a cron-like environment before relying on a new host.
- **A new host needs a git identity** (`user.name`, `user.email`), or every Pickup stops before its first commit.
