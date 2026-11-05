# Grilling: #1142, See every machine's Commands from one Console

Issue: https://github.com/JacobStephens2/thirdshift/issues/1142
Research: [ADR 0016](../adr/0016-the-console-is-a-long-running-reader.md), [ADR 0015](../adr/0015-security-findings-stay-private-until-fixed.md), #1446 (Other-account Commands)

## Round 1 (answered)

**Q1.1 - How do the other machines' Commands reach this Console?** This is the trunk: the sign-in question, the machine list and the CLI pair all hang off it. ADR 0016 says the Console does nothing the CLI can't and "no HTTP API is promised to anyone", and `thirdshift status --json` already gives every Command with its `hostname`, kind, issue, stage and outcome. The options:
- (a) **Links only.** The Inbox lists the other Consoles (`other_consoles` already puts them in the header), and nothing is fetched. This would not have fixed the 2026-10-23 miss: nothing on the first Console said that work was running elsewhere.
- (b) **Summaries over HTTP.** Each Console serves a signed-in, read-only summary that the others fetch and merge in. This needs a machine-to-machine credential and amends ADR 0016's "no HTTP API".
- (c) **Logs synced to one host.** This conflicts with #994 (each host keeps its own logs) and with ADR 0015's spirit.
- (d) **Summaries over SSH.** This Console (and `status`) runs `ssh <machine> thirdshift status --json` for each listed machine and merges the results, labelled by machine. The pair is the CLI's own command, sign-in is the operator's existing SSH key, and no new HTTP surface or credential exists. The cost: the Console's account needs a non-interactive SSH login to each machine, and the remote `thirdshift` must be new enough.

*Recommended:* (b), now that you're open to an HTTP API. It means amending ADR 0016's "no HTTP API is promised to anyone" for this one read-only summary. (b) and (d) carry the same data, and least privilege decides between them: (d) needs a passphrase-less SSH key giving a full shell on every other machine, held by a long-running web process. (b) needs only a read-only token that each machine issues and can revoke, which opens nothing else and works on a host where you have no shell. The CLI pair is `thirdshift status` fetching the same summary. (a) stays as the fallback for a machine the Console can't reach.

*Answer:* agree, I'm trying to decide whether to keep the scope of the HTTP API more limited here to just what the consoles would need which it sounds like it's probably just read-only but alternatively I'm thinking about extending the command line interface principle of anything that you can do with third shift you can do either well that you can do through the CLI.

So for example, everything you see in the console is information you can get or things you can do on the command line and sell in a similar way that approach could be taken to the A to the API that anything you can read or do with third shift like being able to do it through the API I think could have a lot of flexibility and so I'm I'm drawn to that option.

Even if it means an additional spec and tickets that are done before the spec with tickets related specifically to being able to connect or have one console web UI that displays the activity from multiple hosts. 

**Q1.2 - Which machines belong in the view?** The candidates are the Day shift's own machines (`thirdshift` and stephens), plus the two ETA servers that run thirdshift under other people's accounts. The options:
- (a) Only machines listed in this machine's User config, so the operator chooses. In practice that means `thirdshift` and stephens, with the ETA servers left out.
- (b) Machines discovered from GitHub: Pickup reservations (which record `Host:`) and Claims. Discovery could find a machine but not reach it, and Claims don't name their host.

*Recommended:* (a). It's an explicit list, like `other_consoles` and `other_accounts`, and the ETA servers stay out unless someone with access there adds them. A machine that is unreachable or out of date is shown as such on the page and never breaks it.

*Answer:* Agree, in my case I'm going to ultimately want to see the stephens, thirdshift, and ETA servers all together in one view.

**Q1.3 - Read-only across machines?** The issue sets "Stop is local" as a constraint. #1446 made Other-account Commands readable but never stoppable or dismissable.

*Recommended:* Yes, read-only. There's no Stop and no Dismissal on another machine's row, and the CLI refuses both with a message naming the machine and the command to run there. A Dismissal is per machine because it lives in that machine's logs, so the row's Dismiss control lives on the owning Console only.

*Answer:* Thinking back to my answer to question 1.1, I think that I do want there to be a stop and dismissal for in this console web UI for any of the hosts that it is displaying four. So this this means you could just use one console and control your whole fleet of third shift operations from that single location without having to switch tabs or switch consoles. 

**Q1.4 - How much of each machine shows: the whole Inbox, or only what asks for attention?** The 2026-10-23 miss was Working rows on stephens that didn't appear on `thirdshift`. The options:
- (a) Every Inbox group, with each row tagged by machine and merged into the same groups, and the tab title's "need you" count including them. Details (waterfall, logs, Session traces) open on the owning Console via a link, and never get copied.
- (b) Only Needs you and Working.
- (c) A separate "Other machines" section under the local Inbox.

*Recommended:* (a). It mirrors how #1446 merged Other-account Commands, so the Inbox stays one list of what asks something of the Day shift, whichever machine it's on.

*Answer:* agree

## Round 2 (answered)

**Q2.1 - How wide is the HTTP API: the Console's needs, or everything thirdshift can do?** You're drawn to the CLI principle turned outward: anything you can read or do with thirdshift, you can do through the API. "Everything" covers three layers, and each crosses a bigger line than the one before:
- (a) **Everything the Console reads and does:** the Inbox and status, a Command's stages and logs, Stop, Dismissal and Undo, and the eval judgments (scoring a Run, confirming a Bug origin, approving an Eval task). Every call is the HTTP twin of an existing CLI command with `--json`. Nothing starts work, so ADR 0016's "it never starts work" holds and only its "no HTTP API" line is amended.
- (b) **(a) plus starting Commands:** a Ticket run on an Issue URL, a Pass, a Custom process. This overturns ADR 0016's first reason and the spirit of ADR 0009, which says the operating system's scheduler or a person starts every Command. A leaked token then spends Harness subscriptions and pushes code, instead of only stopping work.
- (c) **(b) plus the machine itself:** writing the User config and running `update`, `setup` and `console install`. A leaked token then reconfigures the machine.

*Recommended:* (a) now, built so it can grow. Each endpoint is the CLI command's twin, with the same name and the same `--json` body, so adding one later is mechanical. Starting work stays a separate decision with its own ADR, made once (a) has run for a while. (a) already gives you fleet control (Q1.3). (b) is a one-way door on trust, and ADR 0016's three reasons are what made a long-running Console acceptable.

*Answer:* c, I'm going to rely on strong token security to protect this, but I want the flexibility that comes with a comprehensive HTTP API.

**Q2.2 - How does one Console prove itself to another?** Q1.3 makes the credential write-capable: whoever holds it can stop any Command on the machine that issued it. The options:
- (a) **A token per pair of machines.** The owning machine issues it from the CLI (for example `thirdshift console token create thirdshift`, which names who will hold it), and can list and revoke it there. The asking machine keeps it in its **Credentials** file, never in the User config. Each token carries a scope, `read` or `act`, so a machine can be shown without being controllable.
- (b) **The browser carries each Console's own sign-in.** The page fetches every Console directly, with your sign-in on each one. There's no machine credential, and every action is tied to a Console account. But you'd sign in once per machine, each Console needs cross-site cookies and CORS, and `thirdshift status` in a terminal has nothing to sign in with, which breaks the CLI pair.
- (c) **One shared fleet secret** in every machine's Credentials. It's simplest, but one leak opens every machine, and you can't revoke a single machine.

*Recommended:* (a). It's revocable per machine and scoped, and the CLI uses the same token. That fits ETA servers, where you may want `read` only, or someone else holds the issuing account.

*Answer:* agree

**Q2.3 - Who does the owning machine's Activity log name for a Stop or Dismissal from another Console?** Today it names "who asked and whether from the CLI or the Console". The options:
- (a) The signed-in Console account on the asking machine, plus the machine, e.g. "Stop by jacob@stephens.page from the Console on `thirdshift`". The owning machine trusts the asking machine's claim about who is signed in, because it trusts that machine's token.
- (b) The asking machine only, e.g. "Stop from the Console on `thirdshift`".

*Recommended:* (a). The Activity log keeps saying which person acted. The claim is only as good as the token, and the log says so by naming the machine too.

*Answer:* agree

**Q2.4 - Do a Command's details come across too, or only its row?** In Q1.4 you agreed that details open on the owning Console. In Q1.3 you said you want one Console "without having to switch tabs". The options:
- (a) **Rows only.** A row's details link to the owning Console, which asks you to sign in there.
- (b) **Live details for ordinary Commands.** The stage waterfall, the Command log and Session traces are fetched live through the API when you open them, and never stored on the asking machine. A **Security run** shows its row and stages only, and its details link out, so its traces and anything near a finding stay put (ADR 0015).
- (c) Everything, Security runs included.

*Recommended:* (b). It gives the one-Console experience you described, and Security runs are the line ADR 0015 draws. "Fetched, never stored" keeps #994's "each host keeps its own logs" intact.

*Answer:* agree, except for the Security run recommendation - instead I want to be able to do everything from the central console on each host which I can do from a host's local console.

*Checked:* ADR 0015 governs what goes public on GitHub, not which machine shows what. The "Security runs stay on the machine that ran them" constraint came from #776's Console Spec, so your answer overturns #776's constraint, not ADR 0015. A finding write-up shown on the central Console stays behind sign-ins and off GitHub, which keeps ADR 0015 intact.

**Q2.5 - A name for the set of machines one Console shows.** You said "fleet". The glossary has no term yet, and **Console** currently says "each machine serves its own Console" and "shows the serving OS account's Commands". The options:
- (a) **Fleet:** the machines a Console shows, its own and each one its User config lists with a token. _Avoid_: cluster, network, hosts. A Command already names its machine, so a row needs no new term of its own.
- (b) Something else you prefer.

*Recommended:* (a). It's your word, and fleet-management tools use it in the same sense (devices managed from one place). I'll update **Console** and add **Fleet** to `GLOSSARY.md` once you agree.

*Answer:* Agree

**Q2.6 - Does the fleet list replace `other_consoles`?** `other_consoles` is a list of header links that are never fetched. The options:
- (a) **Replace it.** One list under `[console]`, each entry a name and an address. An entry with a token in the Credentials is fetched and merged into the fleet. One without a token stays a header link, as today. Existing `other_consoles` values are read as link-only entries.
- (b) Keep `other_consoles` for links and add a separate list for fleet machines.

*Recommended:* (a). Having two lists of the same Consoles invites drift, and "a token makes it live" is the whole difference between the two.

*Answer:* agree

**Q2.7 - How fresh is another machine's view?** The local Inbox streams live. The GitHub read happens once a minute. The options:
- (a) Fetch each fleet machine every 15 seconds, only while a Console page is open. Each machine's rows show how old they are, and a machine that hasn't answered for a minute is shown as unreachable, with its last rows greyed.
- (b) Fetch all the time, even with no page open.
- (c) A live stream from each machine.

*Recommended:* (a). It costs nothing while nobody is looking, and the tab title count and browser notifications only matter while a page is open. A live stream is a later improvement, if 15 seconds feels slow.

*Answer:* agree

## Round 3 (answered)

Q2.1 (c) makes the API able to start work and change the machine itself. It rests on "strong token security", so this round is mostly about what that means.

**Q3.1 - What can a token be allowed to do?** A token's scopes decide what a holder can do on the machine that issued it. The options:
- (a) **Five scopes:** `read` (every view, except Security run details), `security` (Security run details and findings), `act` (Stop, Dismissal and Undo, and the eval judgments), `run` (start Commands), and `admin` (User config, `update`). The issuer picks the scopes when it issues the token, and `read` is the default. Your own machines' Consoles get all five, while an ETA server could give out `read` and `act` only.
- (b) Two scopes, `read` and `all`.
- (c) No scopes. Every token can do everything.

*Recommended:* (a). Each scope matches a different kind of harm if the token leaks: what you see, the private findings, stopping work, spending Harness subscriptions and pushing code, and changing the machine. `security` is separate so a fleet member can be fully controllable without showing its findings to every Console that lists it.

*Answer:* agree

**Q3.2 - Strong token security: what's in it?** Here is what I'd count as "strong". Strike or change any line:
- **Generated by the issuer:** 256 random bits, shown once when issued, and kept on the issuer only as a hash.
- **Stored by the holder:** in its **Credentials** file (owner-only, mode 0600), never in the User config.
- **HTTPS only:** a token sent over plain HTTP is refused, except to 127.0.0.1.
- **Named and listed:** each token names its holder. `thirdshift console token list` shows its scopes, when it was issued and when it was last used.
- **Revocable:** revoking a token takes effect on the next request.
- **Logged:** every call that changes anything (`act`, `run`, `admin`) is written to the issuer's **Console log**. Reads aren't logged, as today.
- **Optional expiry:** a token can be given an expiry date with `--expires`, and has none by default.
- **Brute force:** failed token attempts are rate-limited per address, like failed sign-ins.

*Recommended:* all eight as written.

*Answer:* agree

**Q3.3 - Does the owning machine check the person a Console names?** In Q2.3 you agreed the Activity log names the person signed in on the asking Console. But the asking Console's sign-in list may not match the owning machine's list. For example, someone allowed on `thirdshift`'s Console would gain control of an ETA server. The options:
- (a) The owning machine refuses any call that names a person missing from its own Console sign-in list (`allowed_emails`). A person you haven't allowed on a machine can't reach it through someone else's Console.
- (b) The token alone decides, and the named person is only for the log.

*Recommended:* (a). It keeps each machine's sign-in list meaning what it says, and costs nothing on your own machines, where the list is the same.

*Answer:* agreed

**Q3.4 - Who besides Consoles may hold a token?** A comprehensive API invites other clients, such as a script, an agent, or the CLI on a laptop. The options:
- (a) Any named holder. A Console is just one kind. A call that names no person is attributed in the Activity log to the token's holder, for example "Stop by token `laptop-cli`".
- (b) Consoles only.

*Recommended:* (a). It's the flexibility you asked for in Q2.1, and the token's name keeps every action traceable.

*Answer:* agreed

**Q3.5 - What stays off the API, even with `admin`?** Some things can't, or shouldn't, be done remotely:
- **Issuing and revoking tokens, and changing `allowed_emails`.** If the API could do these, a stolen `admin` token could mint itself a lasting replacement or add an attacker's sign-in. ADR 0016 already keeps the allowlist out of the page's reach.
- **`console install`:** its last step is a `sudo` script, and the Console has no root.
- **`setup`:** it's interactive. Its effect is the same as setting User config values, which `admin` already covers.

*Recommended:* all three stay CLI-only on the machine itself. That makes the machine's own shell the root of trust, and the API can never widen its own access.

*Answer:* agreed

**Q3.6 - One API for both the page and other clients?** The options:
- (a) The Console page becomes a client of the same API. Its sign-in session is one more way to authenticate, alongside tokens, and each endpoint returns the same JSON as its CLI twin's `--json`, under a versioned path (`/api/v1/...`) so machines on different versions of thirdshift can still talk.
- (b) A separate API beside the page's own routes.

*Recommended:* (a). There's one surface to secure and test, and ADR 0016's pairing test extends naturally to three ways in: CLI, page and API.

*Answer:* agree

**Q3.7 - Write an ADR for this now?** It's hard to reverse, surprising without context ("why does a long-running web process start agent sessions?"), and the result of a real trade-off (Q2.1). It would supersede part of ADR 0016 (its first reason, and "no HTTP API") and record why ADR 0009's concern is still met.

*Recommended:* Yes. I'd draft `docs/adr/00NN-each-console-serves-an-api-that-can-do-what-the-cli-can.md` once Q3.1 to Q3.5 settle, and amend ADR 0016 to point to it.

*Answer:* agreed

## Round 4 (answered)

The ADR is drafted: `docs/adr/0026-each-console-serves-an-api-that-can-do-what-the-cli-can.md`, with ADR 0016 amended to point at it. What's left is how a signed-in person's power is set, how machines are paired, and how the work splits into Specs.

**Q4.1 - What can a person signed in to a Console do?** Q3.6 makes the page an API client authenticated by its sign-in, so a sign-in needs scopes too. Today every allowed address can do everything the page offers. With `run` and `admin`, that means starting work and rewriting the User config. Open #952 asks for this same thing: "control which have write and read access" when several people can sign in. The options:
- (a) Every signed-in person gets all five scopes, minus the Q3.5 exclusions.
- (b) Each address on the Console sign-in list carries its own scopes. The list is still changed only from the CLI (Q3.5). An address with no scopes listed gets all five, so your own sign-in works as before. Combined with Q3.3, the owning machine checks the person's scopes there too, so a person with `read` on an ETA server can't stop work there from `thirdshift`'s Console.
- (c) A sign-in gets everything except `admin`, which stays with the CLI and tokens.

*Recommended:* (b). It answers #952, it keeps today's behaviour for a one-person list, and it makes the fleet safe to share with the ETA servers' other users.

*Answer:* agreed

**Q4.2 - How do two machines get paired?** Here is a flow for the Spec to build:
1. On the owning machine: `thirdshift console token create <holder> --scopes read,security,act,run,admin [--expires <date>]` prints the token once.
2. On the holder: `thirdshift console fleet add <name> <address>` asks for the token on standard input (never as an argument, so it stays out of shell history and `ps`), checks it by calling `/api/v1/whoami`, then saves the entry to the User config and the token to the Credentials.
3. `thirdshift console fleet list` shows each entry with its scopes, version and last contact. `fleet remove` drops the entry and its token.

*Recommended:* this flow as written.

*Answer:* agreed

**Q4.3 - How does the work split into Specs?** `docs/agents/triage-labels.md` says to split a Spec where later work needs earlier work released and installed. A Fleet can't be checked live until every machine runs a thirdshift with the API and has issued its tokens. The options:
- (a) **Four issues:**
  - **Spec A, the Console API:** `/api/v1`, the page as a client, tokens and their CLI, per-person scopes (Q4.1), and the `read`, `security` and `act` endpoints.
  - **Release step** (`ready-for-human`, blocked by A): release, install on every Fleet machine, and pair the machines (Q4.2).
  - **Spec B, the Fleet in the Console:** the list replacing `other_consoles`, the 15-second fetch, merged Inbox rows, live details, and Stop and Dismissal across machines, with their CLI twins (`status`, `--machine`).
  - **Spec C, starting work and admin over the API:** the `run` and `admin` endpoints, and their Fleet controls, blocked by A. It can run alongside B, and it absorbs #952's "readable and writeable User config".
- (b) One big Spec.
- (c) A and B only, leaving `run` and `admin` until fleet viewing has been used for a while.

*Recommended:* (a). Fleet viewing and control, the thing #1142 asked for, lands without waiting on the riskier `run` and `admin` endpoints, and C still follows straight after.

*Answer:* agreed, and running to-spec then to-tickets skills for each spec to create the specs and tickets

**Q4.4 - What becomes of #1142?** The options:
- (a) #1142 becomes Spec B: retitled, with the agreed scope as its body, linked to A, the release step and C, and still `needs-triage` until its Tickets are published.
- (b) #1142 is closed in favour of new issues.

*Recommended:* (a). It keeps the history and this grilling's link with the issue that started it.

*Answer:* agree

**Q4.5 - Can the Inbox hide a machine?** With five or more machines the Inbox gets long. `[console] hidden_sections` already hides groups, and #1219 toggles repository rows on Stages. The options:
- (a) A machine filter on the Inbox, kept in the URL like the page keys, so you can bookmark "ETA only". Nothing is saved to the User config.
- (b) A saved `hidden_machines` setting.
- (c) Neither, for now.

*Recommended:* (a). It's cheap and easy to reverse, and the default stays "everything".

*Answer:* agreed

## Technology (2026-11-05, settled after Round 4)

The Console's existing stack already covers the API, so no new web framework comes in (ADR 0016: async code lives only inside the Console):
- **Server:** `axum` 0.8 on `tokio`, with `/api/v1` on the Console's existing router. The page and the API share handlers (Q3.6).
- **Client:** `reqwest` 0.13 with rustls. The CLI (`status`, `--machine`, `fleet add`) uses its blocking client, and the Console's 15-second Fleet fetch uses its async client.
- **Tokens:** generated from 32 random bytes with `rand`, and stored as their SHA-256 (`sha2`). A 256-bit random token needs no slow hash. `argon2` stays for passwords.
- **JSON:** the CLI's own `--json` structs through `serde_json`, so the API and the CLI can't drift apart.
- **OpenAPI:** Spec A publishes an OpenAPI description, generated (e.g. with `utoipa`) from the same route table the pairing test walks. A test keeps the description and the routes in step, so scripts and agents can discover the API.
- **Rate limiting:** a small in-memory, per-address limiter written in thirdshift rather than a new dependency, covering failed tokens and failed passwords (see the Q3.2 correction).

## Settled by earlier answers (shout if any is wrong)

- ~~From the issue and ADR 0015: a Security run's Session traces and finding write-ups never leave the machine that ran them.~~ Overturned by Q2.4. They're fetched live to the asking Console and never stored there (see the Q2.4 check).
- From ADR 0016: whatever the Console shows from other machines, the CLI (`thirdshift status`) can show too.
- From Q1.1: each Console serves an HTTP API that other Consoles (and the CLI) call. ADR 0016's "no HTTP API is promised to anyone" is amended. How wide the API is goes to Q2.1.
- From Q1.1: you'd accept the API as a Spec of its own that lands before the fleet Console's Spec. Splitting it up comes in a later round, once Q2.1 settles.
- From Q1.2: the fleet is an explicit list in the User config, not discovered from GitHub. It will include stephens, `thirdshift` and the ETA servers.
- From Q1.2: a machine that is unreachable, or runs a thirdshift too old for the API, is shown as such and never breaks the page.
- From Q1.3: the issue's constraint "Stop is local" is overturned. A Console offers Stop and Dismissal on every fleet machine's rows, and the CLI does the same (for example `thirdshift stop --machine <name>`).
- From Q1.3: the owning machine's own thirdshift carries out a remote Stop or Dismissal. The asking Console never signals another machine's processes or writes into its logs, so the Dismissal and the Activity log line stay where they are today, on the owning machine.
- From Q1.3 and #1446: **Other-account Commands** stay read-only. A fleet machine acts as its serving account, so another account's Commands there still can't be stopped from any Console.
- From Q1.4: every Inbox group merges every fleet machine's rows, each tagged with its machine, and the tab title's "need you" count includes them.
- From Q2.1: the API covers everything the CLI does, including starting Commands and changing the machine (Q3.5 asks about the few exceptions). This overturns ADR 0016's "it never starts work" and goes against the spirit of ADR 0009. Q3.7 offers the ADR.
- From Q2.1: each endpoint behaves exactly like its CLI twin, limits included. A Ticket run started on an Issue URL always runs, as a typed command does. A Pass started through the API can be skipped by its **Command limit**, as one started by the scheduler can. A Command started through the API runs detached from the Console with a login environment, so restarting or updating the Console doesn't end it.
- From Q2.2: each pair of machines has its own token, issued and revocable by the owning machine and kept in the holder's **Credentials**. Q3.1 and Q3.2 refine it.
- From Q2.3: the owning machine's Activity log names the person and the machine they acted from, e.g. "Stop by jacob@stephens.page from the Console on `thirdshift`".
- From Q2.4: every view comes across, Security runs included. Details are fetched live when opened and never stored on the asking machine, which keeps #994's "each host keeps its own logs".
- From Q2.5: **Fleet** is the term, now in `GLOSSARY.md`, and the **Console** entry says it can show its Fleet.
- From Q2.6: one list under `[console]` replaces `other_consoles`. An entry with a token is fetched, one without is a header link, and old `other_consoles` values read as link-only entries.
- From Q2.7: each fleet machine is fetched every 15 seconds, only while a page is open. Each machine's rows show how old they are, and a machine silent for a minute shows as unreachable.
- From Q3.1: there are five token scopes, `read`, `security`, `act`, `run` and `admin`, with `read` as the default.
- From Q3.2: all eight token-handling rules stand, as recorded in ADR 0026. One correction to the brute-force rule: "like failed sign-ins" assumed a limiter that doesn't exist. Only an emailed sign-in code has a limit, 5 tries in 15 minutes per code (`src/console/accounts/codes.rs:79`), and failed passwords have none. Spec A adds a per-address limiter for failed tokens, and puts failed passwords behind the same one.
- From Q3.3: the owning machine refuses a call naming a person missing from its own Console sign-in list.
- From Q3.4: any named holder may have a token. A call with no person is attributed to the token's holder.
- From Q3.5: issuing and revoking tokens, the Console sign-in list, `console install` and `setup` stay CLI-only on the machine itself.
- From Q3.6: the page is a client of `/api/v1`. Each endpoint returns its CLI twin's `--json`, and the pairing test covers CLI, page and API.
- From Q3.7: ADR 0026 records the decision, and ADR 0016's "never starts work" and "no HTTP API" lines point to it.
- From Q4.1: each address on the Console sign-in list carries its own scopes, all five by default, and the owning machine checks them. This answers #952, and it's recorded in ADR 0026 and in the glossary's **Console account**.
- From Q4.2: pairing is `console token create` on the owning machine, then `console fleet add` on the holder (the token read on standard input and checked against `/api/v1/whoami`), with `fleet list` and `fleet remove`.
- From Q4.3: the work is four issues: Spec A (the API with `read`, `security` and `act`), a `ready-for-human` release step blocked by A, Spec B (the Fleet in the Console) blocked by the release step, and Spec C (`run` and `admin`, absorbing #952) blocked by A. Each Spec goes through `to-spec`, then `to-tickets`.
- From Q4.4: #1142 becomes Spec B.
- From Q4.5: the Inbox gets a machine filter kept in the URL, and nothing is saved to the User config.
- The frontier is empty: no open questions remain.

## Research results (2026-10-28)

- `thirdshift status --json` on `thirdshift` (0.17.4) returns `hostname` at the top level and on each Command, with kind, repository, issue, stage, process and harness. Confirmed by running it.
- The Pickup reservation commit message records `Host:`, `Launch:`, `Process:` and `Started:`. Confirmed in source, `src/pickup/reservation.rs:41`.
- `other_consoles` under `[console]` holds addresses that are linked in the header and never fetched (empty on `thirdshift`). Confirmed in source, `src/config/console.rs:136` and `src/console/header.html:2`.
- ADR 0016's Consequences say "no HTTP API is promised to anyone". Confirmed in the ADR.
- A Security run's row in `status --json` carries its kind, stages with their times, and each stage's `session_log` as an absolute local path. It carries no finding titles or write-ups. A summary served to another machine would leave out the absolute `session_log` paths and offer the traces through the API instead (Q2.4). Confirmed by running `thirdshift status --json` on `thirdshift` against the 2026-10-28 04:00 Security run.

## Waiting on research

- Nothing.
