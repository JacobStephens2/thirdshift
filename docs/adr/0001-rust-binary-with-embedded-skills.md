# Rust single binary with the factory skills embedded

The factory is a Rust binary that compiles `skills/` into itself (`include_dir!`). At runtime it writes the skills and a generated `.claude-plugin/plugin.json` to a temp directory and loads them with `claude --plugin-dir`. We chose this over a TypeScript script that reads the skills from this repo on disk, so the binary is self-contained: it runs on any machine with `claude`, `gh` and `git`, and no checkout of this repo is needed.

## Consequences

- Editing a skill has no effect until the binary is rebuilt and reinstalled (`cargo install --path .`).
- The skills are loaded as a plugin, so they are namespaced (`/thirdshift:implement`), and their references to each other must use that namespace.
