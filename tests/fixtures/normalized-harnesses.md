# Normalized Harness decoder examples

`normalized-muse.jsonl` reduces a Muse Code `--json` recording to its task lifecycle, scheduled tool-call correlation and tool result. IDs, timestamps, text and commands are replaced with small deterministic examples. The retained-record test uses Muse's transaction envelope and `runtime.session` run records.

`normalized-opencode.jsonl` and `normalized-opencode-export.json` use OpenCode 2's recorded stream and standalone export shapes. IDs, timestamps, text and commands are replaced. The export contains one call omitted from the stream; the stream also omits its final `step_finish`. Session totals deliberately include unrelated usage so a decoder that counts the title call fails the assertions. The first streamed call's token values retain the recorded cache and reasoning split.

Agy's reduced examples are inline in its decoder tests. Its `DONE`-only steps, checkpoint usage, text deltas and tool error objects follow the [headless format reference](https://www.antigravity.google/docs/cli/headless/). Tests distinguish per-step usage from cumulative result usage.
