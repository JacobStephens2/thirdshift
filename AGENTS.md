## Agent skills

### Issue tracker

Issues and specs live as GitHub issues and are managed with the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Five canonical roles, each label string equal to its name: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `CONTEXT.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.

### Generated prompts

`prompts/` is generated from the prompt module, `src/prompt.rs`, with each prompt's title and when sentence from `src/prompts_page.rs`: don't edit it. Edit the prompt module (or `src/prompts_page.rs`) and regenerate with `UPDATE_PROMPTS=1 cargo test prompts_page`.
