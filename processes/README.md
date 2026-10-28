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
and Limits. Every Ticket run executes this file, including those started by
Spec, Architect and Security runs. The optional Security review and Self-merge keep
their command and User config gates. Commands still appear as Ticket runs.

Command records and Session traces retain the Process name, whether it is
Built-in or Custom, and the SHA-256 hash of its exact file bytes. Each child
Ticket run records `ticket-run` independently of its parent. Custom processes
hash the file loaded for that run; Built-in processes hash the compiled-in
file. Runs awaiting a Built-in file retain their run-type name without a hash.

Inspect and check it with `thirdshift process show ticket-run` and
`thirdshift process check ticket-run issue <issue>`. To run a copy, save the
shown file as `~/.thirdshift/processes/<new-name>.toml`, then use
`thirdshift process <new-name> issue <issue>`. An issue with sub-issues still
starts a Spec run before any Built-in process is selected; the Claim is
made before the Issue-branch worktree is created.
