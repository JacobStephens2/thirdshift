# Session logs become Session traces, in OpenTelemetry's format

Each **Harness** prints its own stream of events, so a **Session log** comes in one of six formats, and no eval tool reads them as they are. Evals need every session's model calls, tool calls, tokens and times, from every Harness and across **Runs** (#698). So each session writes a **Session trace** in place of its Session log: one OTLP JSON Lines file where the Session log is now. It holds an OpenTelemetry log record for each Harness event as the event arrives, and a span for the session and for each model call and tool call as it ends. A **Trace** is a **Command**'s whole tree, and all of its Session traces share its trace ID. Each record keeps the GenAI semantic conventions' fields, `thirdshift.*` attributes for what those lack, and the raw Harness event, so nothing a Session log holds is lost.

## Considered Options

- **Keep Session logs, and write a Trace beside each.** Rejected. Every session would be recorded twice, once in its Harness's own format, and the **Console** would go on reading only the two formats it knows.
- **Spans only.** Rejected. A span is written when it ends, so a file of spans stops growing during a long tool call. The Console's live view would stall, and a working Run would look **Silent**. Log records are written as each event arrives.
- **Each Harness's own OpenTelemetry export.** Rejected. Claude Code's export has no token counts and redacts content by default, Codex's has no prompts, outputs or model name, and none of them knows about Runs.
- **The GenAI conventions alone.** Rejected. They have no place for cost, the cache split, rate limits, permission denials, killed work, sub-agents' lifecycles, diffs or pushes, and evals need most of those.

## Consequences

- The GenAI conventions and the OTLP file format are both still in Development, so each Session trace records the conventions version it follows. Records keep the raw Harness event until a round-trip check shows that a trimmed record loses nothing in use.
- The Console's log view renders Session traces for all six Harnesses, at first by unwrapping each record's raw event for its Claude and Codex code. The Waterfall takes tool times from the records instead of matching Command log lines. Silent still watches the files grow.
- Model evidence and review reports stay beside each session's file, named from its path.
- **Command logs** and **Activity logs** stay logs (ADR 0011): thirdshift writes them itself, and they aren't a Harness's stream.
- A **Security run**'s Session traces stay on the machine, as its Session logs do (ADR 0015).
- The glossary renames Session log to Session trace, and adds Trace, when the switch is built. This answers #700.
