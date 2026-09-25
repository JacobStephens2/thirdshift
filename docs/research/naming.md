# Naming research: pocockfactory

Research date: 2026-09-24. All availability checks were live requests made that day against the first-party APIs listed below. Anything I couldn't check is marked **unchecked**.

**Method.** I checked names against these sources:

- crates.io: `https://crates.io/api/v1/crates/<name>` (200 = taken, 404 = free).
- npm: `https://registry.npmjs.org/<name>` (200 = taken, 404 = free).
- GitHub: `https://api.github.com/search/repositories?q=<name>&sort=stars` and `https://api.github.com/repos/<owner>/<repo>`. The local `gh` token was invalid (`gh auth status`: "The token in /root/.config/gh/hosts.yml is invalid"), so these were unauthenticated API calls.
- Domains: the registry's own RDAP server (200 = registered, 404 = not registered).
  - `.dev`: `https://pubapi.registry.google/rdap/domain/<d>`
  - `.com`: `https://rdap.verisign.com/com/v1/domain/<d>`
  - `.io`: `https://rdap.identitydigital.services/rdap/domain/<d>`

**Out of scope.** I did not search the USPTO or other trademark registers beyond Apple's own list, so trademark status for every name except "Night Shift" is **unchecked**.

## TL;DR

- **"nightshift" is blocked in practice.** Two things stand in the way:
  - An established tool in the exact same niche already uses the name: [marcus/nightshift](https://github.com/marcus/nightshift) (1,038 stars, MIT, Go) "uses your leftover Claude / Codex budget to surprise you with useful PRs". At least four other AI-agent projects also use the name.
  - "Night Shift®" is a registered Apple trademark, and Apple's guidelines forbid using its marks as part of a product name.
- **"issue2pr" is usable.** It's free on crates.io and npm, and issue2pr.com, .dev and .io are all unregistered. The only prior GitHub use is a dormant 2014 repo.
- **Don't put "Pocock" in the name.** It isn't prohibited, but Matt Pocock ships his own AFK-agent orchestrator ([mattpocock/sandcastle](https://github.com/mattpocock/sandcastle)), so the name would read as first-party. Credit him in the README instead.
- **Top pick: `thirdshift`.** It keeps the owner's shift metaphor: in a 24/7 plant the third shift is the overnight one. It's free on crates.io and npm, and has no meaningful GitHub collision.

## 1. "nightshift" and "issue2pr"

### Registries and domains

| Check | nightshift | issue2pr |
|---|---|---|
| crates.io | **Taken**: `nightshift` is a placeholder, description "<renamed to nightlight>", v0.0.5-pre, last updated 2020-04-27 ([API](https://crates.io/api/v1/crates/nightshift)). `night-shift` is free ([API](https://crates.io/api/v1/crates/night-shift)). | Free ([API](https://crates.io/api/v1/crates/issue2pr)) |
| npm | `nightshift` is free ([registry](https://registry.npmjs.org/nightshift)). `night-shift` is **taken** ("Toggle OS X Night Shift", v1.0.0, [registry](https://registry.npmjs.org/night-shift)). | Free ([registry](https://registry.npmjs.org/issue2pr)) |
| .com | Registered. DNS resolves (A 64.68.202.11), but HTTPS did not respond (https://nightshift.com) | Not registered ([RDAP 404](https://rdap.verisign.com/com/v1/domain/issue2pr.com)) |
| .dev | Registered ([RDAP 200](https://pubapi.registry.google/rdap/domain/nightshift.dev)). HTTPS did not respond. | Not registered ([RDAP 404](https://pubapi.registry.google/rdap/domain/issue2pr.dev)) |
| other | nightshift.sh is live: a "Sovereign AI for Business" company, linked from the [nightshiftco GitHub org](https://github.com/nightshiftco) | issue2pr.io is not registered ([RDAP 404](https://rdap.identitydigital.services/rdap/domain/issue2pr.io)) |
| github.com/JacobStephens2/… | Free ([API 404](https://api.github.com/repos/JacobStephens2/nightshift)) | Free ([API 404](https://api.github.com/repos/JacobStephens2/issue2pr)) |

### GitHub and existing tools

**nightshift**: a GitHub search returns 896 repos ([search](https://github.com/search?q=nightshift&type=repositories&s=stars)). Many are direct competitors in the same niche:

| Repo | Stars | What it is |
|---|---|---|
| [marcus/nightshift](https://github.com/marcus/nightshift) | 1,038 | Overnight Claude/Codex agent that opens PRs. Installs as the binary `nightshift` via `brew install marcus/tap/nightshift` / `go install`. **Same niche, same binary name.** |
| [orwa-mahmoud/nightshift](https://github.com/orwa-mahmoud/nightshift) | 66 | "Accountable long-running coding shifts for Cursor, Codex, and Claude Code". Site: nightshift.orwamahmoud.com |
| [openslop/nightshift](https://github.com/openslop/nightshift) | 60 | "Nightly agent jobs and PR review for any repo" |
| [nightshiftco/nightshift-agent-runtime](https://github.com/nightshiftco/nightshift-agent-runtime) | 37 | Agent runtime from a company named Nightshift (nightshift.sh) |
| [dujunyi416/claude-nightshift](https://github.com/dujunyi416/claude-nightshift) | 35 | Claude Code quota automation |
| [GodModeAI2025/NightShift](https://github.com/GodModeAI2025/NightShift) | 14 | Claude Code autonomous-worker skills |
| [JudyaiLab/ai-night-shift](https://github.com/JudyaiLab/ai-night-shift) | 224 | "let your AI work while you sleep" |
| [ppuliu/night-shift](https://github.com/ppuliu/night-shift) | 23 | "autonomous overnight development agent" for Claude Code |
| [smudge/nightlight](https://github.com/smudge/nightlight) | 309 | macOS Night Shift CLI. This is the owner of the renamed `nightshift` crate. |

**issue2pr**: a GitHub search returns 12 repos ([search](https://github.com/search?q=issue2pr&type=repositories&s=stars)).

| Repo | Stars | What it is |
|---|---|---|
| [steveklabnik/issue2pr](https://github.com/steveklabnik/issue2pr) | 39 | "Transmute your Issues into Pull Requests". Ruby, last push 2014-06-09. Its Heroku site is dead. |
| [djmitche/git-issue2pr](https://github.com/djmitche/git-issue2pr) | 4 | 2017 CLI to convert an issue into a PR |
| [LONGSASASASASA/dsh-issue2pr](https://github.com/LONGSASASASASA/dsh-issue2pr) | 1 | 2026 issue→merged-PR pipeline |
| [bengabay11/ticket2pr](https://github.com/bengabay11/ticket2pr) | 2 | AI agent that turns Jira tickets into GitHub PRs (a related name) |

The old steveklabnik repo belongs to a well-known Rust figure. It isn't a blocker because it's dormant and did something different: it converted an issue into a PR object, with no agent involved. Still, it's worth a one-line nod in the README.

### Apple "Night Shift"

- Apple's trademark list includes **"Night Shift®"** with the generic term "software feature". The ® marks it as registered ([Apple Trademark List](https://www.apple.com/legal/intellectual-property/trademark/appletmlist.html)).
- Apple's third-party guidelines say: "You may not use or register, in whole or in part, … any other Apple trademark … as or as part of a company name, trade name, product name, or service name" ([Guidelines for Using Apple Trademarks](https://www.apple.com/legal/intellectual-property/guidelinesfor3rdparties.html)).
- Whether a one-word "nightshift" for a dev tool is legally "confusingly similar" is a legal question I can't settle. The practical risks are:
  - Search results for "nightshift" are dominated by the macOS feature and the tools that toggle it (for example [thompsonate/Shifty](https://github.com/thompsonate/Shifty), 1,297 stars).
  - Enforcement risk is small but non-zero.

**Verdict:** avoid "nightshift". The Apple mark alone would be a soft risk. What blocks it is marcus/nightshift: a 1,000-star tool that does nearly the same job and installs the same binary name.

## 2. Using "Pocock" or Matt Pocock's name

What the sources say:

- **License.** mattpocock/skills is MIT, "Copyright (c) 2026 Matt Pocock" ([LICENSE](https://github.com/mattpocock/skills/blob/main/LICENSE)).
  - The only condition is that "the above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software."
  - That applies only if this project redistributes the skill files, for example by vendoring them. Invoking skills the user installed separately creates no obligation.
  - MIT grants no trademark or name rights and says nothing about naming.
- **README.** It says "Hack around with them. Make them your own." It contains no guidance on naming derivatives and no attribution request beyond the license ([README](https://github.com/mattpocock/skills/blob/main/README.md)).
  - I also checked the rest of the repo's top-level files (AGENTS.md, CLAUDE.md, package.json, plugin.json, `.out-of-scope/`, via the [git tree API](https://api.github.com/repos/mattpocock/skills/git/trees/main?recursive=1)). None has naming guidance.
  - The package and plugin are themselves named `mattpocock-skills`.
- **Public statements.** A web search found no statement from him objecting to, or endorsing, the use of his name in derivative project names. That makes it **unchecked** beyond the search.
- **Community precedent.** Repos using his name exist without visible objection:
  - [mcdays94/pocock-agents](https://github.com/mcdays94/pocock-agents) (34 stars)
  - [lsimons/pocock-skills](https://github.com/lsimons/pocock-skills)
  - [seanrobertwright/archon-pocock-workflow](https://github.com/seanrobertwright/archon-pocock-workflow)
- **Confusion risk.** Matt ships his own agent orchestrator, [mattpocock/sandcastle](https://github.com/mattpocock/sandcastle) (8,135 stars, MIT).
  - Its README describes running "AFK agents" with prompts like "Fix issue #42 in this repo" ([README](https://github.com/mattpocock/sandcastle/blob/main/README.md)).
  - A tool named "pocockfactory" in the same space could easily be read as his, or as endorsed by him.

**Recommendation (balanced).** Using the name is legally permissible, and there is precedent. Even so, don't use it in the product name, for three reasons:

1. It implies an affiliation that doesn't exist, next to his real, similar product.
2. It ties the name to one dependency, which becomes a problem if the tool later supports other skill sets.
3. For a portfolio piece, a name you own is worth more.

Credit him prominently instead. A README line such as "Built on [mattpocock/skills](https://github.com/mattpocock/skills) (MIT)" does that. Nominative use like this is normal, and it keeps the search-discovery benefit. If skill files are ever vendored, include his LICENSE text.

## 3. Candidate brainstorm and collision check

Checked live on 2026-09-24:

- **crates.io / npm:** "Taken" means the exact name exists, with a short note on what it is.
- **Top GitHub repo:** the highest-starred result of `q=<name> in:name` ([GitHub search API](https://docs.github.com/en/rest/search/search#search-repositories)); the number in brackets is the total result count.

| Name | Theme | crates.io | npm | Top GitHub repo (stars) | Notes |
|---|---|---|---|---|---|
| **thirdshift** | 24/7 factory overnight shift | Free | Free | DataBassGit/ThirdShift (2) [7] | No agent-tool collisions. thirdshift.dev ([RDAP](https://pubapi.registry.google/rdap/domain/thirdshift.dev)) and .com ([RDAP](https://rdap.verisign.com/com/v1/domain/thirdshift.com)) are both registered by others. |
| **nightmill** | night + factory | Free | Free | none [0] | Cleanest name found. nightmill.dev is **not registered** ([RDAP 404](https://pubapi.registry.google/rdap/domain/nightmill.dev)); .com is registered ([RDAP](https://rdap.verisign.com/com/v1/domain/nightmill.com)). |
| **owlshift** | night owl + shift | Free | Free | dhruv1495/owlshift (0) [5] | A `owlshift` GitHub org profile appeared 2026-09-12 ([owlshift/owlshift](https://github.com/owlshift/owlshift)). owlshift.dev is not registered ([RDAP 404](https://pubapi.registry.google/rdap/domain/owlshift.dev)); .com is registered. |
| **lightsout** | lights-out manufacturing | Free | Free | icyguider/LightsOut (334) [1,873] | The top result is an AMSI/ETW-bypass offensive-security tool, an unflattering association. [DreamChaserEric/claude-lights-out](https://github.com/DreamChaserEric/claude-lights-out) (12) is in the same niche. `lights-out` is taken on crates and npm. lightsout.dev is registered. |
| graveyard-shift | overnight shift | Free | Free | cgsdev0/graveyard-shift (13) [66] | Long; a board game uses the name. `graveyard` is taken on crates and npm. |
| nightly-crew | night crew | Free | Free | none [0] | "nightly" clashes with Rust nightly toolchain terminology. |
| bydawn | "PRs by dawn" | Free | Free | none [0] | bydawn.dev is registered ([RDAP](https://pubapi.registry.google/rdap/domain/bydawn.dev)). Weak as a noun. |
| skeletoncrew | minimal overnight staff | Free | Free | no-specs/skeletoncrew (1) [15] | Collides with the *Star Wars: Skeleton Crew* TV series (trademark **unchecked**). |
| afkpr | Pocock's "AFK agent" term | Free | Free | Alcatraz323/afkprotect (12) [30] | Hard to say aloud. |
| ticket2pr | issue→PR | Free | Free | bengabay11/ticket2pr (2) [8] | An existing AI Jira→PR agent uses the exact name. |
| issuesmith | issue craftsman | Free | Free | DarrenfJ/IssueSmith (0) [3] | Reads as "makes issues", not "resolves" them. |
| nightcrew | night crew | Free | **Taken**: "Your coding agents on the night shift…" for Codex/Claude Code, v2.0.0 ([registry](https://registry.npmjs.org/nightcrew)) | iskana-fardan/nightcrew (1) [19] | **Blocker**: an npm package in the exact niche. |
| darkfactory / dark-factory | lights-out factory | `darkfactory` taken (ONNX tool) | `dark-factory` taken: "AI-powered development pipeline for Claude Code" ([registry](https://registry.npmjs.org/dark-factory)) | coleam00/dark-factory-experiment (160) [227] | "Dark factory" is already a named AI-coding pattern with many projects ([awesome-software-factories](https://github.com/varun1505/awesome-software-factories)). Crowded. |
| nightforge | night + forge | Free | Taken (Midnight blockchain tool) | kingfish600/Audiobook-NightForge (7) [35] | Minor collision. |
| overnight | overnight | Free | Taken (color theme) | seanpmaxwell/overnight (871) [988] | Too generic. |
| redeye | overnight flight | Taken | Taken | cisagov/RedEye (2,768) [655] | Crowded. |
| lamplighter | night worker | Free | Taken (debug lib) | YZS17/LampLighter (6) [82] | lamplighter.dev is registered. |

None of the names in `JacobStephens2/<name>` are taken for nightshift, issue2pr, thirdshift, nightmill, lightsout or owlshift (all return 404 from `https://api.github.com/repos/JacobStephens2/<name>`).

## 4. Shortlist

| Rank | Name | Suggested binary | Why | Watch-outs |
|---|---|---|---|---|
| 1 (top pick) | **thirdshift** | `thirdshift`, with an optional `3s` alias | Keeps the owner's metaphor (humans run days, agents run the overnight "third shift" of a 24/7 factory) without the Apple mark or the marcus/nightshift clash. Free on crates.io and npm. No agent-tool collisions on GitHub. | Both .com and .dev are taken, but a `github.io` page is enough for a portfolio project. Short alias collisions are **unchecked**. Avoid the alias `shift`, which is a shell builtin. |
| 2 | **nightmill** | `nightmill` (avoid `nm`, which is binutils' `nm` and is present at `/usr/bin/nm` on this host) | Night + mill (factory) covers both halves of the pitch. Zero hits on crates.io, npm and GitHub, and nightmill.dev is unregistered. | Less self-explanatory. The mill metaphor needs a README tagline. |
| 3 | **issue2pr** | `issue2pr` (avoid `i2p`, which is the name of [The Invisible Internet Project](https://geti2p.net/en/)) | Says exactly what it does. Free on crates.io and npm, and .com, .dev and .io are all unregistered. | Descriptive, so it's less memorable as a brand. There's prior art in steveklabnik/issue2pr (39 stars, dormant since 2014). Could be used as the tagline or a subcommand under a brand name instead: `thirdshift issue2pr <url>`. |
| 4 | **owlshift** | `owlshift` (`owl` alias **unchecked**) | Night owl + shift. Free everywhere, and owlshift.dev is unregistered. | Someone created an `owlshift` GitHub org on 2026-09-12. Slightly cute. |

**Blockers to avoid:** `nightshift` (marcus/nightshift, same niche and same binary name, plus Apple's Night Shift®), `nightcrew` (npm package in the same niche), and `dark-factory` (npm package in the same niche).

**Suggested pairing:** name the project **thirdshift** and use "issue → PR, overnight" as the tagline. The README intro could say "Built on mattpocock/skills."
