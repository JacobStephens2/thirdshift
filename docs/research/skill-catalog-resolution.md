# Skill catalogs and Factory resolution (#735)

Investigation: 2026-10-17 UTC, checkout `bf918a774d7028998c1b61b7991c55c8a9027bff`.
Sources: [issue and Agent Brief](https://github.com/JacobStephens2/thirdshift/issues/735),
[loading contract #391](https://github.com/JacobStephens2/thirdshift/issues/391),
[ADR 0012](../adr/0012-factory-skills-linked-into-the-worktree-codex-unsandboxed.md).

## Recommendation

Keep session startup unchanged. This investigation establishes redundant catalog
rows and a bounded opportunity to reduce prompt size, but finds no demonstrated
harmful skill selection in the sampled evidence. A blanket plugin-off or skills-off
policy would remove capabilities without a measured correctness benefit. There is
no approved isolation contract in #735; this PR changes documentation only.

Preserve repository instructions, the operator's standing accountability guardrail,
Security requirements, and the complete Factory skill set. Preserve generic repository
skills too: this repository's coding standard explicitly calls for generic
`codebase-design`. Intentionally enabled plugins and tools stay available through
the normal Harness discovery and existing user configuration; none is disabled,
reinstalled or moved by this work. A future restriction needs an explicit capability
inventory and a separate startup contract, including continuations and reviewers.

## What was actually measured

The [captured baseline](skill-catalog-baseline-2026-10-17.json) contains only skill
catalog text and selected session metadata from four local Codex rollouts. Each
`<skills_instructions>` block is a developer message actually supplied to a session,
not an inventory of directories assumed to be loaded. Catalogs are stored once per
SHA-256; samples identify the source rollout and its one-based JSONL line.
No complete transcript, credentials, personal config or tool results are published.

| Context | Session evidence | CLI | Entries / distinct names | UTF-8 bytes | Proxy tokens |
| --- | --- | --- | ---: | ---: | ---: |
| Fresh Run, #735 | Oct 17, 17:31:13; `01a14aea-f144-7192-9cba-46cfea6f20f4` | 0.163.0 | 65 / 51 | 21,077 | 4,933 |
| Continuation, #189 | Oct 10, 03:46:41; `01a123eb-8b0a-7cc1-b339-0c976df25176` | 0.162.1 | 50 / 36 | 15,596 | 3,667 |
| Standards reviewer, #734 | Oct 17, 16:45:13; `01a14ac0-d1d4-7b61-9398-182841faf2a1` | 0.163.0 | 65 / 51 | 21,077 | 4,933 |
| Spec reviewer, #734 | Oct 17, 16:45:23; `01a14ac0-fc78-76f3-a172-f6b3b193a77f` | 0.163.0 | 65 / 51 | 21,077 | 4,933 |

All four catalogs occur at line 3. The reviewer metadata records distinct
`thread_spawn` sources with `/root/standards` and `/root/spec` paths and the same
parent thread. These are real reviewer contexts from the preceding Run, not
synthetic lists or claims about these PR reviewers. The Continuation's user prompt
at line 10 explicitly says: “You are continuing work on branch issue-189”. It is
a new Harness session continuing an existing Issue branch, **not a Resume** of a
killed turn. The older sample is the available continuation baseline, not a
measurement of 0.163.0 continuation behavior. The counts cannot isolate effects of
session kind, CLI version, time or operator configuration.

Search was bounded to at most 400 most recent retained rollout headers (first 19
lines per file; 194 files existed when enumerated). A separate search of at most
120 Oct 17 and 120 Oct 16 rollouts, each at most 20 MB, found no multi-turn exec
session suitable as a Resume sample. No fresh production Run or scheduled work
was launched to fill that gap. No actual Claude/agy/Grok/Muse/OpenCode model-visible
catalog was measured. These are four purposive observations, not a population
estimate or an experiment demonstrating latency, accuracy or cost effects.

Bytes include the complete skill instruction wrapper, root aliases, names,
descriptions, paths and use instructions, ending at `</skills_instructions>`.
They exclude separately supplied tool schemas, recommended-plugin lists,
repository instructions, explicit skill-body expansion and subsequent skill reads.
Tokens use `tiktoken==0.12.0`, `o200k_base` ([tokenizer source](https://github.com/openai/tiktoken)).
This is a reproducible proxy encoding, **not** the actual GPT-6 tokenizer,
billed input tokens or cache-adjusted savings. Catalog-specific server token
attribution is unavailable; total session usage cannot provide it.

## Loaded names and duplication

Every loaded name, description and path appears in the baseline's `text_lines`.
In the current fresh/reviewer catalogs the origins are:

| Origin | Entries |
| --- | ---: |
| Repository `.agents/skills` (`r7`) | 26: 16 generic, 10 Factory |
| User `.agents/skills` (`r1`) | 20 |
| Codex bundled system skills (`r2`) | 4 |
| User `.codex/skills` (`r0`) | 1 |
| Plugin skills (`r3`–`r6`, including two under `r5`) | 14 |

The historical Continuation has 25 repository entries, 20 user entries, four system
entries and one `.codex` entry; no plugin skill rows are present. That is a catalog
observation, not evidence that plugins were deliberately disabled. The historical
report's “44 skills / 16 KB” is not a current count and was not independently
reproduced.

Fourteen exact names occur twice, once in the repository and once in user skills,
in **all four** samples:

`code-review`, `codebase-design`, `diagnosing-bugs`, `domain-modeling`,
`git-guardrails-claude-code`, `grilling`, `migrate-to-shoehorn`, `prototype`,
`research`, `scaffold-exercises`, `setup-pre-commit`, `tdd`, `wizard`,
`writing-for-agents`.

Twelve pairs have identical catalog descriptions; `code-review` and
`domain-modeling` have different descriptions. Matching descriptions do not prove
identical skill bodies. Six generic/Factory pairs also coexist: `code-review`,
`codebase-design`, `pr`, `resolving-merge-conflicts`, `security-audit`, and `tdd`,
each alongside its `thirdshift-` counterpart. Those are **different names**, not
exact-name collisions. All ten Factory names are unique in these catalogs.

As a counterfactual only, deleting the second row for each exact duplicate name
while leaving the rest of the catalog unchanged removes 3,782 bytes and reduces
the proxy token count by 895: 4,933 → 4,038 in the current samples (18.1%), and
3,667 → 2,772 in the historical Continuation. This retains repository rows by
catalog order; it neither establishes the Harness's chosen precedence nor justifies
deleting the user's skills. It is not the measured effect of any CLI setting or
a measured benefit from disabling plugins. No such setting was applied.

## Observed selection and deterministic Factory resolution

The fresh #735 session expanded `thirdshift-implement` at rollout line 12 from
`/tmp/thirdshift-skills-0tW5Zn/thirdshift-implement/SKILL.md`. Its first command
at line 15 read that file and `.agents/skills/thirdshift-{tdd,code-review,pr}/SKILL.md`.
The continued #189 session expanded the same namespaced implement skill at line 11
from `/tmp/thirdshift-skills-pCWykD/thirdshift-implement/SKILL.md`; its first command
at line 14 read it and the same three namespaced repository paths. The recorded
command results completed successfully. These directly exercise the requested
Factory-name resolution; they show no replacement by generic `implement`, `tdd`,
`code-review` or `pr` in those opening reads.

The Standards reviewer #734 read `.agents/skills/codebase-design/SKILL.md` in
its line-18 command, as `CODING_STANDARDS.md` requires. The Spec reviewer's opening
commands (lines 12 and 17) read domain guidance and the diff without loading a
generic review workflow. Neither reviewer was asked to invoke a top-level Factory
skill of its own. A generic standards-reference read is not evidence of an
incorrect selection. Observations here cover these specific opening expansions
and commands only; no automated classifier or exhaustive later-read audit was
performed. **Zero harmful selections demonstrated in this sample** does not prove
that collisions can never matter.

The current implementation follows this path:

```text
Sessions::within
  skills::link_into(worktree, Harness adapter)
    extract the embedded skills once per Command
    link thirdshift-* into the adapter's project skill directory
  Harness session prompt
    invoke thirdshift-implement (or another named Factory skill)
    refer to thirdshift-code-review / thirdshift-pr explicitly
```

Sources: [`src/session.rs`](../../src/session.rs),
[`src/skills.rs`](../../src/skills.rs),
[`src/harness/adapter.rs`](../../src/harness/adapter.rs),
[`src/harness/codex.rs`](../../src/harness/codex.rs),
[`src/harness/claude.rs`](../../src/harness/claude.rs),
[`src/prompt.rs`](../../src/prompt.rs). `SkillLoading::Dollar` converts the leading
Factory slash invocation to `$thirdshift-*` for Codex; Claude keeps the slash form.
Generic names are deliberately not used for these Factory invocations.

A read-only check of this Worktree found all ten `.agents/skills/thirdshift-*`
directories symlinked into `/tmp/thirdshift-skills-0tW5Zn/`. Seven linked `SKILL.md`
bodies match this checkout's `skills/`; three differ: `thirdshift-code-review`,
`thirdshift-tdd` and `thirdshift-improve-codebase-architecture`. The checkout adds
bounded-reading/generated-output review guidance missing from this Command's
embedded bundle. This is version provenance, not wrong-name resolution: skills are
embedded at binary build time, as [ADR 0001](../adr/0001-rust-binary-with-embedded-skills.md)
states. Updating a checkout does not update an already extracted Command bundle.
No Factory link was changed for this investigation.

The prefix makes resolution unambiguous **in these catalogs**, not against an
arbitrary user/plugin skill that also calls itself `thirdshift-*`. Factory linking
replaces a same-named project file or symlink, and refuses an existing real
directory at that name; unrelated project skills are preserved.
Harness precedence for a competing same-name skill remains relevant. See the
[verified Harness controls and limitations](skill-catalog-harness-controls.md)
before proposing any filter or assuming that a project config enforces one.

## Compatibility and bounded validation

No isolation is implemented, so fresh Runs, Continuations, Resumes and reviewer
sessions keep the existing discovery, tool access, auth, config and instruction
paths. The Factory namespace and skill references stay intact. Replacing
`CODEX_HOME` would relocate instructions/auth/state and require the same home for
Resume; ADR 0012 rejects that approach. Disabling all skills would also disable
required Factory workflows. Disabling plugin packages could remove tools/hooks,
not just catalog prose. Filtering descriptions without checking explicit invocation
and reviewer discovery would be insufficient validation.

If a later issue authorizes a restriction, compare one disposable fresh session,
one Continuation, one same-thread Resume and both fresh-context reviewers using
the same Harness version and intentionally enabled capabilities. Record their
actual prompt catalogs, named Factory loads and a tool availability check; retain
repository/accountability/Security instructions and test required generic skills.
Use owned fixtures and explicit task IDs with deadlines, never scheduled work or
global configuration changes. This is proposed future validation, not a run made
here and not evidence of current support on other Harnesses.

## Recomputing the captured measurements

The snapshot is sufficient to recompute every byte/count/token metric without
access to personal sessions. In an isolated Python environment with
`tiktoken==0.12.0`, run from the repository root:

```sh
python3 - <<'PY'
import collections, hashlib, json, pathlib, re
import tiktoken
data = json.loads(pathlib.Path(
    'docs/research/skill-catalog-baseline-2026-10-17.json').read_text())
encoding = tiktoken.get_encoding('o200k_base')
row = re.compile(r'^- ([^\n]+?): ([^\n]+?) \(file: ([^)\n]+)\)$', re.M)
for digest, catalog in data['catalogs'].items():
    text = ''.join(catalog['text_lines'])
    assert hashlib.sha256(text.encode()).hexdigest() == digest
    assert len(text.encode()) == catalog['utf8_bytes']
    assert len(encoding.encode(text)) == catalog['proxy_tokens']
    rows = list(row.finditer(text))
    names = collections.Counter(match[1] for match in rows)
    assert len(rows) == catalog['entry_count']
    assert len(names) == catalog['distinct_names']
    assert {n: c for n, c in names.items() if c > 1} == catalog['duplicates']
    seen, extras = set(), []
    for match in rows:
        if match[1] in seen:
            extras.append(match.group(0) + '\n')
        seen.add(match[1])
    reduced = text
    for extra in extras:
        reduced = reduced.replace(extra, '', 1)
    assert len(''.join(extras).encode()) == catalog['extra_rows_utf8_bytes']
    assert len(encoding.encode(reduced)) == catalog['proxy_tokens_after_first_name_only']
    print(digest, len(rows), len(names), len(text.encode()),
          len(encoding.encode(text)), len(encoding.encode(reduced)))
for sample in data['samples']:
    assert sample['catalog_sha256'] in data['catalogs']
print('all captured metrics verified')
PY
```

For a future capture, read only `response_item` records with role `developer` and
text starting `<skills_instructions>` from the selected owned rollout; save the
whole block before normalization, its SHA-256 and line number, and the session's
CLI version/source metadata. Expand aliases from that block, not a later config.
Keep session kinds and dates distinct rather than treating an on-disk skill list
or a debug rendering as proof of a model session's input.
