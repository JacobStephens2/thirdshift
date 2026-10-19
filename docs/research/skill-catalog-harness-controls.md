# Skill catalog controls in the installed Harnesses

Investigation for issue #735, 2026-10-17. This note records supported controls and limits; it does not propose or implement catalog isolation. Catalog presence, automatic selection, and harmful selection are different observations. No harmful skill selection has been established here. Actual fresh, continued, and reviewer catalog measurements belong in the companion investigation.

This source research belongs to the concurrent [catalog investigation](skill-catalog-resolution.md) retained from `6006f33`. The [other session's note](skill-catalogs.md) adds invocation-only no-turn Codex probes; its applied-probe claims are separate from the inspected-only controls described here.

The installed executables report **Codex CLI 0.163.0** and **Claude Code 2.1.302**. Findings below distinguish locally verified help, matching tagged upstream source, current documentation, and behavior that still needs a controlled measurement. No model turn, live thirdshift Run, authentication inspection, personal configuration edit, plugin change, trust change, or guard change was performed for this note.

## Existing Factory loading

[Factory skills](../../GLOSSARY.md) are the embedded `skills/` directories, written once per Command and linked into every Run worktree by [`src/skills.rs`](../../src/skills.rs). Claude uses `.claude/skills`; Codex uses `.agents/skills`. Every embedded directory is linked, rather than only the skill named by the current Session prompt. Ordinary repository skills are left present. Directory validation refuses symlinked project-skill parent components. Replacing an existing individual Factory entry removes only that entry; an existing real directory at that name causes a failure. These details follow [`CheckedDirectory`](../../src/skills/directory.rs), rather than an assumption that linking overrides everything.

[`SkillLoading`](../../src/harness/adapter.rs) turns the first `/thirdshift-…` into `$thirdshift-…` for Codex, leaves it as a slash command for Claude, and uses a textual skill-tool instruction for Muse and OpenCode. It does not select by an absolute embedded-file identity. The prefix avoids ordinary unprefixed skill collisions; it does not reserve that namespace against a personal, administrator, or plugin skill using the same name.

[`Codex::session`](../../src/harness/codex.rs) and `codex_args` reuse the same argument builder for fresh and resumed sessions; the latter adds `resume SESSION_ID`. Code review adds the fresh-subagents instruction. Security adds that instruction when requested and always adds its concurrency setting. [`Claude::session`](../../src/harness/claude.rs) similarly uses one argument builder and adds `--resume SESSION_ID`. There is no catalog-pruning flag in either ordinary argument builder. Release-summary invocations have intentionally narrower tools; they are a separate flow and are not evidence for a normal Run's catalog.

[ADR 0012](../adr/0012-factory-skills-linked-into-the-worktree-codex-unsandboxed.md) rejected a temporary `CODEX_HOME` because it loses user configuration and global instructions, breaks keyring authentication, and complicates Resume. [ADR 0013](../adr/0013-every-harness-runs-unattended-and-fully-trusted.md) records instruction fallbacks and unattended execution requirements. Any later proposal must preserve those obligations, the standing accountability instructions, Security workflows, and useful tools. This investigation does not change either ADR.

## Codex: supported controls

Local `codex exec --help` and `codex exec resume --help` both expose repeatable `-c/--config key=value` TOML overrides, plus `--strict-config`. The former explicitly overrides values otherwise loaded from user configuration. `--strict-config` reports unknown fields; an ordinary successful parse does not establish that an arbitrary configuration key does anything.

The [official build-skills guide](https://learn.chatgpt.com/docs/build-skills#enable-or-disable-local-codex-skills) documents `skills.config` with a path and `enabled=false`. It also documents automatic discovery across repository, user, administrator, and system locations, duplicate names remaining separate, and `agents/openai.yaml`'s `allow_implicit_invocation=false` policy. The last control preserves explicit invocation but requires changing the skill package; it is not a Run-scoped CLI selector.

The [current configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference) documents `skills.config`, `skills.max_context_tokens`, and plugin enablement/MCP policies. The catalog budget defaults to 2% of model context and caps explicit values at 10,000 tokens. A small budget is a presentation limit, not a stable allowlist: descriptions may shorten and entries may disappear. Plugin enablement can affect more than a skill catalog.

The [exact 0.163.0 configuration schema](https://github.com/openai/codex/blob/rust-v0.163.0/codex-rs/core/config.schema.json) additionally declares `skills.config[].name`, `skills.include_instructions`, and `skills.bundled.enabled`. These are present in the tagged release, not inferred from unknown-key acceptance. The [live official schema](https://developers.openai.com/codex/config-schema.json) also contained them at investigation time; that mutable URL should not substitute for the pinned source in a future compatibility claim.

The [0.163.0 implementation of skill rules](https://github.com/openai/codex/blob/rust-v0.163.0/codex-rs/config/src/skills_config.rs#L99) establishes the semantics:

- Enablement rules are read from **User and SessionFlags layers only**. Putting one in project `.codex/config.toml` is not an equivalent control.
- A rule has exactly one nonempty name or one path selector. Both or neither are ignored with a warning.
- Names use exact equality, not wildcards. One name rule matches every discovered skill with that name.
- Path selectors canonicalize when possible. Use the `SKILL.md` document identity; a directory is not established as equivalent by this code.
- Rules apply in order; later matches can enable or disable an earlier match. Session rules can override user rules without changing the user file.

The [host service](https://github.com/openai/codex/blob/rust-v0.163.0/codex-rs/ext/skills/src/host_service.rs#L350) resolves these rules after merging loaded host skill roots and records disabled document paths. This is a narrower mechanism than disabling a plugin: this code does not change plugin enablement, MCP configuration, hooks, or authentication. Coverage of cloud-owned and executor-owned skills is **not established** by this host-path mechanism; the extension has distinct authorities for those sources.

The [extension](https://github.com/openai/codex/blob/rust-v0.163.0/codex-rs/ext/skills/src/extension.rs#L395) selects explicitly mentioned skills separately from the catalog-rendering branch guarded by `include_instructions`. Consequently, `skills.include_instructions=false` should not be described as disabling discovery or all explicit skill loading. Its effect across the installed execution modes still needs a prompt-input comparison. It is also broader than a targeted catalog adjustment and may hide required skill-use instructions.

[`skills.bundled.enabled`](https://github.com/openai/codex/blob/rust-v0.163.0/codex-rs/ext/skills/src/host_service.rs#L220) filters system-scope roots when false; it is not a user/plugin/repository allowlist. No evidence here justifies using it to remove required built-ins.

Two supported **argument fragments**, shown for exact syntax and not executed in this investigation, are:

```text
-c 'skills.config=[{name="example-unwanted-skill",enabled=false}]'
-c 'skills.config=[{path="/absolute/path/to/example/SKILL.md",enabled=false}]'
```

Any future controlled experiment should first select a harmless fixture and preserve required skills. Do not turn an inventory into an automatic denylist.

### Codex name resolution and continuation limits

The [host merge](https://github.com/openai/codex/blob/rust-v0.163.0/codex-rs/ext/skills/src/loader/host_merge.rs#L228) deduplicates document paths and sorts by scope, name, and native document path; it does not merge equal names. The [extension's explicit selector](https://github.com/openai/codex/blob/rust-v0.163.0/codex-rs/ext/skills/src/selection.rs#L21) accepts structured path mentions and recognized skill-path mentions in text, matching canonical/main/display identities. Its plain-name branch selects the first enabled matching catalog entry. These are source observations, **not a tested guarantee that a proposed prompt spelling reaches a particular embedded Factory file**. A deterministic-resolution change needs a fixture containing a deliberate same-prefix collision and an assertion about the injected body.

Local `codex debug prompt-input --help` promises rendering of the model-visible input list as JSON. It exposes no Resume/session-id argument. Therefore a fresh rendering, or a fresh rendering whose user text merely says “continue,” does not measure a real persisted Resume. Fresh subagent configuration inheritance and old skill content retained in conversation history also remain untested here. Reapplying an override on the next CLI launch does not prove removal of previously injected instructions.

## Claude: supported controls and collateral effects

Local 2.1.302 help verifies the following; these flags were **inspected, not applied**:

| Control | Verified meaning and relevance |
| --- | --- |
| `--settings FILE_OR_JSON` | Additional session settings. The [CLI reference](https://code.claude.com/docs/en/cli-reference#cli-flags) says supplied keys override file-based values while omitted keys remain inherited. |
| `--setting-sources user,project,local` | Chooses whole settings sources. This is too broad to assume it removes only skills. |
| `--plugin-dir PATH` / `--plugin-url URL` | Adds a plugin for that session. It does not say installed plugins become unavailable. |
| `--disable-slash-commands` | Disables all skills, including required Factory skills. |
| `--safe-mode` | Disables customizations including CLAUDE.md, skills, installed plugins, hooks, MCP, commands and agents. Incompatible with the preservation requirement. |
| `--bare` | Skips hooks, plugin sync, memory and CLAUDE.md autodiscovery, among other startup behavior; skills still resolve by slash name. It is not selective catalog isolation. |
| `--strict-mcp-config` | Restricts MCP server loading. This is a tools control, not a catalog selector. |
| `--system-prompt-snapshot on/off` | Help says `on` is default: where recording is enabled, the first request's system prompt is reused verbatim on Resume until compaction, even if a later launch passes different text. |

The [official skills documentation](https://code.claude.com/docs/en/skills#override-skill-visibility-from-settings) offers `skillOverrides` states `on`, `name-only`, `user-invocable-only`, and `off`; `off` hides the skill and blocks invocation. It explicitly excludes plugin skills. The same page documents enterprise > personal > project precedence for equal names, namespaced plugin skills, additional-directory loading, and nested skills discovered during file work. Thus a personal `thirdshift-implement` can shadow the linked project skill; the prefix alone does not prove Factory identity. Removing settings sources also changes discovery, including account-sync eligibility. These are documentation claims, not a local execution test.

This documented argument fragment is a candidate for a harmless fixture experiment, **not an applied change**:

```text
--settings '{"skillOverrides":{"example-unwanted-skill":"off"}}'
```

The [settings precedence documentation](https://code.claude.com/docs/en/settings#settings-precedence) puts managed settings above command-line settings. A Run-scoped overlay must not be presented as a way to override organizational obligations. Settings sources contain permissions, hooks, environment and plugin choices as well as skill controls. `CLAUDE_CONFIG_DIR` changes storage for settings, session history and plugins, so changing it is not a narrow skill-list operation.

The [plugin manifest reference](https://code.claude.com/docs/en/plugins-reference#standard-layout) lists skills alongside agents, hooks, MCP servers, LSP servers, workflows and executables. Disabling a plugin to suppress one skill can remove useful tools or behavior. No verified Claude control in this investigation selectively hides an arbitrary plugin-provided skill while preserving the rest of that plugin. This remains a gap, not justification to disable plugins.

Resume deserves separate evidence: applying a new `--settings` overlay and assuming the recorded catalog was rebuilt would overstate what help establishes. No new Claude session or real Resume was run. Reviewer subagents need their own inventory, invocation visibility, and retained-history measurement.

## Other Harnesses and safe reproduction

The checked-in adapters expose `.agents/skills` for agy, Grok, Muse and OpenCode; agy/Grok use slash invocation and Muse/OpenCode use the textual skill-tool instruction. Their fresh/Resume paths and environments differ. This note has not verified those installed CLIs, source-specific pruning, instruction retention, plugin/tool effects, or reviewer inheritance. Codex/Claude findings must not become a generic six-Harness claim. See [`src/harness/agy.rs`](../../src/harness/agy.rs), [`grok.rs`](../../src/harness/grok.rs), [`muse.rs`](../../src/harness/muse.rs), and [`opencode.rs`](../../src/harness/opencode.rs).

Exact read-only local probes used for the controls above (run separately):

```sh
codex --version
codex exec --help
codex exec resume --help
codex debug --help
codex debug prompt-input --help
codex features --help
claude --version
claude --help
```

Repository reads used `rg -n` for relevant terms followed by bounded `sed -n` ranges. Upstream source was fetched read-only from the `openai/codex` `rust-v0.163.0` tag; current official documentation was fetched on the date above. No auth command or full personal configuration dump was needed.

The evidence supports measuring current catalogs and, only if a concrete problem emerges, comparing narrow controls with harmless fixtures. It does **not** establish a need for isolation, harmful selection, safe plugin disabling, deterministic Factory resolution across all Harnesses, or successful continued/reviewer-session filtering. Those remain explicit questions for later evidence.
