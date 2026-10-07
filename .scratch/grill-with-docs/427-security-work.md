# Grilling: #427, security work

Issue: https://github.com/JacobStephens2/thirdshift/issues/427
Research: `docs/research/security-agent-passes.md` and `docs/research/security-skill-candidates.md` (in the `427-security-grill` worktree, not yet merged)

## Round 1 (answered)

**Q1 - Scope of #427.** What the factory is responsible for in security, and what goes elsewhere. It matters because each kind of problem has a better tool, and an LLM adds something only for flaws in a repo's own code. Prompt injection counts as one of those in thirdshift and vaulted-agent, whose job is passing text to agents. The factory's own attack surface is a separate problem: sessions read issue comments (`docs/agents/issue-tracker.md` says `gh issue view --comments`), which anyone can write on the five public repos, and Codex runs unsandboxed (ADR 0012) with a `repo`-scoped `gh` token. The options:
- (a) **Own-code vulnerabilities only.** Dependencies and committed secrets go to GitHub's features and to CI checks that compare against the base (Q3). The factory's own attack surface gets its own issue, filed now.
- (b) **Own-code vulnerabilities, dependencies and secrets,** all as factory work.
- (c) **All of that plus the factory's own attack surface,** in #427.

*Recommended:* (a). Deterministic tools find known-vulnerable dependencies and committed secrets precisely and for free. The factory's attack surface is open today: one benchmark found 66.5% of malicious issues got past every guardrail in Claude Code, Codex and Cursor, so it shouldn't wait for #427.

*Answer:* agree

*Checked:* this file is on `main` of the public repo (commit 8fd5992), so Q1's description of the path into the factory is public too. Its pieces already were, in ADR 0012 and `docs/agents/issue-tracker.md`. Q5 builds on this.

**Q2 - Which shape first.** #427 proposes a Security axis in each Run's review (shape 2) and a security pass shaped like an Architect run (shape 1). The order decides what protects the repos first. The options:
- (a) **Shape 2 first,** shape 1 later.
- (b) **Shape 1 first.**
- (c) **Both** in one Spec.

*Recommended:* (a), for three reasons:
- **It targets the measured risk.** Agents write vulnerabilities into Merge runs that nobody reviews (`merge.always = true`), and keeplore, chart35 and cascade's web build deploy on merge.
- **There's nothing to disclose.** A vulnerability caught in an unmerged diff was never on the Base branch.
- **It's cheap.** It adds about one sub-agent per Run, the same order as the Standards or Spec one.

Shape 1 has the opposite profile: it can't confirm findings on this machine, it would hold one of the two build slots for hours, and it runs straight into the disclosure problem in Q4.

*Answer:* c

**Q3 - GitHub's free security features.** These are settings, separate from anything the factory does:
- Private vulnerability reporting on the five public repos.
- Secret scanning and push protection on thirdshift, cascade and keeplore. They're already on for muxboard and vaulted-agent.
- Dependabot alerts on all seven. This is the only free feature for chart35 and clave.
- CodeQL default setup on thirdshift, cascade, muxboard and vaulted-agent. Not keeplore: CodeQL has no PHP.
  - Its check fails any PR that adds a high or critical alert, so a vulnerability an agent writes stops the Self-merge. That's a deterministic gate for free.
  - Until the CI-fix Repair is told to read check-run annotations, such a Run fails and is left for you.
  - It also adds CodeQL's run time to each Run's CI wait.
- Dependabot security updates open PRs the factory never picks up. keeplore commits `vendor/`, which composer bumps wouldn't update.

The options:
- (a) **Everything except Dependabot security updates.**
- (b) **(a) without CodeQL,** until the CI-fix Repair reads check-run annotations.
- (c) **None for now.**

*Recommended:* (a). CodeQL failing a Run is the gate working, not a bug, and teaching the Repair about annotations can be a small follow-up.

*Answer:* agree, though file this as a separate issue on my https://github.com/JacobStephens2/ideas repo, as I'll want to probably turn this on separately for all my repos, but keep that work separate from this spec and tickets

**Q4 - Vulnerabilities already on the Base branch.** What a pass would find: who fixes them, and where they're written up. On a public repo, the public fix itself discloses the vulnerability, and GitHub has no private fix path the factory can run alone. The options:
- (a) **Fix unattended** when a proof-of-concept test reproduces the finding, through a normal Run. On a public repo, that means a public PR.
- (b) **Report privately and fix nothing unattended at first.**
  - The report goes to a draft security advisory on the public repos and to the Run notification everywhere.
  - The Day shift grades each finding as real, not real, or hardening, which gives you precision numbers for your own repos.
  - Then you decide on an opt-in for unattended fixes.
- (c) **Split by repo:**
  - Unattended where merging deploys the fix (keeplore, cascade's web build) or the repo is private (chart35, clave).
  - Report-only where users must update before they're protected (thirdshift, vaulted-agent, cascade's native apps).

*Recommended:* (b) now, with (c) as the likely shape of the later opt-in. Either way, an unfixed vulnerability is never described in a public issue or PR body.

*Answer:* a and b, depending on user config, like the base fix option

*Checked:* the security advisories API returns 404 on chart35 and clave, so a private repo can't hold a draft advisory. Q13 builds on this.

## Round 2 (answered)

**Q5 - Where the factory's attack-surface issue goes, and closing the path now.** Q1 gave it an issue of its own. The path is already public: ADR 0012 says Codex runs unsandboxed, `docs/agents/issue-tracker.md` tells sessions to read comments, and Q1 of this file, which is on `main`, connects the two. Anything a stranger writes on a public repo, an issue or a comment, can reach a session. GitHub's interaction limits can restrict commenting, opening issues and opening pull requests on a public repo to collaborators, for up to six months at a time. Owners and admins are exempt. The options:
- (a) **Close it now and file it publicly.** Set interaction limits (collaborators only, six months) on the five public repos, then file a public thirdshift issue that a Pickup run can fix.
- (b) **Close it now and file it privately,** as a draft security advisory on thirdshift that you fix by hand.
- (c) **File it publicly, with no limits.**
- (d) **File it in your private ideas repo, with no limits.**

*Recommended:* (a). With the path already public, keeping the issue private protects little. The limits close the path to strangers in minutes and cost nothing on repos with no outside contributors, and a public issue lets the factory fix it. Both are writes to GitHub, which Claude Code's permission check stopped me making for the ideas issue, so you'd run them; I'll give you the commands.

*Answer:* agree

**Q6 - Where the Security axis lives.** Q2 put it in this Spec. The options:
- (a) **A third axis in `thirdshift-code-review`:** a third parallel sub-agent beside Standards and Spec, with its own `## Security` section. Its findings are Security findings, listed under Security in Unaddressed findings. Every review that uses the skill gets it: the implement session, a Continuation, the Spec review, and the review Repair for Foreign commits.
- (b) **A separate Factory skill,** `thirdshift-security-review`, that each Session prompt calls after the code review.
- (c) **A separate session** after the implement session, the way the Spec review follows a Spec's Tickets.

*Recommended:* (a). It reviews the same diff from the same fixed point, and the skill already reports its axes separately so one can't mask another. It costs one sub-agent, and the Session prompts change only to name Security beside Standards and Spec. (c) costs a whole session per Run.

*Answer:* I'm wary to go with the third access in third shift code review as I'm a little bit wary to to modify the code review skill too much as I'm trying to stick a bit closer to the Matt Pocock skills. So I think I lean towards a separate factory skill and the accompanying additional session and making it such that users can configure whether or not the security review is part of the process that Third Shift runs for them 

I'm also going to want a way to run Third Shift in a security review mode only, sort of like Third Shift Architect except in a way that just looks for security improvements, like `thirdshift security` or something as a new flag.

**Q7 - What the Security axis checks, and where its text comes from.** The options:
- (a) **Write it as thirdshift's own text, the way Standards works:**
  - A fixed baseline, plus the repo's documented threat model, such as cascade's `server/docs/threat-model.md`, which wins where it speaks.
  - The method of Claude Code's `/security-review` (MIT): the diff only, high-confidence findings only, and a fresh sub-agent that tries to refute each one.
  - open-code-review's per-language security sections (Apache-2.0) for Rust, PHP, Python, TypeScript and Swift, with both notices kept.
  - None of `/security-review`'s exclusions that hide prompt injection and unsafe Rust.
- (b) **OpenAI's Codex Security CLI** (`scan --diff`) as the axis. It validates its findings and always runs an OpenAI model, which makes it another family when the Harness is Claude. Every machine needs Node 22+, Python 3.10+ and a ChatGPT login.
- (c) **Claude Code's `/security-review` as it stands.** Claude Harness only, exclusions and all.

*Recommended:* (a). It runs on both Harnesses, covers your repos' languages, keeps prompt injection in scope, and reuses the Standards pattern. Either way, the axis reports only what the diff adds. A vulnerability already on the Base branch is the pass's to find, and never goes in a PR body.

*Answer:* I like A I just want to make sure that the security skill and process is not dependent on a particular harness that any agent that is being used to operate or being used by third shifts can operate the security review. And I and what the security check is and where its text come from - we could /research looking for best practices or existing repos or techniques for this, such as the I think cloudflare and alibaba repos we already examined to this end: https://github.com/cloudflare/security-audit-skill - we could just use cloudflare's security-audit-skill. Looks easier to plugin than https://github.com/alibaba/open-code-review, and I trust Cloudflare. And could help people looking at thirdshift trust its code review skill more because they trust Cloudflare. Then the only new trust for thirdshift needed is in the process thirdshift itself runs, not as much specifically its skills. Or perhaps there is a nother even better security audit skill out there which we could use.

I want the security audit to be somewhat language agnostic so that it can generically be run on any codebase.

*Checked:* Cloudflare's skill has a scoped run over the diff between two refs, so one skill can serve both each Run's Security review and the pass (`docs/research/security-agent-passes.md` §2.1). The new research checks how to ask for one headless, and whether a better upstream skill exists.

**Q8 - When the author fixes a Security finding.** The options:
- (a) **Evidence first.** It fixes a Security finding only when it can show it: with a test that fails, or else a concrete trace from input to sink. The test stays as a regression test, and the fix must turn it green. A finding it can't show stays unaddressed.
- (b) **As for Standards and Spec:** it fixes the ones it agrees with.
- (c) **Every Security finding.**

*Recommended:* (a). Most unverified security findings are wrong, and in a Merge run nobody reviews, each wrong "fix" is churn nobody catches. It's the same bar Q4 set for the pass. A test in the PR discloses nothing live, because the code it exercises was never on the Base branch.

*Answer:* agree

*Q9 had no answer, so it's asked again in Round 3, updated for Q6's answer.*

**Q10 - The pass's name.** The options:
- (a) **`thirdshift audit`:** an Audit run, whose session is the Security audit.
- (b) **`thirdshift security`:** a Security run and its Security audit.
- (c) **A metaphor,** such as `thirdshift patrol`, for the night-shift guard's rounds.

*Recommended:* (a). `architect` and `pickup` name what the factory does, and "audit" does too. "Security run" would blur with the Security axis inside every Run.

*Answer:* I'm torn between `thirdshift audit` and I think `thirdshift secure`, as audit doesn't on its own necessarily mean security.

**Q11 - How the Security audit works.** The options:
- (a) **Adapt Cloudflare's skill** as `thirdshift-security-audit`, close to upstream and with its MIT notice: its phases, coverage ledger, `confirmed` / `needs_validation` / `rejected` records and Node validators, with the Session prompt answering its three questions. It's about 183 KB, 3.3 times all the Factory skills together, and its rule that nothing is confirmed without an OS sandbox meets Q12.
- (b) **Write a lean skill** on the shape Cloudflare's harness and Anthropic's red-team scaffold converged on: map the trust boundaries, hunt each entry surface in order of interest, have a fresh sub-agent try to refute each candidate, and confirm only with a proof-of-concept test that fails on the untouched code. Borrow Cloudflare's text and record schema where they help, with its notice.
- (c) **Run OpenAI's Codex Security CLI** (`scan`, deep mode) as the audit.

*Recommended:* (a). Adapting a proven skill is how the Factory skills began, and its coverage ledger lets passes take turns across a repo instead of re-auditing the same surface. Its size costs context, not much else. (c) ties every audit to OpenAI's models, whatever the Harness.

*Answer:* agree, unless other research suggests another upstream skill

**Q12 - Where proofs of concept run.** Q4's bar is a proof-of-concept test that reproduces the finding, and this machine can't make an OS sandbox. The options:
- (a) **In the worktree, as ordinary tests,** unsandboxed like every Run's tests (ADR 0012), with harmless payloads only, and never against a deployed site such as keeplore.app or a real third-party service.
- (b) **Only in a sandbox,** after a one-time admin step on this machine: an AppArmor profile that lets bubblewrap make namespaces, or Docker. Until then nothing is confirmed, so nothing is fixed unattended.
- (c) **Never:** confirm by reading only.

*Recommended:* (a). Every session already runs any command unsandboxed, so a sandbox around the proof of concept alone protects little. And Q4's bar is a test that fails on the untouched code, which is what the repo's own test suite runs. A sandbox for whole sessions belongs with the attack-surface issue (Q5).

*Answer:* agree. I lean towards running thirdshift on a dedicated droplet / virtual machine anyway - one that doesn't really serve production code, so the virtual machine thirdshift runs on its somewhat sandboxed to begin with.

*Checked:* as written, Cloudflare's skill confirms nothing without an OS sandbox. How its rule meets this answer waits on the research.

**Q13 - Where a pass's findings are recorded.** The settled answers send every finding to a private record. The options:
- (a) **A draft security advisory per finding on a public repo,** which the API can create and only you, and collaborators you add, can see. On a private repo, where advisories don't exist, an issue labelled as a security finding, private because the repo is. The Run notification lists each finding's severity, title and link, without the write-up, since it goes through Resend.
- (b) **The Run notification only,** write-up included.
- (c) **A local file under `~/.thirdshift`,** plus the Run notification.
- (d) **Code scanning alerts from uploaded SARIF.** Public repos only; only writers see them.

*Recommended:* (a). It's where GitHub expects a maintainer's private vulnerability work, it can be published with a CVE once fixed, and each finding gets a link the fix Run and the Day shift can share. A local file dies with the machine, and an email isn't a record you can grade.

*Answer:* agree

## Round 3 (open)

Updated on 7 October for the second research note, `docs/research/security-skill-candidates.md`. Q9, Q14, Q15 and Q16 now carry what it found, and Q20–Q23 are new.

**Q9 - A Security finding the Run introduced, left unaddressed.** Asked in Round 2 with no answer, and updated for Q6's answer and the research. The Security review may leave a finding the Run introduced unaddressed, because it couldn't show it or disagreed, or the review may be refused or end early. What happens then to a Self-merge into the Base branch? The options:
- (a) **Hold the Self-merge.** The Run ends with its PR ready for review, as a Failed run whose cause names the findings or the refusal, the way a Policy refusal leaves its PR ready. A Spec PR holds the same way while any such finding on its branch is unaddressed.
- (b) **Hold only for high or critical findings,** and merge with lower ones listed.
- (c) **Never hold:** list them and merge.

*Recommended:* (a). A merge the factory makes is one it stands behind. Listing a finding in a public PR is safe only while the code stays off the Base branch, so under (c) a merge on a public repo publishes a live vulnerability's write-up. A review the model refused proved nothing, so it holds too. Under Claude Code, Opus 5.5's cybersecurity classifier can move a session to Opus 4.8 or end it, so thirdshift should log the Model each security session actually ran on. A Run that isn't a Merge run just lists its findings for you, as it does Standards and Spec findings, and a Ticket's Run still merges into its Spec branch, since that merge doesn't reach the Base branch.

*Answer:* agree

**Q14 - Turning the Security review on.** Q6 made it your choice. The research puts a price on it: in full audit mode, the one measurement found a review cost about ten implement sessions (Q21). The options:
- (a) **Like the Base fix:** a User config setting, plus a word on the command each way, off by default, and Setup asks about it. A Pickup run passes its words to the Run it starts, so the word on one cron line turns it on for that one repo.
- (b) **On by default,** with a word and a setting to turn it off.
- (c) **The User config only.**

*Recommended:* (a). It matches the glossary's rule that with no User config a Run does only what its command asks, and every other opt-in thirdshift has. The word lets you start with the repos where the stakes are highest, such as muxboard and keeplore.

*Answer:* agree, I imagine most of my use of the security review will be dedicated rather than integrated with the security audit flag run by a cron job rather than run as part of the standard implement sessions.

**Q15 - Where the Security review sits in a Run.** The options:
- (a) **Right after the opening session** (the implement session, or a Continuation's), reviewing the branch's diff against the Base branch, before the Repair loop, so its commits get CI like the rest.
- (b) **Last, once the PR is mergeable and green,** just before the Self-merge, so it sees exactly what will merge, Repairs' changes and Foreign commits included. Any commit it pushes sends the PR back through the Repair loop.
- (c) **Both.**

*Recommended:* (a). It slots in beside the opening session the Delivery already runs, and that session's diff is nearly all of what merges: Repairs mostly fix CI and conflicts, and Foreign commits get a review Repair. (b) is the complete gate, at the cost of another CI round whenever it changes something, and (c) doubles a cost that's already high.

*Answer:* agree, and I wan ta way to configure whether this happens at each ticket or only at the spec review level after all the tickets have been completed.

**Q16 - The Security review in a Spec run.** The options:
- (a) **Once, on the Spec PR,** after the Spec review, over the whole Spec branch. A Ticket's Run gets none.
- (b) **In every Ticket's Run,** and none on the Spec PR.
- (c) **Both.**

*Recommended:* (a). The Spec PR is the merge that reaches the Base branch, and it sees how the Tickets' work fits together. At up to ten implement sessions a review, one per Spec instead of one per Ticket matters. A standalone Run and a Base fix each get their own, since each merges into the Base branch.

*Answer:* agree, though see my previous answer, though I'm considering whether to even just drop the option for doing the security review at the ticket level, as i'm not sure that I'll use it, maybe we can actually leave that out for now.

**Q17 - Names.** Q10 left you torn between `audit` and `secure`. Five things need names: the command, its pass, the pass's session, each Run's session, and that session's findings. The options:
- (a) **`thirdshift secure`:** the pass is a **Secure run**, and its session the **Security audit**. Each Run's session is the **Security review**, and its findings are **Security findings**.
- (b) **`thirdshift audit`:** an **Audit run** and its **Security audit**, with the rest as in (a).
- (c) **`thirdshift security`:** a **Security run** and its **Security audit**, with the rest as in (a).

*Recommended:* (a). `secure` is a verb like `architect` and `pickup`, and unlike "audit" it says security on its own. Capitalised, as the glossary's terms are, "Secure run" reads as a name, not a claim. "Security run" would sit too close to the Security review inside every Run.

*Answer:*a, `thirdshift secure`, and Security audit as the session, except call the pass a Security run, as Secure run can have a bit of connotation of a run that is secure as opposed to a run which is about increasing security in the codebase. 

**Q18 - What a pass's fix Run works from.** When fixing is allowed, the pass dispatches a Run for each reproduced finding, and a Run needs an issue. The options:
- (a) **On a public repo, a terse public issue** that links the draft advisory, whose write-up the fix session reads through the API. **On a private repo, the finding's own issue.**
- (b) **The advisory itself,** with a new kind of Run started on an advisory's URL.
- (c) **A public issue with the full write-up.**

*Recommended:* (a). Every Run stays a Run on an Issue URL, as the glossary has it, and the write-up stays private until the fix lands.

*Answer:*I want to have a way to run `thirdshift secure` as a cron job which can automatically do the Cloudflare security audit and address the findings and self-merge them - so a way I can leave a process running that can just improve the security of the application. I agree with a here. I'm curious even about the possibility of a spec issue and tickets sub-issues type structure for bigger fixes, like we have for architect runs, that might benefit from breaking the work up for multiple agent sessions to manage the context load of each agent session. 

**Q19 - How the pass yields.** The options:
- (a) **Like an Architect run:** skipped while another pass on the repo is running on the machine, while the repo has a Ready issue, and while one of its own findings still waits for the Day shift.
- (b) **(a) without the last condition:** it keeps auditing while findings wait.
- (c) **Only the machine lock.**

*Recommended:* (a). Work you shaped goes first, and findings shouldn't arrive faster than you read them. With fixing allowed, a reproduced finding doesn't wait, because its fix Run is dispatched. What ends a finding's wait comes in a later round.

*Answer:*agree

**Q20 - A vulnerability the Security review finds that was already on the Base branch.** The skill's scoped run audits the surfaces a diff touches, not only its added lines, so it will find old vulnerabilities too, and fixing one in the Run's public PR would disclose it. A proof-of-concept test tells old from new: one that also fails at the merge base shows a vulnerability already on the Base branch, and one that passes there shows the Run introduced it. The options:
- (a) **Treat it as a pass's finding.** It goes to the private record, stays out of the PR, and doesn't hold the merge, the way an Inherited failure isn't the branch's to fix. If fixing is allowed, it's fixed as the pass would fix it.
- (b) **Fix it in the Run anyway,** disclosing it in the public PR.
- (c) **Drop it,** and leave it for the pass.

*Recommended:* (a). It keeps the settled rule that a public PR never carries an unfixed vulnerability's write-up, and it keeps what the review found. Google's CodeMender draws the same line: it labels such findings legacy, and doesn't fail the scan on them.

*Answer:* a for when the Security audit is part of a spec run or architect run or just implement run, but b for when I just run `thirdshift secure` standalone

**Q21 - What each Run's Security review runs, given the cost.** Even scoped to a diff, the skill's full audit mode starts at least seven sub-agents, four of them reading the whole repo. In the one third-party measurement (one target, three runs, by a competitor), a `quick` run cost a median $29.95 and 40 minutes at list price, against $2.06 and 7 minutes for a plain session that found as much. Cloudflare keeps the audit off its own per-PR path. The skill also has a guidance mode for focused reviews, which runs none of its phases. The options:
- (a) **Full audit mode, `quick`, scoped to the diff,** on every Run that has the Security review on.
- (b) **Guidance mode:** one session reviews the diff with the skill's attack classes and method, without its sub-agents, and Q8's failing test is the verification. It costs roughly what a plain session does, and it's still the one upstream skill.
- (c) **Anthropic's security-guidance review prompts** (Apache-2.0) for each Run, and Cloudflare's skill only in the pass.
- (d) **Trial first:** run (a) and (b) report-only over a handful of recent PRs on both Harnesses, then choose from their cost, their time and the findings you grade.

*Recommended:* (d), expecting (b) to win. The trial answers the two questions no source can: what a review costs on your repos, and whether Codex runs the skill at all, which matters because Codex is your default Harness.

*Answer:* aree

**Q22 - Proofs of concept and the skill's sandbox rule.** This revisits Q12, because the research found the skill can't do what Q12 assumed. The skill runs target code only in an OS sandbox with four controls: no external network, an empty environment, a read-only target, and resource limits. No statement about the machine satisfies it. Used as written, it confirms nothing here, and its unconfirmed findings carry no severity. The options:
- (a) **Give the factory VM a sandbox the skill accepts:** Docker, or an AppArmor profile that lets bubblewrap run. It's a one-time admin step. The skill then confirms findings itself, with a severity, and runs proof-of-concept code away from the VM's GitHub, model and Resend credentials.
- (b) **Keep Q12 as it is.** The skill's findings come back unconfirmed, each with a local validation plan, and thirdshift's session turns each plan into a failing test in the worktree. Advisories carry no severity unless the session sets one.
- (c) **Ask Cloudflare for an opt-in policy** that allows a target's own tests on a disposable machine, as Google's CodeMender allows, and wait for it.

*Recommended:* (a) on the dedicated VM you plan, with (b) until then. A disposable VM limits what a bad test can break, not what it can read.

*Answer:* 

agree, possibly even a microVM such as sbx as another option, but maybe even making the VM sandbox a separate issue, and just getting this running initially with b.* I'm thinking of for example putting `thirdshift secure` as a cron job on this machine for example.*Q23 - The repo's own threat model.** The skill never looks for a `SECURITY.md` or a threat model. Chrome, Codex Security and Anthropic's harness all feed one in, and Anthropic saw owners dismiss real, reproduced findings because they didn't fit the project's threat model. The options:
- (a) **thirdshift's Session prompts point the review and the audit at the repo's `SECURITY.md` or threat-model file,** when it has one. That's process, not new skill text.
- (b) **Leave it to the skill.**

*Recommended:* (a). It's cheap, and it's the one input every precise reviewer in the research uses that Cloudflare's skill lacks. cascade already has one, in `server/docs/threat-model.md`.

*Answer:* 

#agree# Settled by earlier answers (shout if any is wrong)

- #427 covers vulnerabilities in a repo's own code, prompt injection included where a repo passes text to agents (thirdshift, vaulted-agent). Known-vulnerable dependencies and committed secrets aren't factory work in #427.
- GitHub's free features and the base-safe CI checks are filed as https://github.com/JacobStephens2/ideas/issues/3, outside this Spec, together with the thirdshift follow-up that points the CI-fix Repair at CodeQL's check-run annotations.
- The factory's own attack surface is filed as https://github.com/JacobStephens2/thirdshift/issues/511, labelled `needs-triage`, since how to fix it is a design call. Interaction limits (collaborators only) are on all five public repos until 2027-04-07. Once you've shaped #511, a Pickup run can take it.
- One Spec covers both shapes. The order of their Tickets is left to `/to-spec`.
- `thirdshift-code-review` stays as it is, Standards and Spec only, close to Matt Pocock's skill.
- Each Run's security work is a separate Factory skill, run in a session of its own when you turn it on (Q14). This file calls it the Security review; Q17 settles the names.
- The security-only mode you asked for in Q6 is the pass, run on its own like `thirdshift architect`. Q17 names its command.
- The security work is Harness-agnostic. It needs no tool only one Harness has, and like every session it runs on the Command's Harness, Model and Effort. Giving it a different one is #429's per-step mechanism, not this Spec's.
- It's language-agnostic: the skill works by trust boundary and attack class, not by language.
- The Security review and the pass use one upstream skill, Cloudflare's security-audit-skill. The research found no better one: nothing else is MIT, self-contained, agent-neutral, language-agnostic and able to do both jobs. thirdshift adds only its process: the Session prompts, the scope, and what happens to findings. So a reader's trust rests on the skill's publisher and on thirdshift's process.
- The skill sits in `skills/thirdshift-security-audit/` byte for byte, except the frontmatter `name`, which thirdshift's naming rule (ADR 0012) forces. Cloudflare's `LICENSE` sits beside it, the upstream commit is pinned, and a test fails if any other byte drifts.
- Every Session prompt that uses the skill states what the skill would otherwise ask about: the mode, the profile, the scope, an output directory outside the worktree, which earlier runs to read, and to end `incomplete` rather than ask.
- A Run's review in full audit mode gets an output directory of its own, so it doesn't inherit the pass's open leads. The pass keeps one output root per repo and reads its own earlier runs.
- Q7's other sources drop out: no open-code-review rules and no `/security-review` text.
- The Security review fixes a finding only when it can show it with a failing test, or else a concrete trace from input to sink. The test stays as a regression test, and the fix must turn it green.
- A proof of concept runs as an ordinary test in the worktree, unsandboxed like every Run's tests, with harmless payloads only, never against a deployed site or a real third-party service. You plan to run thirdshift on a dedicated VM that serves no production code. Q22 revisits this.
- The pass fixes a finding unattended only when a proof-of-concept test reproduces it and the fix is allowed the way a Base fix is: by a word on the command or a setting in the User config, off by default. Otherwise it only reports the finding.
- Like a failed Run's offer of a Base fix, a report whose finding wasn't fixed only because nobody allowed it offers both ways to allow it: the command, and the User config setting.
- A finding no proof of concept reproduces is never fixed unattended, whatever the setting. That settles #427's question about `needs_validation`.
- With fixing allowed, a fix on a public repo goes out as a public PR, which discloses the vulnerability when it's pushed. That window is the price of the speed.
- Every finding of the pass goes to a private record with its write-up: a draft security advisory on a public repo, or an issue labelled as a security finding on a private one. The Run notification lists each finding's severity, title and link, without the write-up. Public issues and PR bodies say only what the change does.
- Draft advisories follow the rules OpenAI's Codex Security (Apache-2.0) wrote for this step: one draft per finding; a severity only from the finding's own, and none for an unconfirmed or informational finding; a CWE only when certain; the package from the manifest; no version range claimed from one commit; the fingerprint and commit in the description; and an existing draft matched first, in every advisory state.

## Research results (2026-10-06, 2026-10-07)

- Unverified LLM security findings are mostly wrong: across 11 Python web apps, Claude Code (Sonnet 4) was right on 14% of its findings and Codex (o4-mini) on 18%, confirmed by source (Semgrep's 2025 study).
- Findings verified by a check the agent can't game are mostly right: 92.7% of 6,123 Anthropic findings were valid on external review, and Cloudflare treats a finding with no working proof-of-concept test against the untouched code as fake, confirmed by source (Anthropic's disclosure ledger; Cloudflare's blog).
- Current agents leave about 60% of their functionally correct solutions insecure (Claude Code with Opus 4.8, Codex with GPT-5.5, on the SusVibes leaderboard), confirmed by source.
- In a replay of 70 real vulnerabilities, a security-hardening skill cut reintroduction by 13.8 points, and "review your changes" by 1.9, confirmed by source (arXiv:2606.23130, Table 4).
- Security fixes that pass the proof of concept are often wrong: PatchBench solved 45.3% of tasks correctly, and 37.7% of Claude Code's fully validated AIxCC patches were semantically wrong, confirmed by source.
- A public fix discloses the vulnerability: from a public Firefox security diff, Mythos Preview wrote a working exploit in under an hour, confirmed by source (Anthropic's red team).
- GitHub's API can create a draft advisory and a temporary private fork, but no CI runs in the fork, and merging it is a web-UI button, confirmed by docs.
- This machine can't create a sandbox: `kernel.apparmor_restrict_unprivileged_userns = 1`, `unshare` fails, and bwrap, Docker and Podman aren't installed, confirmed by test.
- cloudflare/security-audit-skill (MIT, about 183 KB) confirms a finding only by running a proof of concept in an OS sandbox, asks a human in three places, and finds in one run about half of what repeated runs find, confirmed by source (the repository).
- alibaba/open-code-review (Apache-2.0) is a general diff reviewer in Go in which security is one of eight categories. It needs its own API key unless run in delegation mode, and its skill asks the user four times, confirmed by source (the repository).
- Claude Code's `/security-review` excludes prompt injection ("not a vulnerability") and calls Rust memory bugs impossible, and it runs only on Claude, confirmed by source (its MIT prompt) and docs.
- OpenAI's Codex Security CLI (Apache-2.0) runs `scan --diff` with `--fail-on-severity` on a ChatGPT login or an API key, confirmed by docs.
- A median Architecture review takes 6.7 minutes and $2.84 at list price, and a median implement session 16.1 minutes and $2.78 (Claude Code, `claude-opus-5-5` at medium, 3–5 October), confirmed by test (this machine's Command logs).
- Public repos get CodeQL (Rust yes, PHP no), secret scanning, private vulnerability reporting and SARIF upload for free; user-owned private repos get only Dependabot, confirmed by docs.
- On the seven repos, private vulnerability reporting, Dependabot alerts and CodeQL are off, and secret scanning is on only for muxboard and vaulted-agent, confirmed by test (`gh api`).
- Every Run on this machine is a Merge run (`merge.always = true`), and keeplore, chart35 and cascade's web build deploy on every push to `main`, confirmed by source (the User config and the deploy workflows).
- A failing CodeQL check explains itself in check-run annotations, not in the Actions log the CI-fix Repair is told to read, confirmed by docs.
- Dependency audits run on every PR would turn every Run red when a new advisory lands, and thirdshift wouldn't treat that as an Inherited failure until the Base branch re-runs the check, confirmed by source (inferred from `src/ci.rs`, ADR 0008 and the tools' docs).
- Sessions read issues with `--comments`, Codex runs unsandboxed, and the `gh` token has `repo` scope, confirmed by source (`docs/agents/issue-tracker.md`, ADR 0012) and test (`gh auth status`).
- 66.5% of malicious issues got past every guardrail in Cursor, Claude Code and Codex Desktop, confirmed by source (the IssueTrojanBench preprint).
- GitHub's interaction limits restrict commenting, opening issues and opening pull requests on a public repo to existing users, contributors or collaborators, for one day up to six months, and owners and admins are exempt, confirmed by docs. They're now on all five public repos, collaborators only, until 2027-04-07, confirmed by test.
- The security advisories API returns 404 on chart35 and clave, confirmed by test, and GitHub's docs scope repository security advisories to public repos, confirmed by docs.
- This file is on `main` of the public repo (commit 8fd5992), confirmed by test.
- A Base fix is allowed by the word `base-fix` on the command (`no-base-fix` forbids one) or by `fix` under `[base]` in the User config, off by default, confirmed by source (`src/args.rs`, `CONTEXT.md`). A Pickup run takes the same words for the Run it starts, confirmed by source (`src/args.rs`).
- No upstream skill beats Cloudflare's for these two uses. The runner-up, OpenAI's Codex Security skills, needs the Codex runtime, its own MCP server and helper scripts, confirmed by source (the candidates' repositories).
- Cloudflare's skill is asked for a scoped diff run in plain words. Asked loosely ("security review of this branch"), it falls into guidance mode, which runs none of its phases and writes no findings, confirmed by source (its `SKILL.md`).
- Even scoped to a diff, a full audit starts at least seven sub-agents, four of them reading the whole repo, confirmed by source (its `SKILL.md` and `RECONNAISSANCE.md`).
- One third-party measurement put a `quick` run at a median $29.95 and 40 minutes (Claude Opus 5, one target, three runs, by a competitor), against $2.06 and 7 minutes for a plain session with the same recall, confirmed by source (TheColliery's report).
- Cloudflare keeps its audit off the per-PR path ("a periodic backlog sweep and not a per-PR check"), and its own merge-request reviewer averages $1.19 a review, confirmed by source (Cloudflare's blog).
- The skill names no agent CLI and calls itself agent-neutral. The agent maps its `research` and `general` roles, which no Harness ships. It needs sub-agents (sequential ones would do) and Node.js. Whether it runs under Codex is untested. Confirmed by source (its `SKILL.md`).
- The skill's sandbox rule needs four controls, and no statement about the machine satisfies it. New test code goes in a scratch copy, never the target. Confirmed by source (its `SKILL.md`, "Universal execution safety").
- Its findings carry no CWE, package, version range or credit, and unconfirmed findings carry no severity, confirmed by source (its `report-schema.json`).
- It never looks for a repo's `SECURITY.md` or threat model, confirmed by source.
- Under Claude Code, Opus 5.5's cybersecurity classifier can move a session to Opus 4.8 or end it, and it ended one of the research's own sub-agents, confirmed by docs and test.
