# PROTOTYPE: Console looks for #711 (Q27)

Throwaway. Eighteen Console variants on one page, switched with `?variant=` and the red bar at the bottom (or ← and →). All data is made up and lives in memory: Stop and sign-in are stubs.

```sh
python3 .scratch/prototypes/711-console/serve.py
```

It is Python's static server with caching turned off, so a reload always shows the latest files. Then open <http://localhost:8711/>, which starts on the shortlist: `console` (themes), `inboxpress`, `inbox`, `lanes`, `lanespress`, `gridpress`. Keys, ranked best first: `console`, `inboxpress`, `inbox`, `lanes`, `lanespress`, `gridpress`, `pressplain`, `briefingwide`, `briefing`, `trace`, `grid`, `accent`, `press`, `presslayout`, `plain`, `garden`, `pixelpress`, `pixel`.

- `phone.html`: every variant side by side at 390×844 (`?only=<key>` for one), a quick stand-in for the device toolbar.
- `data.js`: the made-up Commands and two Session logs, in Claude Code `stream-json` and Codex `exec --json` shapes.
- `shared.js`: state, ages, the Stop and sign-in stubs, and Session log folding (tool calls grouped, thinking collapsed, a renderer per tool). The live Session log appends a line every 4 s.
- `variants/<key>.js` and `.css`: each variant's own layout.

Assumption: the "requeue instead" choice is shown as `thirdshift stop --requeue <issue>`; that flag does not exist yet.

`docs/research/console-run-monitoring-ui.md` is the research behind the `trace` and `grid` variants and the shared revisions: the "needs you" strip, silence (not stage age) as the stall rule, "usually" ranges, failures open by default, two fold levels at most, what Stop keeps, tokens, and the Session log's search, raw view and new-entries pill.

`docs/research/software-factory-console-trends.md` is the research behind the `inbox` variant and the tab title's "‹n› need you" count and urgent dot, which every variant shares.
