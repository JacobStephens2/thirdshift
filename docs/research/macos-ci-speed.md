# Keeping macOS CI coverage without the queue: runner limits, workflow shapes, alternatives

Research date: 2026-10-20, for #893/#894

**Question.** macOS CI on thirdshift often waits 15-45 minutes for a runner, and the owner is tempted to drop macOS support or the macOS runners. What faster ways are there to keep macOS coverage, and what does each option cost, dropping macOS included?

**Method.**

- Measured queue and execution times for every macOS job in the last ~48 hours with the jobs API (`gh api repos/JacobStephens2/thirdshift/actions/runs/{id}/jobs`). That covers 160 CI runs and 112 Release runs created from 2026-10-18 17:00 UTC to 2026-10-20 17:00 UTC: 753 macOS jobs, 686 of them succeeded or failed. "Queue" means job `created_at` to `started_at`. "Execution" means `started_at` to `completed_at`, setup and build included.
- Read `.github/workflows/ci.yml`, `.github/workflows/release.yml`, [docs/ci-performance.md](../ci-performance.md), `tests/support/fakes.rs` and `Cargo.toml`/`Cargo.lock`.
- Ran `cargo check --all-targets --target aarch64-apple-darwin` once on the x86_64 Linux factory host.
- Read GitHub's runner, limits, billing, merge queue and security docs; the `actions/runner-images` build scripts at commit `68bd77a`; the nextest docs; and the vendors' own pricing pages.
- Read the current workflow files of ripgrep, cargo, uv, jj, nextest and starship at their default-branch SHAs.
- Claims are cited inline. Anything no primary page confirmed is marked **[unverified]**.

**Out of scope.** Changing the workflows or GitHub settings. Attributing the remaining per-test macOS time (#894). The ownership-recheck security decision (#858). Signing and notarization of release binaries.

## TL;DR

- **The queue is a capacity problem, not a GitHub hiccup.** Personal accounts get at most **5 concurrent macOS jobs** on standard runners, on Free and on Pro ([GitHub: Actions limits](https://docs.github.com/en/actions/reference/limits)). Each CI run needs 4 of them for ~18 min, and a Release run takes another. Over the 47.5 hours measured, macOS jobs used **180.8 runner-hours**, an average of **3.8 of 5 slots busy (76%)**. When demand stayed under ~3 slots, the median queue was **9 s**. In the 4-hour windows where it reached 4.2-5.1, the median was **10-20 min** and the worst was **85 min** (5,108 s). Median macOS completion was **33 min** from job creation (p90 55 min), against **11 min** for the Linux `test` job.
- **Paying GitHub for bigger macOS runners isn't open to this repo.** Larger runners are "only available for organizations and enterprises using the GitHub Team or GitHub Enterprise Cloud plans" ([GitHub: runner pricing](https://docs.github.com/en/billing/reference/actions-runner-pricing)), and even there the 5-job macOS cap is shared with standard runners ([limits](https://docs.github.com/en/actions/reference/limits)). Merge queues are also org-only ([GitHub: merging with a merge queue](https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/merging-a-pull-request-with-a-merge-queue)).
- **The cheap lever is running fewer full macOS suites.** PRs into Spec branches used **29%** of macOS CI time (51.8 of 176.3 runner-hours). Pushes to main used **23%** (41.0 h). #893 alone would bring average demand to about 2.7 slots, the zone where the queue was seconds. That matches how uv, the closest peer, runs macOS tests: only on main, or on PRs that opt in with a label (uv `test.yml`, below).
- **Keep a cheap macOS gate on every PR.** A cross-compile check from Linux doesn't work as is: `cargo check --target aarch64-apple-darwin` failed here in 14 s because `aws-lc-sys` (pulled in by reqwest's rustls) compiles C with `-arch arm64` and the host `cc` rejects it. One macOS job running `cargo clippy --all-targets` plus a nextest filter over the macOS-specific code (`src/logs/process_identity.rs`, `src/skills/directory.rs`, `src/child_run.rs`) takes one slot for a few minutes, where the full suite takes four slots for ~18 min each.
- **Third-party macOS runners would fix the cap, but only Cirrus is cheap at this volume.** At the measured rate (~164,000 macOS minutes a month), per-minute vendors cost thousands a month: Namespace $0.06/min, WarpBuild, Depot and Blacksmith $0.08/min. Cirrus Runners charges a flat **$150/month per M4 Pro runner** with unlimited minutes ([Cirrus pricing](https://cirrus-runners.app/pricing/)). Self-hosting a Mac mini is cheapest, but GitHub says self-hosted runners "should almost never be used for public repositories" ([GitHub: secure use](https://docs.github.com/en/actions/reference/security/secure-use)).
- **XProtect is a plausible but unproven part of the per-test slowness.** nextest's docs say "XProtect can cause nextest runs to be significantly slower", and that adding the terminal to Developer Tools excludes its children ([nextest: macOS](https://nexte.st/docs/installation/macos/)). GitHub's image runs `DevToolsSecurity --enable` but grants no `kTCCServiceDeveloperTool` entry. None of the six Rust projects surveyed applies such a tweak in CI. It is worth one probe job under #894, not a plan.
- **Recommendation:** don't drop macOS.
  1. Do #893.
  2. Replace the per-PR full suite with a one-job macOS gate on PRs into main; run the full 4-shard suite on pushes to main and nightly.
  3. Keep pulling per-test time down under #894.
  4. If that still queues, pay for Cirrus Runners, 2-5 runners at $150 each.

Dropping macOS support is mostly a README and release-target change. It costs the Apple Silicon users and the `aarch64-apple-darwin` artifact, and leaves nothing exercising the `#[cfg(target_os = "macos")]` process-identity and skills-directory code.

## 1. GitHub-hosted runner limits, sizes and prices

**Concurrency.** The limits page gives standard GitHub-hosted runners these caps ([GitHub: Actions limits](https://docs.github.com/en/actions/reference/limits)):

| Plan | Total concurrent jobs | Concurrent macOS jobs |
| --- | ---: | ---: |
| Free | 20 | 5 |
| Pro | 40 | 5 |
| Team | 60 | 5 |
| Enterprise | 500 | 50 |

- For larger runners: "The maximum concurrent macOS jobs is shared across standard GitHub-hosted runners and GitHub-hosted larger runners" (same page).
- A queued job is cancelled after 24 hours (same page).
- `gh api repos/JacobStephens2/thirdshift` reports `owner.type: User` and `private: false`, so the cap is 5 on either personal plan.

**Measured usage confirms the cap binds.** I summed the time the running macOS jobs spent at each concurrency level, counting only jobs that succeeded or failed. The level was exactly 5 for 80,566 s (22.4 h) and 4 for 41,502 s (11.5 h). It went above 5 for only ~2,000 s, which is timestamp overlap at job hand-off.

**Standard macOS runner specs** are the same for public and private repositories ([GitHub: GitHub-hosted runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)):

| Labels | Arch | CPU | RAM | SSD |
| --- | --- | ---: | ---: | ---: |
| `macos-latest`, `macos-14`, `macos-15`, `macos-26`, `xcode-27` | arm64 (M1) | 3 | 7 GB | 14 GB |
| `macos-15-intel`, `macos-26-intel` | Intel | 4 | 14 GB | 14 GB |

- Public repositories use these runners "free and unlimited" (same page).
- `macos-13` no longer appears in either table.
- The 3-vCPU M1 is the machine the 4 shards run on. nextest's default `--test-threads` is `num-cpus`, "the amount of available parallelism" ([nextest: running](https://nexte.st/docs/running/)), so each shard runs about 3 tests at once.

**Larger macOS runners** ([GitHub: larger runners](https://docs.github.com/en/actions/reference/runners/larger-runners)):

| Size | Spec | Labels |
| --- | --- | --- |
| `-large` | Intel, 12 cores, 30 GB | `macos-15-large`, `macos-26-large` |
| `-xlarge` | M2, 5 cores + 8 GPU cores, 14 GB | `macos-15-xlarge`, `macos-26-xlarge` |

- They need a card on file and a spending limit above zero (same page).
- They are "always billed at the per-minute rate", public repositories included ([GitHub: larger runners concepts](https://docs.github.com/en/actions/concepts/runners/larger-runners)).
- The prices are $0.102/min for xlarge and $0.077/min for large. The pricing page also says larger runners "are only available for organizations and enterprises using the GitHub Team or GitHub Enterprise Cloud plans" ([GitHub: runner pricing](https://docs.github.com/en/billing/reference/actions-runner-pricing)).
- So a personal account can't use them. Moving thirdshift to an org on Team still leaves the macOS cap at 5.

## 2. Image differences and known queueing

- `actions/runner-images` lists `macos-26` arm64 as the image behind `macos-latest`. `macos-14` carries a "deprecated" badge, and `xcode-27` is "preview" ([runner-images README](https://github.com/actions/runner-images)).
- The image support policy is "2 GA images and 1 beta image at a time" (same page).
- I found no official statement that `macos-26` is faster, slower or scarcer than `macos-15`. Both run on the same 3-vCPU M1 spec. Whether the two pools have separate capacity is **[unverified]**.
- Neither the README nor the runner docs document macOS queueing. The measured queue follows thirdshift's own demand (section 4), so GitHub's capacity isn't needed to explain it.
- Intel images have more cores (4) and RAM (14 GB). Whether they run this suite faster is **[unverified]**; it would take one shard run on `macos-26-intel` to find out. GitHub's Intel runway is short: jj already runs its x86_64 macOS job on `macos-26` arm64 with `--target x86_64-apple-darwin` ([jj ci.yml L32-38](https://github.com/jj-vcs/jj/blob/5176b1d8a3b0e569bb79b5e681c37874543ec617/.github/workflows/ci.yml)).

## 3. Third-party and self-hosted macOS runners

All prices come from each vendor's own page, checked 2026-10-20.

| Vendor | Hardware | Price | Concurrency | Personal account | Label |
| --- | --- | --- | --- | --- | --- |
| [Namespace](https://namespace.so/pricing) | M5 Max / M4 Pro; 6 vCPU/14 GB, 12/28, 12/56 | 6x14: $0.06/min prepaid, $0.09 overage | Developer plan 12 vCPU total, Team 24, Business 48 | Yes, per [its docs](https://namespace.so/docs/solutions/github-actions) | `nscloud-macos-tahoe-arm64-6x14` ([runner config](https://namespace.so/docs/reference/github-actions/runner-configuration)) |
| [WarpBuild](https://www.warpbuild.com/pricing) | M4 Pro; 6 vCPU/14 GB, 12/28 | $0.08 / $0.16 per min | "contact support" for high macOS concurrency ([docs](https://www.warpbuild.com/docs/ci/cloud-runners)) | **[unverified]** | `warp-macos-15-arm64-6x` |
| [Cirrus Runners](https://cirrus-runners.app/pricing/) | M4 Pro, 4 vCPU, 16 GB | **$150/month per runner, unlimited minutes** | 1 job per runner bought | Setup docs describe orgs only ([setup](https://cirrus-runners.app/setup/)) **[unverified]** | `ghcr.io/cirruslabs/macos-runner:tahoe` |
| [Depot](https://depot.dev/docs/github-actions/runner-types) | M4 (macOS 26) or M2, 8 CPU, 24 GB | $0.08/min | "Unlimited", but macOS capacity "not fully elastic" | Docs describe orgs ([quickstart](https://depot.dev/docs/github-actions/quickstart)) **[unverified]** | `depot-macos-latest` |
| [Blacksmith](https://www.blacksmith.sh/pricing) | M4; 6 vCPU/24 GB, 12/48 | $0.08/min; 3,000 free x64 min/month, ~150 macOS min | "does not impose any concurrency limits" ([docs](https://docs.blacksmith.sh/blacksmith-runners/overview)) | **[unverified]** | `blacksmith-6vcpu-macos-latest` **[unverified]** |
| [MacStadium](https://www.macstadium.com/pricing) | Dedicated M4 10-core/16 GB; M4 Pro 12-core/48 GB | $149 / $349 per month; no trials | 1 job per host | Self-hosted | Your own |
| Scaleway Mac mini | M4 / M4 Pro | ~€0.22/h or €149/month **[unverified]**: the pricing pages returned 403 | 1 job per host | Self-hosted | Your own |
| [AWS EC2 Mac](https://aws.amazon.com/ec2/instance-types/mac/) | mac2-m2 8-core/24 GB; mac2-m2pro 12/32; mac-m4 10/24 | mac2-m2 $0.878/h, mac2-m2pro $1.56/h, us-east-1 ([AWS pricing feed](https://b0.p.awsstatic.com/pricing/2.0/meteredUnitMaps/ec2/USD/current/dedicatedhost-ondemand.json)) | 1 job per host | Self-hosted | Your own |

- **AWS bills a minimum of 24 hours.** "Billing for EC2 Mac instances is per second with a 24-hour minimum allocation period" ([AWS EC2 Mac](https://aws.amazon.com/ec2/instance-types/mac/); also [Dedicated Hosts pricing](https://aws.amazon.com/ec2/dedicated-hosts/pricing/)). One always-on mac2-m2 host is about $632/month, which is no cheaper than a Cirrus runner and adds the administration.
- **Self-hosted runners on a public repo are the risk GitHub names.** "self-hosted runners should almost never be used for public repositories on GitHub", because anyone can open a pull request that compromises the environment ([GitHub: secure use](https://docs.github.com/en/actions/reference/security/secure-use)). That covers a Mac mini at home, MacStadium, Scaleway and AWS. thirdshift's own PRs all come from same-repo branches, so a job guard like `if: github.event.pull_request.head.repo.full_name == github.repository` would keep fork PRs off the machine. That is a mitigation, not an exemption; the guidance stays. Managed vendors run each job in a fresh VM, which addresses persistence but still exposes any secrets the job gets.
- **Cost at today's volume.** 180.8 runner-hours in 47.5 hours is ~2,740 h, about **164,000 min a month**, if sustained.
  - At Namespace's $0.06/min that is ~$9,900/month before any speed-up.
  - At $0.08/min it is ~$13,100.
  - Faster M4/M5 hardware would cut the minutes, but by how much on this spawn-bound suite is **[unverified]**.
  - After #893 and moving the full suite off PRs (section 4), the volume falls to roughly a quarter, about $2,500-3,300/month at those rates.
  - Five Cirrus runners keep today's concurrency for a flat $750/month, and two keep a main-only plan moving for $300.

## 4. Workflow options

**Where the macOS time goes, by trigger** (CI only, 47.5 h window):

| Trigger | CI runs | macOS runner-hours | Share |
| --- | ---: | ---: | ---: |
| PR into `main` | 80 | 83.5 | 47% |
| Push to `main` | 37 | 41.0 | 23% |
| PR into a Spec branch (`issue-776`, `issue-711`, `issue-794`) | 43 | 51.8 | 29% |

Release added 106 `aarch64-apple-darwin` builds: median 116 s execution, 4.1 runner-hours in all. Each still takes one of the 5 slots.

**Queue delay per macOS job**, in 4-hour buckets by job creation (UTC). "Busy" is runner-hours executed per elapsed hour, out of a cap of 5. Values slightly above 5 come from bucketing by creation time.

| Window (UTC) | Jobs | Of which Release | Median queue (s) | p90 (s) | Max (s) | Busy slots |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 10-18 16:00-20:00 | 35 | 7 | 9 | 337 | 864 | 1.9 |
| 10-18 20:00-00:00 | 34 | 6 | 8 | 10 | 18 | 2.0 |
| 10-19 00:00-04:00 | 47 | 7 | 9 | 159 | 947 | 2.9 |
| 10-19 04:00-08:00 | 44 | 8 | 8 | 10 | 14 | 2.7 |
| 10-19 08:00-12:00 | 43 | 7 | 9 | 741 | 1,167 | 2.9 |
| 10-19 12:00-16:00 | 40 | 4 | 10 | 871 | 1,165 | 3.0 |
| 10-19 16:00-20:00 | 63 | 10 | 825 | 1,492 | 2,485 | 4.4 |
| 10-19 20:00-00:00 | 62 | 10 | 298 | 1,178 | 2,204 | 4.2 |
| 10-20 00:00-04:00 | 86 | 14 | 971 | 2,074 | 2,836 | 4.9 |
| 10-20 04:00-08:00 | 78 | 14 | 608 | 1,945 | 2,675 | 4.6 |
| 10-20 08:00-12:00 | 67 | 11 | 1,188 | 2,546 | 3,360 | 4.3 |
| 10-20 12:00-16:00 | 78 | 7 | 1,152 | 2,923 | 5,108 | 5.1 |
| 10-20 16:00-17:00 (partial) | 9 | 1 | 99 | 1,326 | 2,065 | 0.6 |

Across all 142 complete CI runs:

- The slowest shard of each run waited a median of 844 s (p90 2,204 s).
- 57% of runs waited over 10 minutes for their last shard, and 22% waited over 30 minutes.
- The worst cases match the session's observations: run 39446079560 (`issue-878`) waited 5,108 s, and run 39444870246 (push to main) waited 4,543 s.
- Per-shard execution held steady at a median of 1,067 s (p90 1,272 s), so the runners aren't slowing down; the queue is what moved.

The options, from cheapest:

- **#893: macOS only for PRs into main and pushes to main.** This removes 29% of demand. The 4-hour windows above say ~2.7 busy slots is where the median queue sits at seconds. Spec Ticket PRs still get macOS coverage when the Spec branch's own PR into main runs CI. The cost is that a macOS-only break inside a Spec is found at the Spec PR, after several Tickets have stacked on it.
- **Full suite on pushes to main and nightly; a small gate on PRs.**
  - Cutting the full suite from PR runs takes demand down to about 41 h of 176 (23%), roughly 0.9 busy slots.
  - The cost is that a macOS-only break lands on main and is fixed forward. Linux still gates every PR, and the macOS-specific code is small: `src/logs/process_identity.rs`, `src/skills/directory.rs`, and the `cfg!(target_os = "linux")` branches in `src/child_run.rs`.
  - The gate keeps compile errors and the macOS-specific tests on the PR. It is one `macos-26` job running `cargo clippy --all-targets -- -D warnings` and `cargo nextest run -E '<filterset naming those modules>'`.
  - The factory's `src/github.rs` accepts success, neutral or skipped checks (docs/ci-performance.md), so a skipped or filtered `test-macos` doesn't strand a Self-merge.
  - This is uv's pattern. Its `test.yml` says "Only run macOS tests on main without opt-in" (L161-163), and `plan.yml` L253 turns them on for a `test:macos` label ([uv test.yml](https://github.com/astral-sh/uv/blob/2b50cbc05bc99393dd5fe26aa2166fdfe3f2d8e4/.github/workflows/test.yml), [uv plan.yml](https://github.com/astral-sh/uv/blob/2b50cbc05bc99393dd5fe26aa2166fdfe3f2d8e4/.github/workflows/plan.yml)). A `test:macos` label opt-in would let a PR that touches the macOS code paths ask for the full suite.
- **Merge queue (`merge_group`).** Not available: merge queues work in "any public repository owned by an organization" or private ones on Enterprise Cloud ([GitHub: merge queue](https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/merging-a-pull-request-with-a-merge-queue)), and JacobStephens2 is a User account. cargo and jj both run macOS on `merge_group` (below), but that needs an org.
- **Path filters.** The CI workflow runs on every PR, docs-only ones included. A `paths-ignore` on `docs/**`, `site/**` and `*.md` for the macOS jobs only, like starship's ([starship workflow.yml L3-10](https://github.com/starship/starship/blob/ae8fecf316ede2258e90fb2a6001ca2a7076961c/.github/workflows/workflow.yml)), would skip the docs and site PRs in the table above (`architecture-page`, `video`, `explainer-*`). A workflow-level filter would also skip Linux. To keep the stable `test-macos` check, the filter belongs in a job-level `if:` on a changed-files output, so the aggregate still reports.
- **Shard count.** Under saturation more shards don't help. The cap limits runner-seconds, and every shard repeats checkout, toolchain, cache restore and nextest install, ~60-90 s of the 1,067 s median (~33-41 s of that is build). Sum of execution per run was ~4,300 s at 4 shards.
  - 2 shards free a slot so two runs can overlap, but each finishes in ~36 min instead of ~18.
  - 5+ shards make one run hold every slot.
  - Shard count only moves the trade between one run's latency and other runs' queue.
- **Partitioning.** nextest's `slice:` assigns tests round-robin and is the even option; `hash:` is stable but "smaller test runs may result in an uneven distribution" ([nextest: partitioning](https://nexte.st/docs/ci-features/partitioning/)). Neither balances by duration. The shards' execution spread (842-984 s in run 38990121846) is small, so this is a minor gain.
- **`--test-threads`.** On a 3-vCPU runner the default is 3 tests at once ([nextest: running](https://nexte.st/docs/running/)). The suite is spawn-bound (12.3 s system time in a 19.6 s test on Linux, docs/ci-performance.md), so oversubscribing to 4-6 may or may not help; that is **[unverified]** and costs one PR to measure.
- **Cross-compile check from Linux.** Not usable as is. On 2026-10-20, `cargo check --all-targets --target aarch64-apple-darwin` failed after 14 s in `aws-lc-sys` 0.45.0. Its build script runs the host `cc` with `-arch arm64 -mmacosx-version-min=11.0`, which GCC rejects. Making it work needs clang plus a macOS SDK for the C build. aws-lc arrives through reqwest's `rustls` and axoupdater's `default-tls` (comment in `Cargo.toml`), so the dependency can't simply be dropped.

## 5. Why process spawning is slow on macOS CI

- **The suite's cost is spawning processes, not compiling.** Each shard builds in ~33-41 s, then tests for ~16-20 min. One `spec_run` test ran 5,415 programs on Linux, about 82% of them git (docs/ci-performance.md). The same tests run about five times slower on macOS (same doc).
- **The fakes are built once per process tree, so a rebuilt binary is not re-scanned per test.** `tests/support/fakes.rs` builds one `fakes-<hash>` executable under `CARGO_TARGET_TMPDIR` and symlinks `gh`, `claude`, `codex` and the rest to it. git comes from Homebrew on the image (`brew_smart_install "git"` in [install-git.sh](https://github.com/actions/runner-images/blob/68bd77adbdf0017e6e3d622348abdb2763ca26f4/images/macos/scripts/build/install-git.sh)), not the `/usr/bin/git` xcrun shim. Whether macOS re-runs any assessment on each exec of an already-seen binary is **[unverified]**.
- **XProtect and Gatekeeper.**
  - nextest: "XProtect can cause nextest runs to be significantly slower", and "Adding your terminal to Developer Tools will cause any processes run by it to be excluded from XProtect". The setup is `sudo spctl developer-mode enable-terminal`, then a GUI toggle in Privacy & Security > Developer Tools ([nextest: macOS](https://nexte.st/docs/installation/macos/)).
  - nextest's older page puts the sign of the problem at "even the simplest tests taking more than 0.2 seconds". It also notes reports of slowness that persist after the exemption ([nextest: antivirus and Gatekeeper](https://nexte.st/book/antivirus-gatekeeper)).
  - Howard Oakley traced syspolicyd's Gatekeeper assessment of a small notarized tool: about 0.19 s on Ventura and 0.29 s on Sequoia, with the XProtect scan alone growing from ~0.03 s to ~0.13 s as the rules file grew ([Eclectic Light, 2025-05-04](https://eclecticlight.co/2025/05/04/last-week-on-my-mac-checking-code-can-take-longer-now/)). That is a secondary source tracing Apple's logs; Apple doesn't document these timings.
- **What the GitHub image already does.**
  - `configure-machine.sh` runs `sudo /usr/sbin/DevToolsSecurity --enable` ([runner-images configure-machine.sh L10](https://github.com/actions/runner-images/blob/68bd77adbdf0017e6e3d622348abdb2763ca26f4/images/macos/scripts/build/configure-machine.sh)).
  - `configure-tccdb-macos.sh` writes TCC grants for `/bin/bash`, the runner provisioner and the hosted-compute agent: Accessibility, AppleEvents, ScreenCapture, SystemPolicyAllFiles and others. None of them is `kTCCServiceDeveloperTool`, the Developer Tools grant ([configure-tccdb-macos.sh](https://github.com/actions/runner-images/blob/68bd77adbdf0017e6e3d622348abdb2763ca26f4/images/macos/scripts/build/configure-tccdb-macos.sh)).
  - The image writes the system TCC database at build time, so a workflow step might add a Developer Tools row for the runner's shell. Whether SIP allows that at job time is **[unverified]**; a `csrutil status` step would show it.
- **Nobody surveyed applies it.** No `spctl`, `DevToolsSecurity`, `mdutil` or XProtect setting appears in the CI of ripgrep, cargo, uv, jj, nextest or starship (section 6). Code search found Rust projects using `DevToolsSecurity -enable` or `spctl developer-mode enable-terminal` only for lldb or profiling, for example [ocaml/ocaml build.yml L280-281](https://github.com/ocaml/ocaml/blob/77e1335d8729e68d792a69f5cf0d8a1e495fe750/.github/workflows/build.yml) and [IlyaGulya/bend2-lsp-rs perf-diagnostics.yml L220](https://github.com/IlyaGulya/bend2-lsp-rs/blob/8ba06b117e8453959baa2157f857fa2e055fc887/.github/workflows/perf-diagnostics.yml). Satellite-im/Uplink runs `sudo spctl --master-disable` so its UI tests can open an unsigned app ([ui-test-automation.yml L246](https://github.com/Satellite-im/Uplink/blob/c3228ec463549684b725566b0eea2665d0cd7bc9/.github/workflows/ui-test-automation.yml)). A Rust forum user reported `spctl --master-disable` did not fix first-run slowness ([users.rust-lang.org](https://users.rust-lang.org/t/cargo-run-slow-on-macos-when-binary-already-built/117450)).
- **The useful #894 experiment.** In one throwaway workflow run on `macos-26`:
  1. Time 1,000 `git rev-parse` calls and 1,000 runs of the fakes executable.
  2. Run `csrutil status`.
  3. Try inserting the Developer Tools grant, then time the two loops again.
  4. Compare with `macos-26-intel`.

  That separates per-exec security cost from plain fork/exec cost. Cutting git processes per operation, as #846/#857 did, is the mitigation that works either way.

## 6. What other Rust CLI projects do

Read at each project's default-branch SHA on 2026-10-20.

| Project | macOS labels | macOS on every PR? | Merge queue | Sharding on macOS | Notes |
| --- | --- | --- | --- | --- | --- |
| [ripgrep `ci.yml`](https://github.com/BurntSushi/ripgrep/blob/3fce3b5bb0236da2df6d99672afb8a719642eca7/.github/workflows/ci.yml) | `macos-latest` (L106-108) | Yes, plus push and nightly cron | No | No | Plain `cargo test` |
| [cargo `main.yml`](https://github.com/rust-lang/cargo/blob/15be64aa0859b5c36f7a86d52f7aa0db44ba4d84/.github/workflows/main.yml) | `macos-15` (L74, L162-168) | Yes, and every `merge_group` (L2-6) | Yes | No | `cargo test -p cargo` |
| [uv `test.yml`](https://github.com/astral-sh/uv/blob/2b50cbc05bc99393dd5fe26aa2166fdfe3f2d8e4/.github/workflows/test.yml) | `namespace-profile-macos-15` (Namespace, L164) | **No:** main, release builds, or `test:macos` label | No | No (Windows uses `hash:N/3`) | nextest `ci-macos` profile, 20 min timeout |
| [jj `ci.yml`](https://github.com/jj-vcs/jj/blob/5176b1d8a3b0e569bb79b5e681c37874543ec617/.github/workflows/ci.yml) | `macos-15`; `macos-26` cross-targeting x86_64 (L32-38) | Yes, and `merge_group` | Yes | No | nextest, 30 min timeout raised for slow x86 macOS (L67-68) |
| [nextest `ci.yml`](https://github.com/nextest-rs/nextest/blob/6c2ab7e669761d9ef063bf00b2440c0fb5cca1ac/.github/workflows/ci.yml) | `macos-latest` (L102-108, L169-180) | Yes | No (bors-style `auto` branch) | No | rust-cache `save-if: false` on test jobs |
| [starship `workflow.yml`](https://github.com/starship/starship/blob/ae8fecf316ede2258e90fb2a6001ca2a7076961c/.github/workflows/workflow.yml) | `macOS-latest` (L40, L144) | Yes, except docs-only (L3-10) | No | No | `cargo llvm-cov` |

- None uses GitHub's `-xlarge` or `-intel` labels.
- Only uv pays for third-party Macs, and it still gates macOS behind main or a label.
- None shards macOS. Their suites fit one standard runner; thirdshift's needs four.
- The pattern among projects that run macOS on every PR: it either runs in a merge queue that batches merges (cargo, jj, both orgs), or the suite is small (ripgrep, starship).

## Options compared

"Wall time" is from job creation to the last macOS shard done. Today the median is 33 min and p90 55 min, against 11 min for Linux. Runner-hours are per 47.5 h at the current PR rate.

| Option | Expected wall-time effect | Cost | macOS coverage lost | Effort |
| --- | --- | --- | --- | --- |
| Do nothing | Median 33 min, p90 55 min, worst 85+ min; worse as PR volume grows | $0 | None | None |
| #893: skip macOS on PRs into Spec branches | Demand 3.8 → ~2.7 slots; queue mostly seconds outside bursts (bursts still queue) | $0 | Spec Tickets: macOS signal arrives at the Spec PR into main | Small: one `if:` on base ref; the `test-macos` aggregate must treat skipped as OK on those PRs |
| Path/label gating (docs/site PRs skip macOS; `test:macos` opt-in) | A few percent less demand | $0 | Docs/site PRs: none that matters | Small |
| One-job macOS gate on PRs into main; full 4 shards on push to main + nightly | PR macOS check ~5-8 min in 1 slot; main-run queue near zero (~0.9-1.2 slots) | $0 | macOS-only regressions in tests outside the gate land on main and are fixed forward | Medium: filterset for macOS-specific tests, second job, aggregate logic |
| macOS nightly only | No macOS wait on any PR or push | $0 | Up to a day's merges before a macOS break shows | Small |
| Fewer shards (2) | Each run ~36 min of execution, but two runs overlap | $0 | None | Trivial; doesn't raise throughput |
| More shards (6-8) | Lower single-run latency when idle; worse queue when busy (one run holds all 5 slots) | $0 | None | Trivial; counterproductive at current load |
| XProtect/Developer Tools probe (#894) | Unknown; 0 to large | One experiment run | None | Small to try; may be blocked by SIP |
| Linux cross-check `--target aarch64-apple-darwin` | Would be a ~1 min gate | $0 | Runtime behaviour untested | Blocked: `aws-lc-sys` needs a clang + macOS SDK toolchain |
| GitHub larger macOS runners | n/a | $0.102/min | n/a | **Unavailable:** Team/Enterprise orgs only, and the cap stays 5 |
| Cirrus Runners | Removes the queue at 5 runners; M4 Pro likely faster per test **[unverified]** | $150/runner/month flat: $300-750 | None | Small: change `runs-on`; personal-account install **[unverified]** |
| Namespace / WarpBuild / Depot / Blacksmith | Removes the cap; faster M4/M5 | $0.06-0.08/min: ~$9,900-13,100/month at today's volume, ~$2,500-3,300 after the workflow cuts | None | Small: change `runs-on`; account and app install |
| Self-hosted Mac mini or EC2 Mac | Fast and no cap; 1 job per machine unless isolated in VMs | Hardware or ~$632/month for mac2-m2 (24 h minimum) | None | Medium-high: upkeep, isolation, fork-PR guard; GitHub advises against it on public repos |
| Drop macOS CI, keep the support claim | Linux-only 11 min | $0 | All: the README's Apple Silicon claim becomes untested | Trivial |
| Drop macOS support entirely | Linux-only 11 min | $0, plus losing every Apple Silicon user and the cargo-dist `aarch64-apple-darwin` artifact | All | Small: README, release targets, `cfg(target_os = "macos")` code becomes dead weight |

## Recommendation

**Keep macOS. Cut how often the full suite runs before paying for anything.**

1. **Ship #893 now.** It removes 29% of macOS demand at no cost. The measurements show demand around 2.7 slots is where the queue sat at seconds.
2. **Move the full 4-shard suite to pushes to main, plus a nightly `schedule`.** On PRs into main, run one macOS gate job: clippy for the macOS target plus a nextest filterset over the macOS-specific modules. Add a `test:macos` label to opt a PR into the full suite, as uv does. Keep the `test-macos` aggregate reporting success, neutral or skipped, so Self-merge isn't stranded.
3. **Under #894, run the one-off probe in section 5:** spawn loops, `csrutil status`, the Developer Tools grant, and the Intel image. Keep cutting git processes per operation, the change that already took one test from 5,415 to 4,221 programs.
4. **If main runs still queue behind each other after steps 1-2, buy 2 Cirrus Runners** ($300/month flat) before any per-minute vendor. At this volume the per-minute vendors cost 4-40 times more. Confirm first that Cirrus installs on a personal account.
5. **Don't move to self-hosted Macs** while the repo is public, and don't plan around GitHub's larger runners or merge queue: both need an organization, and the macOS cap stays at 5 for Team orgs.

Dropping macOS support would save the queue but cost the only platform that exercises the `target_os = "macos"` branches. It would also cost the Apple Silicon release artifact. Steps 1 and 2 recover most of the queue time without giving either up.
