# Grilling: #428, more Harnesses

Issue: https://github.com/JacobStephens2/thirdshift/issues/428
Research: `docs/research/harness-candidates.md`, `docs/research/model-strength-for-review.md` §0

## Round 1 (answered)

**Q1 - Scope of #428.** Agreed in principle to pursue agy, Grok Build, Muse Code and MiMo-V2.6-Pro.
- *Answer:* Pursue all four. For MiMo, compare running it in opencode, which is set up on this server, with MiMo Code (`mimo`).
- *Status:* settled by Q15: MiMo-V2.6-Pro runs in OpenCode.

**Q2 - Deepen the Harness module first?** Gather everything one Harness does into one adapter (CLI name, args, resume, prompt sigil, skills dir, environment, stopping, Model/Effort check, stream parser) in a Ticket with no behavior change, ahead of the new Harnesses.
- *Answer:* yes.

**Q3 - Stopping: one rule for every Harness.** Before signalling, snapshot the session's descendant tree with `ps`, then signal every process group in it: the Harness's stop signals, the grace period, then SIGKILL. This applies to Claude and Codex too, and works on Linux and macOS.
- *Answer:* agree.

**Q4 - Grok's folder trust.** thirdshift sets `GROK_FOLDER_TRUST=0` in every Grok session, so no one has to run `grok --trust` once per repository.
- *Answer:* agree.

**Q5 - Instruction files for agy.** In an agy worktree with no `AGENTS.md` or `GEMINI.md`, link `GEMINI.md` → `CLAUDE.md` at the worktree root and add it to `.git/info/exclude`.
- *Answer:* agree. Plan for repositories to use `AGENTS.md`, but still handle the other cases. Keep any deliberate difference between `AGENTS.md` and `CLAUDE.md` (Jacob himself keeps only `AGENTS.md`).

**Q6 - Harness names.** The CLI names: `agy`, `grok`, `muse` and `mimo` (or whatever Q1 settles for MiMo).
- *Answer:* agree. Recorded in `CONTEXT.md` for agy, Grok and Muse.

**Q7 - Checking the Model on Muse.** Read Muse's cached catalog under `~/.local/share/muse/model-catalog/` when it exists; otherwise make a test call.
- *Answer:* agree. Use `muse-spark-1.3`, not `-contributor`.

## Round 2 (answered)

**Q8 - Which instruction file each Harness reads.** The rule:
- Claude reads `CLAUDE.md`.
- Every other Harness reads `AGENTS.md`, and reads `CLAUDE.md` only where there is no `AGENTS.md`.

Codex, Muse and agy (after Q5) already follow it. Two CLIs break it on their own:
- Grok loads both files when both exist.
- MiMo also loads `CLAUDE.md` when the `AGENTS.md` text is under 500 characters.

The options:
- (a) State the rule and document the two exceptions.
- (b) Hide `CLAUDE.md` from those Harnesses.

*Recommended:* (a). The exceptions matter only when a repository has both files with conflicting rules, and hiding a tracked file means fighting git.

*Answer:* I want Claude to read AGENTS.md if CLAUDE.md is not present. I agree with a.

*Checked:* headless Claude Code 2.1.291 already does this. In a repository with only `AGENTS.md`, `claude -p` returned the codeword from `AGENTS.md`. So the rule is: each Harness reads its own file and falls back to the other, and thirdshift adds the fallback only where a CLI lacks it (agy, and opencode if chosen).

**Q9 - Muse's final message and usage.** An Architecture review decides its outcome from the session's final message. Muse's `run.terminal.completed.text` runs every reply together, and its stream carries no usage. The options:
- (a) Build the final message from the stream's `run.output.delta` events, and show no usage.
- (b) Once the session exits, read Muse's session log (`~/.local/share/muse/sessions/<date>/<id>/session.jsonl`) for the last `model_completed` reply and the usage.

*Recommended:* (b), falling back to (a) when the log is missing or can't be parsed.
*Answer:* agree

**Q10 - Muse's default Model.** "No Model set" means the Harness's own default, and Muse's default is `muse-spark-1.3-contributor`. The options:
- (a) Setup proposes `muse-spark-1.3` and writes it to `[harness.muse]`.
- (b) thirdshift passes `muse-spark-1.3` whenever no Model is set.
- (c) Both, and the check also refuses `-contributor` unless it is named explicitly.

*Recommended:* (a), plus a line in the docs. The rule stays the same for every Harness.
*Answer:* agree

**Q11 - What Setup offers.** The options:
- (a) List only the installed Harnesses, with claude as the default when it is installed.
- (b) List all six and mark which are installed.

*Recommended:* (a). The command line and the User config still accept every name, and `NAMES` still lists all six.
*Answer:* agree

**Q12 - Each Harness's own environment.** Each adapter carries a fixed environment and flags, with nothing a user can configure. They apply to every session, every Resume and the Model check:
- Muse: `MUSE_NO_AUTO_UPDATE=1`
- Grok: `GROK_DISABLE_AUTOUPDATER=1` and `GROK_FOLDER_TRUST=0`
- MiMo: `MIMOCODE_DISABLE_CRON`
- agy: `AGY_CLI_DISABLE_AUTO_UPDATE=true` (found and tested; `=1` does **not** work)
- opencode, if chosen in Q15: `OPENCODE_DISABLE_AUTOUPDATE=1`

*Recommended:* yes, and documented. If agy has no off switch, accept that and document it.
*Answer:* agree

**Q13 - Spec shape.** One Spec, with these Tickets:
1. Deepen the Harness module. No behavior change.
2. Stop the whole process tree for every Harness. Blocked by 1.
3. agy. Blocked by 1.
4. Grok. Blocked by 1 and 2.
5. Muse. Blocked by 1.
6. MiMo-V2.6-Pro, on the route Q1 settles. Blocked by 1 and 2.

Each Harness Ticket also updates `CONTEXT.md`, the docs and Setup's list.

*Recommended:* one Spec rather than one per Harness.
*Answer:* we'll end by running the to-spec skill and then the to-tickets skill after we reach understanding, but this is sounding good.

**Q14 - An ADR?** "Every Harness runs unattended and fully trusted in the worktree":
- Muse runs with `--yolo`.
- Grok runs with folder trust off.
- agy and MiMo skip permission prompts.

It supersedes the Codex-only part of ADR 0012.

*Recommended:* yes, drafted once Q1's MiMo route is settled.
*Answer:* agree

## Research results (2026-10-06)

### MiMo-V2.6-Pro: opencode 2.0.24 compared with mimo 0.1.15

Both reach MiMo-V2.6-Pro through the same PrimaLabs provider. In opencode the model id is `primalabs/primalabs-ai/MiMo-V2.6-Pro`, and the Effort is a `#variant` suffix.

This command works, exits 0, and committed in a linked worktree:

```
opencode run --format json -m 'primalabs/primalabs-ai/MiMo-V2.6-Pro#high' --auto "<prompt>" </dev/null
```

| | opencode | mimo |
|---|---|---|
| Failure exit code | 1 | **0** |
| Bad Model or Effort | both fail before any turn | an unknown variant is silently ignored |
| Effort on PrimaLabs | low/medium/high, works with no changes | `variants: {}`; it needs a `MIMOCODE_CONFIG_CONTENT` override |
| Final event and usage | no final event, and the last `step_finish` is often dropped; `opencode session export <id>` gives the totals and the outcome | no final event, so usage has to be summed over the steps |
| `/name` skill expansion | **no**: the model has to call the `skill` tool itself | yes |
| Instruction files | `AGENTS.md` only, no `CLAUDE.md` and no `~/.claude/CLAUDE.md` | `AGENTS.md`, `CLAUDE.md` fallback, `~/.claude/CLAUDE.md` |
| Stopping | the default background-service mode leaves both the command and the turn running after SIGTERM. SIGINT, or `--standalone` plus SIGTERM, stops cleanly | SIGTERM orphans the running command |
| Cron PATH | standalone binary, works when called by its absolute path | an nvm node wrapper, so not found |
| Quirks | `-s <id>` creates the session if the id doesn't exist; a prompt passed as an argument gets wrapped in quotes when it contains a space (pass it on stdin); the first catalog call can come back empty | memory notes written outside the worktree; scheduled prompts are on by default |

### agy self-update

`AGY_CLI_DISABLE_AUTO_UPDATE=true` turns it off. `=1` does not.

### Side effects of the research

- agy was upgraded from 1.2.17 to 1.3.0 while testing `=1`. The research note's agy facts were measured on 1.2.17.
- The test sessions are now in `~/.local/share/opencode/opencode.db`.
- The opencode `service.json` password appeared in the agent's tool output.

## Round 3 (answered)

**Q15 - The Harness for MiMo-V2.6-Pro.** The options:
- (a) **opencode** as the Harness (`opencode`), with MiMo-V2.6-Pro as its Model and the variant as its Effort.
- (b) **MiMo Code** (`mimo`), using a `MIMOCODE_CONFIG_CONTENT` override for the Effort, and reading failures from the stream.

*Recommended:* (a). It has meaningful exit codes, checks the Model and Effort before any turn, and needs no config override. opencode then becomes a general Harness that can run any Model it routes, not just MiMo.

*Answer:* agree

**Q16 - Update the research note?** `docs/research/harness-candidates.md` predates the opencode comparison, the agy update switch, the Claude `AGENTS.md` fallback, and agy 1.3.0. The options:
- (a) Add these findings to the note, in a new section or as dated corrections, before the Spec is written.
- (b) Leave the note as it is, and let the Spec carry the new facts.

*Recommended:* (a). The Spec and the ADR will cite the note, and the note's §1 assumes no off switch for agy. Re-checking §1 against agy 1.3.0 is out of scope; the note will say which version its findings were measured on.

*Answer:* agree

Recorded: OpenCode (`opencode`) added to the Harness entry in `CONTEXT.md`.

## Settled by earlier answers (shout if any is wrong)

- **opencode instruction files** (Q5, Q8): in a worktree with no `AGENTS.md`, link `AGENTS.md` → `CLAUDE.md` at the worktree root and add it to `.git/info/exclude`.
- **opencode skills directory:** `.agents/skills/`, as for Codex.
- **opencode prompt:** passed on stdin, not as an argument (argument words containing spaces get wrapped in quotes). The agent is confirming this.
- **opencode Resume:** `-s <id>` creates a session when the id doesn't exist, but thirdshift only resumes ids it read from the stream, so a typo can't happen.

## Round 4 (answered)

**Q17 - Loading the skill on a Harness that doesn't expand `/name`.** Claude, Codex, agy and Grok load the skill named on the prompt's first line themselves. **Muse and opencode don't**: in both tests the model chose to call its own skill tool (`read_skill` in Muse, `skill` in opencode), and nothing makes it do so. The options:
- (a) For those Harnesses, the prompt's first line tells the model to load `thirdshift-<skill>` with its skill tool. thirdshift checks the stream for that load and writes a warning to the progress log and the Command log if it never happens.
- (b) The same, but the session fails if the skill was never loaded.
- (c) thirdshift puts the `SKILL.md` body, with its path so relative references resolve, into the Session prompt itself.

*Recommended:* (a). (c) makes the Session prompt differ a great deal between Harnesses, and the Prompts and skills page would no longer show what these sessions actually see. (b) would fail a session that read `SKILL.md` with its file tool instead. If (a)'s warnings show up in practice, move to (c).

*Answer:* agree

**Q18 - How Setup handles opencode's Model.** For Codex, Setup asks for a Model and an Effort and checks them against the catalog. opencode's Model ids depend on the providers each user has configured (here `primalabs/primalabs-ai/MiMo-V2.6-Pro`), so Setup can't propose a default.

*Recommended:* treat it like Codex. Setup asks "Model for opencode", proposes no default, and checks the answer against opencode's catalog. How the catalog is read waits on the agent.

*Answer:* agree

## Research results: opencode `--standalone` (2026-10-06)

- **Prompt on stdin:** works, and is stored exactly as sent. A prompt passed as an argument is stored wrapped in literal quotes.
- **Under cron's minimal environment:** works, by absolute path and without the daemon; the credential loads. Exit 0 on success. A bad model or variant exits 1 after about 2.5 s, with a `provider.no-route` error event and no model call. A failed run still creates an empty session.
- **Resume:** `run -s <id>` works in standalone, both for sessions made in standalone and for sessions made through the daemon. Both use the same database.
- **Free catalog:** **none works without the daemon.** In standalone, `models` and `api GET /api/model` returned nothing on every try. Only the daemon's `opencode api GET /api/model` lists each model's variants.
- **Skills:** loaded. The `skill` tool call shows in the stream as `tool_use` with `part.tool == "skill"` and `state.input.id == "thirdshift-…"`. No flag injects a skill from the command line.
- **Final message and usage:** the last `step_finish` was still dropped in 2 of 4 runs. `opencode session export --standalone <id>` works offline in about 1 s and gives:
  - `info.outcome` (`succeeded` or `failed`);
  - `info.tokens` and `info.cost`;
  - `info.model.variant`;
  - `messages[]`, where the final text is the last assistant message's last `text` part.

  `info.tokens` includes the automatic title-generation call. Summing the assistant messages gives the session's own usage.
- **Start-up:** standalone adds about 2–3 s. The private server is in the run's process group, so a group SIGTERM stops everything, and nothing is left running afterwards.

## Round 5 (answered)

**Q19 - Stopping opencode.** The options:
- (a) Every session runs `--standalone`, and the Q3 tree-stop applies with SIGTERM.
- (b) Sessions go through the user's shared daemon and are stopped with SIGINT or the interrupt API.

*Recommended:* (a). It's self-contained: it works under cron, Resume and export both work, and nothing is left running. It doesn't depend on a long-lived daemon whose version and environment thirdshift doesn't control. The cost is 2–3 s per session.

*Answer:* agree

**Q20 - Checking opencode's Model and Effort before any work.** There's no free catalog without the daemon. The options:
- (a) A **test call**, as for Claude: a standalone `Reply with OK.` run on the Model and variant. A bad one fails in about 2.5 s at no model cost; a good one costs one tiny turn.
- (b) Read the daemon's `/api/model` when the daemon is running, and fall back to (a) when it isn't.
- (c) No check up front. A `provider.no-route` in the first session is reported as a Model or Effort error.

*Recommended:* (a). It's one way that always works, and it's what Claude already does. (c) would only report the error after the worktree, the Claim and the branch exist. Q18 changes to match: Setup checks the answer with the same test call.

*Answer:* agree

**Q21 - opencode's final message, usage and failure.** Once each session exits, run `opencode session export --standalone <id>`:
- the final message is the last assistant `text`;
- the usage is the sum over the assistant messages;
- `info.outcome == "failed"` fails the session, whatever the exit code.

If the export fails, fall back to the stream's last `text` and show no usage. This is the same pattern as Muse (Q9).

*Recommended:* yes.

*Answer:* agree

**Q22 - Transient backend failures.** agy hit a 503 after about 60 s in one of four calls, and its start-up varied from 8 to 95 s. thirdshift doesn't retry a failed session today: a session that fails takes the Failed run path. Should #428 add a retry for a session that fails before doing any work (no tool call in its stream), for every Harness?

*Recommended:* no. Keep it out of #428 and file a separate issue if agy's 503s show up in real Runs. Retrying is a policy for every Harness, unrelated to adding new ones, and four test calls aren't evidence that it's needed.

*Answer:* agree

## Written so far

- `CONTEXT.md`: the **Harness** entry names agy, Grok Build, Muse Code and OpenCode.
- `docs/adr/0013-every-harness-runs-unattended-and-fully-trusted.md` (Q14), with a pointer to it at the end of ADR 0012.
- `docs/research/harness-candidates.md` §6 (Q16): OpenCode, the agy update switch, agy 1.3.0, Claude's `AGENTS.md` fallback, and MiMo Code's Effort override.

## Round 6 (answered)

**Q23 - Your `~/.claude/` instructions on the other Harnesses.** Your global `~/.claude/CLAUDE.md` and `~/.claude/rules/` hold rules you want every agent to follow. Grok and Muse read `~/.claude/CLAUDE.md`, and Grok also reads `~/.claude/rules`. **Codex, agy and OpenCode read neither.** Each has its own user-level file: `~/.codex/AGENTS.md`, `~/.gemini/AGENTS.md` and `~/.config/opencode/AGENTS.md`. The options:
- (a) thirdshift leaves user-level files alone. The docs and ADR 0013 say which Harnesses read `~/.claude/`, and you mirror the rules into each Harness's own user file if you want them there.
- (b) thirdshift passes the user's `~/.claude/CLAUDE.md` to those Harnesses itself, for example by adding it to the Session prompt or linking it in.

*Recommended:* (a). thirdshift writes nothing outside the worktree and its own files, and a user's global rules are the user's to place. It also matches Codex today (ADR 0012). Mirroring your rules once into the three user files is a five-minute job outside #428.

*Answer:* agree

**Q24 - Shared understanding.** This is the design the Spec will be written from:

1. **Harnesses:** agy, Grok Build (`grok`), Muse Code (`muse`) and OpenCode (`opencode`, running MiMo-V2.6-Pro on PrimaLabs) join Claude and Codex. Each is named by its CLI.
2. **The Harness module is deepened first.** It becomes one adapter per Harness, holding: the CLI name, session and Resume arguments, how the prompt names its skill, the skills directory, the instruction-file fallback, the fixed environment, the stop signals, the Model and Effort check, the stream parser, and where the final message and usage come from. This change alters no behavior.
3. **Stopping:** before signalling, thirdshift snapshots the session's descendant tree and signals every process group in it, for every Harness.
4. **Unattended and trusted:** ADR 0013's flags. Self-update is off (and MiMo's scheduled prompts don't apply, since MiMo Code isn't used).
5. **Instruction files:** each Harness reads its own file and falls back to the other: a `GEMINI.md` link for agy, an `AGENTS.md` link for OpenCode, both kept out of git. Grok reading both files is documented.
6. **Skills:** linked into `.agents/skills/` for every new Harness. On Muse and OpenCode, the prompt tells the model to load the skill, and thirdshift warns when the stream shows it never did.
7. **Model and Effort checks:**
   - agy and Grok: their free catalogs.
   - Muse: its cached catalog, else a test call.
   - OpenCode: a test call.

   Setup checks its answers the same way.
8. **Final message, usage and failure:**
   - agy and Grok: from the stream.
   - Muse: from its session log, falling back to the stream.
   - OpenCode: from `session export`, where `outcome == failed` fails the session, falling back to the stream.
9. **Setup** lists only the installed Harnesses. For Muse it proposes `muse-spark-1.3`; for OpenCode it proposes no default.
10. **The Spec:** one Spec. Ticket 1 deepens the Harness module; Ticket 2 adds the tree-stop and is blocked by 1. Then one Ticket per Harness: agy and Muse blocked by 1, Grok and OpenCode blocked by 1 and 2. Each Harness Ticket updates `CONTEXT.md`, the docs and Setup.
11. **Out of scope:** retrying on transient backend errors (Q22), a sandboxed mode, MiMo Code, and #429's reviewers.

Is this the shared understanding? If so, the next steps are to-spec, then to-tickets.

*Answer:* we have shared understanding.

## Waiting on research

(none)
