# Grilling: #711, see every live Run, on every machine, and its stage

Issue: https://github.com/JacobStephens2/thirdshift/issues/711 (gathers #368 and #574)
Research: [ADR 0009](../../docs/adr/0009-the-operating-system-schedules-thirdshift.md), [ADR 0011](../../docs/adr/0011-thirdshift-keeps-a-per-repository-activity-log.md), [ADR 0015](../../docs/adr/0015-security-findings-stay-private-until-fixed.md)

## Round 1 (answered)

*Pre-answer* I don't want the UI to be able to do anything the CLI or an API of thirdshift can't do.

**Q1 - What the first version shows.** You asked for "the runs in progress and the logs from the runs". In the glossary, what's in flight is a **Command**. A Pickup run can be running a Spec run, which can be running three Ticket Runs at once. Each of those Runs is somewhere in implement, review, Repair, CI watch or Self-merge, with a Session log growing. The options:
- (a) **Live Commands only.** Each one has its child Runs under it. Each Run shows its issue, its stage, the Harness and Model of its current session, and links to its Command log and Session logs.
- (b) **As (a), plus Commands that ended recently,** say in the last 24 hours, with their outcome and logs. These come from the Activity logs.
- (c) **As (b), plus a Stop button** (#574's part).

*Recommended:* (b). After a Run notification, or a morning of Weeding, a Run that just ended is usually the one you want to look at, and the Activity logs already have the data. Leave Stop for later. It is the only thing on the page that changes anything, so it needs an answer to Q4 that the read-only page doesn't.

*Answer:* if there's anything from https://github.com/warpdotdev - any source for Warp Factories for example - we can take and use that or get inspiration for it. Or other software factory UI repos that are open source too - maybe OpenHands Agents? Not sure. /research may be useful here

c, plus viewing all commands ever run on this host for which we have data to display

*Checked:* An earlier survey found that Warp's Factories control plane is closed SaaS. Only the Warp terminal client is open source, under AGPL-3.0, which can't be copied into an MIT project like thirdshift. OpenHands' Agent Canvas, its "control center" UI, is MIT ([warp-factories-and-alternatives.md](../../docs/research/warp-factories-and-alternatives.md), [openhands-fit.md](../../docs/research/openhands-fit.md)). A research agent is now looking at what can be taken or borrowed from these and other factory UIs and Session log viewers (see Waiting on research).

*Checked:* Corrected on 2026-10-18. Activity logs and Command logs begin on 2026-10-03, but Session logs go back to 2026-09-25. So for the first eight days, the only data left is Session logs. Confirmed by the stamps in the 528 Command log and 952 Session log names under `~/.thirdshift/logs/JacobStephens2/`.

**Q2 - One page per machine, or one page for every machine.** The issue says "every machine". Logs live on the machine that ran the Command, and GitHub sees only the Claim: no stage, no logs, and nothing at all for a Ticket's Run or a Base fix. The options:
- (a) **Each machine serves its own page** at its own address. A new server gets the page by setting it up there, and doesn't need to know about any other machine.
- (b) **One page for every machine.** One host either asks the others for their state or receives it from them, so that host has to know every machine. Logs from other machines are either copied to it or fetched when someone opens them.
- (c) **(a), plus a short list of every machine's page** kept in one User config, or a page that finds the machines from the Claims on GitHub.

*Recommended:* (a) now, and (c) once there's a third machine. (a) is the only option where setting up a new server touches nothing but that server, which you asked for. #574's problem, not knowing which host a Run is on, is solved if each Command's Activity log line names its machine. A Claim's label could carry that too.

*Answer:* a

**Q3 - Where a live Command's state comes from.** ADR 0009 rules out a daemon, so nothing remembers a Command except what it writes down. The Activity log has a start line with no end line, but it can't tell a live Command from one that `kill -9` or a reboot cut off, and it has no stage. The options:
- (a) **Read the Activity logs and Command logs,** and check the process table to see if each Command is still alive.
- (b) **Each Command keeps a live record on its machine.** This is a small file holding its process id, its issue, and each child Run's stage and current Session log. thirdshift rewrites it at every stage change and removes it when the Command ends. A record whose process is gone shows up as "died", with its last stage.
- (c) **(b), with the Activity log also getting a line at each stage change.**

*Recommended:* (b). It's the only option where the page knows the stage without parsing Command log output meant for people. "Died at review" is exactly what #574 needed and couldn't get. (c) adds lines to a log that ADR 0011 worked to keep short.

*Answer:* I think b, though I think about SQLite for example and wonder if that would be useful over files To enable easier analysis of aggregate data, for example. 

*Checked:* Files or SQLite is Q11 below.

**Q4 - Who can reach the page.** The logs hold chart35's private code, every agent's reasoning, and Security sessions. Under ADR 0015, a Security finding's write-up stays private until it's fixed. The options:
- (a) **Only from inside a private network:** an SSH tunnel, a VPN like Tailscale, or localhost. There's no login, because nobody outside can reach it.
- (b) **The public internet, behind a login:** a password or a GitHub login, over HTTPS.
- (c) **(b), with Security sessions and findings left off the page.**

*Recommended:* (c). The deciding question is yours: should the page open on your phone with no VPN running? Tailscale isn't installed here, so (a) would mean an SSH tunnel every time. Caddy already gets a certificate for any name on its own, and `*.thirdshift.app` already points at this host. (c) beats (b) because a leaked password then exposes no unfixed vulnerability. Security work stays in the command (Q7), behind SSH. How you sign in, Caddy's built-in password or a login in the page, is for Round 2.

*Answer:* b

**Q5 - What serves the page.** A page needs something listening. ADR 0009 rejected a long-running `thirdshift watch`. Its reasons were keeping it alive, its going stale after `thirdshift update`, and its hanging silently. They were aimed at a process that *starts* work. A viewer that only reads files could hang without losing anyone work. The options:
- (a) **`thirdshift serve`,** a read-only process kept up by a systemd user unit. It renders Session logs on the server and streams them as they grow.
- (b) **No thirdshift process.** thirdshift writes the live records (Q3), and the web server already on the machine serves them, the logs and one static page. The page's JavaScript reads them and renders the Session logs in the browser. Setup is one web server config block.
- (c) **(a), but started on demand** (`thirdshift serve` in a terminal or tunnel, stopped when you're done).

*Recommended:* (a), listening on localhost only, with Caddy in front. Research changed this from (b), for two reasons:
- Caddy runs as its own user and can't enter `/home/jacob` (mode 750). (b) would mean opening your home folder to it, then writing deny rules in the Caddy config for every Security log folder, and a Security log kind added later would leak until someone remembered to add a rule. In (a), thirdshift runs as you and leaves Security logs out in code, under test.
- (a) is how tracewake's dashboard already works on this host: an app on 127.0.0.1, with one `/etc/caddy/sites/<name>.caddy` file that reverse-proxies to it with `flush_interval -1`, so streaming works.

On a new server it needs nothing at all to start with: `thirdshift serve` plus an SSH tunnel. A Caddy file comes in only when you want it from your phone. It contradicts ADR 0009, so it gets an ADR of its own: a reader runs all the time, but nothing that starts work does. To deal with the stale binary, `serve` re-executes itself when `thirdshift update` replaces it, or the unit restarts it.

*Answer:* agree

**Q6 - What the page is called.** Its name ends up as the subdomain and in the glossary. A name for the thing works better than a name for the medium. The options:
- (a) **`floor`**, as in the shop floor, where you walk the line to see what each station is doing right now. It fits Day shift, Weeding and Fencing, and "the floor" reads naturally: "it's on the floor at review".
- (b) **`console`**: plain, and widely understood. But it suggests a place where you do things, and the page only shows.
- (c) **`dashboard`**: what #368 called it, and the most common word for it. It's generic, though, and suggests charts and history more than what's in flight.
- (d) `ui` or `web` name the medium, not the thing. `thirdshift.thirdshift.app` repeats itself and leaves no room for a name per machine (Q2).

*Recommended:* (a) `floor`. Whatever you pick becomes the glossary term. The exact address (`floor.thirdshift.app`, `floor.<machine>.thirdshift.app`, or the machine's own domain) waits on Q2 and Q4, so it's in Round 2.

*Answer:* I might move third shift to more of an agricultural metaphor at some point, more garden-oriented. So I'm not sure how deeply I want to ingrain floor or shop floor type language, though I do like floor, but I also envision this web UI becoming something that has a possibly a way to to do things to third shifts such as start runs or stop runs.

So that that part of console could work eventually. And I want to probably show charts and history eventually as well. So that opens up dashboard too. So in light of this, what what do we think might what do you think might fit? 

*Checked:* Answered as Q8 below.

**Q7 - A command too.** The issue asks for "one command, or one page". A command that prints what the page shows works over SSH with no setup, and it's the obvious place for #574's stop. The options:
- (a) **Both, from the same live records:** a command such as `thirdshift floor`, and the page.
- (b) **The page only.**

*Recommended:* (a). The command costs little once Q3's records exist. It's how an agent you ask "what's running?" would answer, and it's where Stop lands first, behind the terminal you're already logged into.

*Answer:* agree, And this goes along with me wanting to have a all the information, for example, that's accessed on the web UI. I want there to be some kind of third shift API or CLI flag commands, I think, that can get that information and data or take that action action.

And this will make it easier for agents to work with third shift with without them having to use a web UI to fully interact with and control it. 

## Round 2 (answered)

**Q8 - The name, and its address here.** You plan to start and stop Runs from the page and to add charts and history, and the metaphor may move from factory to garden. That rules out a factory word like `floor`, and a word that only means watching. The options:
- (a) **`console`**: plain, and it works under either metaphor. It covers viewing and control, and charts sit naturally on a console. Here it would live at `console.thirdshift.app`, which already resolves to this host.
- (b) **`dashboard`**: fits charts and history, but suggests something you read rather than act from.
- (c) **A garden word,** chosen when the metaphor moves. If it never moves, you're left without a name.
- (d) **This host's name, `thirdshift.thirdshift.app`.** This would set the pattern for more machines (`<host>.thirdshift.app`), but it repeats itself here, and Q2 already gives each machine its own address.

*Recommended:* (a), `console.thirdshift.app`. Whatever you pick goes into `GLOSSARY.md` as the **Console**. The address is a User config setting, so a second machine picks its own, and renaming one later means changing one Caddy file. Commands for the CLI are named in a later round, after Q10.

*Answer:* agree

**Q9 - How you sign in.** This builds on Q4 (b) and Q1 (c). The page is on the public internet, it shows Security findings that aren't fixed yet, and it can stop a Run. So whoever gets in can see an open vulnerability and stop your work. The options:
- (a) **Caddy's built-in password** (`basic_auth`): one line in the Caddy file and no code. But browsers send it on every request, including requests other sites trigger, so Stop still needs its own CSRF guard. It has no sign-out, and you type the password on your phone.
- (b) **A password in the page,** as tracewake's dashboard does: a session cookie and CSRF tokens. You choose a password, and its hash is kept in the Credentials.
- (c) **Sign-in links from the CLI.** A command such as `thirdshift console login` prints a link that works once, for a few minutes. Opening it gives that browser a long-lived session, with CSRF tokens on Stop. Another command ends every session. There's no password to leak or guess, and signing in a new device takes SSH to the machine, the same access that already controls thirdshift.
- (d) **GitHub sign-in, limited to your account.** It needs an OAuth app registered for each server, and its secret stored there.

*Recommended:* (c). It's the only option that adds no secret to choose, store or rotate, and none to set up on a new server. It also follows your rule: signing in is itself something you do from the CLI.

*Answer:* b

**Q10 - What "an API" means.** You want everything the page shows or does to be reachable by an agent without the page. The options:
- (a) **The CLI is the API.** Every view has a command with `--json`: list Commands, show one, print or follow its logs. Every action is a command too, starting with Stop. `serve` calls the same code in-process, and its HTTP routes exist only for the page; nobody else is promised them. A test pairs each page route with the CLI command it mirrors, so the page can't get ahead.
- (b) **A documented HTTP JSON API** from `serve`, which agents call with a token, and which the CLI calls too.
- (c) **Both documented.**

*Recommended:* (a). An agent working on this machine already has a shell, so it needs no token and no running `serve`. An HTTP API for other machines can wait for Q2 (c), when something actually needs to read a machine from outside.

*Answer:* agree

**Q11 - Files or SQLite for the records.** This builds on Q3 (b). Each Command keeps a record of its process id, its child Runs and their stages. Call it the **Command record**. You asked whether SQLite would make it easier to analyse many Commands together. The options:
- (a) **One file per Command,** next to its Command log, and kept after the Command ends. It holds every Run's stages with their start times and its outcome. History and charts come from scanning these files. That's about 300 files for the first two weeks, which takes a fraction of a second, and `jq` and agents read them directly.
- (b) **One SQLite database per machine,** which every Command writes to. The Activity log stays, so the two records have to keep agreeing. Eight Passes a minute, plus their child Runs, write to it at once. A locked or broken database must never fail a Run, the same rule as for Run notifications. Each release target also gets SQLite's C code built in (ADR 0003's static binaries).
- (c) **(a), plus a SQLite index** that `serve` builds from the files and can rebuild whenever it likes. Nothing is lost if the index is deleted.

*Recommended:* (a) now, and (c) the day a chart or query is too slow on files. Files are how thirdshift keeps everything else: logs, config, the Activity log. A Command that crashes or writes at the same moment as another can't damage anyone else's record. And (c) adds SQLite later without moving any data.

*Answer:* agree

**Q12 - What Stop does to the issue.** Today, stopping a Command (`SIGTERM`, as in #574) is an interruption that cleans up. Its Claim then follows the usual rules. If the cleanup pushed the Issue branch, the Claim stays, so nothing takes the issue again. If nothing reached `origin`, `ready-for-agent` is put back, and the next minute's Pickup run takes it again: the very thing you stopped it for. In every option, Stop waits until the Command has ended, then prints its outcome and any branch or pull request it left. The options:
- (a) **Labels follow the usual rules.** To keep the issue from being taken again, you remove `ready-for-agent` yourself.
- (b) **Stop leaves the issue for the Day shift.** `ready-for-agent` is not put back, and the issue gets `needs-triage`, which means you need to look at it again before an agent does. A flag such as `--requeue` puts it back in line instead.
- (c) **Stop asks each time.** The CLI asks at the prompt and the page shows a choice. That breaks a scripted stop.

*Recommended:* (b). #574 is the case Stop exists for: the issue wasn't ready. A Stop that hands it straight back to the next Pickup run undoes itself within a minute.

*Answer:* agree

**Q13 - Stopping one Ticket's Run, or only a whole Command.** Each Ticket's Run in a Spec run, and each Base fix, is its own child `thirdshift` process, so either can be signalled on its own. The options:
- (a) **Whole Commands only** in the first version. Stopping a Spec run stops its Tickets' Runs.
- (b) **Any Run, the child Runs included.** The Spec run then counts that Ticket as failed, and the Tickets it blocks never start. That's a new way for a Ticket to end, and Spec runs would need rules for it.

*Recommended:* (a). It covers #574. (b) needs its own rules for how a Spec run accounts for a stopped Ticket, so it gets its own issue.

*Answer:* agree. I at least want a way though in the console to observe the spec run's ticket runs, so I want to be able to see for example what ticket runs are working for a spec run.

*Checked:* Watching a Spec run's Ticket Runs is already covered: Q16 (a) nests child Runs under their Command. Q20 asks whether Tickets that haven't started show too.

**Q14 - Getting the Console running on a new server.** You want it easy to set up. `serve` (Q5) has to keep running and start again after a reboot, and Caddy has to point at it. Writing to `/etc/caddy` takes root, which thirdshift doesn't run as. The options:
- (a) **A command that installs it,** also offered by Setup. It writes and enables a systemd user unit, turns on lingering so `serve` runs with you logged out, and prints the Caddy site file for the address in the User config, with the "restart, don't reload" step. You paste that one file in with `sudo`.
- (b) **README steps only.**
- (c) **(a), with thirdshift writing the Caddy file itself** through `sudo`.

*Recommended:* (a). It brings a new server down to one command and one paste. A server without Caddy still works over an SSH tunnel. (c) would make thirdshift ask for root, which nothing in it does today.

*Answer:* agree. And I'll want thirdshift's console to be able to be served on servers using apache instead of caddy (such as my stephens.page server - I think you can connect to that server by `ssh jacob@stephens.page` to check its setup out. I'm interested in being able to run the console on macOS as well, such as on my laptop, but that's not essential in this pass, and if that is too much of an extra lift, maybe an issue can be filed to capture that idea.

*Checked:* A sub-agent is looking at stephens.page's Apache over SSH, for Q22. macOS is Q21: thirdshift already ships an `aarch64-apple-darwin` build (`dist-workspace.toml`), so the open question is only the always-running part. On macOS that's launchd, not systemd.

**Q15 - What the page is built with.** This builds on Q5 (`serve`) and on the research note. Whatever is built has to ship inside the binary. The options:
- (a) **HTML rendered by Rust, plus htmx.** That means htmx, its server-push extension and idiomorph, about 20 KB gzipped, all 0BSD, copied into the repo rather than loaded from a CDN. One small hand-written JS module covers the rest, and it's all embedded with `include_dir`, which thirdshift already uses for the Factory skills. A server push redraws the Commands list. Each open Session log gets only its new lines. Gas Town, the closest match the research found, is built this way.
- (b) **A React app built with npm and embedded,** as Vibe Kanban and agent-of-empires do. Every release build then needs Node, and the bundle runs 1 to 10 MB.
- (c) **Plain HTML and hand-written JS, with no library.**

*Recommended:* (a). It has no build step, which matches how `site/` is built. It works with no internet connection and loads nothing from anyone else. Rendering on the server also keeps one copy of the logic, in Rust, next to the CLI that's meant to match it (Q10). Lifting OpenHands' UI is out: it's 10 MB of React that renders its own event format, not Claude's or Codex's.

*Answer:* agree - I'm wondering if Axum is relevant here or, any Rust UI frameworks like Leptos or Dioxus, or if those are overkill for what I'm doing here.

*Checked:* Answered as Q17.

**Q16 - What the page borrows.** The research found patterns that recur across Warp, OpenHands, Symphony and Gas Town. The options:
- (a) **Take all of these:**
  - Child Runs nested under their Command.
  - Each Run's stage shown with how long it's been there ("review · 52 min"), so a stall shows without opening a log.
  - A one-line latest event per live Run (`Bash: cargo test`).
  - Stop shown only while the process is alive, with a confirm step on the page but none in the CLI.
  - In a Session log: runs of tool calls folded into one, thinking collapsed, a renderer per tool with a plain fallback, and new lines appended rather than the whole log redrawn. The research found that redrawing a large log on every new line took 15 to 27 seconds.
- (b) **A plainer first version:** the Commands list, and Session logs as lightly formatted text. Rendering comes later.

*Recommended:* (a), starting with the plain fallback renderer and adding renderers per tool as they're needed. Stage age is the cheapest way to spot what #574 had to dig for. The confirm step exists because the page is where a mis-tap on a phone can stop an hour of work.

*Answer:* agree

## Round 3 (answered)

**Q17 - Axum, Leptos or Dioxus.** This builds on Q15 (a): HTML rendered by Rust, htmx in the browser, no build step. The rest of thirdshift is blocking code, and `tokio` and `hyper` are already compiled in, because `reqwest` uses them (`Cargo.lock`). The options:
- (a) **axum,** with templates checked at compile time (askama, or maud's macros). axum is a router on `tokio` and `hyper`. Its server push (`axum::response::sse`) is built in, which Q15 needs for the Commands list and for log tails. The async code stays inside `serve`, and the rest of thirdshift doesn't change.
- (b) **tiny_http,** which is blocking like the rest of thirdshift and lighter still. You write server push by hand, with a thread per open log, and the project has had no push since 2024-08.
- (c) **Leptos or Dioxus.** These are full-stack frameworks: Rust components compiled to WebAssembly that run in the browser, calling server functions. That's the right tool for an app that keeps a lot of state in the browser. Q15 chose to keep state on the server. Each release build would also need a WebAssembly target and the framework's own build tool (`cargo-leptos` or `dx`), which is the npm objection under another name.

*Recommended:* (a). Leptos and Dioxus are overkill here: they solve client-side state, and Q15 deliberately has none. axum costs the least for what Q15 asked of the server, and the async code stays inside the Console.

*Answer:* Agree on a.

I don't have objection to npm, but I'm okay waiting on something like Leptos or Dioxus until client-side state gets large enough to warrant something like these.

**Q18 - The password, in detail.** This builds on Q9 (b). It's one password, with no user names, since the Console is yours alone on each machine. The options:
- (a) **All of these:**
  - It's set by a CLI command, such as `thirdshift console password`, which install (Q14) and Setup also offer. Only its argon2id hash is kept, in the Credentials.
  - A session lasts 30 days from last use. Setting a new password, or a sign-out-everywhere command, ends every session.
  - After five wrong passwords, sign-in slows to one try a minute.
  - Every action (Stop, and later Start) needs a CSRF token.
  - With no password set, `serve` refuses to start, unless it's run with a flag such as `--no-sign-in` for SSH-tunnel use only.
- (b) **As (a), but without the slowdown or the refusal to start.**

*Recommended:* (a). The page is on the open internet guarding unfixed vulnerabilities, so guessing has to be slow. A Console that started without a password would be open to anyone the moment a Caddy file pointed at it.

*Answer:* I want to have email / password login, and a way to reset password by email (using the Resend config for thirdshift on that server). But in user config I want a whitelist of email addresses which can login to the console. And if a user hasn't created a password with `thirdshift console password`, I want them to be able to set their first password using the email password reset - or maybe better a create account flow, but only allowing account creation through the ui for users with their email in the User config console whitelist, and requiring the user to confirm their email with an email verification code or something in order to complete account creation. Otherwise I agree with the recommendation.

*Checked:* This replaces "one password, its hash in the Credentials" from Q9 (b) and Q18. The `GLOSSARY.md` Credentials entry no longer says the Console checks a password against it; it now says the Resend key also sends the Console's sign-in emails. Where accounts live is Q28, and the flows are Q29 to Q31. The User config already has `email.to` and `email.from` (`src/config.rs`).

**Q19 - The CLI commands.** This builds on Q10 (a), the CLI is the API, and on Q12. The name for a Command on the command line is the issue it's working on, written `chart35#239`, with the owner added only when two repositories share a name, or the issue URL. A Pass with no issue is named by its kind and Command log stamp, such as `architect-20261017T142101`. The options:
- (a) **Views and actions at the top level, and the Console's own commands under `console`:**
  - `thirdshift status [--all] [--json]`: live Commands, then recent ones. `--all` lists the whole history.
  - `thirdshift show <command> [--json]`: one Command with its Runs, stages and log paths.
  - `thirdshift logs <command> [--session <kind>] [--follow]`
  - `thirdshift stop <command> [--requeue]`
  - `thirdshift console`, which serves the page and replaces Q5's `thirdshift serve`. With it go `console install`, `console password` and `console sign-out`.
- (b) **Everything under `console`** (`console status`, `console stop` and so on). But status and Stop aren't the Console's: agents use them with no Console running.
- (c) **(a), but keep `thirdshift serve`** as the name of the server command.

*Recommended:* (a). The page is the `console` group, and the views and actions are thirdshift's own. That's your rule written into the command tree.

*Answer:* agree

**Q20 - What a Spec run shows.** This builds on your note on Q13 and on Q16 (a)'s nesting. Running Ticket Runs show either way. The question is whether Tickets that haven't started show too. The options:
- (a) **Every Ticket in the Spec run:** running (with its stage and how long it's been there), merged into the Spec branch, failed, waiting on the Tickets that block it (named), or not started.
- (b) **Only Tickets that have started a Run.**

*Recommended:* (a). "What's working, and what's waiting on what" is how you see why a Spec run is slow, and the Spec run already knows its graph.

*Answer:* agree

**Q21 - macOS.** You want the Console on your laptop eventually, but not necessarily in this pass. thirdshift already ships a macOS build. On a laptop there's no proxy and no password, because it's served to localhost only, and `--no-sign-in` (Q18) covers that. What's missing is keeping `serve` running: launchd, not systemd. The options:
- (a) **Keep the Console's code portable now:** nothing tied to `/proc` or systemd outside `console install`. `console install` handles systemd only, and an issue is filed for launchd.
- (b) **launchd in this pass too.**
- (c) **Linux only,** with no care for portability.

*Recommended:* (a). Running `thirdshift console` by hand already works on a Mac, if the code stays portable. launchd is small, but there's no Mac here to test it on, so it's better as an issue you can try on your laptop.

*Answer:* agree

**Q22 - The Apache site file.** This builds on Q14 and your note on it. stephens.page runs Apache 2.4.58 with `proxy`, `proxy_http`, `headers` and `ssl` on, and certbot's Apache plugin issues one certificate per name. About 50 sites there already proxy to `127.0.0.1` apps. One of them, `baby.stephens.page`, already streams server push through Apache with `flushpackets=on` and gzip turned off on that path, which is what the Console's streams need. The options:
- (a) **`console install` sees which server is running, Caddy or Apache, and prints that one's file.** For Apache, that's a port-80 virtual host that proxies to `serve`, with `flushpackets=on` and gzip turned off on the stream paths. Then `sudo certbot --apache -d <name>` writes the HTTPS one, as for every other site there. nginx and anything else get a README section.
- (b) **Always print both files.**
- (c) **Caddy only, with Apache in the README.**

*Recommended:* (a). It keeps "one command and one paste" true on both machines you run, and a server with neither still gets the SSH tunnel.

*Answer:* agree

**Q23 - Two machines, not one.** Correction: Round 1 said this was the only machine running thirdshift. That was wrong, because the research only looked at this host. stephens.page runs thirdshift 0.11.2, with 15 crontab lines. Those include Pickup runs on thirdshift and clave, which this host also picks up from. So #574's "which machine is it on?" is live today, and Q2 was answered on a wrong fact. The options:
- (a) **Keep Q2 (a) as it is.** Each machine has its own Console, and you check both.
- (b) **Q2 (c): each Console links to the others.** A short list of the other Consoles' addresses goes in the User config, and the page header links them. Nothing is fetched from another machine, so Q10's "no HTTP API promised" stands.
- (c) **(b), plus the Claim says which machine took the issue,** for example in the comment thirdshift leaves on the issue, so GitHub alone answers "where is it?".

*Recommended:* (b) now, and (c) as its own issue. (b) turns "check both" into one tap and touches nothing outside the Console. (c) changes what every Claim writes to GitHub, and that needs its own look.

*Answer:* agree

**Q24 - A user unit or a system unit.** This revisits Q14 (a) with what stephens.page shows. Lingering is off for you there. Both machines keep long-running apps as **system** units with a `User=` line: on this host, the site publisher runs as `thirdshift-site`; on stephens.page, 50 apps run as `jacob`. Turning lingering on can also need root, depending on the machine's policy. The options:
- (a) **A user unit with lingering,** as Q14 said. No root is needed where lingering is allowed, but it isn't how either machine does it.
- (b) **A system unit with `User=` you.** `console install` prints one script to run with `sudo`, which writes the unit and the proxy's site file, enables the unit, and restarts the proxy.
- (c) **`console install` asks which one.**

*Recommended:* (b). It's one `sudo` paste either way, because the proxy file needs root already. It matches how both machines already run apps, and it survives reboots and log-outs whatever the lingering policy is.

*Answer:* agree

**Q25 - The Console's address on stephens.page.** `console.stephens.page` is taken: it redirects to `dashboard.stephens.page`, where muxboard's console lives, and it sits behind Cloudflare's proxy. `*.stephens.page` is a DNS wildcard to that machine, so any other name works without touching DNS. No site there uses `thirdshift`. The options:
- (a) **`thirdshift.stephens.page`.**
- (b) **A name under thirdshift.app,** such as `stephens.console.thirdshift.app`. That needs a DNS record at Porkbun, because `*.thirdshift.app` points at this host.
- (c) **Something else** you choose.

*Recommended:* (a). It resolves already, certbot handles it like any other name there, and it says what it is in your own domain.

*Answer:* agree, maybe in setup there should be a question to the user about what address will be used on that machine for the thirdshift console to make this process more extensible to other peoples' machines who I don't know what address they will want to use. The suggestion by thirdshift perhaps can be <thirdshift.domain.tld>, checking the local machine perhaps for domain.tld to suggest, or just leave it totally up to the user to enter the whole thing for simplicity.

*Checked:* Setup's questions are Q32.

**Q26 - Logs in the old layout.** This builds on Q1's "all commands ever run". stephens.page still has Session logs from before ADR 0011, flat in `~/.thirdshift/logs/` and named like `JacobStephens2-chart35-issue-62-20260930T201407Z-implement.jsonl`. ADR 0011 left them where they were. The options:
- (a) **The Console reads them too,** grouped by repository, issue and stamp, like this host's earliest Session logs.
- (b) **Only the current layout.** You'd move the old files by hand if you want them shown.

*Recommended:* (a). They're part of "all commands ever run for which we have data", their names hold the repository, issue and stamp, and reading them moves nothing.

*Answer:* b, we only need to support the latest data format / log layout. The user can reorganize their older logs if they want to make them in the format that works for thirdshift console.

**Q27 - The Console's look: garden, or plain.** From your message on 2026-10-18. You lean toward a garden metaphor for thirdshift: tending software that improves itself, where a construction metaphor invites big plans up front. The Console could show that, with flora in its visual elements, or stay plain and informational. What it shows is settled (Q1, Q16, Q20), so variations can be drawn now. The options:
- (a) **Prototype first, decide after.** A background agent uses the `prototype` skill's UI branch to build several very different variations of the Commands list and a Session log, on made-up data shaped like real Command records, switchable from one bar at the bottom of the page. For example:
  - fully garden: the beds, the season and the growth are the frame;
  - plain and informational;
  - plain, with a light garden accent in colour, icons and empty states;
  - the press floor, as on thirdshift.app: process inks (cyan, magenta, yellow, black), halftone fills, 1px misregistration, scanlines, Big Shoulders headings over JetBrains Mono, built from `site/styles.css`'s tokens. A Run's stages could read as the press line's units.

  They're throwaway, and get published as private pages you can open on your phone. You pick from those.
- (b) **Decide now: plain and informational,** and revisit once the metaphor itself has moved.
- (c) **Decide now: garden-themed.**

*Recommended:* (a), started as soon as you say yes, so it runs while you answer the rest of Round 3. A look is quicker to judge than to describe. The press floor variation shows whether the Console should look like the site it's sold from, and a plain layout with a light accent may turn out to be the version that can follow the metaphor wherever it goes. Two ways to start it:
- **You type `/claude-handoff`,** naming the prototypes as the focus. It starts a fresh background session, which you manage with `claude agents`. `/handoff` and `/claude-handoff` are marked user-only, so I can't run them myself.
- **I start a background agent** with the `prototype` skill. It reports back here, so I can fold the result into this file.

The variations would live in `.scratch/prototypes/711-console/` and go on a throwaway branch afterwards, as the prototype skill asks.

*Answer:* I'm having an agent on my laptop /prototype from /tmp/handoff-711-console-prototypes.md right now.

*Checked:* Open until you pick a variant (see Waiting on research). The handoff predates Q18's accounts, so its prototype sign-in is a single password field. An email field and the code step (Q30) are worth a look in whichever variant you pick.

*Checked (2026-10-19):* decided on your laptop: the `console` variant, opening on its Inbox page. It's on the throwaway branch `prototype/711-console` (commit `05b1b0a`), with three research notes. The verdict is under "Settled" (the Q27 bullets), and the questions it raised are Round 6. The prototype's sign-in is still a stub; the real one is Q18 and Q30.

## Round 4 (answered)

**Q28 - Where Console accounts are kept.** This builds on your Q18 answer: email and password accounts, for the addresses the User config allows. Each account is an email, a password hash, and whether the address has been verified. Its sign-ins need to survive a restart of `thirdshift console`, since a session lasts 30 days. Accounts change from the page (create, reset), but the Credentials are written by Setup and hold the Resend key. The options:
- (a) **A file of its own beside the Credentials,** readable only by you. It holds each account's argon2id hash and each session's hashed token. Only `thirdshift console` and its CLI commands read or write it.
- (b) **Inside the Credentials file.**
- (c) **SQLite.**

*Recommended:* (a). The page writes accounts, so they shouldn't share a file with the key that only Setup writes. If a bug in the page corrupts the file, you lose sign-ins, not Run notifications. It also matches Q11's choice of files, and no record here needs a query. The term goes into `GLOSSARY.md` as **Console account**.

*Answer:* agree

**Q29 - When an address leaves the allowlist.** The User config lists the emails that may sign in. The options:
- (a) **Removing an address ends its sessions and blocks it at once,** but its account is kept, so adding the address back restores it. With nobody on the list, `thirdshift console` refuses to start, unless `--no-sign-in` is given for SSH-tunnel use (Q18).
- (b) **Removing an address deletes its account too.**

*Recommended:* (a). The allowlist is the one switch that decides who may sign in, so taking an address off it has to work right away, whatever else is stored. Keeping the account means an edit made by mistake costs nothing.

*Answer:* agree, and I might want a way to track a user's activity in the console, or to log their actions in the console (and perhaps their actions on the command line too).

*Checked:* Taken up as Q33.

**Q30 - Creating an account and resetting a password.** You asked for both, by email. Both need to prove you hold the address and then set a password, so they can be one flow. The options:
- (a) **One flow, "set your password",** for both cases:
  - You enter an email.
  - The page always answers the same way, "If that address may sign in, a code is on its way", so it never reveals who is allowed.
  - An allowlisted address gets a 6-digit code by email, valid for 15 minutes, with 5 tries.
  - You enter the code and a new password. That creates the account, or resets its password, and signs you in.
  - Codes can be sent at most once a minute and 10 times a day per address.
  - On the CLI, `thirdshift console password --email <address>` does the same for any allowlisted address, with no email needed.
- (b) **Two flows,** create and reset, each with a link in the email instead of a code.
- (c) **(a), with links instead of codes.**

*Recommended:* (a). A code works when the email opens on a different device from the page, which a link doesn't, and you'll often be on your phone. One flow means one set of rules to get right. The CLI command keeps your rule that the page does nothing the CLI can't.

*Answer:* agree

**Q31 - With no Resend key.** A machine may have no Resend key: someone who doesn't use Run notifications. The options:
- (a) **The email flows are off.** The page says to ask whoever runs the machine to run `thirdshift console password --email <address>`, and sign-in still works.
- (b) **`thirdshift console` refuses to start without a Resend key.**

*Recommended:* (a). Email is a convenience on top of accounts, not what makes them safe. The sign-in emails come from `email.from`, as Run notifications do.

*Answer:* agree

**Q32 - What Setup asks about the Console.** From your note on Q25. Setup (`thirdshift setup`) would ask whether to run the Console. If yes, it asks for its address and the emails allowed to sign in. Then it offers `console install` (Q14, Q24). The options for the address:
- (a) **You type the whole address,** with `thirdshift.example.com` shown as an example. Nothing is detected.
- (b) **Suggest `thirdshift.<domain>`,** with the domain guessed from the machine: its full hostname, or the names the proxy already serves. On this host, though, the guess would be `thirdshift.thirdshift.app`, the name you passed over for `console.thirdshift.app`.
- (c) **(b), with an exception** that suggests `console.<domain>` when the domain itself contains "thirdshift".

*Recommended:* (a). A guess has to be right for people whose machines you've never seen. This host already shows how a guess goes wrong, and typing a whole address once costs little. Allowed emails default to the Run notification address, `email.to`.

*Answer:* agree

## Round 5 (answered)

**Q33 - Recording who did what.** From your note on Q29. Today the Activity log records what the factory did in each repository, but not who asked. There are two kinds of event:
- **Actions on a Command:** Stop now, and Start later. They belong to a repository's story.
- **Account events:** sign-in, failed sign-in, code sent, password set, sign-out. These belong to the machine's Console.

The options:
- (a) **Actions go in the repository's Activity log, naming who.** For example `Pickup run #239 stopped from the Console by <email>`, or `... stopped from the CLI by jacob`, using the Unix user. Account events go in a **Console log**, one per machine, beside the Console accounts, one line each. `thirdshift console log` prints it, and the page shows it too.
- (b) **Everything in one Console log,** CLI Stops included, with nothing added to the Activity log.
- (c) **(a), plus page views:** which Commands and logs each account opened.

*Recommended:* (a). A Stop changes what happened to an issue, so it belongs where you already read that history. In #574 you would have wanted "stopped by me at 9:22" next to "started at 9:17". Account events are about the machine, not a repository. Page views are mostly noise, and recording them watches the reader more than it guards anything.

*Answer:* agree

**Q34 - How the work is published.** This is a lot more than #711 asked for. In the order the parts depend on each other:
1. the Command record;
2. `status`, `show` and `logs`, with `--json`;
3. Stop;
4. `thirdshift console`, read-only, with the Session log view;
5. accounts and sign-in;
6. Stop on the page;
7. `console install` for Caddy and Apache;
8. Setup's questions;
9. the look, once Q27 is decided.

The options:
- (a) **Rewrite #711 as a Spec** from this file, labelled `needs-triage`, then publish its Tickets with their blocking links, following `docs/agents/triage-labels.md`. Also file the follow-up issues on their own: launchd on macOS, stopping a single Ticket's Run, a Claim that names its machine, and an Activity log line naming the machine. Write the ADR for `thirdshift console` as an exception to ADR 0009 before the Tickets, so they can cite it.
- (b) **Two Specs.** #711 for parts 1 to 3, the CLI, which agents can use with no page. A new Spec for the Console, parts 4 to 9.
- (c) **Keep #711 as one issue** and attach this file to it.

*Recommended:* (b). Parts 1 to 3 are useful by themselves and answer #574 completely from a terminal. They also give a Pickup run a small Spec to finish before the larger one starts. The Console Spec waits on Q27 only for its last Ticket. The rest of (a) applies to both Specs: the labels, the follow-up issues and the ADR.

*Answer:* we'll run /to-spec to create the specs, then /to-tickets for each spec

## Round 6 (answered)

From the Q27 verdict. Everything in #711, the CLI, has shipped (see Research results, 2026-10-19), so changes to the CLI below become Tickets in #776, the Console Spec.

**Q35 - The Inbox replaces "live Commands first, then history".** In the prototype you chose, the Console opens on the **Inbox**: Commands grouped by what they ask of you, not by whether they're live. The groups are Needs you, Ready for review, Landing, Working, Done and Resolved. That reopens Q1's "live ones first". The options:
- (a) **The Inbox is the Console's home page.** The History page holds every ended Command, newest first, with filters for outcome and repository, back to the oldest logs.
- (b) Keep "live first, then history" as the home page, and make the Inbox a page of its own.

*Recommended:* (a). It's what you picked after trying both. A failure from last night needs you more than a Run that's working fine, and "live first" puts the Run first.

*Answer:* agree

**Q36 - What goes in Needs you, and in what order.** In the prototype, Needs you holds:
- **Died:** the process is gone with no outcome.
- **Silent:** a live Run whose logs have stopped growing (Q38).
- **Failed.**
- **Triage:** a Stop that labelled the issue `needs-triage`.
- **A Security finding waiting** on you.
- **Slow,** shown as "look" rather than urgent.

It sorts urgent first, then by escalation step, then oldest first. That puts yesterday's failures above a Run that's silent right now. The options:
- (a) **Live problems first** (Silent, then Slow), then the ended ones (Died, Failed, Triage, Finding), each newest first.
- (b) Keep the prototype's order.

*Recommended:* (a), as the laptop agent recommended too. A live problem still costs machine time, and a Stop can still save it. An ended one waits the same either way.

*Answer:* agree

**Q37 - How an item leaves Needs you.** "Acknowledge" is ruled out (nothing the CLI can't do), and the Console reads only this machine's files. In the prototype, a failure stays in Needs you forever, so nine months of failures would pile up. The options:
- (a) **It leaves when a later Command on the same issue starts on this machine, or once it is 7 days old.** After that it's only on History. Done shows the last 24 hours; Resolved shows Commands that ended 1 to 7 days ago with nothing waiting.
- (b) It leaves when GitHub says the issue moved on (closed, or relabelled). That means the Console reading GitHub on every refresh, which neither it nor `status` does today.
- (c) It leaves by age alone.

*Recommended:* (a). A later Command on the issue means someone (you, or a Pickup run) has already acted. 7 days covers a weekend away, and nothing is read from GitHub.

*Answer:* agree, and I want the user to be able to configure in their User config how long they want that period for being moved to resolved is - for 7 to be the default but for the view in the web ui console to be controlled by that user config.

*Checked:* The setting's name, and whether it also sets how far back Resolved reaches, is Q45.

Though alternatively I'm considering allowing users to "Acknowledge" by adding a folder to .thirdshift which stores some web ui state - initially probably just these acknowledgements / dismissals. I'm a bit wary to do this though as that introduces maybe a new kind of state stored on the machine. However, it's not entirely groundbreaking, as while much state is stored on GitHub, the log, trace, and configuration data is still stored on the host. I want to weigh the tradeoffs here. The CLI would need to be able to read this state file. I suppose though that this means there's a way to write to the server from the Console, which may be fine and worth it, just an expanding of the console.

**Q38 - Silent and Slow, precisely.** The research says a stall is silence, not stage length, and you accepted that. These need definitions the CLI and the page share. The options:
- (a) **Silent** means a Run in a stage whose Session log and Command log have both stopped growing for 10 minutes. It escalates at 30 ("silent 45 min · past 30"). Stages that wait, such as CI watch and Self-merge, write a waiting line to the Command log at least every 5 minutes. Today CI watch logs only when the count of running checks changes (see Research results), so a long CI run would read as silent. **Slow** means the stage has run past the top of its **usual range**, and it escalates at twice that. The **usual range** is the middle half (the 25th to 75th percentile) of that stage's times over the last 30 Command records in the same repository. It's shown only when there are at least 3. The thresholds are fixed in code, not User config settings.
- (b) The same, but the usual range is machine-wide, across all repositories, as in the prototype.
- (c) The thresholds go in the User config.

*Recommended:* (a). chart35's checks take far longer than thirdshift's, so a machine-wide range would call one repository slow and miss slowness in the other. Settings can come later if a fixed number turns out wrong.

*Answer:* agree, and I want the silent time definition to be configurable in the User config (default 10 minutes), as well as maybe the waiting line write frequency (default 5 min as it is now).

*Checked:* Both settings are read by `thirdshift status` as well as by the page, so they can't sit under `[console]`. Their names and the check that the waiting line comes more often than the silent time are Q45.

**Q39 - The CLI's side of the new pages.** Every view must also be a CLI command with `--json` (Q10). The options:
- (a) **`thirdshift status` shows the Inbox's groups in the Inbox's order,** Needs you first, with the Silent and Slow words. `--json` gives each Command its group and health. Lanes, Stages and Waterfall draw data that `status --all --json` and `show --json` already carry: stage start times, with the usual range added. So they get no commands of their own, and the parity test pairs each page with the command whose JSON holds its data. Choosing a theme only changes how the page looks, not what it shows about Commands, so it has no CLI command.
- (b) A new `thirdshift inbox` command, with `status` left as it is.

*Recommended:* (a). `status` already answers "what's on this machine", and one ordering for both means the page and the terminal never disagree about what needs you.

*Answer:* agree

**Q40 - Which pages #776 builds.** You chose seven pages: 1 Inbox, 2 Lanes, 3 Stages, 4 Waterfall, 5 History, 6 Logs, 7 Security. #776's Out of Scope says "charts", and the handoff left them out. The options:
- (a) All seven in #776.
- (b) **Inbox, Lanes, Stages, History, Logs and Security in #776. Waterfall gets a follow-up issue.**
- (c) Inbox, History, Logs and Security now; the three time views later.

*Recommended:* (b). Lanes and Stages need only stage start times and the usual range (Q38). The Waterfall goes down to single tool calls, and Session log lines carry no time (see Research results). Its times would have to be matched from the Command log's `HH:MM:SS` lines, which is a parser of its own. Out of Scope then reads "the Waterfall, and charts of totals over time".

*Answer:* a, but we'll handle the scope by the to-spec skill then to-tickets workflow, so we can make additional tickets to handle the waterfall extra, or make two specs then two ticket sets alternatively. Then we can fit charts in somehow too.

*Checked:* How the Specs are cut, and the charts, are Q48.

**Q41 - Where themes and other per-viewer choices live.** In the prototype, the chosen theme, the themes you make, "new since you last looked", hidden sections and Pause motion are all kept in the browser. None of them hides a problem. Each machine's Console is its own site, so one browser's choices don't carry from `console.thirdshift.app` to `thirdshift.stephens.page`. The options:
- (a) **The built-in themes are compiled into the binary. Everything per-viewer stays in the browser,** and themes move between Consoles by JSON export and import. Nothing is saved on the server.
- (b) Also a machine default in the User config (`[console] theme = …`) and theme files in the config folder. The page reads them but doesn't write them.

*Recommended:* (a) now. (b) gets an issue if moving themes by hand gets tiresome. It is read-only, so it fits Q10, but it means more for #776 to build.

*Answer:* a and b - though thinking again here about the Q37 answer and how much if any writing to the server to allow right now from the console. And then wondering whether to make it so that from the console the user can write to the console section of their User Config. That could be nice for them and intuitive - that that is where all their thirdshift settings on that host live, rather than some in their browser and some in the server, and that would smooth out using multiple browsers too - one theme showing on different browsers for a user on that host by the server.

*Checked:* The page writing the User config is a second action beside Stop, so ADR 0016 ("only reads, and does what the CLI does") needs amending, and each write needs a CLI command too (Q10). What the page may write is Q46.

**Q42 - Fonts, given "nothing loaded from another site".** #776 says no CDN, fonts included. The prototype and thirdshift.app both load Google Fonts. The built-in themes use ten families: Big Shoulders Display, JetBrains Mono, Inter, Caprasimo, Nunito, Barlow Condensed, Titillium Web, Atkinson Hyperlegible, IBM Plex Sans and IBM Plex Mono. All are under the SIL Open Font License, which allows bundling. The editor lists eight more. The options:
- (a) **Embed Latin-subset woff2 files for the built-in themes' fonts only, at the weights they use.** The editor offers those, plus the system font stacks. A theme naming any other font falls back to its stack.
- (b) Let themes load from Google Fonts. That breaks the rule, and tells Google about every page view.
- (c) System fonts only. Press and Factory would lose their look.

*Recommended:* (a). Ten families at one or two weights each comes to a few hundred KB in a binary already measured in MB, and the page works with no internet at all.

*Answer:* agree, though I wonder how easy or difficult it will be in this case for users to use their own fonts. I wonder about a theme system that allows putting fonts in a .thirdshift/console/themes folder or something.

*Checked:* A themes folder that holds fonts too is Q47.

**Q43 - Glossary terms and ADRs.** The candidates:
- **For `GLOSSARY.md`:** **Inbox** (naming its groups), **Needs you**, **Silent**, **Slow**, **Usual range** and **Theme**. Each group and page name is a heading on the page, not a term.
- **An ADR,** "The Console has one layout, and its looks are themes made of tokens". It's hard to reverse once people have exported themes, a reader might wonder why there aren't several layouts, and you weighed both.

Silence as the stall rule goes in Silent's definition. "Motion only for thirdshift's activity" and "visual calm" go in #776 as rules, since they're cheap to change.

*Recommended:* yes to both.

*Answer:* agree

*Checked:* `GLOSSARY.md` now has Inbox, Needs you, Silent, Slow, Usual range and Theme, and the Console entry opens on the Inbox. The ADR is [0018](../../docs/adr/0018-the-console-has-one-layout-and-themes.md).

**Q44 - Recording the verdict.** #711 is closed, now that the CLI has shipped, so the pointer belongs on #776. The options:
- (a) **Copy the three research notes from the prototype branch onto `web-ui` and commit them with this round.** Leave the prototype on `prototype/711-console`. Post a comment on #776 with the verdict and the branch commit. Then rewrite #776's body (the look, the pages, the new stories and rules) and run `/to-tickets 776`.
- (b) The same, but leave the research notes on the prototype branch.

*Recommended:* (a). The notes cite primary sources and are the reasons behind Silent, the Inbox and Focus, and `docs/research/` is where such notes live. Posting the comment and pushing are the steps you asked to approve first; "agree" approves both.

*Answer:* agree

*Checked:* The three notes are copied to `docs/research/`, pointing at the prototype branch. The software-factory note's claim that `--requeue` was invented is corrected. Committed and pushed with this round, and the verdict is posted on #776. #776's body is rewritten once Round 7 is answered.

*Checked:* Read as (b), two Specs, since `/to-tickets` runs "for each spec". The ADR is written: [ADR 0016](../../docs/adr/0016-the-console-is-a-long-running-reader.md).

## Round 7 (answered)

**Q45 - The new settings: names, and what they govern.** Q37 and Q38 add three settings to the User config. `thirdshift status` reads them too, so they belong outside `[console]`. The options:
- (a) **`inbox.days = 7`** sets how long an ended problem stays in Needs you, and how far back Resolved reaches. Done stays at one day. **`inbox.silent_minutes = 10`** sets when a Run is Silent; it escalates at three times that. **`logs.waiting_line_minutes = 5`** sets how often a waiting stage, such as CI watch or Self-merge, writes a line. If the waiting line doesn't come more often than the silent time, `status`, the page and Setup warn, and thirdshift uses half the silent time instead, so a waiting stage never reads as Silent.
- (b) The same settings, but one for Needs you and another for Resolved.

*Recommended:* (a). Your answer named one period, and with one number Needs you and Resolved never disagree about what happened this week. Clamping the waiting line keeps a typo from turning every CI watch red.

*Answer:* agree

**Q46 - What the page may write.** You'd like settings on this host to live in the User config, so a Theme follows you from browser to browser, and the page can change them. The options:
- (a) **The page writes only the Console's look, under `[console]`: the chosen Theme and the hidden sections.** It never writes the allowlist, the addresses, the thresholds, or any other section. Every write is also a CLI command: `thirdshift console theme <name>` and `thirdshift console sections`. Each one keeps the file's comments, as Setup does, and goes in the Console log naming the account. The setting is per machine, so every account on that Console sees the same Theme. "New since you last looked", Pause motion, what's folded, and the peek stay in the browser. ADR 0016 is amended: the page reads, Stops, and sets its own look.
- (b) The same, but each account's look is kept with its Console account, so two people on one machine can each have their own.
- (c) As (a), but the page can also change the `inbox.*` settings.

*Recommended:* (a). One place for settings is what you asked for, and a signed-in phone that can change only the look can't widen who signs in. (b) is worth doing only once a machine has more than one person.

*Answer:* agree

**Q47 - Your own Themes and fonts.** The options:
- (a) **A folder, `~/.thirdshift/console/themes/`, holds one `<name>.json` per Theme you made, and a `fonts/` folder of `woff2`, `woff`, `otf` or `ttf` files.** The Console serves those fonts from its own address, so nothing comes from another site. A Theme names a font in one of three ways: a built-in family, a system stack, or a file in `fonts/`. The page's editor saves Themes into the folder. Fonts are added by copying files in, not by uploading them through the page. The CLI has `thirdshift console theme list`, `import <file>` and `export <name>`.
- (b) As (a), with font uploads through the page too.
- (c) Themes inline in the User config, and no font files.

*Recommended:* (a). Copying a font file in is one `scp` for whoever runs the machine. An upload form would be the page's first way of putting arbitrary files on the server, for little gain.

*Answer:* agree

**Q48 - How the Console's Specs are cut, and the charts.** Q40 put all seven pages in, with scope managed through `/to-spec` and `/to-tickets`. The options:
- (a) **Rewrite #776 to cover all seven pages, Themes and the settings.** `/to-tickets` puts the Waterfall's Tickets last, including matching tool calls to the Command log's times. Charts get an issue of their own (`needs-triage`), to be grilled later, since nothing yet says what they'd show. Candidates are Commands per day by outcome, each stage's time week by week, and tokens per day.
- (b) Two Specs: #776 without the Waterfall, and a second for the Waterfall and the charts.

*Recommended:* (a). The Waterfall is settled enough to cut Tickets for, and the charts aren't. Putting the charts in a Spec now would hold the Waterfall up behind questions nobody has asked yet.

*Answer:* agree

## Settled by earlier answers (shout if any is wrong)

- Nothing schedules itself to keep the Console current. State comes from what each Command writes as it goes (ADR 0009).
- Logs keep their place, `<logs.dir>/<owner>/<repo>/`, and their names. The Console reads them where they are (ADR 0011).
- The Console can't assume `thirdshift.app`. Someone else running thirdshift has their own domain, or none, so the address is a setting, not built in. Here it's `console.thirdshift.app` (Q8).
- **The page does nothing the CLI can't** (your note before Q1, Q10). Every view and action reaches the CLI first or at the same time, Stop included, and a test pairs each page route with its command. The CLI's `--json` is the only API anyone is promised.
- **The first version can act, so it's more than a viewer** (Q1 c). It lists every Command on this host, with their logs, and it can Stop a live one. "Live ones first" is reopened by the Inbox (Q35).
- **History goes back as far as the logs in the current layout do** (Q1, Q26 b). From 2026-10-03, Commands come from the Activity logs and Command logs, without stages until the Command record exists. From 2026-09-25 to 2026-10-02 there are only Session logs, grouped by issue and stamp, with no outcome.
- **Each machine serves its own Console** (Q2 a). Nothing needs a list of machines. Finding from GitHub which machine a Run is on is left for when there's a second machine.
- **Each Command keeps a Command record:** one file beside its Command log, kept after it ends, with its process id, its issue, each child Run's stages and when each began, and its outcome (Q3 b, Q11 a). One whose process is gone, with no outcome, shows as died at its last stage. A SQLite index built from these files can come later, if a query gets slow. The term is now in `GLOSSARY.md`.
- **Security sessions and findings appear on the Console,** behind the sign-in (Q4 b). ADR 0015 keeps write-ups away from third parties, and your own server isn't one.
- **`thirdshift console` serves the page** as you, on 127.0.0.1, with a proxy in front (Q5 a, Q19 a). It's built on axum, with templates checked at compile time, and the async code stays inside the Console (Q17 a). It re-executes itself when `thirdshift update` replaces the binary. It's an exception to ADR 0009, a reader that runs all the time where nothing that starts work does, so it gets an ADR of its own: [ADR 0016](../../docs/adr/0016-the-console-is-a-long-running-reader.md).
- **Sign-in is by email and password, for the addresses an allowlist in the User config names** (Q9 b, replaced by your Q18 answer). It has a session cookie, CSRF tokens on every action, and 30-day sessions. After five wrong passwords, sign-in slows to one try a minute. Setting a new password, or `console sign-out`, ends that account's sessions. An account is created, or its password reset, through a code sent by email (Q30), or with `thirdshift console password`. Where accounts are kept is Q28.
- **Agents use the CLI, not the page** (Q7).
- **Stop** leaves the issue for the Day shift: `needs-triage`, with `ready-for-agent` not put back, unless `--requeue` (Q12 b). It waits for the end and reports the outcome and anything left behind. It works on whole Commands, and a Spec run's Stop stops its Tickets' Runs (Q13 a). Stopping a single Ticket's Run gets its own issue. `GLOSSARY.md` now has Stop, and the Claim entry says a Stop doesn't put `ready-for-agent` back.
- **Setting up a new server is one command and one `sudo` paste** (Q14 a, Q22 a, Q24 b). `console install` sees whether Caddy or Apache is running and prints one script for `sudo`. The script writes a system unit that runs as you, and the proxy's site file. For Apache, that's a port-80 virtual host with `flushpackets=on` and gzip off on the stream paths, followed by `certbot --apache -d <name>`. Then it enables the unit and restarts the proxy. nginx and others get a README section, and a machine with no proxy uses an SSH tunnel.
- **This host's Console is `console.thirdshift.app`, and stephens.page's is `thirdshift.stephens.page`** (Q8, Q25). Each Console's User config lists the others' addresses, and its header links to them. Nothing is fetched from another machine (Q23 b). An issue will propose that a Claim say which machine took the issue (Q23 c).
- **The page is HTML rendered in Rust, with htmx, its server-push extension and idiomorph copied into the repo,** embedded with `include_dir`, with no npm and no CDN (Q15 a). The page's files live under `src/`, since the crate publishes only `/src`, `/skills`, `README.md` and `LICENSE`.
- **The CLI commands** (Q19 a):
  - `thirdshift status [--all] [--json]`, `show <command> [--json]`, `logs <command> [--session <kind>] [--follow]` and `stop <command> [--requeue]`;
  - `thirdshift console`, with `console install`, `console password` and `console sign-out`.

  A Command is named by its issue, such as `chart35#239` or the issue URL, or for a Pass with no issue, by its kind and stamp.
- **A Spec run shows every Ticket:** running (with its stage and age), merged, failed, waiting on named blockers, or not started (Q20 a).
- **The code stays portable,** with systemd only in `console install`. An issue will be filed for launchd on macOS (Q21 a).
- **Only the current log layout is read** (Q26 b). stephens.page's flat Session logs from before ADR 0011 aren't shown unless you move them. **Corrected 2026-10-19:** this host's Session logs from 2026-09-25 to about 2026-10-12 are in `sessions/`, which isn't the current layout. Since #599 (2026-10-09), Session logs sit beside their Command log. So those older Commands show from their Command logs, without their Session logs, unless you move them. Shout if you'd rather the Console also read `sessions/`.
- **Console accounts are kept in a file of their own** beside the Credentials, readable only by you, holding argon2id hashes and hashed session tokens (Q28 a). Taking an address off the allowlist ends its sessions and blocks it at once, but keeps the account. With nobody on the list, `thirdshift console` refuses to start unless `--no-sign-in` is given (Q29 a).
- **One "set your password" flow by email code creates an account or resets its password** (Q30 a). The page answers the same way whether or not an address is allowed. A code is 6 digits, valid for 15 minutes, with 5 tries, and codes go out at most once a minute and 10 times a day per address. `thirdshift console password --email <address>` does the same on the CLI. Without a Resend key, the email flows are off and the CLI is how accounts get passwords (Q31 a). Sign-in emails come from `email.from`.
- **Setup asks whether to run the Console,** its whole address (with an example), and the allowed emails, which default to `email.to`. Then it offers `console install` (Q32 a).
- **A Command record that can't be written is a warning, and never changes a Run's outcome,** the same rule as for Run notifications. Shout if you'd rather it fail the Run.
- **Who did what** (Q33 a). A Stop writes an Activity log line naming who asked: the account's email from the Console, or the Unix user from the CLI. Account events go in a **Console log** per machine, which `thirdshift console log` prints and the page shows. Page views aren't recorded. `GLOSSARY.md` has the Console log, and the Activity log and Stop entries now mention who asked.
- **Two Specs, made with `/to-spec`, then `/to-tickets` for each** (Q34 b). #711 is the CLI: the Command record, `status`/`show`/`logs`, then Stop. A new Spec covers the Console: the read-only page with the Session log view, accounts and sign-in, Stop on the page, `console install`, Setup's questions, the Console log, and the look last, once Q27 is decided. The follow-up issues are filed separately: launchd on macOS, stopping a single Ticket's Run, and a Claim that names its machine. The CLI Spec goes first and doesn't wait on Q27; the Console Spec's last Ticket, the look, does (your note on 2026-10-18).
- **ADR 0016 records `thirdshift console` as a reader that runs all the time,** next to ADR 0009's "nothing schedules itself".
- **The look (Q27), from the `console` prototype,** in your words where quoted:
  - **One layout, many themes:** "I want the layout to be the same, but for there to be an idea of appearance themes that the user can choose between or even create themselves". A theme is design tokens: colours, fonts, shape, effects and motion. There's a picker and an editor, and themes export and import as JSON.
  - **The default is Focus, following the system's light or dark setting.** The picker lists Focus, Plain, Press, Signal, Garden and Factory, each as auto, light and dark. Focus means "weight follows need". Plain is the quiet inbox look. Press is thirdshift.app's press look in plain words. Signal uses colour only for state, in Okabe–Ito colours. Garden and Factory are themed; Factory takes after Factorio. Every built-in theme passes AA text contrast, and Focus also passes APCA.
  - **Pages, a level above sections:** 1 Inbox, 2 Lanes, 3 Stages, 4 Waterfall, 5 History, 6 Logs, 7 Security. Keys `1` to `7` switch pages, and the page is in the URL as `?page=`. Lanes has 3.5 h, 12 h and 24 h windows: "I like how the lanes variant shows the time different processes take". Which pages #776 builds is Q40.
  - **Inbox rows** show a state word (never colour alone), the issue and title, a one-line now-line and the age. A Spec run's Tickets show under it as a plan checklist. Needs you is open on load and can be folded.
  - **The peek** is the details beside the list, or a bottom sheet on a phone, and it's closed on load.
  - **A stall is silence, not stage length.** "Slower than usual (12–17 min), still working" is different from "no output for 14 min, may be stuck". Problems escalate in words: "silent 45 min · past 30", "3× usual". It's defined in Q38.
  - **Motion only for thirdshift's activity:** "I want animation to be reserved to communication of some activity of thirdshift". That keeps the Working spinner, a heartbeat dot when a log moves, the latest-line flash, the live log cursor, the stopping bar, and stripes on the running stage. A silent alarm pulses three times, then holds. Factory's conveyor and Garden's bed move only while a Command is live. A Pause motion control meets WCAG 2.2.2.
  - **Visual calm:** no coloured bars between sections, though the title's bar is fine. With Needs you open, every section heading fits above the fold on a 14" laptop.
  - **Keys:**
    - `j` and `k` walk rows and section headings, and wrap.
    - `space` folds a heading, or opens or closes the peek on a row. `enter` opens the full log.
    - `esc` closes things one layer at a time, then hides the selection mark.
    - `g` plus a letter jumps to a section or page; a tooltip lists them. `z` folds the current section. `?` lists every key. `Shift+T` opens themes.
  - **Chrome:** a Sections chooser hides the sections you pick. The tab title says "‹n› need you", and the favicon gets a dot when something is urgent.
  - **The page has no acknowledge, shelve or steer,** since the CLI can't do them (Q10). The research agrees.
  - **It works at 390 px** with no sideways scroll.
- **The Inbox is the home page** (Q35 a). History holds every ended Command, newest first, filtered by outcome and repository.
- **Needs you puts live problems first** (Q36 a): Silent, then Slow, then Died, Failed, Triage and Finding, each newest first.
- **An ended problem leaves Needs you** when a later Command on the same issue starts on this machine, or once it's older than a User config setting, 7 days by default (Q37 a). Done is the last day. Nothing is read from GitHub. The setting is Q45.
- **Silent and Slow** (Q38 a). Silent is no growth in the Session log or Command log for the silent time, a User config setting with a default of 10 minutes. Waiting stages write a line at a set interval, a User config setting with a default of 5 minutes. Slow is past the top of the usual range, which is the middle half of that stage's last 30 times in the repository, shown once there are 3. Their names are Q45.
- **`thirdshift status` shows the Inbox's groups and order,** and `--json` carries each Command's group and health (Q39 a). Lanes, Stages and Waterfall draw `status --all --json` and `show --json`, with the usual range added. Choosing a Theme has a CLI command only because the page writes it (Q46).
- **All seven pages are wanted** (Q40 a). Scope is handled when the Specs and Tickets are cut (Q48).
- **Built-in Themes are compiled in. Settings live in the User config, which the page can write in part** (Q41 a and b). What it may write is Q46, and where user Themes live is Q47.
- **The built-in Themes' fonts are embedded,** as Latin-subset woff2 files at the weights they use (Q42 a). Users' own fonts are Q47.
- **`GLOSSARY.md` has Inbox, Needs you, Silent, Slow, Usual range and Theme,** and [ADR 0018](../../docs/adr/0018-the-console-has-one-layout-and-themes.md) records one layout with Themes made of tokens (Q43). "Motion only for thirdshift's activity" and "visual calm" go in #776 as rules.
- **The research notes are on `web-ui`, the prototype stays on its branch, and the verdict is on #776** (Q44 a).
- **Three new User config settings** (Q45 a):
  - `inbox.days = 7`: how long an ended problem stays in Needs you, and how far back Resolved reaches. Done is one day.
  - `inbox.silent_minutes = 10`: when a Run is Silent. It escalates at three times that.
  - `logs.waiting_line_minutes = 5`: how often a waiting stage writes a line.

  If the waiting line doesn't come more often than the silent time, `status`, the page and Setup warn, and thirdshift uses half the silent time.
- **The page writes only its own look, under `[console]`: the chosen Theme and the hidden sections** (Q46 a). Each write is also a CLI command (`thirdshift console theme <name>`, `thirdshift console sections`). It keeps the file's comments and goes in the Console log naming the account. The look is per machine, the same for every account. "New since you last looked", Pause motion, what's folded and the peek stay in the browser. ADR 0016 is amended to say the page reads, Stops and sets its own look.
- **Your own Themes are files in `~/.thirdshift/console/themes/`, with a `fonts/` folder** (Q47 a). The Console serves those fonts from its own address. A Theme names a built-in family, a system stack or a file in `fonts/`. The page's editor saves Themes there. Fonts are copied in, never uploaded through the page. The CLI has `console theme list`, `import <file>` and `export <name>`.
- **#776 is rewritten to cover all seven pages, Themes and the settings,** with the Waterfall's Tickets last (Q48 a). Charts get an issue of their own, `needs-triage`, to be grilled later.
- **The confirm step's "requeue instead" is `thirdshift stop --requeue`,** which shipped with #773 and #774. The laptop agent thought the flag was invented; it was settled in Q12.
- **The page borrows** nested child Runs, each stage with its age, a one-line latest event, and Stop only while the process is alive, with a confirm step on the page. Session logs get folded tool calls, collapsed thinking, a renderer per tool over a plain fallback, and appends rather than redraws (Q16 a).

## Research results (2026-10-17)

- The Activity log has start and end lines, and no stage. Example: `Pickup run #740 started: commands/pickup/740-….log, on codex · gpt-6.1-sol · high`, then `ended: failed: git push … failed` an hour later. Confirmed by `~/.thirdshift/logs/JacobStephens2/thirdshift/activity.log`.
- Logs for eight repositories sit under `~/.thirdshift/logs/JacobStephens2/`, each with `activity.log`, `commands/` and `sessions/`, and for Security, `audits/`, `reproductions/` and `security-reviews/`. `credentials.toml` sits in `~/.thirdshift/`, beside `logs/`. Confirmed by `ls`.
- #574's Run was stopped by `SIGTERM` to the Pickup's parent process. Cleanup then ran, pushed the branch, and wrote `ended: failed: interrupted`. So a graceful stop already exists; it just has no command. Confirmed by #574's body.

- **Caddy 2.6.2 is the only web server here.** `/etc/caddy/Caddyfile` is managed by tracewake's Ansible and imports `/etc/caddy/sites/*.caddy`. Every other site is one file there, which Ansible leaves alone: `thirdshift.app` (with `www`) and `staging.thirdshift.app`. Caddy gets one certificate per name on its own; there's no certbot. After adding a site you restart Caddy rather than reload it, or HTTP/3 fails on Android (`site/deploy/README.md` step 4). Confirmed by `systemctl`, the Caddy files and the README.
- **`*.thirdshift.app` is a wildcard CNAME to `thirdshift.app`, at 137.184.138.99, this host.** So any subdomain already resolves here, and DNS needs no change. Nameservers are Porkbun. Confirmed by `dig`.
- **No proxy-level login exists on this host,** and Tailscale, oauth2-proxy and cloudflared aren't installed. The one app with a login, tracewake's dashboard, does it in the app: email and password, session cookie, CSRF tokens, on 127.0.0.1:8100 behind Caddy. Confirmed by `which`, the Caddy files and `/srv/tracewake/web/app.py`.
- **Caddy runs as user `caddy`, and `/home/jacob` is mode 750,** so Caddy can't read `~/.thirdshift/logs` today. Confirmed by `systemctl cat caddy`, `id caddy` and `ls -ld`.
- **Wrong, corrected 2026-10-18 (see Q23): "This is the only machine running thirdshift."** The check only looked at this host. Its hostname has been `thirdshift` since 2026-10-16; "tracewake" was its earlier name, and is also a separate project deployed here. Nothing in the repo or `~/.thirdshift` lists hosts. thirdshift knows only its own hostname (`src/host.rs`), which it uses in Run notifications and failed-run commits. Confirmed by `hostname`, the capacity memory note and `rg`.
- **thirdshift has no HTTP server code.** Its only web dependency is `reqwest` (a blocking client). Confirmed by `Cargo.toml` and `rg` for listeners.
- **The crontab on this host** runs a Pickup run every minute in seven repositories, and an Architect run (`--plan-only`) every minute in thirdshift. Confirmed by `crontab -l`.

## Research results (2026-10-18)

- **Each Ticket's Run in a Spec run, and each Base fix, is a child `thirdshift` process** started from the same Launch directory, so either can be signalled on its own. Confirmed by `src/child_run.rs`, lines 1 to 40.
- **`needs-triage` means a maintainer needs to evaluate the issue,** and `ready-for-human` means it needs a human to do the work. Confirmed by `docs/agents/triage-labels.md`.

- **What the Console can take or borrow,** from [console-ui-inspiration.md](../../docs/research/console-ui-inspiration.md). Licences were read from `gh api` and the LICENSE files; code is cited at a commit.
  - **Warp:** nothing of the Factories UI is public. Its run panel is in the client, which is AGPL-3.0 and drawn natively, so it's off-limits to copy and doesn't fit a web page. From its docs: children nest under their parent, and Stop shows only while a run can be cancelled. Cancelling a parent doesn't cancel its children, which bears on Q13: in Q13 (a), stopping a Spec run stops its Tickets' Runs.
  - **OpenHands Agent Canvas** is MIT, but heavy: React 19, Vite, Tailwind, Monaco, a 10.3 MB build. It renders its own event format. Its ideas carry over: phase plus time in phase, folded tool calls, a renderer per tool.
  - **Symphony** has a read-only dashboard, served from a JSON API and bound to localhost, with no Stop.
  - **Gas Town** (MIT, Go) is the closest match. It's server-rendered htmx compiled into the binary, and its only actions are an allowlist of its own CLI commands, each marked Safe or Confirm.
  - Off-limits to copy: Claude Squad, opcode and claudecodeui (AGPL), Superset (Elastic licence), and claude_code_agent_farm (MIT with an extra restriction).
  - **Session log viewers** mostly read Claude's saved transcripts or Codex's rollout files, not the `stream-json` and `codex exec --json` output that thirdshift keeps. The message shapes match, so rendering ideas carry over but file readers don't. Vibe Kanban's Rust types (Apache-2.0) parse exactly the Claude stream thirdshift keeps.
  - **The crate publishes only `/src`, `/skills`, `README.md` and `LICENSE`,** so the page's files have to live under one of those (`Cargo.toml` `include`).

- **Session log names give the kinds of session,** which are most of a Run's stages: `implement` (435), `spec-review` (34), `repair` (33), `security-review` (12), `implement-resume`, `architecture-review`, `security-audit`, `security-reproduction` and `security-fix-publishing`. Delivery, CI watch, Self-merge and cleanup run no session, so only the Command record will show them. Confirmed by counting names under `~/.thirdshift/logs/JacobStephens2/*/sessions/`.
- **The code is mostly portable already.** `/proc/self/exe` is used once, in `src/child_run.rs` line 375, inside a Linux-only branch. Confirmed by `rg`.

- **stephens.page is a second thirdshift machine.** Hostname `stephens`, Ubuntu 24.04.3, 4 CPUs and 15 GiB. It has thirdshift 0.11.2 in `~/.local/bin`, its own `~/.thirdshift`, and 15 every-minute crontab lines (`secure`, `architect` and `pickup`) for thirdshift, stephens.page, clave, horarium, bountywell, fareloch and voice-agent-scheduling. Its oldest Session logs are in the layout from before ADR 0011. Confirmed by a read-only SSH check.
- **stephens.page's Apache:**
  - Version 2.4.58 with the prefork MPM and mod_php. Sites use Debian's `sites-available` and `sites-enabled`, one `X.conf` and one certbot `X-le-ssl.conf` per name.
  - `proxy`, `proxy_http`, `proxy_wstunnel`, `headers`, `ssl` and `rewrite` are on; `http2` is off.
  - About 50 sites proxy to `127.0.0.1` apps run as system units with `User=jacob`.
  - `baby.stephens.page` streams server push with `flushpackets=on` and `no-gzip` on its stream path.
  - certbot 2.9.0 with the Apache plugin holds 101 certificates, one name each.
  - Confirmed by a read-only SSH check (`apache2ctl`, the site files, `sudo -n certbot certificates`).
- **stephens.page's DNS:** Cloudflare, with a `*.stephens.page` wildcard to 68.183.62.24. `console.stephens.page` is taken: a redirect to `dashboard.stephens.page`, behind Cloudflare's proxy. No enabled site mentions `thirdshift`. Confirmed by `dig` and `grep` over `sites-enabled`.
- **Lingering is off for you on stephens.page,** and the long-running apps there are system units. `systemctl --user` works only because a desktop session is open. Confirmed by `loginctl` and `/etc/systemd/system/`.

## Research results (2026-10-19)

- **#711, the CLI, has shipped and is closed.** Its Tickets #768 to #774 merged as #777 and #779 to #784, `thirdshift stop --requeue` included. Confirmed by `gh issue view` and `src/args.rs` on `main`.
- **#776 is also labelled `architect-idea`,** so an Architecture review chose it as the next move. While it's `needs-triage`, every Architect run on thirdshift is skipped (GLOSSARY.md, Architect idea). Confirmed by `gh issue view 776`.
- **Session log lines carry no time.** A Codex Session log is bare `thread.started`, `item.completed` and similar events. The Command log stamps every line `HH:MM:SS`, including one line per tool call (`implement: $ rg …`). Confirmed by `712-20261018T210511-0400-implement.jsonl` and its `.log`.
- **CI watch logs only when the count of running checks changes.** #712's CI watch had gaps of up to 2.5 minutes, so a longer CI run could go 10 minutes without a line. Confirmed by `712-20261018T210511-0400.log`, lines 218 to 223.
- **Session logs moved beside their Command log on 2026-10-09** (#599, commit `b26821d`). The ones before that are in `sessions/`. Confirmed by `git log` and `ls`.
- **thirdshift.app and the prototype load Google Fonts.** The built-in themes use ten families, all under the SIL Open Font License. Confirmed by `site/index.html` and the prototype's `themes.js`.
- **From the laptop's research notes** (on `prototype/711-console`, each citing primary sources):
  - `docs/research/console-run-monitoring-ui.md` covers CI, workflow and trace UIs, and the SRE book. It's the source of silence as the stall rule, "usually" ranges, the Stop wording and the Stage grid.
  - `docs/research/software-factory-console-trends.md` covers 2025–26 agent consoles. It's the source of the Inbox's grouping and the tab title's count.
  - `docs/research/focus-theme-attention.md` covers alarm and HMI standards (ISA-18.2 and ISA-101, EEMUA 191), WCAG 2.2 and APCA. It's the source of the Focus theme.

## Waiting on research

- Nothing. Q27 came back from your laptop on 2026-10-19.
