# Temporary-file accumulation (#569)

Investigation date: 2026-10-17 UTC. Source revision: `a225cea0168d4cd8f6c6d53770ae4481cfa3bd5e`.

Concurrent work on the same Issue branch supplied a [second investigation](thirdshift-tmp-accumulation.md) and its [captured metadata](thirdshift-tmp-inventory-2026-10-17.json), preserved by merging commit `6a39f52`. That session recorded a different `/tmp` view at overlapping times (6,608 entries versus 5,603 here). Host/mount-namespace identity was not recorded, and this session cannot reconcile the views. Keep the snapshots separate: neither their difference nor the merged documents establish creation, deletion or a change rate. The measurements below belong only to this session's observed workspace view.

## Scope and method

This is a read-only diagnosis for [#569](https://github.com/JacobStephens2/thirdshift/issues/569#issuecomment-6228273472), not a cleanup implementation. Existing temporary entries, processes and configuration were left in place. No test suite, reproduction, thirdshift Command, installation or cleanup was run to gather evidence. Repository documentation and the requested review/delivery artifacts are the only new files.

The inventory below lists at most 20,000 direct entries in `/tmp`. It reads metadata, never file contents, and samples at most eight directories per group in sorted name order. Each directory sample has a 10,000-entry and depth-16 limit, with a shared 20-second deadline; symbolic links and other filesystems are not traversed. Counts describe names matching these rules, **not proven owners or stale resources**. Byte counts cover regular files only, count hard-linked paths separately and exclude directory metadata. Directory sizes are samples, not estimates of group totals. Concurrent changes can make observations partial. Names and ages do not authorize deletion.

The complete repeatable command is in the appendix. It records its own observation timestamps and errors. The command does not enumerate processes or read credentials, file payloads or unrelated directory trees.

## Current inventory

**Freshly observed metadata**, 2026-10-17 **09:41:09.488–09:41:09.687 UTC**, on this Linux workspace host: 5,603 direct `/tmp` entries, with no top-level truncation. All 24 sampled directory traversals finished without errors or hitting limits. No lifetime failure was freshly reproduced.

| Name group (ownership unproven) | Direct entries | Direct regular-file bytes / allocated bytes | Sampled directory regular bytes / allocated bytes |
| --- | ---: | ---: | ---: |
| `thirdshift-skills-` + six alphanumerics | 756 directories | 0 / 0 | 8 directories: 3,018,384 / 3,997,696 |
| Other `thirdshift*` | 8 directories, 97 regular files | 14,642,219 / 14,827,520 | All 8 directories: 1,983,913,265 / 1,992,613,888 |
| Historical scratch patterns (`pw`, `vac5docs`, `vac5.log`, `pr[digits]-body.md`) | 5 regular files | 33,175 / 40,960 | No directories present |
| Generic `.tmp*` / `tmp*` | 44 directories, 1 empty regular file | 0 / 0 | 8 directories: 1,269,526 / 1,658,880 |
| Outside selected patterns | 4,692 | Not measured | Not traversed |

For auditability, the extraction samples were suffixes `025WTx`, `032ZTY`, `04im2F`, `08ZSay`, `0KnbLf`, `0P0p43`, `0P5wqV`, `0R900p`. Each contained 66 entries and 377,298 regular-file bytes (499,712 allocated bytes). This is consistent with repeated bundles but does not prove their content, origin or staleness.

The eight other `thirdshift*` directory samples were `thirdshift-189-rustup`, `thirdshift-606-rustup`, `thirdshift-607-rustup`, `thirdshift-621`, `thirdshift-666-browser`, `thirdshift-audit-triage-w_kjxsqk`, `thirdshift-limit-triage-20261009` and `thirdshift-limit-triage-issues-20261009`. Each of the three `rustup`-named directories accounted for 645,977,621 regular-file bytes (646,365,184 allocated bytes): a few large tooling candidates can matter more than a large count of small extractions. Their names alone do not establish who installed them or whether they are still needed.

The generic samples were `.tmp4ZuIgZ`, `.tmpAuaYaX`, `.tmpoHkkFC`, `tmp.0U3szJBBtt`, `tmp.3aai8pu0Nc`, `tmp.4O3L64zIWR`, `tmp.8chtt9lorH`, `tmp.D10e0JmYx9`. None is attributed to thirdshift. Installed tools elsewhere (including home directories), alternate `TMPDIR` roots, unrecognized scratch names and retained Worktrees are outside this inventory; this is not a host-wide disk-usage total.

## Historical comparison

The October 16 handoff, as summarized in the October 17 [Agent Brief](https://github.com/JacobStephens2/thirdshift/issues/569#issuecomment-6228273472), separated Factory-skill extraction, agent scratch/tooling and test fixtures. The current inventory keeps those distinctions, but only matching names can be counted without attribution. The original handoff's numeric sizes/counts were not supplied with the issue and were not located in the bounded local document/session search. After this report's initial commit, the concurrent investigation supplied a transcription from `/tmp/thirdshift-triage-handoff-2026-10-16/presort-assessments.json`: 1,904 extraction directories ("828 MB"), about 508 scratch/tooling entries ("2.3 GB"), 84 `.tmp*` entries ("177 MB") and 23 `fakes-*` entries ("134 MB"). Those original handoff files are absent in this session's `/tmp`; these figures are **historically observed according to the concurrent report, not independently verified here**. Its recorded units/bounds are incomplete. The concurrent report compares its own view to those figures; this report cannot calculate a comparable growth/reduction from this session's uncorrelated view. None of the old measurements are presented as current.

Two independently recorded **historical observations** are available: [#736](https://github.com/JacobStephens2/thirdshift/issues/736) reports 24 setup PTY children idle for roughly 10–16 days, and [#737](https://github.com/JacobStephens2/thirdshift/issues/737) reports two release-script processes polling fake `gh` for roughly 17 days. Both were ended at the maintainer's direction on October 16. This investigation neither inspected those original processes nor reproduced their lifetime failures. These are process counts, not temporary-directory counts or proof of which entries remain today.

## Ownership, likely causes and independent remedies

### Factory-skill extraction

**Source-inspected, high confidence in ownership/lifetime:** [`src/skills.rs`](../../src/skills.rs) stores one extracted `TempDir` in the process-global `WRITTEN` static (lines 27–29). `written_out` creates `thirdshift-skills-*` once and reuses it (63–77); `link_into` points Worktree links at it (36–46). `remove_written` explicitly takes/drops that owner (50–59), and [`Started::finish`](../../src/command.rs) calls it (144–170). Ordinary Run/Architect/Pickup/Security entrypoints route through finishing in [`src/main.rs`](../../src/main.rs). This is intended Command retention until sessions finish, not an extraction per Session. Child Runs spawn another binary process ([`src/child_run.rs`](../../src/child_run.rs), 339–357, 382–404), so each child has its own cache.

**Hypothetical cause, medium confidence; not reproduced:** direct skill-link unit tests ([`src/skills/tests.rs`](../../src/skills/tests.rs), 82–153) and Session execution helpers ([`src/session/execution_tests.rs`](../../src/session/execution_tests.rs), 93–122) populate that static without Command finishing. Their local Worktree `TempDir`s do not own the extraction. A normal test-process exit without dropping the static owner is a plausible repeated extraction leak: at most one cached extraction per process, not one per test. Abrupt process death before finishing is a separate hypothesis. Neither explains any particular one of the 756 names without provenance. `remove_written` also has no result with which to report a failed removal; this is source-inspected, not evidence that removal actually failed here.

**Minimal proposed remedy:** give direct-test extraction an explicit owner released after all its users exit, or inject a test-owned extraction root. Do not call `remove_written` independently from parallel tests while others need the links. Consider observable cleanup errors separately from ownership changes. A crash-reclamation policy requires a new maintainer decision about durable ownership/liveness; a prefix sweep is not justified.

**Bounded future validation:** under a new isolated temporary parent, run one existing direct-link test in its own test executable, wait for complete exit and assert only that parent's recorded extraction root is gone; repeat once to test per-process accumulation. Give that owned child a deadline and reap it on timeout. Separately verify successful/failed ordinary CLI finishing removes its own extraction. These checks are proposed, not executed, and do not touch historical entries.

### Agent scratch and tooling

**Source-inspected policy, high confidence; present-entry attribution unknown:** [`README.md`](../../README.md), “What sessions leave behind” (124–129), explicitly says agent-created files outside factory-owned cleanup remain by design. It records Playwright in `/tmp/pw`, a JDK under `~/.local/jdk/` and PR drafts as examples. [#23](https://github.com/JacobStephens2/thirdshift/issues/23) is a closed historical proposal, not authorization for a per-Run `TMPDIR` change. The inventory's named scratch/tooling candidates are consistent with that category but do not freshly reproduce those examples.

**Minimal proposed remedy:** first choose whether to change scratch retention at all. Existing advice to preinstall common toolchains may reduce repeated downloads; it is not a measured fix here. Any managed scratch directory, prompt convention, automatic tool/cache expiry or host retention change needs a **new maintainer decision**, separate from this diagnosis and from closed #23. Do not automatically delete reusable installations or place disposable scratch where Failed run salvage might commit it.

**Bounded future validation if authorized:** a fake Session writes inside its assigned owned root and leaves an outside sentinel. Verify only that root is removed after successful, interrupted and Failed runs, with all owned child processes finished first and unpublished Worktree work retained. This would test a new retention contract, not prove the history of today's files.

### Test fixtures and child-process lifetimes

**Source-inspected, high confidence in normal ownership; historical causes uncertain:** [`src/test_support.rs`](../../src/test_support.rs), 11–23, returns a local scratch `TempDir`. Integration [`Scenario`](../../tests/support/mod.rs) owns its fixture (113–119), creates `home/bin/tmp/work` (384–392) and passes child `TMPDIR` into that fixture (799–815). Normal cleanup assertions concern this isolated subtree (1219–1230), not arbitrary host entries. Unit-test static extraction has a different owner.

[#736](https://github.com/JacobStephens2/thirdshift/issues/736) and [#737](https://github.com/JacobStephens2/thirdshift/issues/737) remain **related lifetime investigations**, not confirmed causes of all accumulation. Source inspection confirms `TerminalCommand::Drop` kills/waits for the immediate PTY child ([`tests/support/mod.rs`](../../tests/support/mod.rs), 201–224); ordinary assertion unwinding already has cleanup. Abrupt harness death is a distinct hypothesis. Release tests set `RELEASE_POLL_SECONDS=0` and wait without a test-level deadline ([`tests/release.rs`](../../tests/release.rs), 306, 468–474); [`scripts/release.sh`](../../scripts/release.sh) deliberately documents no overall production timeout. Changing production polling is a separate policy decision.

**Minimal proposed remedy:** let #736/#737 establish fixture-owned child deadlines and termination/reaping, keeping each change independent and production terminal/release behavior intact. If fixture cleanup errors are identified, make them observable locally. Do not kill processes by name or infer fixture ownership from a generic temp prefix.

**Bounded future validation:** separately test normal completion, assertion unwinding, timeout and abrupt fixture-harness death with one actual owned PTY child (#736) or fake-`gh` missing/pending response (#737). Capture the owned PID/process group and diagnostics; clean up only those created by that fixture. No live process inventory was needed or attempted here.

### Preservation is not a leak

**Source-inspected, high confidence:** Failed runs preserve unpublished work before disposal ([`src/delivery.rs`](../../src/delivery.rs), 278–309; [`src/worktree.rs`](../../src/worktree.rs), 179–192; [`src/worktree/ownership.rs`](../../src/worktree/ownership.rs), 102–167). Failed push, changed/uncertain ownership and failed disposal can intentionally retain a Worktree/local Issue branch; [`README.md`](../../README.md), 565–570, documents recovery. Do not count these as disposable test or agent scratch. Factory-skill removal in Command finishing is independent of Worktree retention.

No cleanup is proposed for active or unattributed entries, preserved Worktrees, Session/Command logs or recovery evidence. Future fixes must verify Failed run salvage failure still keeps unpublished work, and uncertain ownership still refuses disposal. This report establishes separate likely mechanisms and their limits; it does not establish a host-wide leak rate or that all accumulated entries are abandoned.

## Inventory command

Run from any directory on this Linux host; it prints JSON to stdout and creates no files. Limits apply to enumeration work, not a guarantee against an individual filesystem syscall stalling.

```sh
python3 - <<'PY'
import collections, datetime, itertools, json, os, re, stat, time

def utc():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()

started = utc()
deadline = time.monotonic() + 20
with os.scandir('/tmp') as entries:
    roots = sorted(itertools.islice(entries, 20001), key=lambda e: e.name)
truncated = len(roots) > 20000
roots = roots[:20000]
groups = collections.defaultdict(list)
for entry in roots:
    name = entry.name
    if re.fullmatch(r'thirdshift-skills-[A-Za-z0-9]{6}', name):
        group = 'factory_extraction_name'
    elif name.startswith('thirdshift'):
        group = 'other_thirdshift_name_unknown_owner'
    elif name in ('pw', 'vac5docs', 'vac5.log') or re.fullmatch(r'pr\d*-body\.md', name):
        group = 'historical_scratch_name_unknown_owner'
    elif name.startswith(('.tmp', 'tmp')):
        group = 'generic_temp_name_unknown_owner'
    else:
        group = 'outside_selected_patterns'
    groups[group].append(entry)

def sample(path):
    result = dict(entries=0, regular_bytes=0, allocated_bytes=0, errors=0, limited=False)
    def walk(path, depth, device=None):
        if time.monotonic() >= deadline or result['entries'] >= 10000 or depth > 16:
            result['limited'] = True
            return
        try:
            fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
            try:
                here = os.fstat(fd)
                if device is not None and here.st_dev != device:
                    result['limited'] = True
                    return
                with os.scandir(fd) as entries:
                    for entry in entries:
                        if time.monotonic() >= deadline or result['entries'] >= 10000:
                            result['limited'] = True
                            break
                        result['entries'] += 1
                        info = entry.stat(follow_symlinks=False)
                        if stat.S_ISREG(info.st_mode):
                            result['regular_bytes'] += info.st_size
                            result['allocated_bytes'] += info.st_blocks * 512
                        elif stat.S_ISDIR(info.st_mode):
                            walk(f'/proc/self/fd/{fd}/{entry.name}', depth + 1, here.st_dev)
            finally:
                os.close(fd)
        except OSError:
            result['errors'] += 1
    walk(path, 0)
    return result

output = dict(started_utc=started, top_level_count=len(roots), top_level_truncated=truncated)
for group, entries in sorted(groups.items()):
    result = dict(count=len(entries))
    if group != 'outside_selected_patterns':
        types = collections.Counter()
        regular_bytes = allocated_bytes = 0
        directories = []
        for entry in entries:
            try:
                info = entry.stat(follow_symlinks=False)
                if stat.S_ISDIR(info.st_mode):
                    types['directories'] += 1
                    directories.append(entry)
                elif stat.S_ISREG(info.st_mode):
                    types['regular_files'] += 1
                    regular_bytes += info.st_size
                    allocated_bytes += info.st_blocks * 512
                else:
                    types['links_or_other'] += 1
            except OSError:
                types['stat_errors'] += 1
        result.update(types=types, top_regular_bytes=regular_bytes,
                      top_allocated_bytes=allocated_bytes,
                      directory_samples={e.name: sample(e.path) for e in directories[:8]})
    output[group] = result
output['finished_utc'] = utc()
print(json.dumps(output, indent=2))
PY
```
