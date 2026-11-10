# Normalized Harness decoder examples

`normalized-muse.jsonl` reduces a Muse Code `--json` recording to its task lifecycle, scheduled tool-call correlation and tool result. IDs, timestamps, text and commands are replaced with small deterministic examples. The retained-record test uses Muse's transaction envelope and `runtime.session` run records.

`normalized-opencode.jsonl` and `normalized-opencode-export.json` use OpenCode 2's recorded stream and standalone export shapes. IDs, timestamps, text and commands are replaced. The export contains one call omitted from the stream; the stream also omits its final `step_finish`. Session totals deliberately include unrelated usage so a decoder that counts the title call fails the assertions. The first streamed call's token values retain the recorded cache and reasoning split.

Agy's reduced examples are inline in its decoder tests. Its `DONE`-only steps, checkpoint usage, text deltas and tool error objects follow the [headless format reference](https://www.antigravity.google/docs/cli/headless/). Tests distinguish per-step usage from cumulative result usage.

`normalized-kimi.jsonl` follows Kimi Code 2.5.2's `-p --output-format stream-json` writer: OpenAI-shaped assistant and tool messages, the `system.version` line before the turn and the `session.resume_hint` line after a completed one. IDs, text and commands are replaced. `kimi-wire.jsonl` holds the session record's `step.end` usage shape, which the stream leaves out, reconstructed from Kimi Code 2.5.2's published source (its loop writes `step.end` with `usage` as a `context.append_loop_event` record), not recorded from a live run; `kimi-providers.json` is `kimi provider list --json` with a Fireworks provider keyed by environment variable and the managed Kimi Code provider.
