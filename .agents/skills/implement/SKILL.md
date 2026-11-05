---
name: implement
description: "Implement a piece of work based on a spec or set of tickets."
disable-model-invocation: true
---

Implement the work described by the user in the spec or tickets.

If the user passes a ticket reference, fetch it from the issue tracker and state its title before starting. If the reference is ambiguous, ask.

When the task touches rendered UI, HTML, CSS, templates or visual layout and a headless browser is available, create each Before capture with `mktemp` (`/tmp/before-desktop.XXXXXX.png`, `/tmp/before-mobile.XXXXXX.png`, or `.gif`) and write the unpatched baseline into that file before editing code, at `1440×900` desktop plus `390×844` mobile when responsive layout is affected. When verifying the change, create matching After files the same way at the same viewports and states, and keep those exact paths for the PR.

Call the Skill tool with "tdd" where possible, at pre-agreed seams.

Run typechecking regularly, single test files regularly, and the full test suite once at the end.

Once done, call the Skill tool with "code-review" to review the work.

Commit your work to the current branch.
