# Pickup re-runs Base branch CI before filing, and retains flaky evidence

Pickup requests one failed-jobs re-run of a failed first attempt of the Base
branch's latest push workflow, immediately. A later failed attempt gets the
ordinary CI-red Ticket after its own 30-minute grace; a successful attempt after
a failure gets a Flaky-test Ticket carrying both attempts and the recorded test
failure. Refused or impossible re-runs warn and fall back to ordinary filing.

This is separate from ADR-0010's Check re-run and ADR-0020's Abandoned-job
re-run. Those concern Delivery on work branches; Pickup repairs the Base branch
and must retain evidence even when the same commit turns green. A Flaky-test
Ticket carries `flaky-test` as well as `bug`, bypasses diagnosis, and asks the
implementation session to remove timing dependence or the exposed race. Its
recorded failure is evidence; demanding an on-demand reproduction strands it
in `needs-info` (#1561).

## Consequences

- A CI-red Ticket in `needs-info` cannot suppress a new report. Otherwise an
  open CI-red Ticket receives new failing commits as comments, once per commit.
- A CI-red Ticket is titled `CI red on <base>` and labelled `bug`. Its body
  and later comments name the failing commit and attempt URLs and omit job
  names and log lines. The Day shift writes the task and applies
  `ready-for-agent` before a Pickup run can take it (amended 2026-10-30, #1647).
- Flaky reports are deduplicated by failed run attempt, including closed
  reports, and by tests already named in open Flaky-test Tickets. Unknown test
  names still permit a report deduplicated by attempt.
- The origin CI filing reservation encloses the current evidence read, re-run
  request, deduplication and publication. An occupied reservation skips these
  actions and leaves normal Pickup work available.
- Read failures still warn and leave Pickup running. Base fixes and Self-merge
  gates are unchanged. This introduces neither workflow retries nor extra CI
  workflows.
