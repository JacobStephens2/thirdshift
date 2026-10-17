# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

## Before exploring, look up the issue's terms

- Search **`CONTEXT.md`** for the issue's terms with `rg -n`, then read the matching definitions and nearby context in bounded ranges (for example, `sed -n '65,78p' CONTEXT.md`). If **`CONTEXT-MAP.md`** exists, use it to locate the relevant context's glossary first.
- Follow the separate [domain-to-code index](../code-navigation.md) to the owning modules and supporting tests. Keep implementation paths there; glossary definitions stay implementation-free. Search the named symbols with `rg -n` and inspect bounded ranges before widening the read.
- Check **`docs/adr/`** for decisions that touch the area you're about to work in and read the applicable ADRs. In multi-context repos, also check `src/<context>/docs/adr/` for context-scoped decisions. Targeted reading still includes applicable coding standards.

If any of these files don't exist, **proceed silently**. Don't flag their absence; don't suggest creating them upfront. The `/domain-modeling` skill (reached via `/grill-with-docs` and `/improve-codebase-architecture`) creates them lazily when terms or decisions actually get resolved.

## File structure

Single-context repo (most repos):

```
/
├── CONTEXT.md
├── docs/adr/
│   ├── 0001-event-sourced-orders.md
│   └── 0002-postgres-for-write-model.md
└── src/
```

Multi-context repo (presence of `CONTEXT-MAP.md` at the root):

```
/
├── CONTEXT-MAP.md
├── docs/adr/                          ← system-wide decisions
└── src/
    ├── ordering/
    │   ├── CONTEXT.md
    │   └── docs/adr/                  ← context-specific decisions
    └── billing/
        ├── CONTEXT.md
        └── docs/adr/
```

## Use the glossary's vocabulary

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a test name), use the term as defined in `CONTEXT.md`. Don't drift to synonyms the glossary explicitly avoids.

If the concept you need isn't in the glossary yet, that's a signal: either you're inventing language the project doesn't use (reconsider) or there's a real gap (note it for `/domain-modeling`).

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than silently overriding:

> _Contradicts ADR-0007 (event-sourced orders), but worth reopening because…_
