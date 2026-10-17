# Skill catalogs and Factory skill resolution (#735)

Investigated 2026-10-17 UTC at `bf918a774d7028998c1b61b7991c55c8a9027bff`, for [#735](https://github.com/JacobStephens2/thirdshift/issues/735), including its Agent Brief. Installed Harnesses: `codex-cli 0.163.0` and `Claude Code 2.1.302` (`codex --version`, `claude --version`).

**Recommendation: keep session startup unchanged.** Duplicate descriptions have measurable overhead, but these samples demonstrate no incorrect Factory skill selection. A narrow Codex catalog probe saved 908 proxy tokens; it establishes neither lower Run cost/latency nor compatibility with a Harness Resume. Blanket plugin isolation is not justified. A future opt-in duplicate suppression contract could use exact skill paths while retaining tools and instructions; it needs the validation described below before implementation.

## Evidence and measurement limits

The [captured baseline](skill-catalog-baseline-2026-10-17.json) records every loaded name grouped by its actual catalog root, duplicate multiplicities, timestamps, CLI versions, source rollout identifiers, byte counts and catalog SHA-256 hashes. These are extracted **model-visible `<skills_instructions>` blocks**, not a filesystem inventory. Personal transcripts, credentials and unrelated tool outputs are not copied into the repository.

| Context | Observed session | Entries / distinct names | Catalog UTF-8 bytes | Proxy tokens |
| --- | --- | ---: | ---: | ---: |
| Fresh Run | #735 opening session, 2026-10-17 13:31:13 UTC | 47 / 32 | 13,277 | 3,159 |
| Continuation | #671 opening session, 2026-10-17 02:01:00 UTC | 59 / 44 | 17,728 | 4,205 |
| Reviewer | #734 Standards sub-agent `/root/standards`, 2026-10-17 12:47:58 UTC | 61 / 46 | 18,345 | 4,339 |

The Continuation is a new session working on an existing Issue branch, identified by its actual opening prompt, `You are continuing work on branch issue-671`. It is **not** `codex exec resume`. The reviewer is an actual independent code-review sub-agent, not a fabricated reviewer prompt. Samples were selected from recent local thirdshift rollouts; they are three observations, not a controlled comparison of session types. The rollout filenames use local dates; the table uses their metadata's UTC timestamps. Source identifiers and complete name inventories are in the baseline.

Token counts use `tiktoken==0.12.0`, `o200k_base`, over each complete catalog block, including its wrapper, roots, descriptions and paths. These are reproducible **proxy tokenizer counts**, not the Harness model's verified tokenizer, billed input, cache misses, tool schemas, full system context, or cumulative Run cost. Byte counts are exact UTF-8 sizes; character counts differ because descriptions contain Unicode. No latency or causal selection experiment was performed. The historical “44 skills / 16 KB” is not a current universal count.

All samples have 15 exact duplicate names, each occurring twice, once from the repository and once from the Host:

```text
code-review, codebase-design, diagnosing-bugs, domain-modeling,
git-guardrails-claude-code, grilling, migrate-to-shoehorn, pr,
prototype, research, scaffold-exercises, setup-pre-commit, tdd,
wizard, writing-for-agents
```

For this checkout, all 15 Host/repository `SKILL.md` pairs were byte-identical when read on 2026-10-17. That is a present-file comparison, not proof that archived versions or supporting resources were identical. Keeping one repeated catalog line per name would remove 3,771 bytes / 908 proxy tokens in all three samples. Names such as `tdd` and `thirdshift-tdd` are **different names**, with different workflows; they are not exact-name collisions and must not be deduplicated as equivalents.

The inventories differ beyond the repeated names. The Fresh sample has four system skills, 26 repository entries (10 Factory plus 16 generic), and 17 Host entries. The Continuation has four system, 25 repository, 16 Host and 14 plugin entries. The reviewer has four system, 26 repository, 17 Host and 14 plugin entries. Plugin names include Google Drive, Pages, plugin management, Sites and Work Pets; their complete names are recorded in the baseline. The Fresh catalog has no plugin skill entries; this does **not** show that plugin tools are unavailable. Skills and tool catalogs are separate. These observations do not establish why plugin skill visibility differed.

## Observed selection and deterministic Factory resolution

In both the Fresh and Continuation samples, the initial tool call read the extracted `thirdshift-implement/SKILL.md`, then the worktree's `thirdshift-code-review`, `thirdshift-tdd` and `thirdshift-pr` files. This #735 session also received the resolved Factory implement content as a skill attachment. The #734 Standards reviewer read generic `codebase-design`, as directed by `CODING_STANDARDS.md`. That is appropriate repository guidance, not evidence of selecting generic `code-review` in place of Factory review. A later reference to generic `code-review/SKILL.md` in the #671 transcript occurs inside a report-writing command; counting any path mention as a skill read would produce a false positive.

Selection evidence here is the explicit attachment and startup read commands, not a complete semantic audit of all decisions. No demonstrated wrong-name or wrong-Factory selection was found in these bounded samples. They cannot establish that harmful selection never happens or that every catalog entry was useful. The retrospective's “16 generic reads, no visible harm” remains historical, attributed evidence rather than a freshly reproduced defect.

The current resolution contract is already explicit:

1. [`skills/`](../../skills/) embeds ten Factory skills with matching `thirdshift-*` directory and frontmatter names. [`src/skills.rs`](../../src/skills.rs) extracts them once per Command and links each named entry into the Harness's worktree skill directory. Existing Factory entries are replaced; ordinary repository skills remain. These links point to the running binary's embedded version, which need not equal an edited checkout until rebuilt.
2. [`src/session.rs`](../../src/session.rs), `Sessions::within`, links before starting the Harness. Codex uses `.agents/skills`; Claude uses `.claude/skills` ([Codex adapter](../../src/harness/codex.rs), [Claude adapter](../../src/harness/claude.rs)). The shared Git exclude prevents these links entering commits. [`src/skills/directory.rs`](../../src/skills/directory.rs) rejects unsafe directory traversal rather than silently loading skills from a redirected directory.
3. Fresh and Continuation prompts both start with `/thirdshift-implement` and explicitly name `thirdshift-code-review` and `thirdshift-pr` ([`src/prompt.rs`](../../src/prompt.rs)). [`SkillLoading::prompt`](../../src/harness/adapter.rs) translates the leading invocation to `$thirdshift-implement` for Codex and retains `/thirdshift-implement` for Claude. Factory skill references remain namespaced plain prose.
4. [#391](https://github.com/JacobStephens2/thirdshift/issues/391) and [ADR 0012](../adr/0012-factory-skills-linked-into-the-worktree-codex-unsandboxed.md) reserve those names rather than renaming or excluding generic repository skills. This makes intended **Factory** resolution deterministic through an explicit namespace and replaced worktree link; it is not a general promise about Harness precedence between arbitrary duplicate generic names. A Host skill maliciously reusing a `thirdshift-*` name is not represented in these measurements.

Existing tests exercise linking repeatedly for every adapter while retaining a repository entry, plus the Continuation prompt/branch flow. They do not measure a real model's implicit skill choice. No production Rust, embedded skills, generated prompts, instruction files or startup flags change in this investigation.

## Supported Run-scoped configuration

Official docs and schemas below were fetched on 2026-10-17; they are live, unversioned sources. Installed CLI help verifies the invocation interface. Neither current docs nor a key appearing in a binary proves every runtime behavior on that installed version.

| Harness | Narrow available interface | Scope and proof |
| --- | --- | --- |
| Codex 0.163.0 | `-c 'skills.config=[{path="/absolute/skill/SKILL.md",enabled=false}]'` | Invocation-only TOML override; accepted by `exec`, `exec resume` and `debug prompt-input`. Actual path suppression was verified below without a model turn. |
| Codex | `plugins."plugin@marketplace".enabled=false` | Documented for identified local-marketplace plugins, not a universal plugin-off switch or override of workspace activation. Disables the package, potentially tools/hooks too; not exercised here. |
| Claude 2.1.302 | `--settings '{"skillOverrides":{"some-skill":"off"}}'` | Official standalone personal/project skill control. Plugin skills are explicitly excluded. Help verifies `--settings`; effective catalog suppression was not exercised. |
| Claude | `--settings '{"enabledPlugins":{"plugin@marketplace":false}}'` | Identified whole-plugin override; not skills-only. Managed settings can prevail. Not exercised here. |

Sources: OpenAI [skills](https://developers.openai.com/codex/skills), [config basics](https://developers.openai.com/codex/config-basic), [configuration reference](https://developers.openai.com/codex/config-reference), [official schema](https://developers.openai.com/codex/config-schema.json), [CLI reference](https://developers.openai.com/codex/cli/reference), and [plugin enablement](https://developers.openai.com/plugins/build/plugins); Claude [skill visibility overrides](https://code.claude.com/docs/en/skills#override-skill-visibility-from-settings), [settings precedence](https://code.claude.com/docs/en/settings#settings-precedence), [enabledPlugins](https://code.claude.com/docs/en/settings-reference#enabledplugins) and [CLI reference](https://code.claude.com/docs/en/cli-reference).

The Codex schema includes exact `name` selectors in addition to paths; no wildcard/all-except semantics were established. Paths are preferable for a same-name duplicate because a name selector cannot express which copy to retain. Prose docs and the config reference differ about file versus directory paths; the installed no-turn probe accepts a `SKILL.md` file path. Do not replace an operator's existing `skills.config` array without defining how to preserve existing entries/disables. The probe only supplies an invocation override and deliberately makes no claim to merge arbitrary arrays correctly.

Claude's `--plugin-dir` adds session-only plugins; it does not exclude installed plugins. `--disable-slash-commands` disables all skills, including Factory skills. `--setting-sources` can exclude hooks, permissions and settings along with skill discovery. Broad `--bare`, `--safe-mode` and `--restricted` modes remove required parts of the operating context according to installed help. They are not suitable catalog-only controls. No blanket plugin-off option is proposed.

For either Harness, whole-plugin deactivation can remove MCP/tools/hooks intentionally available to a Run. Standalone skill suppression and plugin/tool suppression must not be treated as equivalent. With startup unchanged, intentionally enabled tools continue through the Harness's existing configuration; no new tool allowlist is introduced. In particular, a future catalog optimization must retain repository instructions, global accountability instructions and the accountability hook, Security skills/tools, authentication, and capabilities requested by the issue.

### Bounded Codex probe actually run

Run from this Issue branch's worktree, using its existing home/config and fallback instruction setting:

```sh
codex debug prompt-input \
  -c 'project_doc_fallback_filenames=["CLAUDE.md"]' \
  'Catalog inspection only; do not start work.' > /tmp/issue-735-evidence/prompt-input.json

codex debug prompt-input \
  -c 'project_doc_fallback_filenames=["CLAUDE.md"]' \
  -c 'skills.config=[{path="/home/jacob/.agents/skills/code-review/SKILL.md",enabled=false}]' \
  'Catalog inspection only.' > /tmp/issue-735-evidence/one-disabled.json
```

The second run emitted 46 entries / 12,812 bytes and retained the repository `code-review` entry. Both outputs retained the accountability instructions. A further identical command, with 15 entries in `skills.config` (one Host path for each duplicate listed above), emitted **32 entries / 32 distinct names / 9,506 bytes / 2,251 proxy tokens**, retaining all ten Factory names and accountability instructions. A before/after byte hash of personal `config.toml` matched. This tests model-visible catalog filtering, not explicit invocation, hook execution, plugin tools or resumed context. No agent/model work was launched by these probes.

No scheduled thirdshift Command, Pickup, production Run or configuration write was used to gather evidence. The #735 implementation/research and its code-review sessions are the authorized work, separate from diagnostic probes. A pinned tokenizer was installed only in `/tmp/issue-735-evidence/tokenizer`; archived transcripts were read in place. Required global instructions and accountability guardrails were never edited. The baseline JSON and this note are the only committed changes.

## Compatibility and next decision

The measured maximum benefit of suppressing the 15 repeated lines is 908 proxy catalog tokens per render (28.7% of the Fresh catalog, 21.6% of the Continuation catalog, 20.9% of the reviewer catalog). It is not 908 tokens of proven billed savings on every request. There is no measured quality, latency or total-cost improvement to justify changing the loading contract now. Prefixing Factory names already addresses the demonstrated ambiguity between generic and Factory workflows.

Keep the existing home and settings sources. ADR 0012 rejects a temporary `CODEX_HOME` because it can lose config/global instructions/auth and requires a stable home for Resume. [ADR 0013](../adr/0013-every-harness-runs-unattended-and-fully-trusted.md) requires each Harness's unattended flags on every session and Resume. Catalog trimming is not grounds to change either decision or bypass hook trust.

Any future opt-in suppression must reconstruct the same selection on fresh, continued, reviewer, Repair and Harness Resume launches. A Continuation starts a new session and is not sufficient Resume coverage. `codex exec resume --help` accepts overrides, but this investigation did not resume a conversation to verify history/catalog replacement. Claude's installed `--help` says its default system-prompt snapshot is reused on Resume until compaction; even a supported setting may leave old catalog text in conversation history. Exact-name selection, plugin namespaces, existing-disable merging, model invocation and sub-agent catalog inheritance need their own proof.

If a later issue establishes a benefit worth a startup change, validate in an owned temporary repository with synthetic Host/repository duplicate fixtures and a fixed capability manifest. Bound each child to a recorded ID and timeout, retain the normal instruction/hook sources, and never launch scheduled production work. Check a fresh session, a Continuation, an independent reviewer and a Harness Resume separately: the exact Factory skill is loaded, intended generic repository skills still work, explicit skill invocation remains supported, existing operator disables remain effective, and required tools/auth/Security/accountability hooks still work. Record catalogs, versions and token estimates before/after. This is proposed validation, not an already implemented or approved isolation contract.

## Reproducing the catalog measurements

The JSON baseline is a portable name/count snapshot; the private source rollouts are not prerequisites to reading it. On the evidence host, the following bounded extraction reproduces the table from those exact files. It reads only the first developer catalog and metadata from each selected rollout. It never prints transcript text or credentials. To measure another host, first capture `codex debug prompt-input` as above and extract its developer catalog with the same counting rule; do not substitute on-disk skill counts.

```sh
/tmp/issue-735-evidence/tokenizer/bin/python - <<'PY'
import collections, hashlib, json, pathlib, re, tiktoken

baseline = json.loads(pathlib.Path(
    'docs/research/skill-catalog-baseline-2026-10-17.json').read_text())
enc = tiktoken.get_encoding('o200k_base')
home = pathlib.Path('/home/jacob/.codex/sessions')
for sample in baseline['samples']:
    with (home / sample['rollout']).open() as source:
        for index, line in enumerate(source):
            if index >= 32:
                raise RuntimeError('No initial catalog within 32 records')
            record = json.loads(line)
            payload = record.get('payload', {})
            if record['type'] != 'response_item' or payload.get('role') != 'developer':
                continue
            blocks = [c.get('text', '') for c in payload.get('content', [])
                      if '<skills_instructions>' in c.get('text', '')]
            if blocks:
                catalog = blocks[0]
                break
        else:
            raise RuntimeError('No initial catalog')
    names = re.findall(r'^- ([\w:-]+):', catalog, re.M)
    counts = collections.Counter(names)
    observed = (len(names), len(counts), len(catalog.encode('utf-8')),
                len(enc.encode(catalog)), hashlib.sha256(catalog.encode()).hexdigest())
    expected = tuple(sample[k] for k in ('entries', 'distinct_names',
        'catalog_utf8_bytes', 'o200k_base_tokens', 'catalog_sha256'))
    assert observed == expected, (sample['context'], observed, expected)
    assert {n: c for n, c in counts.items() if c > 1} == sample['duplicate_names']
    print(sample['context'], *observed)
PY
```

`tiktoken==0.12.0` and its encoding data are required for proxy counts. Install them in an owned temporary virtual environment if reproducing elsewhere; no personal configuration changes are necessary. SHA-256 covers the catalog block's exact text, excluding its JSON transport escapes. Catalogs can vary with CLI, working directory, installed skills/plugins and inherited context; do not treat this snapshot as a permanent expected catalog.
