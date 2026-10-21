# Viewers for traces and Replay results

Research date: 2026-10-21 (America/New_York).

**Question.** Which pages should show thirdshift's traces and its Replay results? The traces are the Session traces of [ADR 0019](../adr/0019-session-logs-become-session-traces.md). A Replay result is a Baseline arm against a Candidate arm across Eval tasks, with repeated Trials and their Grades. The choices are the **Console**, an existing viewer, or both, with the Console linking to the viewer. This feeds Q4.1 and Q4.12 of the [#698 grilling](../../.scratch/grill-with-docs/698-evals.md). After using the Console, Jacob is open to an existing viewer, but wary of two that overlap and of the upkeep.

The candidates:
- MLflow;
- Phoenix;
- Raindrop Workshop;
- Grafana with Tempo and Loki ([#914](https://github.com/JacobStephens2/thirdshift/issues/914));
- Langfuse;
- promptfoo's viewer;
- Inspect's `inspect view`;
- and, in brief, Braintrust, LangSmith and LangChain.

**Constraints, as settled in the grilling.**
- **Nothing leaves the host but the agent sessions.** No hosted service, no server telemetry, no browser analytics. Data stays on machines Jacob runs, and a Security run's Session traces stay on their own machine.
- **The `thirdshift` host** has 4 cores, 8 GB, a 4 GiB swap file and no Docker. It runs the factory, about two building Runs at once, and Caddy, which serves the Console at `console.thirdshift.app`.
- **`stephens`** has 4 cores, 15,993 MiB, 4.4 GiB of swap with 2.7 GiB in use, 34 GB of free disk and Docker 29.1.3. It runs Apache and factory Pickup runs.
- **A Session trace** is one OTLP JSON Lines file per session:
  - a log record for each Harness event, as it arrives;
  - spans, as they end;
  - the GenAI conventions, `thirdshift.*` attributes and the raw Harness event.

  All of a command's sessions share one trace ID, and an OTLP endpoint can be set.
- **The Console** sits behind its own sign-in and loads nothing from another site. Every view and action it offers is also a CLI command with `--json` ([ADR 0016](../adr/0016-the-console-is-a-long-running-reader.md)).

**Method.**
- **Read-only, from primary sources:**
  - each project's source at its latest release commit;
  - the packages it publishes (PyPI wheels, the npm tarball, a release binary);
  - its official docs, fetched on 2026-10-21;
  - its first-party issue tracker.
- **Nothing was installed, run or signed up for.** Only GET requests left the host. Downloads stayed in the session's scratch folder and were never executed.
- **thirdshift's own code** was read at this branch's `dfadafa`, where `main` was merged on 2026-10-20, after v0.13.0.
- **One read-through per candidate ran in parallel.** The claims that decide fit were then rechecked against the downloaded sources.
- **What earlier notes established isn't repeated here:** [eval-tools.md](eval-tools.md), [eval-tools-landscape.md](eval-tools-landscape.md), and promptfoo's network audit, summarized in the grilling file's "Research results (2026-10-20)".

**Pins.** Citations below link to these commits.

| Candidate | Read at | Released | Commit |
|---|---|---|---|
| MLflow | v3.17.2 | 2026-10-21 | `d0b397f` |
| Phoenix | v20.23.0 | 2026-10-16 | `f4d3998` |
| Raindrop Workshop | v0.1.21, also `main` | 2026-08-22 | `a6b82d7` |
| Grafana, Tempo, Loki | v13.2.4, v3.1.0, v3.7.9 | 2026-10-13, 09-29, 10-20 | `af4a33e`, `77fc456`, `d3b4a2f` |
| Traces Drilldown; Tempo and Loki datasources | v3.0.0; v13.2.4, v13.2.2 | 2026-10-13 | `df90873`; `cdacb74`, `6d076df` |
| Langfuse; its docs | v4.67.0; `main` | 2026-10-21 | `21931f2`; `b0e8f96` |
| promptfoo | 0.124.1, still the latest | 2026-10-08 | `421e795` |
| Inspect AI; its viewer in `ts-mono` | 0.3.282 | 2026-10-19 | `386e099`; `b07f814` |
| Caddy's docs; the CSP Level 3 draft | Caddy v2.11.7 | 2026-10-03 | `1957519`; `b1bad63` |

**Tags.**
- **[interpretation]:** a judgment beyond what the source says.
- **[needs live check]:** only a real run would settle it.
- **[unverified]:** no adequate primary source was found.

## TL;DR

- **No viewer shows a Session trace as written.**
  - MLflow, Phoenix, Langfuse and Workshop accept spans only, so the log records, one per Harness event, never arrive.
  - Grafana keeps them in Loki, a click from the span, and promptfoo turns them into 1 ms spans during its own evals.
  - The others show what the spans carry: GenAI messages and attributes. They show raw events only if thirdshift copies them onto spans, and only MLflow and Phoenix display those (§2.1).
- **No viewer computes a Replay's verdict.**
  - Phoenix's compare view comes closest to Q4.1's Replay page: Eval tasks by Arm, a selector for repeated Trials, each Grade's explanation and a link to each trace, all loaded over REST.
  - MLflow pairs Trials by order, and Langfuse has no repetitions. The rest have no view that pairs Arms.
  - So thirdshift computes Q4.4's interval whichever viewer shows it (§2.2).
- **Fit, by candidate** (§3):
  - **MLflow fits best for traces across Runs.** It is Apache-2.0, one Python process on SQLite, and takes OTLP JSON in. It has GenAI mapping, a Gantt timeline, search, cost, a side-by-side view of two or three traces, and Grades with reasons. Two variables silence it. Its memory beside the factory is unmeasured; the one report is 1.8 GiB idle at four workers.
  - **Phoenix has the best Replay view,** but takes OTLP over HTTP only as protobuf. It listens on every interface by default, is ELv2, and has had eight major versions since February.
  - **Grafana with Tempo and Loki searches best,** but renders nothing GenAI-specific and has no Replay model. By default it truncates attributes over 2,048 bytes, and it is too big for the `thirdshift` host.
  - **Langfuse** needs more than `stephens` has, drops log records, and has no repetitions. Its update check has no off switch.
  - **Workshop is out.** It drops span events and structured attributes, and has no sign-in or Replay model. Its page beacons to raindrop.sh, and its server can browse the host and start agents.
  - **promptfoo's and Inspect's viewers** show Replays, not traces. Inspect's can be exported as static files.
  - **Braintrust and LangSmith** can't run only on Jacob's machines, and **LangChain** is a framework.
- **Sign-in.**
  - The Console's cookie has the `__Host-` prefix, so Caddy's `forward_auth` can check it only for paths on `console.thirdshift.app`. A viewer there runs in the Console's origin.
  - Anywhere else, a viewer needs its own sign-in, Caddy's `basic_auth`, or new single sign-on code in the Console.
  - A viewer without a sign-in of its own also listens open on 127.0.0.1, where a Trial could read earlier Grades **[interpretation]** (§1.3).
- **The Console could match the core of these** in roughly 2,100–4,000 lines with tests: a span tree, span details, search, a two-trace comparison and the Replay pages. That is about its first Spec again **[interpretation]** (§1.5).
- **Recommendation [interpretation].**
  - **Replays in the Console** (Q4.1 (a)), since every viewer leaves the verdict to thirdshift anyway.
  - **Each trace in the Console too,** with its span tree built during the Session-trace rework that Q3.19 already requires.
  - **A viewer only for search across Runs** beyond #813's charts. If one is wanted, trial MLflow on the `thirdshift` host behind its own sign-in, linked by trace ID, after a live memory check. Grafana only if #914 adopts it anyway.
  - Session traces can go to any OTLP endpoint, so adding or dropping a viewer later changes nothing in them.

## 1. The Console today, and what matching the viewers would take

Read at this branch's `dfadafa`.

### 1.1 What it is made of

- **Size.** 3,108 lines of Rust in [`src/console.rs`](../../src/console.rs) and [`src/console/`](../../src/console/), 244 of them unit tests. Beside that:
  - 321 lines of askama templates;
  - 582 lines of its own JavaScript;
  - 463 lines of CSS;
  - 1,533 lines in [`src/logs/status/`](../../src/logs/status/), which the pages share with the CLI.

  Tests add 1,214 lines in `tests/console.rs`, 1,595 in `tests/console/`, and 1,461 lines of Playwright browser tests in `console/tests/`. Confirmed by `wc -l`.
- **How long it took.** 41 commits on 19 and 20 October 2026, from Spec #776 and its Tickets #814–#829. Confirmed by `git log`.
- **Stack.**
  - axum on tokio, with pages rendered on the server from askama templates, SVG bars for time, and small ES modules.
  - htmx 2.0.7, idiomorph 0.7.4 and htmx's SSE extension are vendored, and the fonts are embedded, so the page loads nothing from another site ([assets/README.md](../../src/console/assets/README.md)).
  - Live pages use server-sent events ([logs.rs L141-245](../../src/console/logs.rs#L141-L245)).
- **The views that bear on this decision.**
  - **The Waterfall** draws one Command's Runs, their stages and their tool calls on one time axis ([waterfall.rs](../../src/logs/status/waterfall.rs), 334 lines, and its 188-line [matcher](../../src/logs/status/waterfall/matching.rs)). It nests by Command and stage, not by span. Its tool bars run "until the next matched call … they include waiting and are not measured execution times" ([waterfall.html L19](../../src/console/waterfall.html#L19)).
  - **The log reader** renders Claude's and Codex's events as a conversation and streams new lines by byte offset ([conversation.rs](../../src/console/conversation.rs), 174 lines).
  - **The Stages page** is a table with a column per Run and a row per stage, with each stage's Usual range ([time.html L43-47](../../src/console/time.html#L43-L47)). A Replay's table of Eval tasks by Arm has the same shape **[interpretation]**.
  - **A Peek panel** shows one Command's details beside any list ([chrome.html L1](../../src/console/chrome.html#L1)).
  - **The header already links to other machines' Consoles,** from `console.other_consoles` ([header.html L2](../../src/console/header.html#L2); [config/console.rs L54](../../src/config/console.rs#L54)). A link to a viewer could sit the same way **[interpretation]**.
- **Rules that shape any addition.**
  - Every route needs a paired CLI command; a test fails on any route registered without one, and only static resources are exempt ([console.rs L645-716](../../src/console.rs#L645-L716)).
  - #776's Out of Scope excluded an HTTP API and an SQLite index (grilling file, "Research results (2026-10-19)").

### 1.2 Sign-in, the session cookie and the Console's own CSP

- **Where it listens.** 127.0.0.1, on `console.port`, which defaults to 8711 ([console.rs L292](../../src/console.rs#L292); [config/console.rs L63](../../src/config/console.rs#L63)). On the `thirdshift` host, Caddy serves it as `console.thirdshift.app` with a plain `reverse_proxy` ([install.rs L124-141](../../src/console/install.rs#L124-L141); `/etc/caddy/sites/thirdshift-console.caddy`).
- **Accounts.** An email and password, hashed with Argon2, for each address on `console.allowed_emails`. Sessions are random 64-hex tokens kept in `~/.thirdshift/console-accounts.json`, with a 30-day idle lifetime ([accounts.rs L17](../../src/console/accounts.rs#L17), [L115-127](../../src/console/accounts.rs#L115-L127)).
- **The cookie is `__Host-thirdshift-session`,** set with `Path=/; HttpOnly; Secure; SameSite=Strict` ([auth.rs L19](../../src/console/auth.rs#L19), [L75-82](../../src/console/auth.rs#L75-L82)). The `__Host-` prefix forbids a `Domain` attribute. So browsers send the cookie only to `console.thirdshift.app` itself, never to a sibling such as `traces.thirdshift.app`.
- **The guard.**
  - Every request except `/sign-in`, `/set-password…` and `/assets/…` needs a valid session, or it is redirected to `/sign-in`.
  - Every request other than a GET also needs a CSRF token ([auth.rs L84-151](../../src/console/auth.rs#L84-L151)).
  - Sign-in always redirects to `/` afterwards ([auth.rs L259](../../src/console/auth.rs#L259)).
- **Its CSP is already strict:** `default-src 'self'; script-src 'self'; style-src 'self'; font-src 'self'; connect-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'` ([auth.rs L153-167](../../src/console/auth.rs#L153-L167)). So the Console can't be framed by a viewer, and it can't frame one either, since `frame-src` falls back to `default-src 'self'` **[interpretation]**.

### 1.3 Putting a viewer behind a sign-in

**How Caddy's `forward_auth` works.** It sends a GET clone of each request to an auth endpoint ([forward_auth docs](https://github.com/caddyserver/website/blob/19575198cce1ea5f1bb37895ef3f9f2ec0f1997a/src/docs/markdown/caddyfile/directives/forward_auth.md?plain=1#L31-L43)):
- a 2xx response lets the request through;
- any other response, such as a redirect to a sign-in page, goes back to the browser.

The clone carries the browser's headers, `Cookie` included, because `reverse_proxy` "passes through incoming headers … without modifications", apart from the `X-Forwarded-*` ones ([reverse_proxy docs](https://github.com/caddyserver/website/blob/19575198cce1ea5f1bb37895ef3f9f2ec0f1997a/src/docs/markdown/caddyfile/directives/reverse_proxy.md?plain=1#L502)).

- **On a sibling host name it can't work as the code stands.** The browser never sends `__Host-thirdshift-session` to `traces.thirdshift.app`, so the Console would see every request as signed out.
- **Under a path of `console.thirdshift.app` it could, with three changes [interpretation]:**
  1. A Console route that answers 204 for a valid session, and redirects to `/sign-in` otherwise. The parity test would have to exempt it, like `/assets/`, since it is neither a view nor an action.
  2. The viewer must serve under a path prefix. MLflow, Phoenix and Grafana can. Workshop and Langfuse's published images can't, and promptfoo's paths all start at `/` (§3).
  3. Sign-in must return to the page it interrupted. Today it always goes to `/`.

  The catch is that the viewer's scripts would then run in the Console's origin. Viewers render text from traces, which agents and repositories write, and some render Markdown. An injected script in the viewer could then read any Console page, Security pages included, and post a Stop with the CSRF token that page carries **[interpretation]**.
- **A sibling subdomain keeps the origins apart.** The Console sends no CORS headers, so a script on `traces.thirdshift.app` can't read Console pages or their CSRF tokens. Sharing the Console's sign-in there would need a redirect handshake that sets a cookie for the viewer's host, as oauth2-proxy does. That is new security code in the Console **[interpretation]**.
- **Options that need no Console change:**
  - the viewer's own sign-in, where it has one;
  - Caddy's `basic_auth`, a second password hashed into the Caddyfile ([basic_auth docs](https://github.com/caddyserver/website/blob/19575198cce1ea5f1bb37895ef3f9f2ec0f1997a/src/docs/markdown/caddyfile/directives/basic_auth.md?plain=1#L7-L29));
  - an SSH tunnel, as Q3.12 settled for promptfoo's viewer.
- **Trials could reach a viewer on loopback [interpretation].**
  - A viewer that relies on Caddy for sign-in listens on 127.0.0.1 with none of its own.
  - Q3.6 hides the host's files from a Trial with bwrap, but says nothing of its network. bwrap keeps the host's network unless it is given `--unshare-net` ([bwrap.xml L142-163](https://github.com/containers/bubblewrap/blob/1fb0aa26d3eb66254f7742e704add7f82f41a582/bwrap.xml#L142-L163)), and a Trial's Harness needs the network to reach its provider. So a Trial could query such a viewer for earlier Trials' Grades, the Graders' packets and the success criteria.
  - The Console isn't exposed this way: it asks for a session even on loopback. The exception is `--no-sign-in`, which `console install` uses only where it finds no proxy ([install.rs L69-70](../../src/console/install.rs#L69-L70)).
  - So a viewer holding Replay data needs a sign-in of its own, a Unix socket that only Caddy can open, or Trials with no network but their provider's.

### 1.4 What a CSP added by Caddy can and can't do

- **Two policies both apply.** If a viewer sends its own CSP and Caddy adds another, a request must pass both ([CSP3, "The effect of multiple policies"](https://github.com/w3c/webappsec-csp/blob/b1bad631b091cdca904138d6947a16b89edfd2b4/index.bs#L4952-L4975)). Caddy's `>` prefix replaces the viewer's header instead, once the proxy has written it ([header docs](https://github.com/caddyserver/website/blob/19575198cce1ea5f1bb37895ef3f9f2ec0f1997a/src/docs/markdown/caddyfile/directives/header.md?plain=1#L45-L67)).
- **`'self'` covers same-host WebSockets, but not `blob:`.** It matches `https:` and `wss:` URLs on the page's own host and port, and never a `blob:` URL ([CSP3 §'self' matching](https://github.com/w3c/webappsec-csp/blob/b1bad631b091cdca904138d6947a16b89edfd2b4/index.bs#L4143-L4161)). So a viewer that starts workers from `blob:` URLs needs `worker-src 'self' blob:`.
- **What a CSP blocks.**
  - `connect-src 'self'` stops a page's analytics requests to other hosts, and `script-src 'self'` stops scripts loaded from them.
  - `img-src 'self' data:` stops remote images, including those that rendered trace content could ask for.
  - It also blocks inline scripts without a hash or nonce, and `eval` without `'unsafe-eval'`. That is what breaks most of these viewers (§3).
- **What it can't block:** anything the server sends itself. That includes promptfoo's `r.promptfoo.app` POST and the usage reports from Grafana, Tempo, Loki, MLflow and Langfuse. Only the server's settings or a network block stop those.

### 1.5 What the Console would need to match the best of these [interpretation]

Sizes are rough estimates from the code above, in lines including tests. They don't count the Session-trace work Q3.19 already plans: the reader, the Waterfall's span times, and `show`.

| Addition | What it means here | Rough size |
|---|---|---|
| Span tree with times | `waterfall::collect` grows from Run, stage and tool to any depth: session, model call, tool call and sub-agent. Rows use real span times, fold, and show Model, tokens, cost and errors | 500–900 |
| Span details | The Peek panel shows a span's attributes, its log records (matched by span ID) and the raw Harness event | 200–400 |
| Search across traces | Filters by Harness, Model, Effort, tool, error, duration and tokens, with `thirdshift traces … --json`. Without an index, each query reads every Session trace; an index would reopen #776's Out of Scope | 400–800 |
| Two traces compared | Two trees side by side, each with its time, tokens, cost and tool counts | 300–500 |
| Replays list and Replay page | `thirdshift replays --json`, and a page per Replay: Eval tasks by Arm, each cell its Trials and Grades, reasons on expand, a link from each Trial to its Session trace, and the Q4.4 verdict. The CLI half is needed anyway under Q4.2 (a) | 700–1,400 |

- **Total:** about 2,100–4,000 lines. That is the same order as the Console's first Spec, which took the factory two days in 16 Tickets.
- **What it would still lack:** query languages, saved views, charts, annotation queues and dataset management. The viewers below have those.
- **Upkeep:** every view would carry its CLI pair and its browser tests. That is the maintenance cost Jacob asked about.

## 2. The candidates side by side

Citations for every cell are in §3.

### 2.1 Traces

| | Log records | Span events | GenAI rendering | Tree and times | Tokens and cost | Search across traces | Two traces compared |
|---|---|---|---|---|---|---|---|
| **MLflow** | Not accepted | Kept, Events tab | Mapped at ingest; chat view, but a reasoning part falls back to raw JSON | Tree and Gantt | Tokens; cost from a price catalog, or one sent | Filters, attributes, full text | Two or three, side by side, no diff |
| **Phoenix** | Not accepted, and not planned | Kept, Events tab | Mapped to OpenInference; messages and reasoning | Tree with bars | Tokens; cost from a bundled table, no Grok | Filter expressions | Yes, side by side |
| **Grafana + Tempo + Loki** | In Loki, one click from the span | Kept | None in open source | Tree, minimap, critical path | Raw attributes; TraceQL totals | TraceQL, the strongest | No |
| **Langfuse** | Not accepted | Only GenAI messages, as input and output | Mapped; chat view | Tree, timeline, agent graph | Tokens; cost sent or from a bundled table, no Grok | Filter bar, full text | No |
| **Workshop** | Not accepted | Dropped | Model spans only, not tools or agents | Tree with bars | Input and output tokens; cost from OpenRouter's list | Substring over runs | Only a replay against its source |
| **promptfoo's viewer** | As 1 ms child spans, during its evals | — | None | Tree with bars, in one result's details | Raw attributes | No | No |
| **Inspect's viewer** | Not OTLP: its own log format | — | Its own events | Transcript and swimlane timeline | Tokens and cost | Samples grid across logs | No |

### 2.2 Replay results

| | Arms side by side | Repeated Trials | Grades with reasons | Trial to trace | Loading, without a hosted service |
|---|---|---|---|---|---|
| **MLflow** | Two evaluation runs | Paired by order | Assessments with a rationale | Yes | REST; assessments are `PUBLIC_UNDOCUMENTED` |
| **Phoenix** | Yes, two or more experiments | `repetitions`, with a selector | Evaluations with explanations | Yes | REST, documented |
| **Grafana** | Only as a dashboard you build | Up to the dashboard | Up to the dashboard | Data link | A results file and a datasource plugin |
| **Langfuse** | Yes, against a baseline | Not supported | Scores with a comment | Yes | REST and OTLP; one trace per Trial |
| **Workshop** | No | No | Annotations with a note | — | `POST /api/import-run` |
| **promptfoo's viewer** | Yes, columns | One row per repeat | Assertion reasons | A link in metadata | `promptfoo import` |
| **Inspect's viewer** | One log per Arm | Epochs, grouped | Score explanations | Its own transcript | Its Python API |

### 2.3 Running it, egress and access

| | Install without Docker | Memory evidence | `thirdshift` host? | Server egress off | Browser egress | Own sign-in | Path prefix |
|---|---|---|---|---|---|---|---|
| **MLflow** | pip; heavy | 1.8 GiB idle at 4 workers (one report) | Plausible at 1 worker **[needs live check]** | Two variables | None of its own | Basic auth or OIDC | Yes |
| **Phoenix** | pip; heavy | Helm: 1 GiB request, 2 GiB limit | Plausible, smaller queue **[needs live check]** | One variable | Off with the same variable | Optional | Yes |
| **Grafana + Tempo + Loki** | .deb or binaries | Tempo: 16 GB with Grafana beside it | No | About ten settings | News feed | Yes; auth proxy | Yes |
| **Langfuse** | No documented path | 16 GiB and ~100 GiB of disk | No | One variable, plus a network block | Badge, videos, newsletter | Yes | Source build only |
| **Workshop** | One binary | None | Plausible, but unsafe (§3.5) | Nothing to turn off | Three requests, no switch | No | No |
| **promptfoo's viewer** | npm | None | Plausible | A network guard | PostHog, fixed at build | No | No |
| **Inspect's viewer** | pip, or static files | None | Plausible; static files cost nothing | Nothing to turn off | None | A proxy token | Static: any |

### 2.4 Deep links, licence and what thirdshift would write

| | Opening a trace by its OTLP trace ID | A Replay view | Licence | thirdshift would write |
|---|---|---|---|---|
| **MLflow** | `/#/experiments/<exp>/traces/tr-<id>` | `/#/experiments/<exp>/evaluation-runs?selectedRunUuid=<a>&compareToRunUuid=<b>` | Apache-2.0 | OTLP JSON with an experiment header; REST loader |
| **Phoenix** | `/redirects/traces/<id>` | `/datasets/<d>/compare?experimentId=<a>&experimentId=<b>` | ELv2 | Protobuf sender or a Collector; REST loader |
| **Grafana** | `/explore?…&panes=<JSON with the ID as TraceQL>` | A dashboard with `?var-…` | AGPL-3.0 | Two OTLP JSON endpoints; dashboard JSON |
| **Langfuse** | `/trace/<id>` | `/project/<p>/experiments/results?baseline=<a>&c=<b>` | MIT, except `ee/` | One trace per Trial; REST loader |
| **Workshop** | `/runs/<id>` | — | MIT | Workshop-only attributes |
| **promptfoo's viewer** | — | `/eval/<eval id>`; a comparison can't be linked | MIT | JSON export plus import |
| **Inspect's viewer** | — | `#/logs/<log>/samples/sample/<id>/<epoch>/` | MIT | A Python converter |

## 3. Each candidate

### 3.1 MLflow

**Traces.**
- **Spans only.** The one OTLP route is `POST /v1/traces`, which takes JSON or protobuf ([otel_api.py L104-107](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/otel_api.py#L104-L107); [otlp.py L18-21](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/tracing/utils/otlp.py#L18-L21)). Nothing in the package handles `/v1/logs`.
- **Span events are kept,** and shown in an Events tab for each span ([span.py L605-615](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/entities/span.py#L605-L615); [ModelTraceExplorerEventsTab.tsx L11](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/js/src/shared/web-shared/model-trace-explorer/v2/right-pane/ModelTraceExplorerEventsTab.tsx#L11)).
- **IDs.** A trace's ID is `tr-` followed by its 32-hex OTLP trace ID, so the Console can compute it ([span.py L566-570](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/entities/span.py#L566-L570); [constant.py L203](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/tracing/constant.py#L203)).
- **Ordering.** Spans may arrive in any order. A trace stays in progress until its root span arrives ([sqlalchemy_store.py L5702-5830](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/store/tracking/sqlalchemy_store.py#L5702-L5830)). So the command's root span, built from the Command record (Q4.10), must be sent too **[interpretation]**.
- **GenAI mapping, done at ingest** ([__init__.py L60-149](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/tracing/otel/translation/__init__.py#L60-L149); [genai_semconv.py L26-59](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/tracing/otel/translation/genai_semconv.py#L26-L59)):
  - span types: `chat` becomes a chat-model span, `invoke_agent` an agent and `execute_tool` a tool;
  - input, output and cache tokens are read, but not reasoning tokens;
  - messages, tool arguments and results, Model, provider and conversation ID are read too.
- **`thirdshift.*` attributes** show raw, with JSON strings decoded ([__init__.py L483-500](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/tracing/otel/translation/__init__.py#L483-L500)).
- **Reasoning breaks the chat view.** A message with a reasoning part is rejected, and the view falls back to raw JSON ([otel.ts L58-64](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/js/src/shared/web-shared/model-trace-explorer/chat-utils/otel.ts#L58-L64)). Claude's thinking would trigger this **[interpretation]**.
- **The trace page** has a span tree with a Gantt timeline, tokens and cost for the trace and for each span, and overview charts of requests, latency, errors, tokens, Grades and tool calls ([TimelineTreeGanttBars.tsx](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/js/src/shared/web-shared/model-trace-explorer/v2/timeline-tree/gantt/TimelineTreeGanttBars.tsx); [ModelTraceHeaderDetails.tsx L60-66](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/js/src/shared/web-shared/model-trace-explorer/v2/ModelTraceHeaderDetails.tsx#L60-L66); [trace UI docs](https://mlflow.org/docs/latest/genai/tracing/observe-with-traces/ui/)).
- **Cost** comes from token counts, priced by LiteLLM or MLflow's catalog, unless the span carries `mlflow.llm.cost` ([__init__.py L127-146](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/tracing/otel/translation/__init__.py#L127-L146)).
- **Search** filters on status, times, run, name, span name and type, tags, metadata and Grades, plus full text ([search-traces.mdx L64-78](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/docs/docs/genai/tracing/search-traces.mdx?plain=1#L64-L78)). The source also accepts `span.attributes.<key>` ([search_utils.py L1921-1958](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/utils/search_utils.py#L1921-L1958)).
- **Comparison.** Two or three traces open side by side, read-only, with no diff ([ModelTraceExplorerDetailCompareView.tsx L11-37](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/js/src/shared/web-shared/model-trace-explorer/ModelTraceExplorerDetailCompareView.tsx#L11-L37)).
- **An open bug affects OTLP senders.** In [#24237](https://github.com/mlflow/mlflow/issues/24237), tool and model spans that arrive over OTLP are typed from the untranslated span ([sqlalchemy_store.py L5729](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/store/tracking/sqlalchemy_store.py#L5729)). So they miss span metrics and the tool-call chart, unless the sender sets `mlflow.spanType`. Also open: #26841, #26489 and #26941.

**Replays.**
- **The Evaluation runs tab** lists runs. A run's rows are its traces with their Grades, and a second run can be picked for comparison ([useExperimentEvaluationRunsData.tsx](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/js/src/experiment-tracking/components/experiment-page/hooks/useExperimentEvaluationRunsData.tsx); [useCompareToRunUuid.tsx L4](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/js/src/experiment-tracking/components/evaluations/hooks/useCompareToRunUuid.tsx#L4)).
- **Pairing is by the root span's inputs,** with duplicates paired in order ([GenAiTracesTable.utils.ts L228-270](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/js/src/shared/web-shared/genai-traces-table/GenAiTracesTable.utils.ts#L228-L270)). A TODO there says it "does not work when a single eval run contains the same input ids". So Trial k of one Arm pairs with Trial k of the other. Giving each Trial a root input such as `{eval_task, trial}` keeps that stable **[interpretation]**.
- **No repetitions.** `mlflow.genai.evaluate` has none ([base.py L62-67](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/genai/evaluation/base.py#L62-L67)).
- **A Grade fits an assessment:** a value, a rationale, a source (human, model or code) and metadata, on a trace or a span, and editable on the page ([assessments.proto L97-148](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/protos/assessments.proto#L97-L148)).
- **Loading without Python.**
  - Experiments and runs are public REST calls ([service.proto L60](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/protos/service.proto#L60); [service.proto L187](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/protos/service.proto#L187)).
  - Traces join a run through the `x-mlflow-run-id` header ([otel_api.py L268-274](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/otel_api.py#L268-L274)).
  - Assessments and `link-to-run` are marked `PUBLIC_UNDOCUMENTED` ([service.proto L927-943](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/protos/service.proto#L927-L943); [service.proto L1334-1355](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/protos/service.proto#L1334-L1355)), so they may change without notice **[interpretation]**.

**Running it.**
- **Install.** `pip install mlflow` on Python 3.10 or later. It pulls in pandas, pyarrow, scikit-learn, scipy and more, and only the full wheel ships the page: 26.6 MB in 408 files (PyPI metadata and the wheel).
- **Processes and storage.** It binds 127.0.0.1:5000, runs four uvicorn workers and stores in SQLite. With an SQL store it also starts a job runner, unless `MLFLOW_SERVER_ENABLE_JOB_EXECUTION=false` ([cli_args.py L162-181](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/utils/cli_args.py#L162-L181); [__init__.py L446-449](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/__init__.py#L446-L449); [__init__.py L485-503](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/__init__.py#L485-L503); [__init__.py L372-382](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/cli/__init__.py#L372-L382)).
- **Memory.** No figure is documented. The only measurement is [#22792](https://github.com/mlflow/mlflow/issues/22792), still open. v3.11.1, with four workers, SQLite and jobs on, sat idle at 1,812 MiB, growing about 1.2 MiB an hour; with jobs off it stayed flat. The Helm docs give an example of 512 Mi requested and a 2 Gi limit ([kubernetes-helm.mdx L296-306](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/docs/docs/self-hosting/kubernetes-helm.mdx?plain=1#L296-L306)).
- **Fit [interpretation].** One worker with jobs off is plausible on the `thirdshift` host beside two building Runs **[needs live check]**. On `stephens` it can run as `ghcr.io/mlflow/mlflow:v3.17.2`.

**Egress.**
- **Server telemetry is on by default.**
  - It fetches a config from `config.mlflow-telemetry.io`, then posts batches.
  - They carry a persistent installation ID, the coding agent it detects, and counts such as traces received ([client.py L184-218](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/telemetry/client.py#L184-L218); [installation_id.py L64-70](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/telemetry/installation_id.py#L64-L70); [otel_api.py L62-73](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/otel_api.py#L62-L73)).
  - `MLFLOW_DISABLE_TELEMETRY=true` or `DO_NOT_TRACK=true` stops it ([utils.py L144-161](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/telemetry/utils.py#L144-L161)).
- **The page has no third-party analytics.** Its events go to MLflow's own server. The server forwards them only while telemetry is on and a remote config fetched from a CloudFront host allows it ([handlers.py L8186-8262](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/handlers.py#L8186-L8262)). The docs suggest blocking `mlflow-telemetry.io`, which would miss that host **[interpretation]**.
- **Cost lookups fetch a price catalog from github.com,** before falling back to the bundled one ([providers.py L256-287](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/utils/providers.py#L256-L287)). An empty `MLFLOW_MODEL_CATALOG_URI`, or sending `mlflow.llm.cost`, stops it. LiteLLM, which MLflow prices with when it is installed, isn't among the wheel's base requirements, so leave it out (the wheel's `METADATA`).
- **Nothing is fixed at build time,** and the page loads only its own files. Monaco's default CDN path is overridden ([JsonRecordEditor.tsx L43-47](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/js/src/experiment-tracking/pages/experiment-evaluation-datasets-v2/components/JsonRecordEditor.tsx#L43-L47)).

**Access.**
- **Sign-in.** `--app-name basic-auth` gives a browser prompt with no sign-out and no limit on attempts. An OIDC plugin exists, and the docs describe leaving sign-in to a reverse proxy ([basic-http-auth.mdx L38-44](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/docs/docs/self-hosting/security/basic-http-auth.mdx?plain=1#L38-L44); [sso.mdx L268-276](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/docs/docs/self-hosting/security/sso.mdx?plain=1#L268-L276)).
- **Behind Caddy.** Its middleware refuses other host names and cross-origin writes, so it needs `MLFLOW_SERVER_ALLOWED_HOSTS` and `MLFLOW_SERVER_CORS_ALLOWED_ORIGINS` ([security.py L38-106](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/security.py#L38-L106)). `--static-prefix` mounts it under a path ([__init__.py L435-441](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/cli/__init__.py#L435-L441)).
- **CSP.**
  - MLflow sends none of its own ([security.py L108-126](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/security.py#L108-L126)).
  - Its page has no inline script, and its workers are same-origin. But its CSS-in-JS inserts styles without a nonce, so `default-src 'self'` breaks the styling.
  - `style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:` should work **[needs live check]**.
  - It would block only remote images in rendered content, since there is no browser analytics.

**Deep links.** MLflow uses a hash router ([MlflowRouter.tsx L254-266](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/js/src/MlflowRouter.tsx#L254-L266)):
- a trace: `/#/experiments/<exp>/traces/tr-<trace id>` ([routes.ts L64-69](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/js/src/experiment-tracking/routes.ts#L64-L69); [ExperimentTraceDetailRedirect.tsx L7-48](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/js/src/experiment-tracking/pages/experiment-traces/ExperimentTraceDetailRedirect.tsx#L7-L48));
- two evaluation runs compared: `/#/experiments/<exp>/evaluation-runs?selectedRunUuid=<a>&compareToRunUuid=<b>` ([useSelectedRunUuid.tsx L4](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/js/src/experiment-tracking/components/evaluations/hooks/useSelectedRunUuid.tsx#L4); [useCompareToRunUuid.tsx L4](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/js/src/experiment-tracking/components/evaluations/hooks/useCompareToRunUuid.tsx#L4));
- a session: `/#/experiments/<exp>/chat-sessions/<id>` ([routes.ts L70-75](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/mlflow/server/js/src/experiment-tracking/routes.ts#L70-L75)).

**Licence and health.**
- Apache-2.0 ([LICENSE.txt](https://github.com/mlflow/mlflow/blob/d0b397f85b602e8f146be0d0373b28bba2ca2b74/LICENSE.txt)). Databricks leads it as a Linux Foundation project.
- Releases come every one to four weeks: 3.17.0 on 2026-10-07, 3.17.1 on 10-13 and 3.17.2 on 10-21.
- 520 commits went into `master` in the 30 days to 2026-10-21, and the OTLP handler changed again after 3.17.2 (GitHub API).

**What thirdshift would write [interpretation].**
- Q4.14 (a)'s sender, with the experiment header and configurable headers.
- `mlflow.spanType` on each span.
- Each Harness event repeated as a span event.
- The command's root span.
- A Replay loader over the REST calls above.
- Console links built from the experiment ID and trace ID.

**Against the Console [interpretation].**
- **Better:** search across traces, the Gantt, chat rendering, totals of tokens and cost, charts, and a Grade editor.
- **Costs:**
  - a heavy Python install, upgraded every few weeks;
  - open bugs in OTLP ingest;
  - a second sign-in;
  - undocumented REST for Grades;
  - log records only as duplicated span events.

### 3.2 Phoenix

**Traces.**
- **Spans only, and over HTTP only as protobuf.**
  - `/v1/traces` returns 415 for any other content type ([traces.py L468-472](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/src/phoenix/server/api/routers/v1/traces.py#L468-L472)). gRPC listens on 4317.
  - There is no logs route ([__init__.py L36-94](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/src/phoenix/server/api/routers/v1/__init__.py#L36-L94)). A maintainer wrote on [#10624](https://github.com/Arize-ai/phoenix/issues/10624), on 2025-12-16: "We do not currently have plans to add otel logging primitives to Phoenix."
  - A separate JSON route takes Phoenix's own span format, but skips the GenAI conversion ([spans.py L1421-1437](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/src/phoenix/server/api/routers/v1/spans.py#L1421-L1437); [otel.py L97-107](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/src/phoenix/trace/otel.py#L97-L107)).
- **Trace IDs stay OTLP hex** ([otel.py L136-141](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/src/phoenix/trace/otel.py#L136-L141)).
- **GenAI is converted to OpenInference,** pinned to the conventions' v1.41.1 ([conversion.py L50-51](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/src/phoenix/trace/gen_ai/conversion.py#L50-L51); [conversion.py L289-325](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/src/phoenix/trace/gen_ai/conversion.py#L289-L325); [conversion.py L127-148](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/src/phoenix/trace/gen_ai/conversion.py#L127-L148)):
  - span kinds: `chat` becomes a model call, `execute_tool` a tool and `invoke_agent` an agent;
  - input, output and cache tokens are read, but not reasoning tokens;
  - messages are read, and `gen_ai.conversation.id` becomes a session.
- **The trace page** has a span tree with a timing bar on each row and an Events tab for each span ([TraceTree.tsx L297](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/js/app/src/components/trace/TraceTree.tsx#L297); [SpanDetails.tsx L256-286](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/js/app/src/pages/trace/SpanDetails.tsx#L256-L286)). It renders messages, reasoning and tool calls, and totals tokens and cost per span, trace, session and project.
- **Cost** comes from a price table shipped in the package: 399 models from LiteLLM, with Claude and GPT-5 Codex but no Grok, and no network fetch (read in the published wheel). A cost the Harness reports, such as Claude's `total_cost_usd`, is never used.
- **Search and comparison.** Python-style filter expressions work over spans, traces and sessions ([filter-expressions.mdx](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/docs/phoenix/tracing/how-to-tracing/filter-expressions.mdx)). Two traces open side by side at `/projects/<id>/traces/compare?traceId=A&traceId=B` ([Routes.tsx L176-182](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/js/app/src/Routes.tsx#L176-L182)).

**Replays.** This is the closest match to Q4.1's Replay page.
- **Loading over REST, without Python:**
  - an Eval suite becomes a dataset, through `POST /v1/datasets/upload`;
  - each Arm becomes an experiment with a `repetitions` count;
  - each Trial becomes a run, with its repetition number and `trace_id`;
  - each Grade becomes an evaluation, with the Grader's kind, a score, a label and an explanation ([datasets.py L346-410](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/src/phoenix/server/api/routers/v1/datasets.py#L346-L410); [experiments.py L79-125](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/src/phoenix/server/api/routers/v1/experiments.py#L79-L125); [experiment_runs.py L26-40](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/src/phoenix/server/api/routers/v1/experiment_runs.py#L26-L40); [experiment_evaluations.py L23-50](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/src/phoenix/server/api/routers/v1/experiment_evaluations.py#L23-L50)).
- **The compare view** ([ExperimentComparePage.tsx L96-108](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/js/app/src/pages/experiment/ExperimentComparePage.tsx#L96-L108); [ExperimentCompareTable.tsx L219-249](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/js/app/src/pages/experiment/ExperimentCompareTable.tsx#L219-L249)):
  - Eval tasks in rows, Arms in columns, and a selector to step through repetitions;
  - each Grader's mean and explanation, and a link from each Trial to its trace;
  - optionally, differences from the Baseline and a count of what improved or regressed.
- **Gaps.** It gives no paired verdict, and its compare query isn't paginated ([#16891](https://github.com/Arize-ai/phoenix/issues/16891), open).

**Running it.**
- **Install.** `pip install arize-phoenix` on Python 3.11 to 3.14, with pandas, pyarrow, grpcio and pydantic-ai among its dependencies.
- **Process and storage.** One process with gRPC inside, on SQLite by default ([config.py L3392-3400](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/src/phoenix/config.py#L3392-L3400)).
- **Memory.**
  - No figure is documented. The Helm chart requests 1 GiB and limits it to 2 GiB ([values.yaml L782-788](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/helm/values.yaml#L782-L788)).
  - The ingest queue can hold about 1 GiB of spans, unless `PHOENIX_MAX_SPANS_QUEUE_SIZE` lowers it ([config.py L334-351](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/src/phoenix/config.py#L334-L351)).
  - Out-of-memory reports #9461 and #6417 are closed.
- **Fit [interpretation].** Plausible on the `thirdshift` host with a smaller queue **[needs live check]**. A Docker image serves `stephens`.

**Egress.** All of this is on by default ([config.py L3871-3875](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/src/phoenix/config.py#L3871-L3875)):
- a Scarf tracking pixel on every page load ([index.html L155-157](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/src/phoenix/server/templates/index.html#L155-L157));
- Google Fonts ([index.html L92-96](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/src/phoenix/server/templates/index.html#L92-L96));
- a PyPI version check and a GitHub star count, from the browser ([useLatestPhoenixVersion.ts L45](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/js/app/src/hooks/useLatestPhoenixVersion.ts#L45));
- a WebAssembly runtime downloaded from github.com at startup, which the Docker image ships instead ([_download.py L25-30](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/src/phoenix/server/sandbox/_download.py#L25-L30)).

Other points:
- `PHOENIX_ALLOW_EXTERNAL_RESOURCES=false` turns all of it off at runtime.
- `PHOENIX_TELEMETRY_ENABLED=false` covers only the pixel and FullStory, and FullStory runs only when configured ([config.py L3706-3720](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/src/phoenix/config.py#L3706-L3720)).
- No analytics SDK is in the shipped page.

**Access.**
- **Sign-in is off by default.** When on, it offers passwords (the first login is `admin@localhost` / `admin`), OAuth2/OIDC or LDAP, and no trusted-header option ([authentication.mdx L17-60](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/docs/phoenix/self-hosting/features/authentication.mdx?plain=1#L17-L60)).
- **Defaults that need locking down:**
  - HTTP listens on `0.0.0.0:6006`;
  - gRPC listens on `[::]:4317`, whatever `PHOENIX_HOST` says ([grpc_server.py L107-109](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/src/phoenix/server/grpc_server.py#L107-L109));
  - its MCP server is on, at `/mcp`;
  - cross-site checks run only when `PHOENIX_CSRF_TRUSTED_ORIGINS` is set ([app.py L998-1006](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/src/phoenix/server/app.py#L998-L1006)).
- **Path prefix.** `PHOENIX_HOST_ROOT_PATH` mounts it under a path.
- **CSP.**
  - It sends none of its own, except on a sandbox worker.
  - Two inline scripts and styles inserted at runtime mean `default-src 'self'` breaks it ([index.html L58-91](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/src/phoenix/server/templates/index.html#L58-L91)).
  - Script hashes plus `'unsafe-inline'` styles should work, but the config script's hash changes with the version and settings **[needs live check]**.
  - Live updates use plain HTTP, not WebSockets.

**Deep links.**
- `/redirects/traces/<trace id>` finds the project and opens the trace, so the Console needs only the OTLP trace ID ([Routes.tsx L1180](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/js/app/src/Routes.tsx#L1180); [traceRedirectLoader.ts L10-40](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/js/app/src/pages/redirects/traceRedirectLoader.ts#L10-L40)).
- `/redirects/spans/<span id>` and `/redirects/sessions/<id>` work the same way.
- The compare view is `/datasets/<d>/compare?experimentId=<a>&experimentId=<b>`, with IDs that Phoenix returns.

**Licence and health.**
- Elastic License 2.0, which isn't OSI-approved. Its one limit that matters here bars offering Phoenix to others as a hosted service ([LICENSE L9-14](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/LICENSE#L9-L14)).
- 12 releases between 2026-09-23 and 2026-10-16.
- Eight majors, v13 to v20, between 2026-02-13 and 2026-08-11. v14 removed a `/v1/` REST endpoint ([MIGRATION.md L369-379](https://github.com/Arize-ai/phoenix/blob/f4d3998ef8bef674ca888b2053aa58a178de6909/MIGRATION.md?plain=1#L369-L379)).

**What thirdshift would write [interpretation].**
- A protobuf sender, which the official types make about 17 crates (Q4.14), or an OpenTelemetry Collector that translates the JSON files.
- Events copied into span events.
- A small REST client for Replays.

**Against the Console [interpretation].**
- **Better:** the Replay compare view, filter expressions, comparing two traces, and message rendering.
- **Costs:** protobuf, ELv2, breaking majors, defaults that must be locked down, and a second sign-in.

### 3.3 Grafana, Tempo and Loki

**Traces.**
- **The trace view** has a minimap, a timeline marking the critical path, span details with attributes, events and links, and span filters ([trace-integration.md L60-119](https://github.com/grafana/grafana/blob/af4a33e3786594ad72f72d97e6744010330c2977/docs/sources/visualizations/explore/trace-integration.md?plain=1#L60-L119); [index.tsx L473-600](https://github.com/grafana/grafana/blob/af4a33e3786594ad72f72d97e6744010330c2977/public/app/features/explore/TraceView/components/TraceTimelineViewer/SpanDetail/index.tsx#L473-L600)).
- **Open-source Grafana renders nothing GenAI-specific.** Its attribute groups have no `gen_ai` category ([attributeCategories.ts L34-151](https://github.com/grafana/grafana/blob/af4a33e3786594ad72f72d97e6744010330c2977/public/app/features/explore/TraceView/components/TraceTimelineViewer/SpanDetail/attributeCategories.ts#L34-L151)). Grafana's AI observability is a Grafana Cloud product ([docs](https://grafana.com/docs/grafana-cloud/monitor-applications/ai-observability/)).
- **Tokens** are raw attributes, but TraceQL metrics can total them by Model, as in `{ } | sum_over_time(span.gen_ai.usage.input_tokens) by (span.gen_ai.request.model)` ([functions.md L151-153](https://github.com/grafana/tempo/blob/77fc4563d1135d0ca2656709c7384c278a9c67b0/docs/sources/tempo/metrics-from-traces/metrics-queries/functions.md?plain=1#L151-L153)).
- **Search.** TraceQL filters on any attribute, and on structure ([construct-traceql-queries.md L62-170](https://github.com/grafana/tempo/blob/77fc4563d1135d0ca2656709c7384c278a9c67b0/docs/sources/tempo/traceql/construct-traceql-queries.md?plain=1#L62-L170)).
- **Comparison.** Two traces can't be compared. Explore's split view has no diff, and Traces Drilldown compares attribute distributions, not traces ([analyze-tracing-data.md L55-74](https://github.com/grafana/traces-drilldown/blob/df90873a7e3163c970c9d6abe1b67fc2974fba41/docs/sources/investigate/analyze-tracing-data.md?plain=1#L55-L74)).
- **Log records live in Loki.** "Logs for this span" runs a LogQL query by trace or span ID in a second pane ([configure-trace-to-logs.md](https://github.com/grafana/grafana/blob/af4a33e3786594ad72f72d97e6744010330c2977/docs/sources/datasources/tempo/configure-tempo-data-source/configure-trace-to-logs.md)). Loki puts `trace_id`, `span_id` and the attributes in structured metadata and the body in the log line, and drops the event name ([otlp.go L505-579](https://github.com/grafana/loki/blob/d3b4a2f76bfc5625abee7ceb00b8896fa3c31549/pkg/loghttp/push/otlp.go#L505-L579)).
- **Defaults that would cut thirdshift's data:**
  - Tempo truncates any attribute key or value over 2,048 bytes, on spans, events and links ([_index.md L375-406](https://github.com/grafana/tempo/blob/77fc4563d1135d0ca2656709c7384c278a9c67b0/docs/sources/tempo/configuration/_index.md?plain=1#L375-L406)), and fails any trace over 5 MB ([manifest.md L760](https://github.com/grafana/tempo/blob/77fc4563d1135d0ca2656709c7384c278a9c67b0/docs/sources/tempo/configuration/manifest.md?plain=1#L760)). All of a command's sessions share one trace.
  - Loki rejects a line over 256 KB, structured metadata over 64 KB or 128 entries, and samples older than 7 days ([limits.go L320-323](https://github.com/grafana/loki/blob/d3b4a2f76bfc5625abee7ceb00b8896fa3c31549/pkg/validation/limits.go#L320-L323); [_index.md](https://github.com/grafana/loki/blob/d3b4a2f76bfc5625abee7ceb00b8896fa3c31549/docs/sources/send-data/otel/_index.md)).
  - Tempo keeps traces 14 days ([manifest.md L971](https://github.com/grafana/tempo/blob/77fc4563d1135d0ca2656709c7384c278a9c67b0/docs/sources/tempo/configuration/manifest.md?plain=1#L971)). It indexes spans older than two minutes at their arrival time, so backfilled history would sort by when it was imported ([slackTimeRange.go L9-28](https://github.com/grafana/tempo/blob/77fc4563d1135d0ca2656709c7384c278a9c67b0/tempodb/encoding/common/slackTimeRange.go#L9-L28)).

**Replays.** Grafana has no model for them. They would be a dashboard provisioned from JSON ([index.md L337-520](https://github.com/grafana/grafana/blob/af4a33e3786594ad72f72d97e6744010330c2977/docs/sources/administration/provisioning/index.md?plain=1#L337-L520)), over one of two datasources:
- the community SQLite plugin, v4.0.6 of 2026-05-11, which reads a file on the Grafana host ([README L9](https://github.com/fr-ser/grafana-sqlite-datasource/blob/327ead8e9b08781e731207ee1466569d92b42c8b/README.md?plain=1#L9));
- Grafana's Infinity plugin, which reads over HTTP but not from local files ([query docs L37-56](https://github.com/grafana/grafana-infinity-datasource/blob/5115bbfeec902a292748394703908e3b009f8623/docs/sources/query/_index.md?plain=1#L37-L56)).

The "Grouping to matrix" transformation pivots Eval tasks by Arm, and a data link opens a Trial's trace. This is building a dashboard, not using a feature **[interpretation]**.

**Running it.**
- **Install.** Each installs from a .deb or a binary, with no Docker, and Tempo runs as one process ([linux.md L84-94](https://github.com/grafana/tempo/blob/77fc4563d1135d0ca2656709c7384c278a9c67b0/docs/sources/tempo/set-up-for-tracing/setup-tempo/deploy/locally/linux.md?plain=1#L84-L94)).
- **Memory.**
  - Grafana asks for at least 512 MB, and suggests 2–4 GB for a small install ([_index.md L46-49](https://github.com/grafana/grafana/blob/af4a33e3786594ad72f72d97e6744010330c2977/docs/sources/setup-grafana/installation/_index.md?plain=1#L46-L49); [_index.md L84-88](https://github.com/grafana/grafana/blob/af4a33e3786594ad72f72d97e6744010330c2977/docs/sources/setup-grafana/installation/_index.md?plain=1#L84-L88)).
  - Tempo asks for 4 CPUs and 4–8 GB, and "16 GB of memory or more if … You run additional local components on the same machine, such as Grafana" ([linux.md L31-49](https://github.com/grafana/tempo/blob/77fc4563d1135d0ca2656709c7384c278a9c67b0/docs/sources/tempo/set-up-for-tracing/setup-tempo/deploy/locally/linux.md?plain=1#L31-L49)).
  - Loki gives no figure.
- **A reliability risk.** Open [tempo#8083](https://github.com/grafana/tempo/issues/8083): an unclean shutdown can leave a block that is never searchable and never deleted.
- **Fit [interpretation].**
  - Not the `thirdshift` host.
  - `stephens` only at Tempo's 16 GB threshold, with swap already in use **[needs live check]**.
  - Only with Security sessions filtered out before they leave the `thirdshift` host.

**Egress.** Each item is on by default, and each has a switch.
- **Grafana** ([defaults.ini L388-405](https://github.com/grafana/grafana/blob/af4a33e3786594ad72f72d97e6744010330c2977/conf/defaults.ini#L388-L405); [defaults.ini L489](https://github.com/grafana/grafana/blob/af4a33e3786594ad72f72d97e6744010330c2977/conf/defaults.ini#L489); [defaults.ini L2001](https://github.com/grafana/grafana/blob/af4a33e3786594ad72f72d97e6744010330c2977/conf/defaults.ini#L2001); [defaults.ini L2212-2233](https://github.com/grafana/grafana/blob/af4a33e3786594ad72f72d97e6744010330c2977/conf/defaults.ini#L2212-L2233)):
  - a usage report to stats.grafana.org (`reporting_enabled`);
  - version and plugin update checks (`check_for_updates`, `check_for_plugin_updates`);
  - plugin preinstall from grafana.com at every start, with auto-update (`[plugins] preinstall_disabled`);
  - signing-key downloads (`public_key_retrieval_disabled`);
  - Gravatar (`disable_gravatar`);
  - a news feed the browser fetches from grafana.com (`news_feed_enabled`, plus replacing the home dashboard).

  The Tempo and Loki datasources come bundled, so preinstall can be turned off; Traces Drilldown doesn't, and would have to be installed by hand.
- **Tempo and Loki** each post usage reports to stats.grafana.org ([config.go L18](https://github.com/grafana/tempo/blob/77fc4563d1135d0ca2656709c7384c278a9c67b0/pkg/usagestats/config.go#L18); [reporter.go L44-62](https://github.com/grafana/loki/blob/d3b4a2f76bfc5625abee7ceb00b8896fa3c31549/pkg/analytics/reporter.go#L44-L62)).
- **Browser analytics and Faro** are off unless configured ([init.ts L86-205](https://github.com/grafana/grafana/blob/af4a33e3786594ad72f72d97e6744010330c2977/public/app/core/services/echo/init.ts#L86-L205)).

**Access.**
- **Grafana has its own sign-in.**
- **Auth proxy.** `[auth.proxy]` would trust a header that Caddy sets after `forward_auth`, but then anything that reaches Grafana's port can claim to be any user ([index.md L56-69](https://github.com/grafana/grafana/blob/af4a33e3786594ad72f72d97e6744010330c2977/docs/sources/setup-grafana/configure-access/configure-authentication/auth-proxy/index.md?plain=1#L56-L69)). It would need a Unix socket or a loopback whitelist **[interpretation]**.
- **Tempo and Loki** have no sign-in, and listen on every interface by default.
- **Path prefix.** `root_url` with `serve_from_sub_path` serves Grafana under a path, and `http_addr` or `protocol = socket` keeps it off the network ([defaults.ini L44-66](https://github.com/grafana/grafana/blob/af4a33e3786594ad72f72d97e6744010330c2977/conf/defaults.ini#L44-L66)).
- **CSP.** A Caddy `default-src 'self'` breaks Grafana's inline boot script. Grafana's own `content_security_policy=true` adds a fresh nonce per request, and its template can drop `grafana.com` ([defaults.ini L542-547](https://github.com/grafana/grafana/blob/af4a33e3786594ad72f72d97e6744010330c2977/conf/defaults.ini#L542-L547)).

**Deep links.**
- `/explore?schemaVersion=1&orgId=1&panes=<URL-encoded JSON>` opens a trace, using a TraceQL query of just the trace ID, and can focus a span ([get-started-with-explore.md L153-185](https://github.com/grafana/grafana/blob/af4a33e3786594ad72f72d97e6744010330c2977/docs/sources/visualizations/explore/get-started-with-explore.md?plain=1#L153-L185); [datasource.ts L336-339](https://github.com/grafana/grafana-tempo-datasource/blob/cdacb742bae90fbf62c48c5cca0eacdc08fd2acd/src/datasource.ts#L336-L339)).
- Traces Drilldown takes `?traceId=&spanId=` ([TraceExploration.tsx L90-91](https://github.com/grafana/traces-drilldown/blob/df90873a7e3163c970c9d6abe1b67fc2974fba41/src/pages/Explore/TraceExploration.tsx#L90-L91)).

**Licence and health.**
- AGPL-3.0 for all three.
- Grafana shipped four supported lines on 2026-10-13.
- Tempo 3.0, on 2026-05-28, removed its ingester and compactor ([v3-0.md L251-263](https://github.com/grafana/tempo/blob/77fc4563d1135d0ca2656709c7384c278a9c67b0/docs/sources/tempo/release-notes/v3-0.md?plain=1#L251-L263)).

**What thirdshift would write [interpretation].**
- Spans to Tempo and log records to Loki's `/otlp/v1/logs`, both as OTLP JSON, with the raw event in the log body.
- Raised limits.
- For Replays, a results file and dashboard JSON.

**Against the Console [interpretation].**
- **Better:** search and totals across traces. #914 may want this stack for metrics and alerts anyway.
- **Costs:** three services, about ten egress settings, a second sign-in, and no view of GenAI content or Replays.

### 3.4 Langfuse

**Traces.**
- **No logs route.** Its OTLP folder has only traces and an empty metrics handler ([index.ts L11-20](https://github.com/langfuse/langfuse/blob/21931f2c29ed7d02c106de290e00b67df9049e75/web/src/pages/api/public/otel/v1/metrics/index.ts#L11-L20)).
- **Span events** are read only when they are GenAI message events, which become input and output ([OtelIngestionProcessor.ts L2113-2232](https://github.com/langfuse/langfuse/blob/21931f2c29ed7d02c106de290e00b67df9049e75/packages/shared/src/server/otel/OtelIngestionProcessor.ts#L2113-L2232)).
- **The OTLP trace ID is kept** ([OtelIngestionProcessor.ts L1304-1306](https://github.com/langfuse/langfuse/blob/21931f2c29ed7d02c106de290e00b67df9049e75/packages/shared/src/server/otel/OtelIngestionProcessor.ts#L1304-L1306)).
- **GenAI maps well** ([ObservationTypeMapper.ts L249-268](https://github.com/langfuse/langfuse/blob/21931f2c29ed7d02c106de290e00b67df9049e75/packages/shared/src/server/otel/ObservationTypeMapper.ts#L249-L268); [default-model-prices.json](https://github.com/langfuse/langfuse/blob/21931f2c29ed7d02c106de290e00b67df9049e75/worker/src/constants/default-model-prices.json)):
  - chat, agent and tool types, and `gen_ai.usage.*`;
  - cost from attributes, or from a bundled price table with no Grok;
  - messages rendered as chat.
- **The trace page** has a tree, a timeline, an agent graph, sessions, and a filter bar with full-text search ([TraceTree.tsx](https://github.com/langfuse/langfuse/blob/21931f2c29ed7d02c106de290e00b67df9049e75/web/src/features/traces/components/TraceTree.tsx); [filter-search-bar.mdx L17-87](https://github.com/langfuse/langfuse-docs/blob/b0e8f9693e0ca9cca833f917b44e9da359eef911/content/docs/observability/features/filter-search-bar.mdx?plain=1#L17-L87)). No comparison of two traces was found under `web/src/features/traces`.

**Replays.**
- **The compare view is strong:** a baseline, differences, and a peek at each Trial's trace ([compare-experiments.mdx L13-50](https://github.com/langfuse/langfuse-docs/blob/b0e8f9693e0ca9cca833f917b44e9da359eef911/content/docs/evaluation/experiments/compare-experiments.mdx?plain=1#L13-L50)). Scores carry a `comment` for the reason.
- **No repetitions.** "Langfuse currently assumes that experiments do not contain repetitions" ([data-model.mdx L153](https://github.com/langfuse/langfuse-docs/blob/b0e8f9693e0ca9cca833f917b44e9da359eef911/content/docs/evaluation/experiments/data-model.mdx?plain=1#L153)). So 3 Trials for each of 2 Arms become 6 separate runs.
- **One trace per Trial.** It "expects exactly one trace per experiment item" ([experiments.mdx L29-31](https://github.com/langfuse/langfuse-docs/blob/b0e8f9693e0ca9cca833f917b44e9da359eef911/content/integrations/native/opentelemetry/experiments.mdx?plain=1#L29-L31)), against ADR 0019's one trace ID per command.

**Running it.**
- **Six containers,** with no memory limits: web, worker, ClickHouse 25.12, MinIO, Redis and Postgres ([docker-compose.yml L6-222](https://github.com/langfuse/langfuse/blob/21931f2c29ed7d02c106de290e00b67df9049e75/docker-compose.yml#L6-L222)).
- **Documented size:** "at least 4 cores and 16 GiB of memory", and storage of "e.g. 100GiB" ([docker-compose.mdx L95-97](https://github.com/langfuse/langfuse-docs/blob/b0e8f9693e0ca9cca833f917b44e9da359eef911/content/self-hosting/deployment/docker-compose.mdx?plain=1#L95-L97)).
- **ClickHouse** may take up to 90% of RAM by default ([ServerSettings.cpp L281-296](https://github.com/ClickHouse/ClickHouse/blob/5d20e95da8f21dbab897b3b4196fe901c7bdfc1c/src/Core/ServerSettings.cpp#L281-L296), at v25.12.9.61). Users report an idle ClickHouse at 3.5 GiB resident, its system logs reaching 67 GiB, and Docker's logs filling a disk ([#13123](https://github.com/langfuse/langfuse/issues/13123), [#16339](https://github.com/langfuse/langfuse/issues/16339)).
- **Fit [interpretation].** `stephens` falls short of the documented size on memory and on disk. The `thirdshift` host is out.

**Egress.**
- **Server telemetry** goes to eu.posthog.com every 12 hours. It is on by default and off with `TELEMETRY_ENABLED=false` ([index.ts L20-33](https://github.com/langfuse/langfuse/blob/21931f2c29ed7d02c106de290e00b67df9049e75/web/src/features/telemetry/index.ts#L20-L33)).
- **An update check** goes to langfuse.com on page load, with no switch, so it has to be blocked at the network ([public.ts L33-41](https://github.com/langfuse/langfuse/blob/21931f2c29ed7d02c106de290e00b67df9049e75/web/src/server/api/routers/public.ts#L33-L41)).
- **In the browser:**
  - a shields.io badge;
  - onboarding videos;
  - a newsletter step, which `LANGFUSE_DISABLE_SIGNUP_ONBOARDING=true` skips.
- **PostHog, Sentry and Plain in the browser** are compiled only into Cloud builds ([Dockerfile L162-207](https://github.com/langfuse/langfuse/blob/21931f2c29ed7d02c106de290e00b67df9049e75/web/Dockerfile#L162-L207)).

**Access.**
- Its own sign-in, by email and password or SSO, with sign-up switchable off and no trusted-header option ([authentication-and-sso.mdx L22-24](https://github.com/langfuse/langfuse-docs/blob/b0e8f9693e0ca9cca833f917b44e9da359eef911/content/self-hosting/security/authentication-and-sso.mdx?plain=1#L22-L24); [authentication-and-sso.mdx L261-268](https://github.com/langfuse/langfuse-docs/blob/b0e8f9693e0ca9cca833f917b44e9da359eef911/content/self-hosting/security/authentication-and-sso.mdx?plain=1#L261-L268)).
- Compose publishes its ports on every interface ([docker-compose.yml L120-121](https://github.com/langfuse/langfuse/blob/21931f2c29ed7d02c106de290e00b67df9049e75/docker-compose.yml#L120-L121)), and Docker's published ports bypass ufw ([Docker docs](https://docs.docker.com/engine/network/packet-filtering-firewalls/)).
- Its own CSP allows `'unsafe-inline' 'unsafe-eval'` ([next.config.mjs L51-67](https://github.com/langfuse/langfuse/blob/21931f2c29ed7d02c106de290e00b67df9049e75/web/next.config.mjs#L51-L67)).

**Deep links.**
- `/trace/<trace id>` redirects to the trace ([getTraceRedirectPageServerSideProps.ts L13-37](https://github.com/langfuse/langfuse/blob/21931f2c29ed7d02c106de290e00b67df9049e75/web/src/features/traces/server/getTraceRedirectPageServerSideProps.ts#L13-L37)).
- A comparison is `/project/<p>/experiments/results?baseline=<a>&c=<b>` ([experimentUrlTranslation.ts L1-44](https://github.com/langfuse/langfuse/blob/21931f2c29ed7d02c106de290e00b67df9049e75/web/src/features/experiments/utils/experimentUrlTranslation.ts#L1-L44)).

**Licence and health.** MIT outside `ee/` ([LICENSE](https://github.com/langfuse/langfuse/blob/21931f2c29ed7d02c106de290e00b67df9049e75/LICENSE)). ClickHouse owns it. There were ten releases in the eight days to v4.67.0.

### 3.5 Raindrop Workshop

**What it is.**
- Raindrop's local trace debugger, "The local debugger your agent is missing". It ships as the `raindrop` binary's `workshop` command, at localhost:5899 ([README.md L5-11](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/README.md?plain=1#L5-L11); [README.md L113-134](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/README.md?plain=1#L113-L134)).
- **Raindrop Cloud** is a separate hosted product. The same binary connects to it only after `raindrop login` ([README.md L49-102](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/README.md?plain=1#L49-L102)).
- It isn't Braintrust's or LangChain's.

**Traces.**
- **OTLP traces only,** as JSON or protobuf; there is no logs route ([server.ts L653-662](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/src/server.ts#L653-L662)).
- **What it drops:** array and map attribute values, span events other than an exception message, resource attributes, and cache and reasoning tokens ([parse.ts L20-28](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/src/parse.ts#L20-L28); [parse.ts L110-122](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/src/parse.ts#L110-L122); [schema.ts L40-41](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/src/db/schema.ts#L40-L41)).
- **GenAI.** It recognises GenAI model spans and messages, but not `execute_tool` or `invoke_agent` ([parse.ts L144](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/src/parse.ts#L144)).
- **Stored but never shown:** other attributes, `thirdshift.*` among them ([schema.ts L42](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/src/db/schema.ts#L42)).
- **The page** has a span tree with timing bars and a chat view. Cost comes from OpenRouter's price list, fetched by the browser ([costs.ts L8-35](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/app/src/utils/costs.ts#L8-L35)).
- **Comparison.** Only a replay beside the run it came from ([replay.ts L126](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/src/replay.ts#L126)); two arbitrary traces can't be compared.

**Replays.**
- No datasets, Arms or Trials.
- Annotations, of kind issue, good or note and with text, can sit on a run or a span ([schema.ts L127-142](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/src/db/schema.ts#L127-L142)).
- Its "replay" re-sends a conversation to your own agent server. That is unrelated to thirdshift's Replay.

**Running it.**
- One 107 MB binary built with Bun, on SQLite, with no Docker ([build-bun.ts L181-182](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/scripts/build-bun.ts#L181-L182)).
- The default `raindrop setup` adds Workshop's MCP server and skills to every coding agent it finds ([init.ts L139](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/src/init.ts#L139)). On the `thirdshift` host, that would change every Harness session **[interpretation]**.
- No memory figures were found, in the docs or the issues.

**Egress.**
- **The server** makes no calls by default.
- **The page makes three,** with no switch at runtime:
  - Google Fonts ([index.html L8-9](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/app/index.html#L8-L9));
  - OpenRouter's model list ([costs.ts L10](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/app/src/utils/costs.ts#L10));
  - a hidden 1×1 image from `raindrop.sh` on every page, naming the version. Its host is fixed at build time ([vite.config.ts L17-23](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/app/vite.config.ts#L17-L23)).
- **Its Ask pane** runs `claude -p --permission-mode bypassPermissions`, or Codex with its sandbox bypassed ([claude-cli-chat.ts L46-84](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/src/claude-cli-chat.ts#L46-L84)).

**Access.**
- Loopback only, with Host and Origin checks. It has no sign-in and no path prefix ([server.ts L114-136](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/src/server.ts#L114-L136)).
- **Even behind a sign-in,** anyone who signs in could use it to browse the host's directories and read any project's Claude transcripts ([server.ts L1391-1408](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/src/server.ts#L1391-L1408); [server.ts L1626-1650](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/src/server.ts#L1626-L1650)).
- Open [PR #30](https://github.com/raindrop-ai/workshop/pull/30) reports a path that executes shell commands.

**Deep links.** `/runs/<trace id>` and `/runs/<trace id>/span/<span id>` ([router.tsx L100-105](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/app/src/router.tsx#L100-L105)).

**Licence and health.**
- MIT ([LICENSE](https://github.com/raindrop-ai/workshop/blob/a6b82d71596cf7e8974e5efae63f9e15260d76ef/LICENSE)).
- 22 releases between 2026-05-14 and 2026-08-22, and none since.
- Its public repository looks like a mirror of releases, with outside pull requests closed **[interpretation]**.

### 3.6 promptfoo's viewer

What follows is new beyond the network audit.

- **Traces.**
  - A Traces tab sits inside one result's details. It nests spans by parent and shows bars and raw attributes ([TraceTimeline.tsx L94-127](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/app/src/components/traces/TraceTimeline.tsx#L94-L127)).
  - No route opens a trace by ID, and nothing renders GenAI content.
  - Its receiver takes `/v1/logs`, as JSON only. Each record becomes a 1 ms child span of the span that emitted it, with the body capped at 8,192 characters ([otlpReceiver.ts L427-451](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/tracing/otlpReceiver.ts#L427-L451); [otlpReceiver.ts L765-846](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/tracing/otlpReceiver.ts#L765-L846)).
- **Replays.**
  - Arms are columns, each headed by its pass rate and cost ([evaluator.ts L2345-2366](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/evaluator.ts#L2345-L2366)).
  - Each `--repeat` is its own row, not grouped ([evaluator.ts L2700-2721](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/evaluator.ts#L2700-L2721)).
  - Each assertion shows its pass, score and reason ([EvaluationPanel.tsx L305-311](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/app/src/pages/eval/components/EvaluationPanel.tsx#L305-L311)).
  - A URL in metadata renders as a link, which could point at the Console ([MetadataPanel.tsx L12-19](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/app/src/pages/eval/components/MetadataPanel.tsx#L12-L19)).
  - `promptfoo import` loads a JSON export, traces included. But it is itself a CLI run, so it makes the `r.promptfoo.app` POST too ([import.ts L103-112](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/commands/import.ts#L103-L112)).
- **CSP.**
  - The shipped page has one inline script, whose hash is fixed per release; no `eval`; and socket.io on the same origin.
  - So `script-src 'self' 'sha256-…'; connect-src 'self'`, with inline styles allowed, should block PostHog's requests and the scripts it injects **[needs live check]**.
  - No CSP stops the server's own POST.
  - The server listens on every interface, with no option to change it ([server.ts L548](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/server/server.ts#L548)).
- **Deep links.** `/eval/<eval id>`, with `?mode=failures` and `?search=`. A comparison can't be linked ([routes.tsx L53-55](https://github.com/promptfoo/promptfoo/blob/421e7959642c5d4cc1c983259a268de1c6f847b9/src/app/src/routes.tsx#L53-L55)).

### 3.7 Inspect's `inspect view`

- **Traces.**
  - It reads Inspect's own eval logs, not OTLP.
  - A sample's transcript holds model events with usage and cost, tool events, and nested spans. Since 0.3.253 it also has a timeline of agent swimlanes ([_model.py L85-145](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/event/_model.py#L85-L145); [_tool.py L13-71](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/event/_tool.py#L13-L71); [CHANGELOG.md L644](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/CHANGELOG.md?plain=1#L644)).
  - thirdshift would write a Python converter through `write_eval_log`, as the docs advise ([eval-logs.qmd L100-103](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/eval-logs.qmd?plain=1#L100-L103)).
- **Replays.**
  - Epochs are Trials, grouped by sample, and scores carry explanations ([log-viewer.qmd L181-185](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/log-viewer.qmd?plain=1#L181-L185); [_metric.py L112-126](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/scorer/_metric.py#L112-L126)).
  - There is no paired comparison: one log per Arm, and a samples grid across logs ([columns.tsx L184-222](https://github.com/meridianlabs-ai/ts-mono/blob/b07f814c50bc3fdeb141d2ba5b31a37d43a12b9d/apps/inspect/src/app/shared/samples-grid/columns.tsx#L184-L222)).
- **Running and access.**
  - One uvicorn process on 127.0.0.1:7575.
  - Or `inspect view bundle`, which writes static files that Caddy can serve behind the Console's sign-in ([view.py L170-195](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/_cli/view.py#L170-L195)).
  - Its proxy-token mode drops the restriction to the log folder ([fastapi_server.py L804-810](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/_view/fastapi_server.py#L804-L810)), so the static bundle is safer **[interpretation]**.
- **Egress and CSP.**
  - No telemetry and no analytics.
  - 0.3.282's page needs blob workers, `new Function` and inline styles.
  - The next release ships a strict CSP ([content-security-policy.json](https://github.com/UKGovernmentBEIS/inspect_ai/blob/7a83ea99eedbf2effaca3f68a21d753b2fd41bdc/src/inspect_ai/_view/dist/content-security-policy.json)).
- **Deep links.** Hash routes such as `#/logs/<log>/samples/sample/<id>/<epoch>/` ([url.ts L99-121](https://github.com/meridianlabs-ai/ts-mono/blob/b07f814c50bc3fdeb141d2ba5b31a37d43a12b9d/apps/inspect/src/app/routing/url.ts#L99-L121)).
- **Licence and health.** MIT. 12 releases in the three and a half weeks to 0.3.282.

### 3.8 Braintrust, LangSmith and LangChain

**Braintrust** is a proprietary eval and observability platform, and it can't run only on Jacob's machines ([self-hosting](https://www.braintrust.dev/docs/admin/self-hosting)).
- Self-hosting is Enterprise-only, and covers only the data plane: the API, Postgres, Redis, object storage and Brainstore.
- Braintrust keeps hosting the control plane: "the web UI, authentication, user management, and metadata storage", such as project and experiment names.
- The browser loads Braintrust's page and talks to the data plane by CORS, and sign-in goes through Clerk.
- By default the data plane reports health, resource metrics and billing usage to Braintrust.
- Its SDK (Apache-2.0) and `autoevals` Graders (MIT) are open; its page is worth looking at only for ideas **[interpretation]**.

**LangSmith** is LangChain's tracing and eval platform. It is out unless Jacob buys an air-gapped Enterprise licence **[interpretation]**.
- Self-hosting is "an add-on to the Enterprise plan", on Kubernetes, with a licence key ([self-hosted](https://docs.langchain.com/langsmith/self-hosted)).
- Unless offline, it needs egress to `beacon.langchain.com`: "Billing telemetry cannot be disabled", and the licence is checked at startup and periodically after ([egress](https://docs.langchain.com/langsmith/self-host-egress)).
- Running fully offline needs an "offline (air-gapped) license" from sales.

**LangChain** is an MIT framework for building agents, alongside LangGraph for orchestration. It doesn't bear on this decision **[interpretation]**.
- LangSmith doesn't need it: it traces agents built with any framework ([overview](https://docs.langchain.com/oss/python/langchain/overview)), and takes OpenTelemetry ([OTel](https://docs.langchain.com/langsmith/trace-with-opentelemetry)).
- LangSmith Studio, formerly LangGraph Studio, works only with apps that "implement the Agent Server API protocol" ([Studio](https://docs.langchain.com/langsmith/studio)). thirdshift's sessions are external CLI Harnesses, not LangGraph graphs.

## 4. What would change the decision

- **A viewer starts accepting OTLP log records.** MLflow, Phoenix, Langfuse or Workshop adding `/v1/logs` would let it show Session traces as written. Phoenix has said it has no plans to (§3.2).
- **Q4.14 chooses protobuf.** Sending with the official types, rather than JSON, would make Phoenix as cheap to feed as MLflow.
- **Memory measured beside two building Runs** on the `thirdshift` host:
  - MLflow with one worker and jobs off;
  - Phoenix with a smaller queue.
- **Jacob wants search across Runs.** Questions such as "which sessions spent the most tokens in tool X" go beyond #813's charts. If they matter, a viewer earns its upkeep. If not, the Console is enough.
- **#914 adopts Grafana** for metrics and alerts. Tempo and Loki would then add little, though their limits and the Security-session filter still apply.
- **Langfuse adds repetitions** (discussion [#5855](https://github.com/orgs/langfuse/discussions/5855)) and a logs route, or a larger second machine appears.
- **How Jacob weighs the sign-in options:** a second password, against mounting a viewer in the Console's origin, against new single sign-on code (§1.3).
- **The runner choice, due next round:**
  - If promptfoo is dropped, its viewer is moot.
  - If Inspect is chosen, its static bundle becomes a cheap per-Trial transcript viewer beside the Console's Replay page.

## 5. Uncertainties and live checks

- **[needs live check]** The memory each candidate uses on the `thirdshift` host beside two building Runs. No candidate documents one; the only measurement is MLflow's #22792, on a different setup.
- **[needs live check]** Each suggested CSP against the real page: MLflow, Phoenix, Workshop, promptfoo, Langfuse and Grafana.
- **[needs live check]** That MLflow's Evaluation runs view pairs Trials as expected when traces are linked by header. And how a session's roughly 250 span events render there.
- **[needs live check]** Whether a Trial in bwrap can reach 127.0.0.1, which decides the leak in §1.3.
- **[interpretation]** The sizes in §1.5 are estimates from the existing code, not from Tickets.
- **Sources read but not run.** Every egress finding comes from source and shipped files. A packet capture of each candidate, once configured, would confirm it.
- **Versions move.** MLflow, Phoenix and Langfuse each released within the week before this note. Recheck the pins before building.

## Sources

thirdshift: [ADR 0016](../adr/0016-the-console-is-a-long-running-reader.md), [ADR 0019](../adr/0019-session-logs-become-session-traces.md), [ADR 0015](../adr/0015-security-findings-stay-private-until-fixed.md), [the #698 grilling](../../.scratch/grill-with-docs/698-evals.md), [eval-tools.md](eval-tools.md), [eval-tools-landscape.md](eval-tools-landscape.md), [`src/console/`](../../src/console/), [`src/logs/status/`](../../src/logs/status/).

Source at the pinned commits:
- [MLflow `d0b397f`](https://github.com/mlflow/mlflow/tree/d0b397f85b602e8f146be0d0373b28bba2ca2b74)
- [Phoenix `f4d3998`](https://github.com/Arize-ai/phoenix/tree/f4d3998ef8bef674ca888b2053aa58a178de6909)
- [Workshop `a6b82d7`](https://github.com/raindrop-ai/workshop/tree/a6b82d71596cf7e8974e5efae63f9e15260d76ef)
- [Grafana `af4a33e`](https://github.com/grafana/grafana/tree/af4a33e3786594ad72f72d97e6744010330c2977), [Tempo `77fc456`](https://github.com/grafana/tempo/tree/77fc4563d1135d0ca2656709c7384c278a9c67b0), [Loki `d3b4a2f`](https://github.com/grafana/loki/tree/d3b4a2f76bfc5625abee7ceb00b8896fa3c31549)
- [Traces Drilldown `df90873`](https://github.com/grafana/traces-drilldown/tree/df90873a7e3163c970c9d6abe1b67fc2974fba41), [Tempo datasource `cdacb74`](https://github.com/grafana/grafana-tempo-datasource/tree/cdacb742bae90fbf62c48c5cca0eacdc08fd2acd)
- [Langfuse `21931f2`](https://github.com/langfuse/langfuse/tree/21931f2c29ed7d02c106de290e00b67df9049e75), [langfuse-docs `b0e8f96`](https://github.com/langfuse/langfuse-docs/tree/b0e8f9693e0ca9cca833f917b44e9da359eef911)
- [promptfoo `421e795`](https://github.com/promptfoo/promptfoo/tree/421e7959642c5d4cc1c983259a268de1c6f847b9)
- [Inspect AI `386e099`](https://github.com/UKGovernmentBEIS/inspect_ai/tree/386e09942510e93de2db6d108f24f62f65eecbdc), [ts-mono `b07f814`](https://github.com/meridianlabs-ai/ts-mono/tree/b07f814c50bc3fdeb141d2ba5b31a37d43a12b9d)
- [Caddy website `1957519`](https://github.com/caddyserver/website/tree/19575198cce1ea5f1bb37895ef3f9f2ec0f1997a), [CSP Level 3 `b1bad63`](https://github.com/w3c/webappsec-csp/tree/b1bad631b091cdca904138d6947a16b89edfd2b4), [bubblewrap `1fb0aa2`](https://github.com/containers/bubblewrap/tree/1fb0aa26d3eb66254f7742e704add7f82f41a582)
- [ClickHouse v25.12.9.61 `5d20e95`](https://github.com/ClickHouse/ClickHouse/tree/5d20e95da8f21dbab897b3b4196fe901c7bdfc1c), [grafana-sqlite-datasource `327ead8`](https://github.com/fr-ser/grafana-sqlite-datasource/tree/327ead8e9b08781e731207ee1466569d92b42c8b), [grafana-infinity-datasource `5115bbf`](https://github.com/grafana/grafana-infinity-datasource/tree/5115bbfeec902a292748394703908e3b009f8623)

Docs fetched on 2026-10-21:
- [MLflow tracing UI](https://mlflow.org/docs/latest/genai/tracing/observe-with-traces/ui/)
- [Grafana Cloud AI observability](https://grafana.com/docs/grafana-cloud/monitor-applications/ai-observability/)
- [Braintrust self-hosting](https://www.braintrust.dev/docs/admin/self-hosting)
- [LangSmith self-hosted](https://docs.langchain.com/langsmith/self-hosted) and [its egress](https://docs.langchain.com/langsmith/self-host-egress)
- [LangChain overview](https://docs.langchain.com/oss/python/langchain/overview), [LangSmith with OpenTelemetry](https://docs.langchain.com/langsmith/trace-with-opentelemetry), [LangSmith Studio](https://docs.langchain.com/langsmith/studio)

Issues: [langfuse#13123](https://github.com/langfuse/langfuse/issues/13123), [langfuse#16339](https://github.com/langfuse/langfuse/issues/16339), [mlflow#22792](https://github.com/mlflow/mlflow/issues/22792), [mlflow#24237](https://github.com/mlflow/mlflow/issues/24237), [phoenix#10624](https://github.com/Arize-ai/phoenix/issues/10624), [phoenix#16891](https://github.com/Arize-ai/phoenix/issues/16891), [tempo#8083](https://github.com/grafana/tempo/issues/8083), [langfuse discussion #5855](https://github.com/orgs/langfuse/discussions/5855), [workshop#30](https://github.com/raindrop-ai/workshop/pull/30).
