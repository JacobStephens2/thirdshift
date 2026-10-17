# Why thirdshift entries accumulate in /tmp

Investigation for [#569](https://github.com/JacobStephens2/thirdshift/issues/569), observed **2026-10-17 09:40:52–09:41:29 UTC** (05:40:52–05:41:29 EDT), against source commit `a225cea0168d4cd8f6c6d53770ae4481cfa3bd5e`.

There are several independent lifetimes. Factory-skill extraction has a likely test-process cleanup gap; agent scratch/tooling is deliberately outside factory cleanup; test fixtures can outlive interrupted harnesses. The inventory establishes accumulation, not which entries are abandoned or what fraction each mechanism caused. **No leak was freshly reproduced.**

## Fresh inventory and limits

The [captured metadata](thirdshift-tmp-inventory-2026-10-17.json) comes from the bounded Python command below and a separate top-level type count at 09:41:29 UTC. The first pass enumerated 6,608 direct entries under `/tmp`; it recursively measured only four recognizable name groups. No file contents, process tables, credentials, configurations or unrelated directory trees were scanned for this inventory. Eight fixture directory listings were sampled for attribution.

| Name group (mutually exclusive top-level entries) | Fresh count | Measured regular-file bytes | Measured allocated bytes | Coverage |
| --- | ---: | ---: | ---: | --- |
| `thirdshift-skills-*` | 2,114 directories | 463,452,980 | 775,307,264 | Lower bounds; 459 roots truncated |
| Other `thirdshift-*` | 559 entries (86 directories, 473 other) | 2,113,451,323 | 2,217,656,320 | Lower bounds; 15 roots truncated |
| `.tmp*` and `fakes-*` | 111 entries (85 directories, 26 other) | 299,625,675 | 347,463,680 | Selected trees fully visited; attribution incomplete |
| Explicit tooling cues: `pw`, `node-install`, `mermaid*` | 2 entries | 243,120,626 | 254,504,960 | Lower bounds; 1 root truncated |
| All other names | 3,822 entries | Unmeasured | Unmeasured | Unknown; not attributed to thirdshift |

Bounds: 20,000 top-level names, 4,096 visited nodes per root, 300,000 nodes overall, depth 16, 20 seconds of recursive measurement. The scan visited 150,815 nodes and reached its time limit; per-root caps can also truncate trees. There were no recorded OS errors. Names/counts came from the initial enumeration even for unmeasured roots. The eight subsequent fixture listings have a separate 101-name cap (none hit it).

Sizes sum regular-file `st_size` and all visited entries' `st_blocks * 512`, respectively. They do not follow symlinks or cross devices; hard links are counted per directory entry, so allocated bytes are not unique physical disk usage. This is a live, non-atomic view with no activity/staleness test, not a deletion list. Prefixes alone establish neither creator nor safe disposal authority. Reading metadata/directory listings can update filesystem access times.

## Comparison with October 16

**Historically observed:** `/tmp/thirdshift-triage-handoff-2026-10-16/presort-assessments.json`, object `number: 569`, recorded the following. These are transcribed historical measurements, not remeasured totals; its unit convention and traversal bounds were not recorded.

| Group | October 16 handoff | October 17 observation |
| --- | --- | --- |
| Factory extraction | 1,904 directories, “828 MB” | 2,114 directories; partial allocated-byte measurement above |
| Session scratch/tooling | About 508 entries, “2.3 GB”; five Mermaid installs about “226 MB” each | 559 other named entries; name selection may differ |
| Generic fixtures | 84 `.tmp*` entries (“177 MB”), including `.tmp3tDpdV` (“103 MB”); 23 `fakes-*` entries (“134 MB”) | 85 `.tmp*`, 26 `fakes-*`; combined sizes above |

The handoff called the collection “about 3.4 GB.” Do not add its subgroups as an exact total or calculate a disk-growth rate from these incompatible measurements. Factory-prefix count is 210 higher, but there is no per-entry identity history establishing creations, removals, or their cause. The historical report also described bursts of 16 extraction directories at 15:51:53 and 16:18:14, on a day with two Commands. That supports investigating tests; it is not fresh evidence of which process created any present directory.

**Historically observed:** `HANDOFF.md` in that same directory recorded two release-test scripts polling for about 17 days and 24 setup PTY children idle for 10–16 days, ended at the maintainer's request. The fresh investigation neither found nor stopped those historical processes. Their termination event and original fixture states are unavailable.

## Causes, ownership and independent remedies

### 1. Factory-skill extraction

**Source-inspected (high confidence in mechanism):** [`src/skills.rs`](../../src/skills.rs), `written_out`, stores one `TempDir` with prefix `thirdshift-skills-` in static `WRITTEN`. `remove_written` takes and drops it. [`Started::finish`](../../src/command.rs) calls that after sessions finish, consistent with [ADR-0012](../adr/0012-factory-skills-linked-into-the-worktree-codex-unsandboxed.md). This retention is intentional during a Command, including its child Runs.

[`src/skills/tests.rs`](../../src/skills/tests.rs) calls `link_into` repeatedly without Command finishing. [`Sessions::within`](../../src/session.rs) also links skills. The locked `tempfile` 3.27.0 source documents that static values and abrupt exit can skip destructors (`src/lib.rs:20–25`); `TempDir::drop` also ignores removal errors (`src/dir/mod.rs:498–502`). These local dependency sources were inspected, not runtime-tested.

**Hypothetical (medium confidence in contribution):** test binaries that extract into the static and exit without finishing leave extraction directories behind; process-per-test runners could multiply this. Abrupt Command death or ignored deletion failures are alternative contributors. No runner invocation, process identity or deletion error was freshly correlated with the 2,114 entries. The source gap and historical bursts do not prove all extraction directories are leaks.

**Minimal proposed remedy, separate follow-up:** give test-process extraction a scoped lifecycle owner that releases only after every skill user has finished. Preserve once-per-Command extraction in production. Do not call global `remove_written` in arbitrary parallel test teardowns: another test/session may still use its links. Consider surfacing explicit-close failures separately from ownership changes.

**Bounded validation plan:** one child test process, a dedicated temporary parent, and one real `link_into` call; after ordinary process exit assert that parent's extraction is absent, first demonstrating the failure on current code. Then cover normal Command finishing, concurrent users and unwinding independently; abrupt death needs a separately specified contract. Inspect/remove only the fixture's own paths. **Day shift decision before implementation:** approve the extraction owner/teardown seam and supported interruption behavior; no stale-directory janitor is implied.

### 2. Agent scratch and tool installations

**Source-inspected:** [README, “What sessions leave behind”](../../README.md#what-sessions-leave-behind) deliberately retains user-level agent writes outside factory resources, giving `/tmp/pw`, PR drafts and a JDK under `~/.local/jdk/` as examples. It recommends preinstalling toolchains and browser dependencies. This is intended retention, not proof of a factory cleanup defect.

**Historically observed:** the handoff attributed about 508 entries and repeated Mermaid installations to sessions. **Hypothetical (medium confidence for named scratch, low for generic tooling):** current other `thirdshift-*` names include PR drafts, test logs and review artifacts, consistent with that history. The 559-name count includes handoffs and other investigations, and the tooling cues have no creator evidence; neither group is wholly attributable to production Runs.

**Minimal proposed remedy:** reuse preinstalled tooling, and in a separately approved change document where disposable session artifacts should go and who removes them. No new scratch convention or cleanup is implemented here. **Day shift decision:** whether to retain the documented behavior or adopt a new scratch lifetime. Closed [#23](https://github.com/JacobStephens2/thirdshift/issues/23) is historical context only; its per-Run `TMPDIR` proposal is not authorization.

**Bounded validation plan after a decision:** one fake-Harness Run that writes to the approved scratch location, followed by success and Failed run cases; verify repair/continuation sessions share the intended lifetime and pre-existing user paths remain untouched. Keep scratch ownership separate from Factory-skill ownership.

### 3. Test fixtures and child processes

**Fresh metadata, source-inspected:** seven sampled `.tmp*` roots contain the characteristic `bin`, `claude-script.sh`, `gh-state.json`, `home`, `origin.git`, `tmp` and `work` layout from [`Scenario::new`](../../tests/support/mod.rs). The eighth, historical `.tmp3tDpdV`, still has `contributor`, `maintainer`, `bin`, `origin.git` and fake-gh record names, consistent with [`Release::new`](../../tests/release.rs). This supports fixture attribution for the samples, not every generic entry. No contents or liveness were inspected.

**Source-inspected:** `Scenario` and `Release` own non-static `TempDir` values, which normally clean up on drop. Scenario children receive `TMPDIR=<fixture>/tmp`, so their extraction is normally nested in the fixture rather than directly under host `/tmp`. `TerminalCommand::drop` kills and waits for the immediate setup child, and prompt/exit waits are bounded. It would be wrong to claim assertion-unwind cleanup is entirely missing.

**Source-inspected:** release-test `with_input` uses `wait_with_output()` with no test-level deadline, while `Release::command` sets `RELEASE_POLL_SECONDS=0`. [`scripts/release.sh`](../../scripts/release.sh) explicitly promises no overall production timeout. **Hypothetical (medium confidence):** harness death can skip fixture drops, and never-completing release polling can retain a live fixture. The historical survivors are compatible with those mechanisms but establish neither the original trigger nor the cause of all leftover directories.

**Minimal proposed remedies and bounded validation plans:** keep the setup-child investigation in [#736](https://github.com/JacobStephens2/thirdshift/issues/736) and release-child investigation in [#737](https://github.com/JacobStephens2/thirdshift/issues/737). Each asks for one isolated fixture and separate completion, unwind, timeout and abrupt harness-death checks, with cleanup restricted to owned children and paths. Those issues are related lifetime investigations, not explanations for every entry in this inventory. **Day shift decisions:** supported-platform child ownership/termination contract; any change to the release script's production waiting policy needs separate approval.

## Preservation and validation of this investigation

Recommendations must preserve active Commands, shared skill users and the **Failed run** contract. [`Worktree::preserve_failed_run`](../../src/worktree.rs) arms retention before salvage, and [`ownership::preserve_failed_run`](../../src/worktree/ownership.rs) aborts an unfinished merge, stages with `git add -A`, commits changes and pushes; unsuccessful preservation retains the Worktree and Issue branch. Do not turn a report about `/tmp` into deletion of retained Worktrees or move disposable scratch into salvage's staging surface without an explicit decision.

Only report/evidence and requested review/PR artifacts were written. Existing temporary entries, processes, configuration and retention policy were left in place; no fixture was launched, no process signalled, and no cleanup was implemented. No full suites, Cargo builds/typechecks or runtime reproductions were run: #569 explicitly excludes full-suite evidence generation, and this change is documentation only. Validation checks the saved inventory's accounting, relative source links and `git diff --check`. The evidence supports the ranked follow-ups above; it does not establish a complete host disk total or a safe automated deletion policy.

## Inventory command

Run from any directory with Python 3 on this Linux host. It reads metadata and directory entries only, emits JSON to stdout, refuses a top-level overflow, avoids symlink traversal through directory file descriptors and does not cross devices. Recursive limits are cooperative between syscalls, not a hard timeout against a stalled filesystem. A rerun observes a new live view; it cannot reproduce the historical numbers.

```sh
python3 - <<'PY'
import collections, datetime, itertools, json, os, stat, time
started = datetime.datetime.now(datetime.timezone.utc).isoformat()
deadline = time.monotonic() + 20
limits = dict(top_level=20000, per_root=4096, total_nodes=300000, depth=16, seconds=20)
used = 0
errors = collections.Counter()
groups = collections.defaultdict(lambda: dict(roots=0, directories=0, regular_files=0, symlinks=0, other=0, file_bytes=0, allocated_bytes=0, truncated_roots=0))
def group(name):
    if name.startswith('thirdshift-skills-'): return 'factory_prefix'
    if name.startswith('thirdshift-'): return 'named_scratch_candidates'
    if name.startswith(('.tmp', 'fakes-')): return 'generic_fixture_candidates'
    if name in ('pw', 'node-install') or name.startswith('mermaid'): return 'tooling_candidates'
    return 'unknown_unmeasured'
def measure(parent, name, g, budget, depth):
    global used
    if used >= limits['total_nodes'] or budget[0] >= limits['per_root'] or time.monotonic() >= deadline or depth > limits['depth']:
        return False
    used += 1
    budget[0] += 1
    try:
        s = os.stat(name, dir_fd=parent, follow_symlinks=False)
        g['allocated_bytes'] += s.st_blocks * 512
        if stat.S_ISREG(s.st_mode):
            g['regular_files'] += 1
            g['file_bytes'] += s.st_size
        elif stat.S_ISLNK(s.st_mode): g['symlinks'] += 1
        elif stat.S_ISDIR(s.st_mode):
            g['directories'] += 1
            if s.st_dev != root_device: return False
            fd = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
            try:
                with os.scandir(fd) as entries:
                    for entry in entries:
                        if not measure(fd, entry.name, g, budget, depth + 1): return False
            finally: os.close(fd)
        else: g['other'] += 1
    except OSError as e:
        errors[type(e).__name__] += 1
        return False
    return True
root_fd = os.open('/tmp', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
try:
    root_device = os.fstat(root_fd).st_dev
    with os.scandir(root_fd) as entries:
        names = sorted(e.name for e in itertools.islice(entries, limits['top_level'] + 1))
    assert len(names) <= limits['top_level'], 'top-level cap reached; narrow scope'
    # Count every selected root even when recursive measurement is truncated.
    for name in names: groups[group(name)]['roots'] += 1
    for name in names:
        category = group(name)
        if category != 'unknown_unmeasured' and not measure(root_fd, name, groups[category], [0], 0):
            groups[category]['truncated_roots'] += 1
    samples = sorted(set(['.tmp3tDpdV'] + [n for n in names if n.startswith('.tmp')][:8]))
    markers = {}
    for name in samples:
        try:
            fd = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=root_fd)
            try:
                with os.scandir(fd) as entries:
                    children = sorted(e.name for e in itertools.islice(entries, 101))
                markers[name] = dict(first_100_children=children[:100], capped=len(children)>100)
            finally: os.close(fd)
        except OSError as e: markers[name] = type(e).__name__
finally: os.close(root_fd)
print(json.dumps(dict(started_utc=started, ended_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(), limits=limits, top_level_entries=len(names), measured_nodes=used, groups=dict(groups), errors=dict(errors), fixture_marker_samples=markers), indent=2))
PY
```
