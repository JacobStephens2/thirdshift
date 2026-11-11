# Each Console serves an API that can do what the CLI can, behind scoped tokens

A Run started by hand on another machine never appeared on the Console the **Day shift** watches (#1142). The Day shift wants one Console to see and run the whole **Fleet**: the ETA servers, `thirdshift` and stephens. So each **Console** serves a versioned HTTP API, `/api/v1`. Every endpoint is the twin of a CLI command, takes the same arguments and returns the same JSON as that command's `--json`. The Console page becomes one client of the API, authenticated by its sign-in. Other Consoles, scripts and agents authenticate with a token that the machine issues to a named holder.

The API can do what the CLI can, starting Commands and changing the User config included. That supersedes ADR 0016's "it never starts work" and "no HTTP API is promised to anyone". The Console still never starts work of its own accord. Every Command it starts is asked for by a signed-in person or a token holder, the same as a typed command, so ADR 0009 still holds: thirdshift never schedules itself. Each endpoint behaves exactly like its CLI twin, the **Command limit** included, and a Command started through the API runs detached, so a Console restart or update doesn't end it.

Tokens carry the weight (#1142 Q3.1 to Q3.5):
- **Scopes:** `read`, `security` (Security run details and findings), `act` (Stop, Dismissal, the eval judgments), `run` (start Commands), `admin` (User config, `update`) and `report` (upload only the token holder's own named **Run metrics** summary when that machine cannot be reached inbound, #2036 Q4.2). `read` is the default.
- **Handling:** 256 random bits, shown once and kept by the issuer only as a hash. The holder keeps it in its **Credentials**. Tokens are accepted over HTTPS only (except 127.0.0.1), named and listed with their last use, revocable at once, optionally given an expiry, and rate-limited after failures. Every call that changes anything goes in the **Console log**.
- **People:** each address on a Console's sign-in list carries scopes of its own, all five viewer scopes (`read`, `security`, `act`, `run`, `admin`) unless the User config narrows them (#1142 Q4.1, answering #952). A Console calling on someone's behalf names the person signed in. The owning machine refuses the call unless that person is on its own sign-in list with the scope the call needs, and its **Activity log** names both the person and the machine.
- **CLI-only, even with `admin`:** the following exclusions are checked against the full CLI command inventory (#1775). The machine's own shell stays the root of trust, so the API can't widen its own access.
  - `console token`: Token management stays in the shell so the API cannot widen its own access.
  - `console scopes`: Address scopes stay in the shell so the API cannot widen its own access.
  - `console fleet add`: Fleet pairing stays in the shell so the API cannot change trusted destinations or their tokens.
  - `console fleet list`: Fleet pairing metadata is inspected through the shell alongside pairing and removal.
  - `console fleet remove`: Fleet removal stays in the shell alongside pairing and its Credentials changes.
  - Details commands with `--machine`: Fleet detail forwarding uses the owning machine's API directly; API reads stay local and never relay through saved Fleet tokens.
  - `cronjobs --fleet`: Fleet schedule aggregation is an explicit client of owning-machine inventories; the API stays local and cannot lend saved Fleet credentials.
  - `metrics --fleet`: Fleet metrics aggregation is an explicit client of owning-machine metrics and cached peer snapshots; the API stays local and cannot lend saved Fleet credentials.
  - `metrics --push`: Outbound metrics reporting sends this machine's local summary to a configured Console using saved credentials; the receiving Console exposes the report endpoint directly.
  - The Console sign-in list and `console sign-out`: Console sign-in administration, including revoking sign-ins, stays in the shell.
  - `console install`: Console installation writes the machine's proxy and service setup.
  - `setup`: Interactive setup bootstraps the machine and its Console sign-in list.
  - `console password`: Console passwords require the machine's shell or the separate sign-in flow.
  - `logs --export`: Log export writes to a caller-selected local path.
  - `evals prepare`: Eval preparation writes a repository to a caller-selected local path.
  - `evals prepare-arms`: Arm preparation writes its record to a caller-selected local path.
  - `console`: Console serving bootstraps the API listener itself; it is not a client operation.
  - `--trial`: The hidden Trial protocol consumes a local request and writes local Trial artifacts.
  - `--oom-exec`: The hidden OOM wrapper replaces its process inside an existing Trial scope.
  - `--release-summary`: The hidden release-summary protocol belongs to the local release script's stdin exchange.

## Considered Options

- **Links only** (`other_consoles`). Rejected: it wouldn't have shown the work running elsewhere, which is the whole complaint.
- **A read-only summary API.** It was the first recommendation. Rejected for fleet control: the Day shift wants to Stop, dismiss and start work on any machine from one Console.
- **Summaries over SSH** (`ssh <machine> thirdshift status --json`). Rejected: a long-running web process would hold a passphrase-less key with a full shell on every machine, which is far more than a scoped, revocable token grants.
- **Logs synced to one host, or a centralized PostgreSQL server.** Rejected: each host keeps its own logs and **Command records** (#994, #2036 Q2.1), avoiding external database infrastructure and offline write queues.
- **The browser carrying each Console's own sign-in.** Rejected: it means a sign-in per machine and cross-site cookies, and leaves the CLI nothing to authenticate with.

## Consequences

- A leaked token with `run` spends Harness subscriptions and pushes code on the issuing machine, and one with `admin` reconfigures it. Scopes and per-holder revocation are the mitigation, not an absence of capability.
- A Security run's details, findings included, reach any Console holding a `security` token, fetched live and never stored there. They stay behind sign-ins and off GitHub, so ADR 0015 holds. #776's constraint that Security runs stay on their own machine does not.
- Compact **Run metrics** bucket summaries fetched from **Fleet** peers—or pushed outward under a `report` token by a sleeping or NATted machine—are cached per peer on the watching Console so offline, sleeping or retired machines stay visible read-only without syncing raw logs or running a database (#2036 Q2.1, Q4.2).
- **Other-account Commands** stay read-only (#1446). A machine acts as its serving account, whoever asks through the API.
- The CLI-and-page pairing test (ADR 0016) covers three ways in: CLI, page and API.
- The API runs on the Console's existing `axum` server, and a published OpenAPI description, generated from the same route table, is tested against the routes.
