## Agent skills

### Issue tracker

Issues and specs live as GitHub issues and are managed with the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Five canonical roles, each label string equal to its name: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. When publishing Specs or Tickets, read `docs/agents/triage-labels.md` and follow its publication procedure; it overrides imported Day shift skills' readiness and parent-modification defaults.

### Domain docs

Before exploring, look up the issue's terms via `docs/agents/domain.md` and its domain-to-code index. Single-context: `GLOSSARY.md` and `docs/adr/` at the repo root.

### Reading and tests

Use `rg -n` for terms/symbols, then bounded ranges (`sed -n '40,100p' file`), one large file per call, especially for `GLOSSARY.md`, `README.md`, `tests/fakes/gh.rs` and `tests/spec_run.rs`. For the fake-test layout, see `docs/code-navigation.md`; for site browser-test setup and runner, see `site/tests/README.md`.

### Generated prompts

`prompts/` and the generated fragment in `site/prompts/index.html` come from `src/prompts_page.rs`: prompts in `src/prompt.rs`, titles/when sentences in `src/prompts_page.rs`, plus session arguments (`src/harness/claude.rs`, `src/harness/codex.rs`) and embedded Factory skills (`skills/`) for the site. Edit the sources and regenerate with `UPDATE_PROMPTS=1 cargo test prompts_page`; verify with `cargo test prompts_page`. Preserve the site's surrounding hand-written HTML.
