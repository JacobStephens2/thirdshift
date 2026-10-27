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

No Built-in process file ships yet. The `ticket-run` file follows in #1463.
