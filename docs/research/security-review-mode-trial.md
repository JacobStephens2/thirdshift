# Security review mode trial

Research date: 2026-10-08. Implements [#549](https://github.com/JacobStephens2/thirdshift/issues/549), the report-only trial in [#427](https://github.com/JacobStephens2/thirdshift/issues/427). At Jacob's request, Codex graded the private results on 2026-10-09. This note records the measurements and grading summary; the operator's choice of Security review mode remains separate.

## Method

The sample is the three most recent merged **code** pull requests on thirdshift's `main` at the start of the trial: [#529](https://github.com/JacobStephens2/thirdshift/pull/529), [#527](https://github.com/JacobStephens2/thirdshift/pull/527), and [#525](https://github.com/JacobStephens2/thirdshift/pull/525), all merged on 2026-10-07. The newer #553 imported the skill being evaluated into `issue-427`; it was excluded from the code sample. This is one factory repository and one language, Rust, with small diffs. It is not a cross-repository benchmark.

| PR | Merge base | Reviewed head | Changed files |
|---|---|---|---:|
| #529 | `54c31ee80fe3fcc7af454b007f18b47d6e6976ad` | `b5c9980a4d23fd72ccc1796807cc617022e9c2cf` | 6 |
| #527 | `5ec943dcb0c4dfe26aa9f5d98916aa947f48ca42` | `ac60d24e79226ef4f1c7b54ebb8442d1826e8077` | 2 |
| #525 | `535c3070073e38a70f9b40f9056c3794a1a4b60c` | `33609e6c20326bb535e084382d85f38a2aa236b5` | 4 |

GitHub's pull-request API supplied each original head and base SHA. `git merge-base <base> <head>` supplied the comparison's start; `git diff <merge-base> <head>` supplied its scope. Each of the twelve sessions got its own fresh local clone, made with `git clone --no-hardlinks`, with `origin` removed before checkout and the session's start. Each target was detached at its PR head. The controller checked remotes and tracked/untracked target changes before and after each session. No prior trial or audit was provided as context.

Both modes used the [embedded Factory skill](../../skills/thirdshift-security-audit/SKILL.md), imported in #553, from Cloudflare's upstream commit [`c1c8a8c1471069fb0e188eeaff69b8e8db6564a8`](https://github.com/cloudflare/security-audit-skill/tree/c1c8a8c1471069fb0e188eeaff69b8e8db6564a8). Its body, companions, schema and validators are unchanged upstream copies; the name is `thirdshift-security-audit` ([credits](../../skills/thirdshift-security-audit/CREDITS.md)).

- **Full audit:** the complete six-phase workflow, `quick` profile, scoped to the merge-base/head diff, with native artifacts outside the target. No strict agent-invocation budget was imposed. It kept the single hunter wave, final coverage critic and independent verifiers required by `quick`.
- **Guidance:** one session read the relevant attack classes and reviewed the same diff. Delegation and the full workflow were explicitly disabled. Findings were returned in the session's final JSON response, with the skill's verdict vocabulary, rather than native audit artifacts.

Both prompts named the target, head, merge base and diff scope; excluded vendored and third-party code; requested applicable threat-model and domain context; and allowed supporting source reads only to resolve in-scope behavior. They forbade target edits, commits, pushes and any public record. They instructed the session to end `incomplete` instead of asking a question, gave it no prior runs, and required it to join or stop its own tasks before ending. The guidance response contract was `{status, asked_question, findings, limitations}`; the full audit response contract was `{status, asked_question, findings_file, limitations}`. A finding's title and detail were retained only privately.

No OS-enforced sandbox for target code was provisioned. The prompts explicitly forbade target-controlled builds, tests, processes and fixtures, and permitted trusted source-reading tools and the skill's Node validators. Where execution was decisive, the skill had to retain a `needs_validation` blocker and safe validation plan. A complete source-only review is not a confirmed vulnerability or a complete security assessment.

The Harnesses were Claude Code **2.1.292**, `claude-opus-5-5` at **medium**, and Codex **0.161.0**, `gpt-6.1-sol` at **xhigh**. Claude used the factory's `auto` permission mode and a private, session-local plugin containing the pinned skill. Codex used the factory's unsandboxed invocation ([Claude adapter](../../src/harness/claude.rs), [Codex adapter](../../src/harness/codex.rs)). The agent CLI being unsandboxed did not authorize target-code execution. Codex's concurrent sub-agent cap was raised to eight, and the full audit prompt required fresh sub-agents with `fork_turns="none"`.

The launch arguments, with path placeholders, were:

```sh
claude -p --permission-mode auto --model claude-opus-5-5 --effort medium \
  --output-format stream-json --verbose --plugin-dir <private-plugin> \
  --add-dir <private-trial-directory>
codex exec --json --dangerously-bypass-approvals-and-sandbox \
  -m gpt-6.1-sol -c 'model_reasoning_effort="xhigh"' \
  -c agents.max_concurrent_threads_per_session=8 \
  -c 'project_doc_fallback_filenames=["CLAUDE.md"]' \
  -o <private-final-response> -
```

Prompts were supplied on stdin. The controller ran at most **two top-level trial sessions** at once, each under a **2,700-second (45-minute) timeout**. Delegated audit agents belonged to those sessions. For each PR in order #529, #527, #525, it queued guidance on Claude then Codex, followed by full audit on Claude then Codex. A freed slot started the next queued trial. Wall time covers the CLI's start through its exit, including initialization, permission review and sub-agent waits, but excludes clone preparation. A timeout terminates only that trial's captured process group; it is not a clean result. No safety refusal was retried on another Model.

## Measurements

Each cell is one independent session; there were twelve CLI sessions, and no cell was rerun. **C / N / R** means `confirmed` / `needs_validation` / `rejected` final records. **Refusal / Model change / question** records visible events for that particular session. All twelve exited 0 before their timeout, reported `complete`, left their target unchanged and kept it without remotes. All six full audits passed both native validators. These completion statements describe the scoped source-only workflow, not exhaustive coverage.

### Claude Code

Cost is the CLI’s reported `total_cost_usd`, rounded to four decimal places; it is a usage estimate, not an invoice. The reported total includes delegated agents; Anthropic documents that `usage` excludes them while `total_cost_usd` and `modelUsage` include them ([cost accounting](https://code.claude.com/docs/en/agent-sdk/cost-tracking#get-the-total-cost-of-a-query)). Sub-agents are distinct `local_agent` task starts in the private stream, cross-checked against delegation calls; resumed calls and background shell tasks are not counted as additional agents.

| PR | Mode | Wall seconds | Cost USD | Sub-agents | C / N / R | Refusal / Model change / question |
|---|---|---:|---:|---:|---|---|
| #529 | Guidance | 64.181 | $0.4574 | 0 | 0 / 0 / 2 | 0 / 0 / no |
| #529 | Full audit, quick | 365.615 | $5.0737 | 8 | 0 / 0 / 0 | 0 / 0 / no |
| #527 | Guidance | 35.393 | $0.3481 | 0 | 0 / 0 / 0 | 0 / 0 / no |
| #527 | Full audit, quick | 360.046 | $4.8014 | 7 | 0 / 0 / 0 | 0 / 0 / no |
| #525 | Guidance | 46.072 | $0.3877 | 0 | 0 / 0 / 0 | 0 / 0 / no |
| #525 | Full audit, quick | 320.071 | $4.5467 | 8 | 0 / 0 / 0 | 0 / 0 / no |

### Codex

Tokens are the sum of each parent and descendant thread’s final `total_token_usage`, matched by exact thread ID and the recorded `parent_thread_id` lineage. Total = input + output; cached input is already part of input, and reasoning output is already part of output. Neither subset is added again. The separate cached column makes repeated cached context visible. Codex reports no dollar cost.

The `exec --json` stream in this installed CLI omitted the newer delegation calls, so its apparent zero-agent count was discarded. The private rollouts establish the actual descendant count, requested Model and Effort, `fork_turns="none"` on every audit spawn, question-tool calls, and terminal completion of every thread. This avoids counting only the parent’s tokens. Refusals were checked in provider failure events; Model changes were checked in Claude’s emitted Model fields, Codex’s requested Models and explicit reroute notices. Codex does not expose a separate server-returned Model here, so zero means no visible switch, not an assertion about provider internals. Source-reading output was excluded from these event checks. Questions combine actual question-tool calls with the final response’s `asked_question` value.

| PR | Mode | Wall seconds | Total tokens | Cached input tokens | Sub-agents | C / N / R | Refusal / Model change / question |
|---|---|---:|---:|---:|---:|---|---|
| #529 | Guidance | 179.166 | 948,329 | 835,968 | 0 | 0 / 0 / 0 | 0 / 0 / no |
| #529 | Full audit, quick | 949.918 | 15,871,642 | 14,784,512 | 9 | 0 / 0 / 0 | 0 / 0 / no |
| #527 | Guidance | 185.451 | 1,069,677 | 946,432 | 0 | 0 / 0 / 0 | 0 / 0 / no |
| #527 | Full audit, quick | 1108.118 | 11,152,997 | 10,264,832 | 8 | 0 / 0 / 0 | 0 / 0 / no |
| #525 | Guidance | 182.523 | 686,666 | 577,408 | 0 | 0 / 0 / 0 | 0 / 0 / no |
| #525 | Full audit, quick | 1088.746 | 13,115,984 | 12,153,984 | 8 | 0 / 0 / 0 | 0 / 0 / no |

### What this sample establishes

| Harness | Mode | Median wall seconds | Median cost or total tokens |
|---|---|---:|---:|
| Claude Code | Guidance | 46.072 | $0.3877 |
| Claude Code | Full audit, quick | 360.046 | $4.8014 |
| Codex | Guidance | 182.523 | 948,329 tokens |
| Codex | Full audit, quick | 1088.746 | 13,115,984 tokens |

Guidance was faster and used less reported cost or fewer tokens on each of these three PRs. Full audit added reconnaissance, a coverage ledger, independent agents and validated native artifacts. Each Harness made its own coverage plan, so the same `quick` profile did not impose identical ledger granularity.

There are no independently confirmed vulnerabilities or seeded ground-truth defects in this source-only sample. Counts alone therefore establish neither recall nor precision, and an empty final array is not evidence that the code is secure. The private results have now been graded by AI source review at the operator’s request; the grading summary below does not establish runtime detection accuracy. One run per cell, one Rust repository, small related diffs and fixed execution order limit generalization. Shared provider caches and other machine activity were not controlled. Dollar estimates and raw tokens are different measures and are not a cross-Harness price comparison. The trial supports a discussion of overhead and unattended operation; it does not choose the Security review’s mode.

## Private grading record

Every reported finding, including rejected candidates, is grouped by PR, mode and Harness in:

`/home/jacob/.local/state/thirdshift/security-review-trial-549-7451fsff/findings.md`

The file has mode **600**, inside a directory with mode **700**. Grades now accompany each finding and each empty-result cell, with the original finding JSON preserved. Raw session transcripts and native audit artifacts remain private on this machine. Finding titles, details, fingerprints, source traces and validation plans are absent from this note and from the pull request. No advisory, issue or label was created for the trial, and nothing was pushed to another repository.

## Grading summary (2026-10-09)

Codex compared the two submitted candidates against the original merge base and reviewed head, read the relevant ownership contracts and ADR, and checked all twelve private analysis records and the six full-audit validator results. This was source review; no target code was built or executed, and it was not a new exhaustive audit of the three PRs. Candidate details and grading rationales remain in the private record.

| Dimension | Assessment |
|---|---|
| Operational feasibility | Pass within the sample: all twelve sessions completed, and all six full-audit artifact validations passed. |
| Submitted candidates | Both were correctly rejected as introduced vulnerabilities under the trial's documented scope, with a rationale caveat retained privately. Neither is a detected vulnerability or an actionable false positive. |
| Empty-result cells | No actionable candidate submitted; no false-negative rate or security-assurance score can be assigned. |
| Detection efficacy | Inconclusive: there were no seeded defects or independently established positive cases. Precision and recall are unmeasured. |
| Coverage process | Full audit provides a coverage ledger and independent review artifacts; guidance does not provide that artifact contract. Passing validators does not prove adequate coverage. |
| Efficiency | Guidance was faster and used less reported cost or aggregate tokens on all three PRs. |

**Recommendation:** use guidance provisionally for the opt-in Security review of a change, based on its lower overhead and no demonstrated additional actionable yield from full audit in this sample. This is an engineering recommendation, not evidence of equivalent coverage or detection ability. The separate whole-repository Security audit retains its full-audit workflow.

The existing Spec evidence requirements still apply: fixes require a failing proof-of-concept test, unaddressed introduced findings hold Self-merge, and pre-existing vulnerabilities stay in private records. A future effectiveness comparison should include known positive and negative cases with safe independent validation.

**Mode decision:** pending the operator's choice. Tickets #550–#552 remain gated until that choice is recorded in their specifications.
