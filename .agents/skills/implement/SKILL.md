---
name: implement
description: "Implement a piece of work based on a spec or set of tickets."
disable-model-invocation: true
---

Implement the work described by the user in the spec or tickets.

If the user passes a ticket reference, fetch it from the issue tracker and state its title before starting. If the reference is ambiguous, ask.

When the task touches rendered UI, HTML, CSS, templates or visual layout and a headless browser is available, capture `/tmp/before-*.png` (or `.gif` for interactive or animated flows) against the unpatched baseline before editing code, at `1440×900` desktop plus `390×844` mobile when responsive layout is affected. When verifying the change, capture matching `/tmp/after-*.png` or `.gif` at the same viewports and states so both Before and After artifacts are ready for the PR.

Call the Skill tool with "tdd" where possible, at pre-agreed seams.

Run typechecking regularly, single test files regularly, and the full test suite once at the end.

Once done, call the Skill tool with "code-review" to review the work.

Commit your work to the current branch.
