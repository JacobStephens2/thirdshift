# Round 1 (answered)

Part of the [#698 grilling](README.md).

**Q1.1 - What #698's evals are for.** "Evals" covers three different jobs, and tools lean toward different ones. Langfuse is built around traces and scoring them, (b) and (c). For (a) it can store and compare runs. The part that rebuilds a past commit without its later fix and drives the CLIs on your logins would still be thirdshift's: the research found no tool that does it without containers. The options:
- (a) **Replays before a change.** A standing set of past situations with a known right answer. You re-run it on demand before changing a Factory skill (such as #321's update), a Session prompt, or a Harness, Model or Effort default, and compare the result with the current setup against a bar set in advance. #429 and #549 did this by hand, each with a throwaway script. #429's graded material would seed the set: 9 bugs with written catch criteria, the plans its arms stopped, and the Spec findings graded real or wrong. Autoresearch (#348) would need something like it to measure against.
- (b) **Run scores.** What each Run already tells us, recorded when it ends: outcome, Repairs and CI rounds used, Unaddressed findings, minutes and cost-equivalent. Later, what became of its pull request: reverted, or a bug traced to it. Shown as trends by Harness, Model, Effort and kind of session.
- (c) **A trace viewer.** Session logs browsable in a UI: each session's tool calls, tokens and timings, across Runs. Any viewer needs an importer that thirdshift writes, since no tool reads its Session logs as they are.

*Recommended:* (a) first, seeded from #429. Then (b), limited to what thirdshift knows when a Run ends. Leave (c) to #700 and #711, unless the tool chosen for (a) shows traces anyway. (a) answers the decisions you keep making, and twice now you've paid for it with a one-off script. (b) is cheap but blunt: 90% of commands since 3 October ended in a Self-merge, so its signals measure friction more than quality. (c) shows sessions but grades nothing.

*Answer:* agree. I generally want a way to measure if changes to the agents make them better or worse. I want to be able to know this before putting the changes into production, and then I want to analyze the traces of production to understand how well things are running largely in terms of accuracy and quality, though I'm somewhat concerned about efficiency too and other good things for evals to consider.

**Q1.2 - Where eval data may live.** Evals keep Session logs, or traces made from them, and their grades. A Session log holds the repository's code, the agent's reasoning, tool output and paths from this machine. Anthropic and OpenAI already see every session, since they run it. A hosted eval service would be one more company holding all of it. The options:
- (a) **Only on machines you run.** No hosted service. On this host that means MLflow or Phoenix, each one process on SQLite. Self-hosted Langfuse needs Docker and about 16 GiB, so a second machine.
- (b) **A hosted service may hold public repositories' logs,** but never a Security session's. Those are easy to keep out: they sit under `commands/secure/`, with the Runs a Security run starts, or in `security-*` files, and a fix you start by hand carries the `security-fix` label. chart35's logs stay local. Langfuse Cloud would cost about $33 a month on its Core plan: the factory would send about 140k units a month, and the free Hobby plan covers 50k.
- (c) **As (b), with private repositories' logs too.**

*Recommended:* (a). It matches the preference for self-hosted in the Warp and OpenHands research, and replays and Run scores need no service at all. On Langfuse Cloud, data past your plan's access window is hidden but still stored until you delete it by API; only the Pro plan, at $199 a month, deletes it on a schedule. Choose (b) if you want Langfuse's grading tools without running a second machine.

*Answer:* are session logs traces? if their data is basically the data of traces and there is a different standard format for traces, I think I want to switch to that. or are the traces formats different depending on the eval framework?

*Checked:* Not quite. A Session log is the stream of events one Harness prints during one session, in that Harness's own format: six Harnesses, six formats. A trace is a tree of timed spans, each with an ID and a parent. One trace could cover a whole command: its Runs, each session in them, and each model call and tool call inside those. Most of a trace's content is already in the Session logs. What they lack is the tree, which thirdshift knows and they don't.

There is one dominant standard, OpenTelemetry, sent over OTLP, and most eval tools accept it. They disagree on the attribute names inside it. OpenTelemetry's own GenAI conventions are still marked Development. Phoenix reads OpenInference's names and takes OTLP only as protobuf. Harbor uses its own ATIF JSON. So one export reaches most tools, but each one shows it a little differently.

thirdshift can't switch the Session logs themselves to a trace format. The Harnesses write them, and thirdshift reads them as they arrive to collect Model evidence and the reviewers' reports. Q2.1 asks what to do instead.
