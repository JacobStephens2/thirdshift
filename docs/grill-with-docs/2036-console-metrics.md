# Grilling: #2036, Console Metrics view for factory health over time

Issue: https://github.com/JacobStephens2/thirdshift/issues/2036
Research: [factory-health-metrics.md](../research/factory-health-metrics.md), [software-factory-console-trends.md](../research/software-factory-console-trends.md), [ADR 0016](../adr/0016-the-console-is-a-long-running-reader.md), [ADR 0018](../adr/0018-the-console-has-one-layout-and-themes.md), [ADR 0026](../adr/0026-each-console-serves-an-api-that-can-do-what-the-cli-can.md)

## Round 1 (answered)

**Q1.1 - Name, route, and CLI twin (`Metrics` on `/metrics` vs a new `Health` command/route).** `/metrics` and `/api/v1/metrics` (`thirdshift metrics`) already exist in `src/console.rs`, with no link in `nav.html` and raw `<pre>` JSON as the HTML body, while `"health"` in `status --json` and `show --json` already names a live Command's `ok` / **Slow** / **Silent** observation. The options:
- (a) Make `/metrics` (`thirdshift metrics`) a first-class Console page in `nav.html` named **Metrics**, and broaden **Run metrics** in `GLOSSARY.md` beyond **Ticket runs** to completed **Commands**.
- (b) Keep `/metrics` and `thirdshift metrics`, and label the Console nav item **Health**.
- (c) Create a separate `thirdshift health` CLI command, `/health` page, and `/api/v1/health` endpoint.

*Recommended:* (a).

*Answer:* agreed, and I want to be able to on the Console Metrics page be able to see metrics for the entire fleet.

*Checked:* Updated **Run metrics** in `GLOSSARY.md` to cover completed **Commands** and list `health score` under `_Avoid_`.

**Q1.2 - Separate signals vs a composite health score.** Uphold `GLOSSARY.md`'s **Run metrics** rule ("Each is kept apart, never combined into one number") by showing explicit per-signal ratios and distributions rather than a synthetic composite score?

*Recommended:* Yes.

*Answer:* agreed

**Q1.3 - Machine scope (`This machine` vs `Fleet`).** Add `thirdshift metrics --fleet` (client-side aggregation over each paired machine's local `/api/v1/metrics`, matching `cronjobs --fleet` and ADR 0026) and default the Console **Metrics** page to **Fleet** when paired Fleet machines exist?

*Recommended:* Yes.

*Answer:* agreed

**Q1.4 - Which signal families belong on the view.** Combine (1) completed Run outcomes and execution signals and (2) post-merge quality trends (**Bug origins** + weekly **Scoring run** criterion pass rates) over time, leaving (3) live operational state (**Needs you**, **Cron jobs**, **Resources**) on their existing Console pages?

*Recommended:* Yes.

*Answer:* agreed, /research may help inform us on which metrics we would want or which would be useful - I don't want metrics for metrics sake, but in service of making thirdshift operate more effectively and identifying room for improvement in it or areas that are failing.

**Q1.5 - Time grain (`Daily` + `Weekly` vs `Weekly` only).** Support **daily** buckets alongside **weekly** Monday buckets so both recent 24h–30d regressions and multi-month trends are legible?

*Recommended:* Yes.

*Answer:* agreed. And I want to be able to track a history of the health as well. To see how it changes over time. Like is it getting better or worse, for example, in various ways? And can that be correlated to particular version changes or something like that? This starts to get me thinking about data storage. Whether we continue using a log type storage approach or whether we think about having a third shift set up with like a centralized PostgreSQL server or something.

*Checked:* Sampled all 6 Fleet machines (`dev`, `orchestrate`, `stephens.page`, `thirdshift.app`, `lima-thirdshift`, `test`): they hold **2,099** `.command.json` records total (~6 MB across the entire Fleet; ~30 MB/year at 10,000 runs/year), `.command.json` files are never deleted by `thirdshift`, and `src/logs/reading/cache.rs` already caches parsed `CommandTree` records in memory inside `thirdshift console` by `(len, modified, inode)`. Also checked version attribution: `src/session/trace.rs` already writes `service.version = env!("CARGO_PKG_VERSION")` into every Session trace's OTLP resource, whereas `CommandRecord` (`.command.json`) does not yet store `thirdshift_version`.

## Round 2 (answered)

**Q2.1 - Where does long-term Metrics history live: per-machine records + Fleet API, a local Console cache of remote buckets, or centralized PostgreSQL?** Across all 6 Fleet hosts today there are 2,099 `.command.json` files (~3 KB each with `run_metrics` saved at completion = ~6 MB total, or ~300 MB for 100,000 runs over years even if multi-GB `.session.jsonl` transcripts are pruned later). Meanwhile, ADR 0001/0003 keep thirdshift a single static binary with zero daemon dependencies, and ADR 0026 explicitly rejected syncing logs to a central host. The one gap in pure live fan-out over `/api/v1/metrics` is **retired or temporarily offline machines** (for example, wiping a Lima VM or decommissioning a droplet loses that host's past buckets unless retained somewhere). The options:
- (a) **Local `.command.json` source of truth + Fleet API fan-out + cached last-seen Fleet buckets on the watching Console.** Each machine keeps writing only its own `.command.json` and Scoring files (zero external DB, runs never block on network storage, `/api/v1/metrics` returns compact pre-aggregated daily/weekly buckets in milliseconds, and `cache.rs` keeps reads in memory). Optionally, when a Console fetches `/api/v1/metrics` from Fleet peers, it also caches each peer's last-fetched historical buckets on disk (`~/.thirdshift/logs/fleet-metrics/<machine>.json`) so an offline or retired machine's past history stays visible (marked cached/offline) without a database server.
- (b) **Pure live `.command.json` + `/api/v1/metrics` fan-out (no cached peer snapshots).** Simplest; an offline machine is listed as unreachable and omitted from totals until it comes back, and retiring a machine means copying `~/.thirdshift/logs` if you want to keep its history.
- (c) **Centralized PostgreSQL server.** Every Fleet machine pushes Run completions to a shared Postgres instance (or syncs via a background shipper). Gives arbitrary SQL across decommissioned hosts, at the cost of provisioning/securing/migrating/backing up a database across ETA, `thirdshift.app`, `stephens.page`, and Lima VMs, plus local queueing whenever Postgres is unreachable—reopening ADR 0001 and ADR 0026.

*Recommended:* (a), or (b) if you rarely retire hosts without migrating `~/.thirdshift`. At ~3 KB per completed Command, `.command.json` *is* already a durable self-contained summary row (especially once #2047 records `run_metrics` for Review, Triage, and Fix runs), and keeping raw `.session.jsonl` prunable independently of `.command.json` gives years of history in tens of megabytes with zero Postgres operational burden.

*Answer:* agree - This makes me wonder if I could use my local host, the Lima virtual machine running locally here, as the fleet control plane and capture this historical data or if that would be inferior to in some ways to the like having third shift.app virtual machine, digital ocean, cloud server or be that have that role, given that it would be on 24-7, unlike the Lima virtual machine running on my MacBook. What I like about running on the MacBook is that then I can use the compute on my laptop for additional third shift work. and so I can see what the whole fleet is doing, including what's happening on my laptop and unless there's a way that we could have the server read from my laptop maybe we could have the best of both worlds the twenty four seven host, but also the compute capacity of my laptop.

*Checked:* Inspected `src/console/fleet.rs` (`pairing_address`) and [ADR 0026](../adr/0026-each-console-serves-an-api-that-can-do-what-the-cli-can.md): Fleet pairing is peer-to-peer rather than single-master, so *both* `thirdshift.app` and `lima-thirdshift` can pair outbound over HTTPS with all 5 cloud hosts at the same time (meaning `lima-thirdshift`'s local Console already sees all 6 hosts—laptop + 5 cloud servers—whenever the MacBook is open). However, `pairing_address` requires inbound HTTPS (`https://`, except `127.0.0.1`), so `thirdshift.app` cannot pull inbound from a NATted or sleeping `lima-thirdshift` VM; **Q3.4** below covers how `thirdshift.app` (24/7 phone/remote hub) and `lima-thirdshift` (laptop compute) combine for the best of both worlds.

**Q2.2 - Recording and correlating `thirdshift` version (and Harness/Model changes) over time.** Today `SessionSignals` records `harness`, `harness_version`, and observed `model`, and Session traces carry `service.version` (`CARGO_PKG_VERSION`), while `CommandRecord` (`.command.json`) itself has no `thirdshift_version` field. To answer *"did things get better or worse after 0.29.0, or after switching Codex/Claude models?"*, the options are:
- (a) **Record `thirdshift_version` on `CommandRecord` (recovering `service.version` from Session traces on older records when present), expose `thirdshift_version` as a grouping/filter dimension in `metrics`, and mark version changes on the Console Metrics timeline.** Whenever a day/week bucket crosses a new `thirdshift_version` (or `harness` / `model` shift), the timeline annotates the transition and lets you slice or compare before vs. after by `thirdshift_version`, `Harness`, `Model`, `Repository`, or `Command kind`.
- (b) **Only annotate `thirdshift_version` transitions on the timeline** without allowing filtering/grouping by `thirdshift_version`.
- (c) **Also track git commit SHAs of target repositories' `AGENTS.md` / prompts.** (High cardinality and repo-specific.)

*Recommended:* (a). Adding `thirdshift_version` to `CommandRecord` at startup is one field, backfillable from existing Session traces' `service.version`, and makes release-over-release regressions directly measurable alongside `Harness` and `Model`.

*Answer:* agreed

**Q2.3 - Which records count as Runs vs Pass wrappers (avoiding double-counting).** When cron runs `thirdshift pickup`, a `Pickup run` record dispatches a child `Ticket run` (`Run`) or `Spec run` (and a `Spec run` dispatches child `Ticket run`s and `Base fix`es). Today `trends::weekly` emits separate rows by `kind`, including `Pickup run` alongside `Run`. On the Console Metrics page, how should hierarchy work so outcomes aren't double-counted?
- (a) **Count leaf/work Runs by their own Command kind (`Ticket run`, `Spec run`, `Base fix`, `Review run`, `Fix run`, `Triage run`, `Architect run`, `Security run`), exclude pure dispatcher `Pickup run` wrappers from outcome ratios unless a `Pickup run` itself failed/died before dispatch, and avoid rolling child `Ticket run`s and parent `Spec run` into a single summed denominator.** Each Command kind is shown on its own row/series (plus a filter for one kind at a time).
- (b) **Show `Pickup run` as just another Command kind row alongside `Ticket run`.** (In practice almost 100% of `Pickup run` outcomes just mirror the `Ticket run` or `Spec run` they dispatched.)

*Recommended:* (a). `Pickup run` carries out no Process of its own (`GLOSSARY.md`); counting both `Pickup run` and the `Ticket run` it dispatched duplicates every picked-up failure and success, unless the `Pickup run` itself failed during selection/reservation before dispatching.

*Answer:* agree

**Q2.4 - Defining outcome buckets ("fail-to-success") across Command kinds with different happy paths.** Different Command kinds succeed differently (`Ticket run` -> `Merged` or `Ready for review` or bug-not-reproduced `needs-info`; `Review run` -> reviewed/pushed or recommend-only; `Triage run` -> triaged/filed; `Experiment` / `Scoring run` -> `Paused` on plan limits, `Stop` -> interrupted by Day shift). How should outcomes be classified for trend ratios?
- (a) **Four explicit outcome classes across every Command kind—`Succeeded`, `Failed` (including `Died` / crashed), `Paused` (plan/quota limit), and `Stopped` (human `Stop`)—with exact sub-outcomes preserved on hover/drilldown.** Success rate is `Succeeded / (Succeeded + Failed)` (excluding intentional human `Stop` and `Paused` from failures while still displaying `Paused` and `Stopped` bars/counts visibly beside them so quota stalls or frequent human Stops never hide). Skipped Passes (idle cron ticks) stay out of Run denominators completely.
- (b) **Count `Stopped` and `Paused` inside `Failed`** (`Succeeded / All ended`).
- (c) **Show only raw outcome strings (`merged`, `ready_for_review`, `failed`, `stopped`, `paused`, …) with no normalized `Succeeded` vs `Failed` ratio.**

*Recommended:* (a). `GLOSSARY.md` explicitly defines **Paused Command** as *"neither success nor failure"* and **Stop** as a human interruption; keeping `Succeeded`, `Failed` (including dead records), `Paused`, and `Stopped` visible as four distinct stacks while computing failure rate over `Succeeded + Failed` gives an honest fail-to-success trend without penalizing plan pauses or manual Stops.

*Answer:* agree

**Q2.5 - Answering "is it getting better or worse?" (window presets and period-over-period delta).** Beyond plotting buckets over time, how should the page and `thirdshift metrics` make direction of travel obvious?
- (a) **Window presets (`7d` daily, `30d` daily, `12w` weekly, `All` weekly) plus an explicit comparison against the immediately preceding equal window** (e.g. last 7d vs prior 7d, or last 4w vs prior 4w), showing the delta alongside sample size (`n`) and keeping `observed` / `missing` coverage visible when older records lack a signal.
- (b) **Time-series buckets only, with no period-over-period comparison summary.**

*Recommended:* (a). Pairing the time series with a plain previous-window delta (always showing `n`, e.g. `Failed: 12% (6/50) vs 29% (14/48) prior 30d`) immediately answers whether reliability, repair churn, or cost per run is improving or regressing without inventing any statistical black box.

*Answer:* agreed

**Q2.6 - Which exact signals earn a place on the Metrics view (and fixing outcome-blind averaging in `trends.rs`).** [factory-health-metrics.md](../research/factory-health-metrics.md) converged on a four-pillar tension model (DORA's 5-metric rework model + SPACE/DevEx + Kapoor et al.'s cost-controlled agent evaluation) and uncovered two active distortions in `src/logs/metrics/trends.rs` today: (1) `total_minutes`, `repairs`, and `ci_rounds` average across *all* runs regardless of outcome—so a week of 30-second startup crashes makes mean duration and mean **Repairs** look *better*; and (2) `record.outcome.is_none()` lumps crashed `died` records together with currently `live` runs under `missing_outcomes`. The options:
- (a) **Organize the page around four paired signal groups (no vanity metrics, survivorship-safe denominators):**
  1. **Yield & PR survival** (per Command kind): `Succeeded` / `Failed` (with crashed `Died` separated from `Live`), `Paused`, `Stopped`, plus Kept GitHub PR survival (`merged` vs unmerged `closed` from `github-inbox.json` when available).
  2. **Post-merge quality & human-attention load**: `confirmed_bug_origins` per merged cohort, weekly calibrated **Scores** criterion pass rates (`scoring.rs`), and **Stop** / **Needs you** intervention counts.
  3. **Efficiency, wasted effort & Repair churn**: compute duration, **Repairs**, and CI rounds **on succeeded runs** (preventing early crashes from flattering speed/repair averages), alongside **first-pass clean rate** (`repairs == 0` among succeeded runs), **effective time/cost per succeeded delivery** (all attempts divided by succeeded runs, separated by Model/Harness with `observed`/`missing` coverage), and **wasted time/spend share** on failed/died/stopped runs.
  4. **Stage bottlenecks & session trajectory friction**: `% Position::Above` (stages exceeding repository **Usual range**), plus per-session `repeated_tool_calls`, `tool_errors`, `permission_denials`, `killed_work`, and non-zero `destructive_commands`—while leaving out LOC, pooled cross-kind pass rates, and raw `review_findings` / `unaddressed_findings` counts as quality proxies (surfacing only `security > 0` holds).
- (b) **Keep `trends.rs`'s current signal list as-is** (outcome-blind averages across all runs, raw token sums, and raw finding averages, without wasted-effort share, first-pass clean rate, or `Scores` trends).

*Recommended:* (a). Every signal in (a) maps directly to an operator action (fixing a broken stage, tuning a noisy prompt/skill, catching a bad model/release, or spotting post-merge regressions), and fixing outcome-blind averaging + `died` vs `live` prevents outages from masquerading as speedups.

*Answer:* agreed, and what are vanity metrics?

*Checked:* Vanity metrics are numbers that track raw activity or move in a flattering direction without telling you what to fix—or even improve when the factory breaks: e.g. lines of code / commits added (rewards diff bloat), raw token totals or Runs started (looks busy even when half fail), outcome-blind mean duration or mean **Repairs** (drops toward zero during a startup-crash wave), raw review-finding counts (fewer findings can mean cleaner code *or* a blinder reviewer), and pooled cross-kind pass rates (frequent short `Triage run`s masking `Ticket run` failures). Also verified in `tests/scoring.rs` and `src/logs/github.rs` that `thirdshift metrics` makes zero live GitHub calls, reading Kept PR state strictly from local `github-inbox.json`.

**Q2.7 - Structured failure attribution (`stopped_at` stage + failure category) so you see *where* runs fail without opening 50 logs.** Today `CommandRecord::finish` (`src/logs/command_record.rs`) receives structured `recovery::Stage` (`src/recovery.rs`) and `FailedRun::held()` / `waiting()` (`src/failed_run.rs`), then discards them and writes only free-text `cause`. Saving optional `stopped_at` stage + deterministic failure category on `CommandRecord` (falling back to `stages.last()` on older records) lets `/metrics` group failures by *where* they broke (e.g. Worktree acquisition, Implement, Code review, CI watch, Self-merge hold, Crash/died) and link straight to the offending Commands. Should we record and surface that failure breakdown?
- (a) **Yes:** persist structured `stopped_at` stage and failure category on `CommandRecord` (with `stages.last()` fallback for historical records) and show a "Where runs failed" breakdown on `/metrics` that links through to the matching History / Command detail rows.
- (b) **No:** show only total failure counts per Command kind; inspect individual Command logs to see where they failed.

*Recommended:* (a). In today's 245-run fleet retrospective, almost every systemic bug (`git worktree add` x47, `Self-merge` hold x9, `pull request changed during Review run`, Triage JSON validation) was immediately obvious once grouped by stopping stage/category and invisible in aggregate failure counts alone.

*Answer:* agreed, agreed

**Q2.8 - Opted-in `Other-account Commands` on a machine.** Unlike **Inbox** and `thirdshift status`, `Snapshot::load_metrics` currently reads only the serving OS account's `~/.thirdshift/logs` and ignores `console.other_accounts`. Should `thirdshift metrics` and `/api/v1/metrics` include readable **Other-account Commands** when configured in `UserConfig` (tagged/filterable by account, matching **Inbox**)?
- (a) **Yes:** include configured readable **Other-account Commands** on each machine alongside the serving account, with account tagging/filtering.
- (b) **No:** keep `metrics` restricted to the serving OS account only.

*Recommended:* (a), so a Console watching multiple OS accounts on one server shows the same factory scope in **Metrics** as it already shows in **Inbox**.

*Answer:* agreed

## Round 3 (answered)

**Q3.1 - 24/7 `thirdshift.app` Fleet hub + MacBook (`lima-thirdshift`) compute (following up on Q2.1).** Comparing `thirdshift.app` vs `lima-thirdshift` as your primary Fleet Console:
1. **`thirdshift.app` is strictly stronger as the 24/7 Fleet Console:** it stays awake when your MacBook lid is closed, is reachable from your phone anywhere, and continuously refreshes/caches peer snapshots (`~/.thirdshift/logs/fleet-metrics/<machine>.json`).
2. **Fleet pairing (`ADR 0026`) is peer-to-peer, not single-master:** *both* `thirdshift.app` and `lima-thirdshift` can pair outbound with the 5 cloud servers at the same time. That means whenever you are on your MacBook, `lima-thirdshift`'s local Console already sees **all 6 hosts** (its own laptop compute + all 5 cloud hosts) with zero new networking.
3. **The one gap is the reverse direction (`thirdshift.app` reading `lima-thirdshift`):** `src/console/fleet.rs` pulls inbound over `https://`, which cannot reach a sleeping or NATted laptop VM. Letting `lima-thirdshift` **push** snapshots outbound over normal HTTPS to `https://console.thirdshift.app` solves that with zero inbound ports on your MacBook—and notice that the exact same desire (*"see what the whole fleet is doing, including what's happening on my laptop"*) applies to live **Inbox / `status`** as well as **Metrics**, needing a narrow per-worker push token scope in `ADR 0026` so a worker token can only update its own named machine slot.

How should we scope that?
- (a) **Make `thirdshift.app` your 24/7 Fleet Console (and optionally pair `lima-thirdshift` outbound for 6-host viewing at your desk); keep `#2036` pull + peer-cache (`~/.thirdshift/logs/fleet-metrics/<machine>.json`), and open a dedicated sibling Spec/issue for outbound NATted-worker push (`lima-thirdshift -> thirdshift.app`) covering both live `Inbox` (`status`) and `Metrics` together.** `#2036`'s peer-cache format will already be ready to accept pushed metrics snapshots, and designing outbound worker push once covers the whole Console cleanly.
- (b) **Include outbound `thirdshift metrics --push <fleet-peer>` inside `#2036` now** (writing `lima-thirdshift`'s metrics snapshot over HTTPS into `thirdshift.app`'s `~/.thirdshift/logs/fleet-metrics/lima-thirdshift.json` behind a dedicated push token scope), leaving live `Inbox` (`status`) push from the laptop for a follow-up issue.
- (c) **Use `lima-thirdshift` alone as the Fleet Console** (sees all 6 hosts when the laptop is open, with no 24/7 phone/remote Fleet view when the MacBook is closed).

*Recommended:* (a)—or (b) if you want laptop **Metrics** visible on `thirdshift.app` immediately inside `#2036` before tackling live laptop **Inbox** push. Either (a) or (b) gives you both the 24/7 `thirdshift.app` Console and your MacBook's compute capacity with zero inbound ports on the laptop.

*Answer:* a assuming this means I can view laptop Metrics on thirdshift.app, which is what I want

*Checked:* Flagging the assumption explicitly so nothing slips: under **(a)** alone, `#2036` only pulls over inbound HTTPS, so laptop (`lima-thirdshift`) Metrics would **not** appear on `thirdshift.app` until the sibling outbound-push Spec also ships—whereas putting outbound Metrics push inside `#2036` makes laptop Metrics appear on `thirdshift.app` as soon as `#2036` lands. Also checked `GLOSSARY.md` (**Fleet**, lines 329–331) and `src/console/tokens.rs` (`Action::Create { holder, scopes, .. }`): `GLOSSARY.md` currently defines every Fleet machine as one on which the Console *"can see and do … whatever that machine's own Console offers"*, whereas a sleeping/NATted laptop pushing outbound is read-only from `thirdshift.app`, and `tokens.rs` already binds every token to a unique `<holder>` name. Opened **Q4.1** and **Q4.2** below to settle the deliverable boundary and push mechanics cleanly.

**Q3.2 - Visual presentation on desktop and 390px phone under ADR 0018 Themes.** [ADR 0018](../adr/0018-the-console-has-one-layout-and-themes.md) gives the Console one layout across desktop and 390px mobile, CSS custom properties from `themes.css`, and zero motion. How should `/metrics` render trends and comparisons in HTML?
- (a) **Server-rendered Askama HTML tables paired with lightweight inline SVG/CSS stacked outcome bars, compact trend sparklines, and version-change markers** styled with `themes.css` semantic tokens (zero client-side JS charting library, zero motion, accessible numbers beside every bar, responsive collapse at 390px).
- (b) **Numbers-only HTML tables** with no bars, sparklines, or visual timeline markers.
- (c) **Client-side JS charting library.**

*Recommended:* (a). Static inline SVG/CSS bars and sparklines make regressions, repair spikes, and version transitions legible at a glance on both phone and desktop without adding a JS chart runtime or breaking Theme contrast rules.

*Answer:* agreed

**Q3.3 - Navigation placement (`data-jump="m"`), URL/CLI filters, and drilldown to offending Commands.** In `src/console/nav.html`, `1`–`9` map to operational views (`1 Inbox` … `9 Resources`) followed by `Experiments` (`x`), `Scores` (`q`), `Bug origins` (`u`), and `Console log` (`c`); `data-jump="m"` is free. How should `/metrics` integrate with navigation, slicing, and investigation?
- (a) **Add `<a href="/metrics" data-jump="m">Metrics</a>` alongside `Experiments` / `Scores` / `Bug origins` in `nav.html`; mirror top-of-page GET filters as CLI flags on `thirdshift metrics` (`machine=fleet|<name>` / `--fleet`, `window=7d|30d|12w|all`, `grain=day|week`, `repo`, `kind`, `harness`, `model`, `version`, `account`, plus existing `from`/`through`); and link every failure bucket and "Where runs failed" row through to `History` / Command details** so clicking a spike shows the exact failing Runs.
- (b) **Add `Metrics` to `nav.html` with `machine` and `window` selectors only**, leaving finer filters (`kind`, `harness`, `model`, `version`) to `thirdshift metrics --json` on the CLI.

*Recommended:* (a). Matching URL query parameters and CLI flags preserves ADR 0016/0026 parity, and one-click drilldown from a bucket or failure stage to the underlying Runs turns every chart spike into an immediate root-cause list.

*Answer:* agreed

## Round 4 (answered)

**Q4.1 - Making sure laptop (`lima-thirdshift`) Metrics appear on `thirdshift.app`: inside `#2036` vs in an immediate companion Spec.** Catching the assumption in your `Q3.1` answer right away: under `Q3.1(a)` by itself, `#2036` only *pulls* over inbound HTTPS, so `thirdshift.app` would show the 5 cloud servers right away and would **not** show `lima-thirdshift` laptop Metrics until the companion outbound-push Spec also lands. Which boundary do you want?
- (a) **Include outbound Metrics push (`lima-thirdshift -> thirdshift.app`) inside `#2036` now** (as `#2036`'s final slice, writing into `thirdshift.app`'s `~/.thirdshift/logs/fleet-metrics/lima-thirdshift.json`), so finishing `#2036` alone lets you view laptop **Metrics** on `thirdshift.app` immediately; file a separate follow-up issue only for pushing live laptop **Inbox** (`status`) to `thirdshift.app`.
- (b) **Keep `#2036` pull + peer-cache only, and create/spec the companion outbound-push issue right alongside `#2036`** (covering both laptop **Metrics** and live laptop **Inbox** on `thirdshift.app` together—knowing laptop Metrics won't appear on `thirdshift.app` until that second Spec also lands).

*Recommended:* (a), since viewing laptop Metrics on `thirdshift.app` is an explicit goal for you now, and `~/.thirdshift/logs/fleet-metrics/<machine>.json` is already being built in `#2036`.

*Answer:* agree

**Q4.2 - Outbound push auth, automatic post-Command trigger, and `GLOSSARY.md` / `ADR 0026` alignment.** **Fleet** in `GLOSSARY.md` currently says that from a watching Console, the Day shift *"can see and do on each of them whatever that machine's own Console offers"*, whereas a sleeping or NATted worker (`lima-thirdshift`) pushing outbound to `thirdshift.app` is **read-only** from `thirdshift.app` (no inbound connection exists to **Stop**, start work, or fetch raw unpushed session transcripts while the laptop is closed). Meanwhile, `src/console/tokens.rs` already binds every API token to a unique `<holder>` name. How should outbound worker push work?
- (a) **Narrow `report` token scope + automatic post-Command push + CLI twin + `GLOSSARY.md` / `ADR 0026` update:**
  1. Add a `report` scope to `thirdshift console token create <holder> --scopes report` where `<holder>` (`lima-thirdshift`) names the reporting Fleet machine; a `report` token can **only** upload snapshots into its own `~/.thirdshift/logs/fleet-metrics/<holder>.json` slot on `thirdshift.app` over HTTPS (zero `read`/`security`/`act`/`run`/`admin` access, and cannot overwrite another machine).
  2. Provide `thirdshift metrics --push <peer>` as the CLI twin **and**, when a push destination is configured on the worker (`lima-thirdshift`), best-effort push automatically whenever a Command finishes (logging a warning and never failing the Run if offline), so ad-hoc manual Runs on your MacBook reach `thirdshift.app` before you close the lid.
  3. On `thirdshift.app`, show pushed peers in **Metrics** with their `pushed_at` age and compact failure summaries carried in the snapshot (showing a clear read-only/offline note rather than broken links where live inbound Command detail logs aren't reachable), and update **Fleet** in `GLOSSARY.md` plus [ADR 0026](../adr/0026-each-console-serves-an-api-that-can-do-what-the-cli-can.md) to cover outbound-reporting Fleet machines.
- (b) **Same `report` token scope, triggered only by manual/cron `thirdshift metrics --push <peer>`** (no automatic post-Command push when a laptop Run ends).

*Recommended:* (a). Automatic best-effort push at Command completion guarantees that interactive Runs on your MacBook reach `thirdshift.app` before the laptop sleeps, without needing a background daemon or failing Runs when you work offline.

*Answer:* agree

*Checked:* Updated **Fleet** in `GLOSSARY.md` and amended [ADR 0026](../adr/0026-each-console-serves-an-api-that-can-do-what-the-cli-can.md) (`report` scope, `metrics --fleet` and `metrics --push` CLI-only exclusions, rejected centralized PostgreSQL option, and cached/outbound-reported peer summary consequence).

## Settled by earlier answers (shout if any is wrong)

- The view lives at `/metrics` in the **Console** (linked as **Metrics** with `data-jump="m"` in `nav.html` alongside `Experiments` / `Scores` / `Bug origins`), paired with `thirdshift metrics` and `/api/v1/metrics` per ADR 0016 and ADR 0026; `"health"` remains the name for live per-Command **Silent** / **Slow** state, and single-Command `thirdshift metrics <command>` stays intact.
- **Run metrics** in `GLOSSARY.md` covers completed **Commands** across Run kinds, and every signal stays in its own units—no composite factory health score.
- Both `thirdshift metrics --fleet` and the Console **Metrics** page aggregate across paired **Fleet** machines (defaulting to **Fleet** in the Console when Fleet peers or reported peer snapshots exist, while `/api/v1/metrics` stays strictly local per owning machine and `metrics --fleet` joins `cronjobs --fleet` in ADR 0026's CLI-only exclusions in `src/console/parity.rs`). `thirdshift.app` serves as your 24/7 Fleet Console (while `lima-thirdshift` can also pair outbound with cloud hosts for local viewing).
- Per-machine `.command.json` and Scoring files remain the sole durable source of truth (no centralized PostgreSQL server; raw `.session.jsonl` transcripts remain independently prunable). When a Console fetches `/api/v1/metrics` from Fleet peers, it caches each peer's last-fetched bucket snapshot under `~/.thirdshift/logs/fleet-metrics/<machine>.json` so offline or retired machines stay visible (marked cached/offline with their last-seen timestamp).
- Outbound Metrics push (`lima-thirdshift -> thirdshift.app`) ships inside `#2036`: `thirdshift console token create <holder> --scopes report` issues a narrow `report` token allowing `<holder>` to upload snapshots **only** into its own `~/.thirdshift/logs/fleet-metrics/<holder>.json` slot over HTTPS; `thirdshift metrics --push <peer>` provides the CLI twin alongside automatic best-effort push on Command completion when configured (warning without failing Runs if offline); pushed peers appear read-only on `thirdshift.app` with their `pushed_at` age and compact failure summaries; and a separate follow-up issue can extend outbound worker push to live **Inbox** (`status`). **Fleet** in `GLOSSARY.md` and [ADR 0026](../adr/0026-each-console-serves-an-api-that-can-do-what-the-cli-can.md) are updated accordingly.
- `CommandRecord` records `thirdshift_version` (`CARGO_PKG_VERSION`) at Command startup and backfills historical records from Session trace `service.version` when present; `metrics` exposes `thirdshift_version` as a filter/breakdown dimension alongside `Harness` (`harness_version`), `Model`, `Repository`, and `Command kind`, and marks version transitions on the timeline.
- Work Runs are counted by their own `Command kind` (`Ticket run`, `Spec run`, `Base fix`, `Review run`, `Fix run`, `Triage run`, `Architect run`, `Security run`); pass-through `Pickup run` dispatch wrappers are excluded from outcome denominators unless a `Pickup run` itself failed or died before dispatch, and child `Ticket run`s and parent `Spec run`s are never summed into one combined denominator.
- Ended Runs classify into four explicit stacks—`Succeeded`, `Failed` (including crashed/died), `Paused` (plan/quota limit), and `Stopped` (human `Stop`)—with exact sub-outcomes preserved on drilldown; success/failure ratio is `Succeeded / (Succeeded + Failed)`, and skipped idle Passes stay out of Run denominators.
- The page covers historical Run outcomes/execution signals plus post-merge quality trends (**Bug origins** and weekly **Scoring run** criterion pass rates) across four paired signal groups in tension—(1) Yield & Kept PR survival (`github-inbox.json`, with zero live GitHub calls), (2) Post-merge quality & human load, (3) Efficiency, wasted effort & Repair churn (duration/**Repairs**/CI rounds on *succeeded* runs, first-pass clean rate `repairs == 0`, effective time/cost per succeeded delivery by Model/Harness, and wasted time/spend share), and (4) Stage bottlenecks (`% Position::Above` Usual range) & session friction (`repeated_tool_calls`, `tool_errors`, `permission_denials`, `killed_work`, non-zero `destructive_commands`, and `security > 0` holds)—fixing `trends.rs`'s outcome-blind averaging and separating crashed `died` records from `live` runs, while omitting vanity metrics (LOC, raw token volume without outcome context, pooled cross-kind pass rates, and raw review-finding counts).
- `CommandRecord::finish` persists optional structured `stopped_at` stage and deterministic failure category (falling back to `stages.last()` on historical records) so `/metrics` renders a clickable "Where runs failed" breakdown.
- `Snapshot::load_metrics` and `/api/v1/metrics` include configured readable **Other-account Commands** (`console.other_accounts`), tagged and filterable by OS account.
- Both **daily** and **weekly** time buckets are supported via window presets (`7d` daily, `30d` daily, `12w` weekly, `All` weekly) paired with an explicit delta against the immediately preceding equal window, always showing sample size `n` and `observed` / `missing` coverage.
- `/metrics` renders server-side Askama HTML tables paired with lightweight inline SVG/CSS stacked outcome bars, compact trend sparklines, and version-change markers styled with `themes.css` semantic tokens (zero client-side JS charting library, zero motion, responsive collapse at 390px per ADR 0018).
- Top-of-page GET filters on `/metrics` mirror CLI flags on `thirdshift metrics` (`machine=fleet|<name>` / `--fleet`, `window=7d|30d|12w|all`, `grain=day|week`, `repo`, `kind`, `harness`, `model`, `version`, `account`, plus `from`/`through`), and every failure bucket and "Where runs failed" row links through to `History` / Command details.
- The frontier is empty: no open questions remain.

## Research results (2026-11-11)

- All 6 Fleet machines currently hold **2,099** `.command.json` records total (~6 MB across the entire Fleet), `.command.json` is never pruned by `thirdshift`, and `src/logs/reading/cache.rs` caches parsed `CommandTree` records in memory inside `thirdshift console`, confirmed by source and Fleet host inspection.
- Every Session trace already writes `service.version = env!("CARGO_PKG_VERSION")` in its OTLP resource (`src/session/trace.rs`), and `SessionSignals` captures `harness`, `harness_version`, and observed `model`, while `CommandRecord` (`.command.json`) does not yet persist `thirdshift_version`, confirmed by source.
- Primary engineering-systems literature (DORA's 5-metric rework model, Forsgren et al. SPACE, Noda et al. DevEx) and agent-evaluation research (Kapoor et al. *AI Agents That Matter* & *Holistic Agent Leaderboard*, Devin Session Insights, SWE-bench/OpenHands trajectory taxonomies) converge on pairing four dimensions in tension—yield/PR survival, post-merge quality & human attention, cost/time per *succeeded* delivery + wasted-effort share + first-pass clean rate, and stage/session friction—while avoiding composite scores, LOC, pooled cross-kind pass rates, and raw finding counts, documented in [factory-health-metrics.md](../research/factory-health-metrics.md).
- Source audit in [factory-health-metrics.md](../research/factory-health-metrics.md) identified four concrete `metrics` bugs/gaps in `src/`: (1) `trends.rs` averages `total_minutes`, `repairs`, and `ci_rounds` across *all* outcomes so fast crashes artificially lower duration and repair averages; (2) `trends.rs` conflates crashed `died` records and `live` runs under `missing_outcomes`; (3) `CommandRecord::finish` drops structured `recovery::Stage` and `FailedRun::held()`/`waiting()` into free-text `cause`; and (4) `Snapshot::load_metrics` omits configured `console.other_accounts` (plus `StageName::is_session` omits `"Code review"`, noted on #2047).

## Waiting on research

- None.
