# Cost accounting for comparing a Built-in process with a Custom process

Research date: 2026-10-29.

**Question.** A comparison of two Processes on the same issues rejects the Candidate at more than
twice the wall-clock time or more than twice the tokens. Can both arms be measured on comparable
terms? The Candidate is a Custom process whose one `skill` Step runs a home skill. That skill
orchestrates its own sub-agents by starting the `claude` and `codex` CLIs through Bash, and then
`delivery` runs.

**Method.** thirdshift source and `docs/guide.md` in this worktree. The Candidate's skills are
private, so what they record is summarised generically in section 2; the details are kept with
the experiment's private notes.

## 1. What thirdshift measures

**Token source.** For a Claude harness session, `src/harness/claude.rs:131` launches with
`--output-format stream-json --verbose`. `src/harness/interpretation/stream/events.rs:90-118` reads
the terminal `result` event: `usage` becomes a Session-scope `Event::Usage`
(`input_tokens`/`output_tokens`/`cache_read_input_tokens`/`cache_creation_input_tokens`, mapped at
`src/harness/interpretation/event.rs:113-120`), `total_cost_usd` becomes the session cost, and each
key of `modelUsage` becomes a Model-scope `Event::Usage` from `inputTokens`, `outputTokens`,
`cacheReadInputTokens`, `cacheCreationInputTokens`, `costUSD`. Separately, every `assistant` message
and every `stream_event` `message_start`/`message_delta` frame accumulates per-model-call usage
(`events.rs:167`, `:459`), closed into `ModelCallEnded` at `:255-280`.

For Codex, `src/harness/codex.rs:106-109` and `:179-180` launch `codex exec --json`;
`src/harness/codex/events.rs:47-55` reads `turn.completed`'s `usage` as `input_tokens`,
`output_tokens`, `cached_input_tokens`, `cache_write_input_tokens`. Codex reports no cost, so
`cost_usd` stays null (`docs/guide.md:280`: cost is recorded "only when reported by the Harness,
without a price table").

**Reconciliation.** `src/session/usage.rs` owns precedence: per-call measurements first, then
`prefer_total` refines only reported fields (`usage.rs:290-315`), then Session totals refine a
single matching model, then turn totals fill unknown-model tokens
(`usage.rs:206-245`; prose at `docs/guide.md:871-881`). Input is normalized to *include* cache:
`Harness::input_excludes_cache()` is true only for Claude and Grok (`src/harness.rs:61-63`), so
Claude's exclusive `input_tokens` has cache read and write added
(`event.rs:101-112`); Codex's is already inclusive. Output, cache read and cache write stay separate
(`usage.rs:28-34`), split by **observed** model, never inferred from the requested one
(`docs/guide.md:279-280`).

**Times.** `RunMetrics` (`src/logs/run_metrics.rs:13-35`) holds `outcome`, `repairs` (count of
Repair stages), `ci_rounds` (CiWatch stages), a `stages` array of `name`/`seconds`/`minutes`/
`usual_range`/`position`, and `total_minutes` = `ended_at - started_at` of the Command record
(`run_metrics.rs:148-154`) — pure wall clock, including CI waits, Repairs and any stall. Stage
seconds come from `record.stage_times(None)`, each stage ending where the next begins or where the
Command ends (`run_metrics.rs:121-124`; `docs/guide.md:272`).

**Storage.** One JSON file per Command: the Command-log stem plus `.command.json`, i.e.
`~/.thirdshift/logs/<owner>/<repo>/commands/issue/<n>-<stamp>~<token>.command.json`
(`docs/guide.md:2046`; `log.with_extension("command.json")` at `src/logs/command_record.rs:64`).
It is a `CommandTree` — the command record plus `dispatched` children
(`command_record.rs:15-21`) — version 2 with `run_metrics` embedded
(`docs/guide.md:271`). Session traces are OTLP JSONL beside it
(`<stem>-<kind>.logs.jsonl`). Read it with `thirdshift metrics <command> --json`
(`docs/guide.md:285-287`).

**Counted vs not counted.** Claude Code's own `Task` subagents **are** counted. Their stream events
carry a non-null `parent_tool_use_id`, which only scopes attribution
(`events.rs:32-34`, `stream.rs:160-170` marks such models `ModelScope::AttributionOnly`), and their
assistant messages still accumulate model-call usage; the root `result` event's `usage` and
`modelUsage` are harness-side aggregates over the whole session. By contrast, a session that shells
out via Bash to `codex exec` or `claude -p` produces **no** events in thirdshift's stream: that
child's tokens are invisible to thirdshift, and only its elapsed time lands in the stage time.
This is the central fairness problem: the Candidate arm spends most of its tokens exactly there.
(Unconfirmed: whether Claude Code's `result.usage` includes subagent tokens is documented nowhere in
this repo; it is inferred from the aggregate's position in the stream. Verify empirically with a
one-session probe before trusting arm totals.)

## 2. What the Candidate side records

The sub-agents a skill session starts are separate CLI processes, and thirdshift never sees their
streams (section 1). Each was launched with machine-readable output: Claude as
`claude --print --output-format stream-json …`, writing a `stream.jsonl`, and Codex as
`codex exec --json …`, writing a `raw_output.jsonl`. They were written to a gitignored run
directory inside the worktree. The skill's own effort records hold input, output and cached-input
tokens and the duration of each call. They have no cache-write field. The Candidate's own process
deletes them after merge, so they have to be copied out after each run and before any merge.

## 3. Wall-clock definition

thirdshift's own answer is `total_minutes` = Command start to Command end, which for a Ticket run
spans Implement, review, Repairs, CI watch rounds and the merge. CI wait and Repairs are in it; the
Candidate arm's CI waits and fix rounds are likewise inside its single `skill` Step. That symmetry
is worth keeping.

Stalls are the exception: a plan-limit or Codex quota stop is wall time neither arm chose.
thirdshift already treats these as a distinct class — `src/harness/interpretation/limits.rs` and
`src/harness/codex/limits.rs` detect them, and Experiments void such Trials and pause scheduling
(`docs/guide.md:708-721`).

**Recommendation.** Define wall clock as *Command start to pull request ready* (first green CI on the
head that is merged, or the merge itself if the arm self-merges), minus detected limit stalls:

    adjusted_wall = total_minutes - sum(limit_stall_intervals)

Report both raw and adjusted, and report the stall time itself. For the Built-in arm, take
`total_minutes` from `run_metrics` and find stalls from the Session traces' limit events. For the
Candidate arm, take the same `total_minutes` (the Custom process is still one Command with its own
record, `docs/guide.md:2017`) and find stalls in the retained `stream.jsonl` / `raw_output.jsonl`
per round. Do not use summed work duration as the wall clock for either arm: `CONTRACT:400,484` is
explicit that parallel work sums above elapsed ("parallel calls take 10 and 12 seconds in a
12-second window: work time is 22 seconds, elapsed is 12").

## 4. Measurement method

Per issue, run both arms and collect:

1. **Built-in arm.** `thirdshift metrics '<repo>#<n>' --json` → `run_metrics.total_minutes`,
   `stages[]`, and `session_signals` tokens by observed model. One command, no parsing of prose.
2. **Candidate arm.** Same command for the outer Command record (gives wall clock and the one
   Step's time, plus the orchestrator session's own tokens). Then, *before merge*, `cp -R` the
   `/.review-runs/<round>/` tree to a keep directory, and sum the children:
   - Claude children: last `{"type":"result"}` line of each `stream.jsonl` → `modelUsage` per model
     (fall back to `usage`). Inclusive input = `inputTokens + cacheReadInputTokens + cacheCreationInputTokens`.
   - Codex children: last `{"type":"turn.completed"}` line of each `raw_output.jsonl` → `usage`
     (`input_tokens` already inclusive; `cached_input_tokens` is a subset).
   A ~50-line script (`jq -s 'map(select(.type=="result"))|last'` per file, keyed by provider
   directory) suffices; none of the Candidate's own scripts needs to run.
3. **Combine.** Report three numbers per arm: Anthropic tokens, OpenAI tokens, and an
   API-price-equivalent USD from one price table frozen for the whole experiment (thirdshift
   deliberately has none — `docs/guide.md:280` — so the table lives in the experiment note, not the
   code). Also report `total_cost_usd` where a harness gave one, as a cross-check.

**Which the 2x rule should use.** Raw token counts across providers are not one unit (different
tokenizers, different cache pricing), and summing them would let a cheap-model reviewer look
expensive. Apply the 2x token rule to the **API-price-equivalent USD** total, and publish the
per-provider token counts beside it so the ratio can be re-derived if the price table is disputed.
Apply the 2x wall-clock rule to **adjusted_wall** as defined above.

**Minimum viable fairness.** Because thirdshift cannot see externally spawned CLIs, the Candidate
arm's tokens are only as complete as the retained round directories. Before trusting any ratio,
confirm on one pilot pair that every reviewer invocation left a stream file, and that the
orchestrator's session tokens are not double-counted against its own Task subagents. Both
unconfirmed here.
