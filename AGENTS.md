## Agent skills

### Issue tracker

Issues and specs live as GitHub issues and are managed with the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Five canonical roles, each label string equal to its name: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. When publishing Specs or Tickets, read `docs/agents/triage-labels.md` and follow its publication procedure; it overrides imported Day shift skills' readiness and parent-modification defaults.

### Domain docs

Before exploring, look up the issue's terms via `docs/agents/domain.md` and its domain-to-code index. Single-context: `GLOSSARY.md` and `docs/adr/` at the repo root.

### Reading and tests

Use `rg -n` for terms/symbols, then bounded ranges (`sed -n '40,100p' file`), one large file per call, especially for `GLOSSARY.md`, `README.md`, `tests/fakes/gh.rs` and `tests/spec_run.rs`. For the fake-test layout, see `docs/code-navigation.md`; for site browser-test setup and runner, see `site/tests/README.md`.

For final local validation, run `scripts/affected-tests <base>` (normally `main`); see `docs/testing.md` for selection, setup and CI responsibilities. This binary-only crate's targeted selectors are `cargo test --bin thirdshift <module>::tests` and `cargo test --test <file>`; it has no lib target.

### Generated prompts

`prompts/` and the generated fragment in `site/prompts/index.html` come from `src/prompts_page.rs`: prompts in `src/prompt.rs`, titles/when sentences in `src/prompts_page.rs`, plus session arguments (`src/harness/claude.rs`, `src/harness/codex.rs`) and embedded Factory skills (`skills/`) for the site. Edit the sources and regenerate with `UPDATE_PROMPTS=1 cargo test prompts_page`; verify with `cargo test prompts_page`. Preserve the site's surrounding hand-written HTML.

### Generated user manual

`site/docs/index.html` comes from `docs/guide.md` and `site/generate-docs.mjs`. Edit the sources and regenerate with `npm run docs --prefix site`; verify with `npm run docs:check --prefix site`. See `site/tests/README.md` for browser checks.

### Hand-written site pages

`site/index.html`, `site/how-it-works/` and `site/architecture/` are written by hand, and nothing checks them against the code. When a change alters user-visible behavior, search them for its issue number, commands and glossary terms, and correct what they say in the same pull request. A new top-level command or workflow gets a mention where those pages cover its neighbours.
