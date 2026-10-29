# Built-in processes

This folder holds the Built-in process files that ship inside thirdshift.
Its top-level `*.toml` files are compiled into the binary, like the Factory
skills in `skills/` ([ADR-0001](../docs/adr/0001-rust-binary-with-embedded-skills.md)).
Editing a file here takes effect after rebuilding and reinstalling the binary;
an installed binary needs no checkout of this repository.

Built-in processes use the same TOML format and validation as
[Custom processes](../docs/guide.md#custom-processes). Use
`thirdshift process list` to see Processes and their descriptions, with Built-in
processes marked `(built-in)`. `thirdshift process show <name>` prints the file
exactly. Save it under a new name in `~/.thirdshift/processes/` as a starting
point for a Custom process; a Custom process cannot take a Built-in name.

`ticket-run.toml` defines `implement` → `security-review` → `delivery` →
`self-merge`, inheriting the Command's Harness, Model and Effort, Child asks
and Limits. Ticket runs execute this file by default, including those started by
Spec, Architect and Security runs. A `run-published-issue` Step can select a
different Ticket Process with `process = "<name>"`; the named Process must exist
and declare an Issue-branch worktree. An issue with Tickets still starts a Spec
run. The optional Security review and Self-merge keep
their command and User config gates. Commands still appear as Ticket runs.

`architect-run.toml` defines `architecture-review` → `run-published-issue`.
Architect runs execute it after the Pass lock and the open-plan, waiting-idea
and Ready-or-settling-issue gates. The review Step owns the session and the
plan-or-idea conclusion and labels. It ends the Process successfully on an idea
or when `--plan-only` was requested; otherwise the next Step runs the plan.
The Command keeps its Architect run name, progress, logs and notification.
Inspect it with `thirdshift process show architect-run` and validate it with
`thirdshift process check architect-run`. Save a copy under a new name to run
it as a Custom process, which takes only the shared Pass lock, without the
Architect run's other skip gates. Pickup has no Process file: it selects work
and dispatches to a Ticket run (defaulting to `ticket-run`) or a Spec run.

A Custom process can declare a `spec-branch` worktree and use `land-tickets`
to push the Spec branch, run its Tickets in dependency order up to
`spec.parallel` at once, and gather their work into a draft Spec PR with its
Tickets checklist. Its `process` setting selects every child Ticket's Process,
defaulting to `ticket-run`, with the same validation as `run-published-issue`.
The file cannot also declare an `issue-branch` worktree. A Spec-branch file
requires an issue with sub-issues; an Issue-branch file refuses one with them.
The Claim precedes the worktree, and remains while the draft Spec PR waits.
`spec-run.toml` owns the Built-in Spec run's sequence on that worktree:
`land-tickets` → `spec-review` → `security-review` → `delivery` → `self-merge`.
Every starter uses it: an Issue URL, Pickup, Architect and Security runs, and
`run-published-issue`. Spec-or-Ticket dispatch, the Claim, Child asks and Limits
stay in front of the file. Sessions, progress and run-type names stay the same.

After `land-tickets`, a Custom process can run `spec-review` →
`security-review` → `delivery` → `self-merge` on that Spec-branch worktree.
The Spec review uses the same prompt and report contract as a Spec run.
Delivery restores the Tickets checklist and leaves the Spec PR ready,
removing its Claim, or merges it when asked. Each Step takes its own
Harness, Model and Effort; Security review and Self-merge keep their flag
and User config gates, and a refused or incomplete review holds the merge.

`fix-findings` → `close-findings` lets a Custom process fix each pending
reproduced Security finding, then recheck merged fixes against the merged
Base branch before closing their private records. Both keep the Security run's
permission and privacy rules (ADR-0015), run outside parallel groups and use
throwaway worktrees. `fix-findings` takes the same optional `process` setting
as `run-published-issue`, defaulting to `ticket-run`. A failed fix stays with
the Day shift. `security-fix` and `security-close` (or their `no-` forms)
override `[security]` settings. The existing Security run is unchanged and
still fixes at most one finding per invocation.

Command records and Session traces retain the Process name, whether it is
Built-in or Custom, and the SHA-256 hash of its exact file bytes. Each child
Ticket run records its selected Process independently of its parent, defaulting
to `ticket-run`. Custom processes
hash the file loaded for that run; Built-in processes hash the compiled-in
file. Runs awaiting a Built-in file retain their run-type name without a hash.

Inspect and check it with `thirdshift process show ticket-run` and
`thirdshift process check ticket-run issue <issue>`. To run a copy, save the
shown file as `~/.thirdshift/processes/<new-name>.toml`, then use
`thirdshift process <new-name> issue <issue>`. An issue with sub-issues still
starts a Spec run before any Built-in process is selected; the Claim is
made before the Issue-branch worktree is created. Inspect the Spec run with
`thirdshift process show spec-run` and check it with
`thirdshift process check spec-run issue <spec>`. Its name is reserved too.
