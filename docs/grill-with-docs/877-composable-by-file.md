# Grilling: #877, making the factory process composable by file definition

Issue: https://github.com/JacobStephens2/thirdshift/issues/877
Decisions: [ADR-0024](../adr/0024-a-process-composes-steps-from-a-fixed-catalogue.md)
Outcome: #877 was rewritten as the Spec, with Tickets #1380–#1390
Research: [file-defined-factories.md](../research/file-defined-factories.md) (written for this grill: Warp, Archon, gh-aw, Anneal), [warp-factories-and-alternatives.md §3.1](../research/warp-factories-and-alternatives.md) ("factory as code"), [open-source-factories.md](../research/open-source-factories.md), [temporal-for-thirdshift.md](../research/temporal-for-thirdshift.md), #108 (factory primitives), #118 (prompts and skills in config), [ADR-0001](../adr/0001-rust-binary-with-embedded-skills.md), [ADR-0009](../adr/0009-the-operating-system-schedules-thirdshift.md), [ADR-0013](../adr/0013-every-harness-runs-unattended-and-fully-trusted.md)

## Round 1 (answered)

*Pre-Answer:* I think I'd want a top level folder for this file / these files (maybe there are multiple processes - such as the current run types and then people could add their own run types), a top level folder like there is for skills and prompts which makes the repo easy to read and use in that way.

*How I've read it:* There should be one file per process, kept in a visible top-level folder rather than a hidden `.thirdshift/`. thirdshift's own repository would hold today's run types there, beside `skills/` and `prompts/`, and other repositories could add their own run types in the same kind of folder. I've worked that into Q1.1(b) and Q1.2(a), and asked about the built-in run types as Q1.5. One thing to keep in mind is that `prompts/` is generated from `src/prompt.rs` today, not read by thirdshift (`src/prompts_page.rs`). Only `skills/` is a source folder, and it's compiled into the binary (ADR-0001).

*Response:* Correct, one file per process in a visible top-level folder. Even if that ends up not being the source dir, there could probably at least be a README in the dir that would point to the source dir / file(s).

**Q1.1 - What a file may define.** Today every **Command** is fixed in code: a Ticket run, Spec run, Architect run, Pickup run or Security run, each with its own sequence of sessions (`src/run.rs`, `src/spec_run.rs`, `src/architect.rs`, `src/pickup.rs`, `src/security.rs`), its own **Session prompts** (`src/prompt.rs`) and the **Factory skills** in `skills/`. Much of that code enforces rules that keep unattended work safe, such as the **Self-merge** checks, **Claims**, **Trusted account** checks before each session, the **Repair loop**'s budgets and the private handling of **Security findings**. The research found three ways other tools do this. In Warp, Claude Code and OpenCode, the file names the agents and the order is written as prose in an orchestrator's prompt. In Archon and Anneal, the order is declared in the file. In gh-aw there is no order at all: the file sets limits around a single agent. The options:
- (a) **Overrides only.** The built-in Commands keep their sequences. A file can replace the prompt or skill a named session uses, and perhaps its Harness, Model or Effort. That's #118 under another name, and nothing new can be composed.
- (b) **Processes declared from a fixed catalogue of steps.** A file in the processes folder defines a named process as steps from a list thirdshift ships: an agent session with a given skill or prompt, **Delivery**, Self-merge, a **Security review**, a **Spec review**, and so on. It also says what happens between the steps. Steps that carry the safety rules stay whole: a file can include Delivery or Self-merge but can't reorder or drop what they do inside. This is Archon's rule: "YAML coordinates. Code computes. Agents judge."
- (c) **Everything in the file,** Warp-style: the sessions, the gates, the repair rounds and the merge rules are all declared. thirdshift becomes an engine that runs whatever the file says.

*Recommended:* (b). (a) adds nothing that #118 doesn't already cover. (c) would expose rules that took many ADRs to get right, such as ADR-0004, ADR-0008, ADR-0010, ADR-0014, ADR-0015 and ADR-0020, to every file author. A file could then turn a Self-merge into a merge with no checks, and none of the tools researched lets a file do that. gh-aw comes closest: a file may permit a merge, but its engine keeps the checks. (b) lets you compose a new process, such as "grill, then spec, then build", while the rules stay inside steps that can't be split. A declared sequence can also be tested without running an agent, as Archon does with stubbed outputs. A sequence written as prose in a prompt can't be.

*Answer:* agree, and Also, I envision the ability to define at a step for for an agent of a particular family, even to be able to be defined to run a particular skill. So let's say someone wants to compose a process in which harness x model x effort pairs from three different families all run their custom bug checking skill.

I want them to be able to do that with this these process composition files. 

*Checked:* This conflicts with the glossary. **Harness**, **Model** and **Effort** are each "one per **Command**". The only exception is the **Security review**, which runs on `security.harness` and falls back to the default Harness when that can't start (GLOSSARY.md, "Harness"). Q2.4 and Q2.5 deal with this. Running a named skill without a Run already exists: `thirdshift skill <skill> [issue <issue>]` looks the name up among the Factory skills and the repository's committed `.agents/skills/` and `.claude/skills/` (docs/guide.md, "Standalone skills"; `src/skill.rs`). A step that runs a skill can reuse that lookup (Q2.8). The multi-family idea also overlaps with #429 (Multi-family review, still open), whose grill is in [429-multi-family-review.md](429-multi-family-review.md).

**Q1.2 - Where the file lives.** thirdshift has one config file today, the per-machine **User config** at `~/.thirdshift/config.toml`, and nothing in the repository. Warp keeps `factory.yaml` and its agent files in the repository and reviews changes to them through pull requests. So do Archon, gh-aw and Anneal. The options:
- (a) **In the repository, in a top-level folder** like `skills/` (its name is a Round 2 question), read from the **Base branch** when the Command launches. A process then travels with the code, and changing it takes a merged pull request.
- (b) **In `~/.thirdshift/`,** next to the User config. A process belongs to the machine and applies to every repository it runs on.
- (c) **Both:** the repository defines its processes, and the User config can override them on one machine.

*Recommended:* (a), as your pre-answer says. A process says how a repository's work gets done, so it should be reviewed and versioned with that repository, the way `AGENTS.md` and `docs/agents/` already are. Reading it from the Base branch, not the worktree, means a session can't rewrite the process it is running in. ADR-0013 runs every session fully trusted, so that matters. Per-machine settings such as Harness and Model stay in the User config, as now.

*Answer:* I'm torn about this, maybe Third shift can come pre-packaged with certain processes built into the binary such as the current way that ticket runs and spec runs and architect runs operate. But I kind of like the idea of in a user's dot third shift home folder there they can add custom processes that they can call with some third shift custom process flag command or something. 

*Checked:* Your answer agrees with your pre-answer. The top-level folder is in thirdshift's own repository, beside `skills/`, and holds the built-in processes compiled into the binary, as the Factory skills are (ADR-0001). Custom processes live in `~/.thirdshift/`. A folder in each target repository is now open again, as Q2.1. The command that starts a custom process is Q2.2. A file in `~/.thirdshift/` is outside every worktree, but sessions run unsandboxed (ADR-0013), so a session could still edit it. That's already true of `config.toml`, so the risk is no greater than today.

**Q1.3 - Who writes these files.** This decides how stable the format has to be. A format only you use can change in any release. One that other people rely on needs a version field, validation with clear errors, documentation on the site, and care over every change that would break existing files. The options:
- (a) **Other people's repositories too.** Treat the format as a public interface: versioned, validated and documented.
- (b) **Your repositories only, for now.** The format may change in any release. It gets documented and versioned once it settles.

*Recommended:* (b). Your pre-answer says people could add their own run types, so outside users are where this is heading. Still, a format frozen before thirdshift's own run types have been written in it would bake in guesses. Leaving room for a version field costs little, so the format could still go public later. Anneal makes a template immutable once it has been used (its ADR-0008), which is one way to make a format safe to share later.

*Answer:* Consider in light of 1.2

**Q1.4 - Triggers.** Warp's files declare triggers too: labels, mentions, cron schedules, webhooks. ADR-0009 settled that thirdshift never schedules itself. The operating system's scheduler starts each pass, and each pass is the same command a person would type. The options:
- (a) **No triggers in the file.** The file defines what a process does. A command starts it, typed by you or run from a crontab line, the way every Command starts today.
- (b) **The file declares triggers,** and thirdshift installs the crontab lines for them. This revisits ADR-0009 in part.
- (c) **The file declares triggers, and thirdshift gets a long-running watcher** to act on them. This reverses ADR-0009.

*Recommended:* (a). ADR-0009's reasons still hold: there's no daemon to keep alive, the next pass picks up a new binary after an update, and every pass ends with an exit code. Triggers have nothing to do with making processes composable, and they could be a separate issue later.

*Answer:* agree

**Q1.5 - Today's run types as files.** Your pre-answer puts the current run types in the folder. The question is whether thirdshift itself runs them from those files. The options:
- (a) **Yes, eventually.** Ticket run, Spec run, Architect run, Pickup run and Security run each become a definition shipped in thirdshift's own folder and compiled into the binary, as the Factory skills are (ADR-0001). A repository can then copy one and change it. The first Spec moves one run type, and the others follow as the step catalogue grows to cover them.
- (b) **Yes, all at once,** in the same Spec that introduces the format.
- (c) **No.** The built-in run types stay in code, and the folder holds only processes that people add.

*Recommended:* (a). Writing a real run type in the format is the best test of the format. The Ticket run is the obvious first candidate, but the Pickup run and Security run do things no step would cover yet, such as reserving an issue across machines and keeping findings private. Moving all five at once would hold the format back until it covers every one of them.

*Answer:* agree

**Q1.6 - Limits the file can set.** gh-aw shows a fourth option, which can be combined with any of Q1.1's. Its file declares no sequence. Instead it sets limits around the run: which writes to GitHub are allowed, caps on turns, tool calls and AI credits, and a cost cap that's on by default. For thirdshift, a file could set things like the Harness, Model and Effort for each kind of session, the Repair cap, the **Claim limit**, paths a run must not change, and whether a Self-merge is allowed at all. The options:
- (a) **Yes, in the same file.** It can set those limits for the repository's processes, but only to make them tighter than the User config. A repository can lower the Repair cap or forbid Self-merge, but it can't raise a cap or allow something the machine forbids.
- (b) **Not yet.** Limits stay in the User config and on the command line.

*Recommended:* (a). This is cheap, and it covers the planned protected-paths follow-up and #966 without new flags. Letting a file only tighten a limit, never loosen it, means a merged pull request can't take away a safeguard the machine set.

*Answer:* agree

## Round 2 (answered)

**Q2.1 - A processes folder in each repository too.** Q1.2 puts the built-in processes in the binary and custom ones in `~/.thirdshift/`. That leaves out a folder in the target repository, which would let a team share a process through pull requests. The options:
- (a) **Not now.** There are two places, the binary and `~/.thirdshift/`. A repository folder can be a separate issue once someone wants to share a process.
- (b) **Yes, now.** Processes also come from a folder in the repository, read from the **Base branch**. A repository process with the same name as one in `~/.thirdshift/` wins.

*Recommended:* (a). This matches your answer, and it means fewer places to look and no rules about which one wins. A step can still use a skill committed to the repository (Q2.8), so a team can share the parts that differ most, which are the prompts.

*Answer:* I like b, but I'm not sure how that would work with the binary nature of thirdshift, unless someone had a forked version of thirdshift they compiled. But I do like the concern of enabling a team to easily share custom processes and version control those. I suppose they could make a repo for something like a ~/.thirdshift/processes folder they keep? So maybe we can stick with a for now and teams could still find a way to version control share processes without modifying the thirdshift repo itself?

*Checked:* My wording of (b) was unclear. I meant a folder in the *target* repository, the one thirdshift works on, read at run time, not one in thirdshift's own repository. So nobody would need a fork. That said, (a) still stands, and your workaround works with it: a team can keep `~/.thirdshift/processes/` as a git clone of a shared repository. thirdshift reads whatever files are in that folder. A shared skill can go in the same clone or in the target repository (Q2.8). A per-repository folder can be a separate issue if someone wants it.

**Q2.2 - The command that starts a custom process.** thirdshift's commands today include `thirdshift <issue>` for a Ticket run or Spec run, `architect`, `pickup`, `secure` and `skill <skill> [issue <issue>]` (`src/args.rs`). The options:
- (a) **`thirdshift process <name> [issue <issue>]`**, shaped like `thirdshift skill`. The issue is optional.
- (b) **Each custom process becomes its own top-level command,** `thirdshift <name> …`. That's shorter, but a process name can clash with a built-in command, or with one added later.
- (c) **A flag on the Ticket run's command,** `thirdshift <issue> --process <name>`. That only works for processes that take an issue.

*Recommended:* (a). A future built-in command can never collide with a user's process name, and it reads like `skill`, the command it extends. AGENTS.md asks for a new top-level command to be mentioned on the site, next to `skill`.

*Answer:* agree

**Q2.3 - A custom process with a built-in's name.** Suppose `~/.thirdshift/processes/` holds a process named `ticket-run`. The options:
- (a) **It's refused.** A custom process can't take a built-in's name. To change a built-in, copy it under a new name and start the copy with `thirdshift process <name>`.
- (b) **It replaces the built-in** for every Command on that machine, including the Ticket runs a **Pickup run** or **Spec run** starts.

*Recommended:* (a). With (b), a file you copied once would quietly change what every scheduled Pickup run does, and a Command log wouldn't show it. Overriding a built-in's prompts is #118's job, which comes up once the Ticket run has moved into a file (Q1.5).

*Answer:* agree

**Q2.4 - Harness, Model and Effort for each step.** Your Q1.1 answer lets a step name its own Harness, Model and Effort. The glossary says one of each per **Command**. The options:
- (a) **A step may name any of the three.** A step that leaves one out uses the Command's, which is chosen as it is today: by the command, then the User config, then the Harness's default. The glossary becomes "one per step, defaulting to the Command's". The **Security review**'s `security.harness` becomes the first case of that.
- (b) **Only custom processes may set them for each step.** Built-in processes keep one per Command.

*Recommended:* (a). It's one rule instead of two, and the built-in Ticket run can keep its current behaviour simply by setting nothing. The command line's `--harness` still decides every step that doesn't name its own.

*Answer:* agree, I want a given process / run to be able to use multiple different harness model effort combinations for different steps. 

And this makes me wonder if the folder should be called processes or runs the folder at the top level of the third shift repo and in the user's third shift home folder. 

**Q2.5 - When a step's Harness can't start.** Your example has three model families each run a bug-checking skill. Today a Security review whose Harness can't start falls back to the default Harness. The options:
- (a) **The Command fails before any session starts.** thirdshift checks every step's Harness up front and names the one that can't start. A process gets the families it asked for or doesn't run.
- (b) **Fall back to the default Harness,** as the Security review does, and say so in a progress line.

*Recommended:* (a). If one of the three families quietly became a second copy of another, the process would still look fine, but a reviewer from that family would be missing. Checking before the first session means no work is wasted.

*Answer:* agree

**Q2.6 - Steps that run at the same time.** In your example, three bug checks run in parallel, presumably on the same branch. Two sessions editing one worktree at once would overwrite each other. The options:
- (a) **Steps that run at the same time can't change the worktree.** Each one reports what it found, and a later step works through all the reports. If a parallel step leaves changes or commits, it fails.
- (b) **Each parallel step gets its own worktree and branch,** and thirdshift merges the branches afterwards, sending conflicts to a **Repair**.
- (c) **No parallel steps for now.** The three checks run one after another.

*Recommended:* (a). This is how a code review works today, where the reviewers find and one session fixes. It avoids merging three branches that each changed the same lines. (c) would make your example take three times as long.

*Answer:* agree (though I'm thinking about the ability for a spec run to operate parallel ticket runs for example as a way that there can be multiple worktrees and parallel editing - right?)

*Checked:* Right. A **Spec run** starts a **Ticket run** for each Ticket once the Tickets blocking it are done, several at once when the dependency graph allows, up to `spec.parallel` (GLOSSARY.md, "Spec run"). Each Ticket run has its own **Issue branch** and worktree. So parallel editing happens through child Ticket runs, one per issue, each with its own Delivery. It doesn't happen between steps in one worktree. Whether a Process can start child runs is Q3.2.

**Q2.7 - How a step passes its results to the next.** A later step has to see what earlier steps found, such as the three bug reports. The options:
- (a) **thirdshift saves each step's final message to a file outside the worktree.** The next step's **Session prompt** gives that file's path. Nothing ends up in the repository unless a step commits it.
- (b) **Only through the repository and GitHub.** A step commits a file, or comments on the issue or pull request, and the next step reads that.
- (c) **Typed outputs,** as in Archon. Each step declares the shape of its output, and later steps, or conditions on whether a step runs, read named fields.

*Recommended:* (a). It handles your example without new GitHub traffic or files in the repository. (c) is what makes Archon's processes testable without an agent, but every step then has to declare an output shape. That can come later, if conditions between steps are ever needed.

*Answer:* agree

**Q2.8 - Where a step's skill comes from.** `thirdshift skill` already looks a name up in the Factory skills and the repository's committed `.agents/skills/` and `.claude/skills/`. A process in `~/.thirdshift/` that names a skill committed in one repository only works in that repository. The options:
- (a) **The same lookup, plus `~/.thirdshift/skills/<name>/SKILL.md`**, so that a custom process can carry its own skills to every repository.
- (b) **The same lookup only.** A custom skill has to be committed to each repository that uses it.

*Recommended:* (a). Your example has a custom bug-checking skill. With (b), it would have to be copied into every repository the process runs on. Your pre-answer's idea of a "skills folder" fits a home folder as well. The skill gets linked into the worktree the way Factory skills are (ADR-0012). The order of lookup is a detail for the Spec.

*Answer:* agree

**Q2.9 - The file format.** The research found that every tool keeps each agent's instructions in their own Markdown file with frontmatter. In thirdshift, a skill's `SKILL.md` already has that shape. What's left to decide is the file that lists the steps. The options:
- (a) **One TOML file per process,** `~/.thirdshift/processes/<name>.toml`, listing the steps, the skill each runs, any per-step Harness, Model and Effort, which steps run in parallel, and the catalogue steps (Delivery, Self-merge and so on). The prompts stay in skills, and there's no new format for steps.
- (b) **YAML,** as Warp, Archon and gh-aw use.
- (c) **One Markdown file per process,** with the steps in its frontmatter and notes for people in the body.

*Recommended:* (a). TOML is already the User config's format, so it uses the same parser and the same error messages. Writing the prompts as skills keeps the process file short, which is the problem Archon's constitution warns about. Only the thirdshift binary validates the file, and it refuses unknown keys, following the research's point about validators that drift out of date.

*Answer:* agree

**Q2.10 - Who writes these files (Q1.3 again).** Q1.2 settled that anyone who installs thirdshift can write a custom process in their home folder. So the format has outside users from its first release. The options:
- (a) **A required `version = 1`, unknown keys refused, and documented on the site as experimental** until the built-in Ticket run has moved into the format. After that, a change that would break existing files bumps the version and gives an error that says how to update the file.
- (b) **Undocumented and unversioned** until the format settles.

*Recommended:* (a). The version field and strict parsing cost almost nothing on day one, and adding them later would break files people already wrote. Calling it experimental leaves room to change the format while the Ticket run moves over.

*Answer:* agree

**Q2.11 - A custom process with no issue.** Your example could run with no issue, for instance bug-hunting the **Base branch** from cron. A **Pass** today is skipped when another Pass on the same repository is still running on the machine. That rule keeps scheduled runs from piling up. The options:
- (a) **A custom process started with no issue is a Pass.** It follows the same one-at-a-time rule, and its outcome is "skipped" when it has to wait, so it can run from cron like the others.
- (b) **No rule.** It always runs, and you keep it from overlapping with other runs yourself.

*Recommended:* (a). A cron line for a custom process then behaves like `pickup` or `secure`, and it can't run its sessions alongside a Pickup run on the same repository.

*Answer:* agree

**Q2.12 - What to call it.** The glossary has no word for this yet. **Command** names one invocation of thirdshift, and the glossary uses "workflow" loosely for **Delivery**. My proposal:
- **Process**: a named sequence of **Steps** that a **Command** carries out. A **Built-in process** is compiled into thirdshift. A **Custom process** is a file in `~/.thirdshift/processes/`.
- **Step**: one entry in a Process. It's either an agent session running a skill, or a catalogue step whose rules stay in thirdshift, such as **Delivery** or **Self-merge**.
- _Avoid_: workflow (taken by GitHub Actions and CI), pipeline, recipe, run type.

*Recommended:* those terms. "Process" is the issue's own word. "Run type" sounds like the stored kind ADR-0022 froze, and a Process isn't one of those.

*Answer:* Agree

## Round 3 (answered)

**Q3.1 - Which Steps may change the worktree.** Q2.6 settled that parallel Steps can't change it. Sequential Steps still need a rule. If a Step changes files and nothing delivers them, the work is lost when the worktree is cleaned up. If it commits to an **Issue branch**, the issue needs a **Claim**, as a Ticket run's does. The options:
- (a) **A Step is read-only unless it declares `edits = true`.** A read-only Step that leaves changes fails the Command, as a parallel Step does. thirdshift refuses a Process that has an editing Step unless it takes an issue (for the Issue branch and the Claim) and ends in **Delivery**. A Process with no editing Step runs in a throwaway worktree detached at the **Base branch**, as `thirdshift skill` does.
- (b) **Any Step may edit.** thirdshift works out afterwards whether there's anything to deliver.

*Recommended:* (a). thirdshift knows before the first session whether the Process needs a branch, a Claim and Delivery, and a file that would lose work gets refused rather than discovered at 3am. It's the same check Q2.6 already needs for parallel Steps.

*Answer:* I want to make the process system pretty flexible, so I want a way for worktrees to be able to be created by it - I want people to be able to get creative with the process, but still benefit from the components and runtime of thirdshift. I'm concerned though about too flexible being too easy to break, but someone perhaps could find out what works and doesn't by trial and error to some extent too - so having good feedback from thirdshift about what worked or not would be helpful - or having a process check command or something possibly - though that may be unnecessary). 

*Checked:* Two existing parts of thirdshift fit this. The **Security run** already reproduces each finding in a throwaway worktree, and the **Spec run** gives each Ticket run its own worktree. So "Steps that make their own worktrees" builds on what thirdshift already does. Q4.1 to Q4.3 turn your answer into rules, and Q4.2 is the check command.

I even envision being able to run experiments on different Processes to compare the process effectiveness and so forth.

**Q3.2 - Starting child runs from a Process.** Following your Q2.6 aside, parallel editing comes from child Ticket runs. The **Architect run** already works this way: its session publishes an issue, and thirdshift starts a Ticket run or Spec run on it. The options:
- (a) **A catalogue Step, "run the issue a Step published".** It starts a **Ticket run**, or a **Spec run** for a Spec, on that issue. For a Spec, that gives you parallel Ticket runs in separate worktrees. A grill-then-spec-then-build Process could be written this way.
- (b) **A Step that starts a Ticket run on a fixed issue** given in the file or on the command line.
- (c) **Neither for now.** A Process works in one worktree, and parallel editing stays with the Spec run.

*Recommended:* (a), but in a later slice (Q3.4). It reuses what the Architect run already does, including its checks on who published the issue, and it's the composition your pre-answer and Q2.6 aside are reaching for. (b) adds little over typing `thirdshift <issue>`.

*Answer:* agree

**Q3.3 - What a Process leaves behind.** `thirdshift skill` prints its final message to stdout. A Custom process run from cron has no one reading stdout. The options:
- (a) **The last Step's final message goes to stdout and into the Command log.** Every Step's message is kept with the Command's records, so `thirdshift show` and the Console can display them. Anything posted to GitHub is done by the Steps' own skills, not by thirdshift.
- (b) **(a), plus thirdshift posts the last message as a comment** on the issue when there is one.

*Recommended:* (a). Posting to GitHub from a file anyone can copy is the kind of write gh-aw makes you declare. A skill that should comment can say so in its own instructions. Your bug-hunting example needs its findings filed as issues, and that's a job for the skill.

*Answer:* agree

**Q3.4 - How to slice the work.** A Process's Command also needs a stored kind, an Ending, Run notifications and a place in the Console, and those come later in the slicing. A proposed order, each slice its own Ticket in one Spec:
1. The format, `version = 1` and validation, `thirdshift process <name> [issue <issue>]`, and read-only Steps with per-Step Harness, Model and Effort, run one at a time or in parallel, with results passed between Steps. That's enough for your three-family bug check.
2. Editing Steps on an Issue branch, with a Claim, ending in Delivery and optionally a Self-merge. It also adds the limits a file can set (Q1.6).
3. A Custom process with no issue becomes a Pass (Q2.11).
4. The Step that starts child runs (Q3.2).
5. The Ticket run moves into a Built-in process (Q1.5). This is probably a Spec of its own, since it needs Steps for bug reproduction, **Continuation**, **Resume** and the **Repair loop**.

*Recommended:* this order. Slice 1 is useful on its own and touches nothing that's already safe. Each later slice adds one kind of risk. Slice 5 comes last because it tests the format the hardest, and by then the format will have been used.

*Answer:* agree, but we will run the to-spec and then to-tickets skills to finalize this.

**Q3.5 - #429 and this issue.** #429's grill decided that replays over past work decide which second opinion to build, as opt-in User config. Once slice 1 lands, a second opinion from another model family can be written as a Custom process instead. The options:
- (a) **#429 keeps its plan.** I add a comment there that a Custom process can now run the replays' winning setup, so #429 may not need new User config.
- (b) **Fold #429 into this issue.**

*Recommended:* (a). #429's question is whether a second opinion catches anything, and the replays answer that. This issue only makes it cheap to try.

*Answer:* agree, though 429 may need rewording to Experiment not replay? If so do that.

*Checked:* I posted the comment ([#429 comment](https://github.com/JacobStephens2/thirdshift/issues/429#issuecomment-6417102907)), using the current terms. Any new evidence would come from an **Experiment**; the glossary already lists "replay" under _Avoid_ for that. I left #429's body alone. Its "replays" are the one-off research from October (#657), finished before Experiments existed, and dated records keep the words they were written with (ADR-0022). `thirdshift evals import-replays` has already moved that material into the code-review and Spec-review Eval suites (docs/guide.md).

**Q3.6 - An ADR for the step catalogue.** Q1.1's decision fits all three tests for an ADR. It's hard to reverse, a future reader would wonder why the file can't simply declare everything, and it came from a real trade-off against the Warp-style option. The proposal is ADR-0024, "A Process composes Steps from a fixed catalogue, and the safety rules stay inside the Steps". It would cite Archon's "YAML coordinates. Code computes. Agents judge.", and the research finding that no tool researched lets a file trigger a merge that skips the engine's own checks.

*Recommended:* write it now, with Round 3.

*Answer:* agree, though I want to think for a moment more about the catalogue composition vs the Warp-style option.

*Checked:* I haven't written the ADR. Q4.4 sets out the choice again, to help you think it over.

## Round 4 (answered)

**Q4.1 - Worktrees a Process creates.** Your Q3.1 answer asks for Processes that create worktrees. Today thirdshift owns two kinds of worktree: a throwaway one detached at the **Base branch** (the Security run's reproductions, `thirdshift skill`), and one on an **Issue branch** (a Ticket run's). The options:
- (a) **A Process declares named worktrees of either kind, and each Step says which one it runs in.** A throwaway worktree can be edited freely and is discarded at the end. What survives is the Steps' final messages and anything they post. An Issue-branch worktree needs an issue and a **Claim**, and the Process must end it in **Delivery**. Steps that run at the same time can't share a worktree that one of them edits, but each can have a throwaway worktree of its own. Q3.1's `edits = true` disappears: a Step may edit any worktree it's given.
- (b) **Q3.1(a) as written:** one worktree per Process, with `edits = true` deciding whether it is an Issue branch.

*Recommended:* (a). It gives you the flexibility you asked for. For example, three families could each try a fix in their own throwaway worktree, and a judging Step then picks one to redo on the Issue branch. Q2.6's rule loosens from "parallel Steps are read-only" to "parallel Steps don't share an edited worktree". Only the Issue branch, the one kind of worktree whose work can be lost, needs Delivery at the end. thirdshift still creates, tracks and cleans up every worktree, as it does today (docs/guide.md, "Worktree ownership and cleanup").

*Answer:* agree

**Q4.2 - Feedback: a check command and the record of each run.** You asked for good feedback, and maybe a check command. The options:
- (a) **Both.** `thirdshift process check <name>` validates the file without running a session, and prints the plan as it resolved it: each Step, its worktree, the Harness, Model and Effort it got and where each came from, where its skill was found, and whether each Harness starts. Every run then writes one line per Step to the Command log: what ran, how long it took, and whether it succeeded, failed or was skipped, with the reason. A refused file names the rule it broke and the line it broke it on. `thirdshift process` with no name lists the Custom processes.
- (b) **Only the run record.** Validation happens when the process starts, and a refused file fails it before any session.

*Recommended:* (a). The research warned against a validator that drifts away from the engine. Here `check` is the same binary that runs the Process, using the same code. Trial and error then costs seconds, not a failed session. Checking whether each Harness starts is the same test Q2.5 runs before the first session.

*Answer:* agree

**Q4.3 - When a Step fails.** A session can end without succeeding, or a Harness can stop partway. The options:
- (a) **The Process stops at the first failed Step and the Command fails.** A Step marked `optional = true` (the name comes from Anneal) records its failure and lets the Process carry on. In a parallel group, the other Steps finish first, then the group fails if any required Step in it failed. Issue-branch work gets the same treatment a failed Ticket run's does: thirdshift keeps it and names it in the failure.
- (b) **Every Step runs regardless,** and the Command fails at the end if any Step failed.

*Recommended:* (a). It's how today's Commands behave. A family that drops out of a three-family check can then be either fatal or tolerated, as the author chooses. Carrying on past a failed Step would hand the next one results that aren't there.

*Answer:* agree

**Q4.4 - The catalogue or Warp-style, put again.** You wanted to think about this some more, so here is the question restated:
- **What the catalogue limits:** only what *thirdshift itself* does for you, which is merging, holding a Claim, budgeting Repairs, and recording Security findings privately. A skill Step's session can already do anything a Warp agent can. ADR-0013 runs it unsandboxed, with `git` and `gh`. So the catalogue doesn't restrict what agents can do. It restricts which safeguards a file can turn off.
- **What Warp-style would add:** declaring those parts. For example "merge when CI is green", "three Repairs", or "skip the Claim". Warp gets away with this because it never merges. thirdshift merges by itself (ADR-0004), and the rules around that took ADR-0008, 0010, 0014 and 0020 to get right.
- **The middle route,** which is what we've settled: catalogue Steps take settings that can only tighten (Q1.6). New Steps get added to the catalogue when someone needs one, and Q4.5 adds plain commands. That's how Archon grew its node types.

The options:
- (a) **The catalogue, as settled.** Write ADR-0024 as proposed.
- (b) **Warp-style for everything except merging.** Delivery's rounds, the Claim and Repair budgets become plain settings in the file, and only the Self-merge stays a whole Step.
- (c) **Wait** until slice 1 has been used.

*Recommended:* (a). (b) would let a file loosen limits the User config sets, which Q1.6 ruled out. ADR-0024 can say how the catalogue grows, so it doesn't freeze the list. (c) costs nothing in itself, but slice 1's Spec has to assume one answer or the other.

*Answer:* agree

**Q4.5 - Steps that run a command, not an agent.** Archon's rule is "Code computes": a node can run a shell script instead of an agent. With such a Step, a Process could run the tests, collect files, or call a script between agent Steps, deterministically and without spending tokens. The options:
- (a) **Yes, a command Step.** It runs a command in its worktree. A non-zero exit fails the Step (Q4.3), and the output passes on like a final message (Q2.7). It sits in slice 1 alongside session Steps.
- (b) **Not now.** Every Step is an agent session or a catalogue Step.

*Recommended:* (a). It's the cheapest kind of flexibility, and it's easy to reason about because it's deterministic. It adds no risk: the command runs as you, as the sessions already do (ADR-0013), from a file in your own home folder.

*Answer:* agree - is part of this Allowing people to add in scripts or deterministic calls to run at certain parts of the process? If so, I like that idea. And if not, I think I l still like that idea - so having steps that run commands or scripts pointed to. This kind of makes me want to compare toml to bash, but I think toml is still better here for the readability.

*Checked:* Yes. A command Step runs a command or a script in its worktree, at whatever point in the Process the file puts it. The file only refers to the script, which can be bash or anything else. TOML stays the format that arranges the Steps, and scripts do the computing, which is Archon's split.

**Q4.6 - Revised slices.** If Q4.1 to Q4.5 go as recommended, Q3.4's order becomes:
1. The format and its validation, `thirdshift process [check] <name> [issue <issue>]`, session Steps and command Steps, throwaway worktrees, parallel Steps, Step failures, results passed between Steps, and per-Step Harness, Model and Effort.
2. Issue-branch worktrees with a Claim, Delivery, an optional Self-merge, and the tightening limits from Q1.6.
3. A Custom process with no issue becomes a Pass (Q2.11).
4. The Step that starts child runs (Q3.2).
5. The Ticket run as a Built-in process, as its own Spec.

As you said in Q3.4, `/to-spec` and `/to-tickets` finalize this.

*Recommended:* this order.

*Answer:* agreed

## Settled by earlier answers (shout if any is wrong)

- **Q4.1:** A Process declares named worktrees, either throwaway (detached at the Base branch and discarded at the end) or on an Issue branch (needing an issue and a Claim, and ending in Delivery). Each Step names the worktree it runs in, and may edit it. Parallel Steps never share a worktree that one of them edits. This replaces Q2.6's "parallel Steps are read-only" and Q3.1's `edits = true`.
- **Q4.2:** `thirdshift process` lists the Custom processes. `thirdshift process check <name>` prints the plan as it would run, without starting a session. Every run logs one line per Step, and a refused file names the rule it broke and the line.
- **Q4.3:** The first failed Step stops the Process, unless the Step is `optional = true`. A parallel group lets its other Steps finish first. Issue-branch work is kept, as a failed Ticket run's is.
- **Q4.4, Q3.6:** The catalogue stands. It's recorded in [ADR-0024](../adr/0024-a-process-composes-steps-from-a-fixed-catalogue.md).
- **Q4.5:** Command Steps run a command or a script in their worktree. A non-zero exit fails the Step, and the output passes on as a final message does.
- **Q4.6:** There are five slices, as listed in Q4.6. Slice 5 is a Spec of its own.

## Left for the Spec

These are details `/to-spec` should settle. None of them changes a decision above.
- The stored kind, the Ending values and the **Command record** for a Process's Command, plus how it shows in `status`, the Console and **Run notifications**. ADR-0022 keeps today's stored kinds.
- The exact TOML keys, the order of skill lookup between `~/.thirdshift/skills/` and the repository, and where Steps' results are stored.
- How interruption, Stop and the `[resources]` checks apply to Steps. They should behave as they do for today's sessions.
- The site's mention of `thirdshift process`, next to `skill` (AGENTS.md, "Hand-written site pages").
- A separate issue for testing a Process with fixtures and no agent, as Archon's `workflow test` does (research §6.3).

- **Q3.2:** A catalogue Step starts a Ticket run or Spec run on an issue an earlier Step published, as the Architect run does, with its checks. Parallel editing across issues works through those child runs.
- **Q3.3:** The last Step's final message goes to stdout and the Command log, and every Step's message is kept with the Command's records. thirdshift posts nothing to GitHub for a Process. The Steps' skills do that.
- **Q3.4:** The slicing is broadly agreed, then finalized with `/to-spec` and `/to-tickets` once this grill ends. Q4.6 revises it.
- **Q3.5:** #429 keeps its plan, with the comment posted.

- **Q2.1:** There's no repository folder for now. Teams share Custom processes by keeping `~/.thirdshift/processes/` under git themselves.
- **Q2.2, Q2.3:** `thirdshift process <name> [issue <issue>]` starts a Custom process. A Custom process can't take a Built-in process's name. The site gets a mention next to `skill`.
- **Q2.4, Q2.5:** Harness, Model and Effort become one per **Step**, defaulting to the Command's. The glossary's "one per Command" for each will be amended when that ships, not before. thirdshift checks every Step's Harness before the first session, and a Harness that can't start fails the Command, with no fallback.
- **Q2.6, Q2.7:** Parallel Steps are read-only. Each Step's final message is saved outside the worktree, and the next Step's Session prompt gives its path.
- **Q2.8:** A Step's skill is looked up as `thirdshift skill` does it, plus in `~/.thirdshift/skills/`.
- **Q2.9, Q2.10:** Each Process is one TOML file, `~/.thirdshift/processes/<name>.toml`. It requires `version = 1`, and unknown keys are refused. Only the thirdshift binary validates it. The site documents it as experimental until the Ticket run has moved into it.
- **Q2.11:** A Custom process started with no issue is a **Pass**.
- **Q2.12:** Process, Built-in process, Custom process and Step are now in GLOSSARY.md, under "Factory work".

- **Q1.1, Q1.5:** A process is built from a fixed catalogue of steps. Steps that carry safety rules, such as **Delivery**, **Self-merge**, **Claims** and the **Trusted account** checks, stay whole in code. A file can include them but can't reorder or drop what they do inside.
- **Q1.1, Q1.2, Q1.5:** The built-in processes are files in a top-level folder of thirdshift's own repository, next to `skills/`, compiled into the binary as the Factory skills are. They move into it one at a time, starting with the Ticket run. Until a run type has moved, it stays in Rust.
- **Q1.2:** Custom processes live in `~/.thirdshift/`, and a command starts them (Q2.2).
- **Q1.4:** No triggers in the file. ADR-0009 stands, and cron lines call the command that starts a process.
- **Q1.6:** A process file may set limits, such as the Repair cap, protected paths and whether a Self-merge is allowed, but only to make them tighter than the User config and the command line. Now that custom processes live next to the User config, the reason is a different one: a process copied from someone else can't quietly loosen a safeguard your machine sets. This may absorb #966.
- **Q1.1:** A step may run a named skill, so "the same skill on three model families" can be written as a process. Its details are Q2.4 to Q2.8.
- **#118** stays open. Overriding a built-in's prompts comes up again once the Ticket run is a file (Q2.3).

## Research results (2026-10-26)

- There's no per-repository config today. The only config file is the User config at `~/.thirdshift/config.toml`, in TOML. Confirmed by source: `src/config.rs:1`.
- Session prompts are separate Rust functions, one for each kind of session. `prompts/` is generated from them, not read. Confirmed by source: `src/prompt.rs`, `src/prompts_page.rs`, AGENTS.md.
- thirdshift reads crontab entries but never writes them. Confirmed by source: `src/cronjobs.rs:1`.
- Warp as shipped writes its sequence and its human gates as prose in the foreman's prompt, under a `## Human gates` section. Its live v1alpha1 schema adds an optional `blueprint`, a declared stage graph, but the condition on each edge is prose that an agent evaluates, and the schema has no declarable cap. Confirmed by Warp's live schema and warp-factory-examples, per [file-defined-factories.md §1](../research/file-defined-factories.md).
- Archon declares a DAG of nodes using `depends_on`, `trigger_rule` and `when:`. Every loop must declare `max_iterations`, and `retry.max_attempts` must be between 1 and 5. Workflows can be tested against stubbed outputs without running an agent. Confirmed by Archon source at `ef046844`, per §2.
- gh-aw allows one agent per file. The file declares permissions, which network hosts it may reach, caps, and `safe-outputs` (every GitHub write it may make). Changing the Markdown body needs no recompile, but changing the frontmatter does. Confirmed by gh-aw docs and schema, per §3.
- Anneal makes each step a numbered Markdown file, with frontmatter such as `approvalGate: true`, and checks it with fifteen named validation rules. That's the closest existing match for one file per step. Confirmed by Anneal source at `bfa821b0`, per §4.
- None of these tools lets a file merge without checks the engine enforces. Warp never merges, and Archon leaves merging to the forge. Confirmed per §6.4.
- Warp's validator refuses to parse factory files locally, because "a stale copy … reports confident, wrong diagnostics". That suggests only the thirdshift binary that runs a definition should validate it. Confirmed by Warp's validator source, per §1.9.
- The paths to `archon-loop.yaml` cited in open-source-factories.md are valid only at the commit SHA those links pin, because loops have since become an Archon node type. Confirmed per §2.

## Waiting on research

- Nothing. Round 2 draws on [file-defined-factories.md §6.3](../research/file-defined-factories.md).
