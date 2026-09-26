# Sep 22 2026: create cli command to implement GitHub issue

Agents, do not modify this file. It is for human handwritten content.

## Round 1

/grill-with-docs I want a way to run a cli command which triggers the run of a claude code session which essentially runs the implement skill on a github issue. I want the command to be like `implement <github issue link>` from the command line, and then that opens a claude session and essentially runs `/implement <github issue link>` inside that claude session. I'm trying to figure out how to handle the feedback from the implement session though, as it is often comes back with spec and standards findings. I usually ask the agent to address the standards findings but not always the spec findings. Maybe for simplicity we could make the initial prompt `/implement <gh issue> and address all the standards findings.` or something, and then we can assume that usually the output will be PR ready, so the prompt could maybe be: `/implement <gh issue>. Address all the standards findings. PR using /pr. Create a file named in this format: <repo>-<issue>-pr.txt Write a link to the PR to the file.` That file writing may be preferable to the following idea: the PR link perhaps could be returned by the original cli command", or easier than that.

## 2

I don't need `implement` to be a new bash command. Running something like the following is sufficient:

`{tool to invoke program} {program} {github issue link}` 

I made the factory-skills directory and put in it skills which this program can use. They're mostly from the Matt Pocock skills repo, just a little bit modified to let models invoke them headlessly more easily or invoke them themselves 

The prompt could be:

```
/implement <gh issue>
Address standards and spec findings.
Create a pull request using /pr and set it to ready for review.
```

1 - Start the Claude session headless as permissive as possible.

2 - Running `git remote get-url origin` in the dir the command was run should return a url that matches the URL of the GitHub issue, so if I run it and get `https://github.com/JacobStephens2/pocockfactory.git`, and I have passed in `https://github.com/JacobStephens2/pocockfactory/issues/1` as the issue URL, those URL's match, so the work can proceed. Otherwise the program should exit with a mismatch error.

3 - The command should create a new worktree from the branch that is checked out in the directory in which the command was run. then the agent session should be started within that worktree. 

4 - Spec findings and standard findings which were not addressed should be added to the pull request body. 

5 - Reconsider in light of what I said earlier about not needing implement to be a new bash command, but rather just a program that I can run somehow. That program can be created within this repo. 

## 3

I want to use claude auto mode, I think, instead of dangerously skip permissions. 

For the delivery problem, maybe this program could somehow have the skills within it. Or yeah. 

I modified the TDD and code review factory skills to not wait for a human. 

I updated the factory-skills dir to be called pocockfactory-skills.

1 - Use a plugin to get the factory skills into the session. I updated pocockfactory-skills/implement/SKILL.md's skill references with the pocockfactory namespace.

2 - The skills should be headless by design. I already started to edit them in this way and if you have any other edits to them you'd suggest run those by me first. And if you would suggest a revision to the prompt format, run that by me as well such as including the base branch for example as the prompt I have here already passes in the GitHub issue:

```
/implement <gh issue>
Address standards and spec findings.
Create a pull request using /pr and set it to ready for review.
```

3 - The agent fixes the findings that it agrees with, then it self-reports the ones it skipped in an unaddressed findings section of the pull request body. There is one run of /pocockfactory:code-review in total. 

4 - Which language would fit this program well? TypeScript sounds like it has potential. I think about Bash, Python, C++, C#, Go, Rust, R, Assembly, Fortran, Delphi/Object Pascal, C, Java, JavaScript, Ada, Julia, Ruby, Perl, Zig, Lisp, Scala, Haskell, Lua, and COBOL as well. 

5 - Different letter casings, a missing .git suffix and ssh origins should all count as matches. 

6 - The work tree location should be put in a sibling directory and the branch name should use the format issue-<n>. Is the br base branch the branch that is checked out in the directory in which the program command will be written. If so, then yes, I want to create off the base branch. The pull request target should be the base branch, assuming base branch is how I just described it. And yes, the agent should push. After the run, the worktree should be deleted. 

7 - I created a jacob user which I'll switch to in order to run the program. I'm curious though, will Claude run in auto mode as root? I want the agent to use the permissions that the user, the Linux user which ran the command has. 

8 - Drop the dot txt file. Keep the transcript log and yes, print the URL checked through gh in a deterministic fashion. 

## 4

I'm envisioning that the work tree creation can be done by the deterministic program, and then that program can launch the agent from within that directory. 

I just installed claude and gh as the jacob user.

1 - Compare TypeScript to Rust here. I'm interested in the single binary potential, and I'm okay with the compile step and toolchain.

2 - Yes, that prompt is good. I revised it a bit, modifying the one-line reason note.

```
/pocockfactory:implement <issue URL>
The base branch is <base>. Review with /pocockfactory:code-review using <base> as the fixed point.
Address the Standards and Spec findings you agree with.
Push branch issue-<issue number> and create a pull request against <base> using /pocockfactory:pr, marked ready for review.
In the PR body, add an "Unaddressed findings" section listing each skipped finding under Standards or Spec, with at least a one-line reason.
Include "Closes #<issue number>" in the PR body.
```

3 - I made revisions to the skills - similar to what was suggested.

4 - Always delete the work tree. Even when the run fails. A new work tree can be created fairly easily. And yes, also delete the local issue-<issue number> branch at the end of the run given it is already pushed at this point in order to keep the workspace tidy. 

5 - ~/.pocockfactory/logs/<owner>-<repo>-issue-<n>-<timestamp>.jsonl, in the home directory of the user who runs it. This directory structure should be created by the program if it doesn't exist. 

6 - Require all of the following, and exit with an error otherwise: a branch is checked out (not a detached HEAD), the branch exists on origin, and the local branch is not ahead of origin. Uncommitted changes are fine, since they're just left out.

## 5

1 - I choose Rust so that the skills can be embedded into the binary.

2 - c: On failure, have the program push the issue-<n> branch and then delete the local branch. I agree that the same cleanup should run when you press ctrl C. I'm trying to think of a way for that branch which is pushed to somehow indicate that it came out of a failed run or that a run on it was failed. Maybe a commit could be added to it or something. 

3 - If origin/issue-<n> or a PR from issue-<n> already exists then the process should work from that and rerun the implement process but using that branch and that and updating that PR. For example, I might want to continue work on that same branch and issue from a different server or a different environment. 

4 - b: Print progress lines to stderr as the session works, and Write only the PR URL to stdout.

5 - Fix it with gh pr ready and still exit with 0 if the agent left the PR as a draft.

6 - Have the seams listed in the PR body only. I updated the /pocockfactory:tdd skill accordingly.

7 - The first version should have no limits on a runaway run. If I press Ctrl-C that can end the process, and cleanup runs.

## 6

I added user.name and user.email to the jacob Linux user. I installed rustup, as well as build-essential, the C compiler and linker to build binaries.

1 - Use this failure commit:

```
git add -A && git commit --allow-empty -m "pocockfactory: failed run (<reason>)

<ISO timestamp>, host <hostname>. Uncommitted work at the time of failure is included in this commit."
```

Skip the push when there is no work, when there are no changes at all. 

2 - For continuing existing work existing work:

`git fetch origin issue-<n>`, then the worktree check out should be the branch that already exists, the issue-<n> branch.

Why should the Base branch have to match if the work tree can just check out the issue branch that's on origin? 

If a local issue-<n> exists in the launch repo and points somewhere other than origin/issue-<n>, exit with an error.

If the branch exists but its only PR is merged or closed, continue with the work anyway, creating a new branch, issue-<n>-branch-<branch number for this issue> and working with that. So if issue-100 exists on origin (not deleted on origin), but is merged or closed, then create branch issue-100-branch-2 and implement the issue on that branch.

Prompt for continuing work:

```
/pocockfactory:implement <Issue URL>

You are continuing work on branch issue-<n>, which already has commits (see git log <base>..HEAD). Build on them; don't start over.

The base branch is <base>. Review with /pocockfactory:code-review using <base> as the fixed point.

Address the Standards and Spec findings you agree with.

Push branch issue-<n>. 

[If a PR exists:] Update PR <PR URL> using /pocockfactory:pr, rewriting its body to cover the whole branch, marked ready for review. 

[Otherwise: the usual create-PR line.]

In the PR body, add an "Unaddressed findings" section.

Include "Closes #<n>" in the PR body.
```

I just added the resolving-merge-conflicts skill to the pocockfactory-skills dir. If there are merge conflicts, the agent should use that skill to resolve them.

The agent or the program should make sure that the CI checks pass and that merge conflicts are resolved.

This could work in a way where after the agent creates the pull request and finishes the program, watches CI until it either goes green or red. And then if it goes red, the program can spin up an agent to either run the resolving merge conflict skill or to address a failed check in CI. Propose prompts for these two situations.

3 - I want to brainstorm a little bit about the name of the binary and this program and project generally. I call it Pocock Factory because I'm largely using the Pocock, the Matt Pocock skills, which is a very popular repository. Top fifteen in the world I think in terms of stars.

So a lot of people are using it. But it is a pretty long name, a bit much to type, and five syllables to pronounce, though pocockfactory.com is available, but I'm also unsure about whether or not to use his name in it or not. Using his name would make it more clear to people that this is related to his workflow and so may make it more findable. I mean I'm thinking an MIT license not and to not commercialize the project, but probably to make it as a public repository on my profile, more for portfolio value. 

Generate plugin.json at runtime.

Make a Cargo project at the root of this repo.

Install with `cargo install --path .`

4 - Write the Rust single binary ADR, and the existing issue-<n> branch or PR meaning continue ADR.

## 7

I rephrased step five of the resolving-merge-conflicts factory skilll to "5. **Finish the merge/rebase.** Stage everything, commit, and push"

1 - When there's an open PR, its base is the Base branch.

2 - 

"Issue branches" are issue-<n> (which counts as branch 1) and issue-<n>-branch-<k> for k ≥ 2. The program looks at the highest-numbered one.
- If that branch has no PR, or an open PR, the program continues it. If its PR is merged or closed, it creates branch k+1 fresh from the Base branch.
- The branch may have been deleted after merging. GitHub often deletes a branch automatically once its PR merges, so issue-100 might be gone even though its merged PR exists. Starting over as issue-100 would reuse a name with a merged PR on it. So the program also checks PR history by head branch name, including closed PRs, and counts a branch as used if it has a merged or closed PR, even when the branch itself no longer exists.

Would doing all this checking make the program slow? 

3 - 

Why is .github/workflows folder prerequisite to checking CI?

1. Conflicts come first. GitHub doesn't run pull_request workflows on a PR with conflicts, so CI can't go green until the conflicts are resolved. The program runs git fetch origin <base> && git merge origin/<base> in the worktree.
   - If the merge is clean, the program pushes it.
   - If it conflicts, the program starts the conflict agent (Q4) with the merge left in progress.
   - It uses merge, not rebase, so there's never a force-push, which is safer when the branch is being worked on from several servers.
2. Watching CI. If the worktree has no .github/workflows/, the CI step is skipped and treated as passing. Otherwise the program waits until check runs appear for the head commit, then waits for them to finish (gh pr checks --watch). Waiting for them to appear avoids a false "no checks, so green" right after a push.
3. When CI goes red, the program starts the CI-fix agent (Q5), then goes back to step 1, because time may have passed and the Base branch may have moved.
4. Repair attempts are capped. This is different from the "no limits" decision: without a cap, a flaky test could keep the fix loop running forever, unattended. Cap it at 3 repair agents per run, counting conflict and CI-fix agents together.
5. The result: stdout always gets the PR URL if a PR exists. The exit code is 0 only if the PR is mergeable and CI is green. Otherwise the exit code is non-zero, and a failure commit is pushed as usual.

4 - 

```
/pocockfactory:resolving-merge-conflicts

A merge of origin/<base> into <branch> is in progress in this worktree and has conflicts.
<branch> implements <Issue URL>; its pull request is <PR URL>.

Resolve the conflicts, finish the merge, and push <branch>. Do not rebase or force-push.
```

5 - 

```
CI failed on pull request <PR URL> (branch <branch>, implementing <Issue URL>).

Failed checks:
- <check name>: <details URL>
- …

Read the failure logs (e.g. `gh run view <run-id> --log-failed`), find the root cause, and fix it. Do not skip, disable, or weaken tests or checks to make them pass.
Run the affected checks locally, commit, and push <branch>.

If a failure is not caused by this branch (it is flaky, or also fails on <base>), do not change code for it. Instead, add it to a "CI notes" section of the pull request body with a one-line explanation.
```

6 - I like nightshift a lot - I've lately been thinking about the idea of morning night me being the day shift and agents being the night shift. And there's a sort of night shift connotation to factory work in that you want the factory to be running twenty-four-seven. 

I kind of like issue2pr as well. /research to find information about these options and to brainstorm more. I like the short binary option as well, but that's not as important to me as a good name.

## 8

1 - The program detects CI via a one-minute grace period.

2 - The program pushes after every agent session. 

3 - In the case of a failed run that already has a PR, the program converts the PR back to a draft so it no longer claims to be ready. The next successful Continuation marks it ready again. This will help output quality PRs, which I want to emphasize above quantity so as to preserve the value of human attention wasted on things which could otherwise be automated. 

4 - Exit the program if the issue URL points at a closed issue. 

5 - Write one log file per session. 

6 - I think I want to go with thirdshift. I have a bit of fondness for it thinking about my dad having worked third shift at the printing press growing up and that feeling like a special thing that other kids didn't have. So we could go on vac school field trips and stuff. 

thirdshift.app is available for $9 first year $15 renewing. or thirdshift.help for $1.54 first year / $26 renewing Or thirdshift.quest for $1.50 first year / $15 renewing. I probably lean towards thirdshift.app, most of these, even though it's not exactly an application. I could eventually see it maybe turning into that. 

Here is some name idea inspiration to consider in docs/research/names.md

## 9

I name this program thirdshift. I just bought thirdshift.app and pointed its DNS to this server.

1 - I did a number of renames already, including the repo rename, but do final checks for this. I'll rename this root dir to thirdshift after we're done with this session.

2 - Just inspiration, the product and binary are both thirdshift. Maybe later I'll add personas for the agents.

## 10

I made the thirdshift repo public.

We have shared understanding. /to-spec

## 11

We can test with the ticket sub-issues of my vacation repo's spec issue https://github.com/JacobStephens2/vacation/issues/2 - or we can test with that later in the process, perhaps on sub issue https://github.com/JacobStephens2/vacation/issues/3 at the end to finally test its function or something.

Having one test seam, the thirdshift binary, sounds good. Is saying "the thirdshift binary as a black box" a meaningful distinction here? And what would it mean to have a second seam at the Origin match parsing and Issue branch selection?

# Sep 25 2026: release

1 - I'm torn between B and C. 

2 - 1.0.0, no one besides me is using this yet, and I consider v0 already done given thirdshift is already working well for me consistently.

3 - The open issues do not block the release. 

4 - Keep them in the repo but out of the published package. 

5 - Fix the stale read me before tagging. 

## 2

The reason I want to create a release is so that I can easily download thirdshift onto another linux server of mine (a Rocky Linux server), and potentially MacBook M5 Pro and run it there. I would also like for someone with running WSL to be able to download it and run it there, but if that's too much of a complicating expansion then we can just do Linux and possibly macOS for now. 

I'm fine with only being able to build thirdshift on Linux. 

6 - 

I have the https://github.com/JacobStephens2/thirdshift/issues/29 to Create a site for thirdshift, and I'm wondering if I should do that before the release. 

My vision for install is to eventually be able to have a command like how users install claude code, something like `curl -fsSL https://claude.ai/install.sh | bash` - and that can be put on the homepage as an easy way for users to install - and that potentially can essentially have the users be downloaded from GitHub or crates.io or somewhere trustworthy, I suppose, more so than a server of mine. So I'm wondering if taking rust off the prerequisites would be necessary for that kind of flow. And so then potentially lean towards B here. But I'm interested in crates.io as well. 

7 - discussed above. If we want to narrow scope though I'm open to just Linux if the macOS and WSL support is a significant lift.
  
8 - Actually in this time of significant development early, maybe I will actually move to start with 0.1.0 as the first version number for more flexibility.

9 - only in the GitHub Release body.

10 - see above.

## 3

11 - What are the tradeoffs to publishing now on crates.io or not? I'm leaning towards binaries only for now, but I'm a bit worried about lossing the opportunity to claim thirdshift as a name on crates.io if I don't get it now.

12 - I would want support for these operating systems:
- >= Rocky Linux 9.8
- >= macOS Tahoe 26.5.2
- >= Ubuntu 24.04.3
- The latest version of WSL

13 - Release before building the site.

14 - ~/.local/bin. There are no existing cargo install users, just me, so the README doesn't need to note that.

15 - I want a `thirdshift update` command to be able to update thirdshift.

16 - Put this in a Releasing section in the README.

## 4

11 - Publish and automate updates to crates.io in the release workflow in order to claim the name with a real crate.

17 - The Rocky and Ubuntu machines I'm targeting are all x86_64, so yeah I think x86_64-unknown-linux-musl and aarch64-apple-darwin should be good, and just planning on WSL2 with Ubuntu 24.04+ is an acceptable expectation for Windows.

18 (revised)- Agreed on running the first two on every PR and the Rocky and WSL smoke tests only in the release workflow.

19 - I accept as proposed.

20 - Yes, though perhaps just thirdshift version instead. Help me weigh between for the update and version commands whether to prefix them with -- or not. I like how with claude I can just run claude update - and that's what I guess without reading documentation and it works, so that's part of what makes me lean towards dropping the --. Maybe there should be a `thirdshift help` as well which shows the commands, including the `thirdshift {github issue url}` command too.

21 - Dogfood it. I want to potentially run the to-spec then to-tickets skill for this.

## 5

20 - Document the bare words and silently accept --version/-V and --help/-h

22 - agreed

23 - agreed

## 6

We have shared understanding. /to-spec

## 7

Yes

# issue 29: thirdshift.app

1 - agree

2 - a or c, the printing-press hall - as that's the kind of place my dad worked in growing up - and he worked third shift to keep the big expensive valuable machines running, sort of like I'm trying to do here with the thirdshift program. And I sort of named the app Third Shift because he worked Third Shift growing up. And so sometimes I would go to the it's called Tersac Printing and he worked on big four color presses, these big machines with rollers and the sort of process he operated. so that's a little bit of the inspiration, maybe aesthetics somewhat pulled from that but also perhaps some inspiration from just more classical printing press too and sort of hearkening to the Gutenberg transformation of printing books as a way to quickly disseminate information.

And with third shift and AI coding I kind of am thinking about a somewhat similar kind of transformation and velocity of delivering code this sort of written material in a sort of way that the printing press did for books. Now Bob Tursack the man who founded my dad's company Tursack Printing, runs Brilliant Studio, https://brilliant-graphics.com/, which my mom still does graphic design for (https://brilliant-graphics.com/about/). The history of Tursack Printing is in repos/thirdshift/docs/research/Tursack_Printing_History.md - maybe some details there about the machines or location could be found for stylistic inspiration. My dad worked at the Morgantown plant, and I would visit it.

I like the Newspaper headline type with cyberpunk colors idea.

3 - agree

4 - agree

5 - I'm interested in trying something I haven't used before. You can see the tools and stacks I've worked with at repos/thirdshift/docs/research/jacob-stephens-stack.md. Or maybe even a Rust / wasm stack to align with the Rust thing, but ultimately my biggest priority is I think #1 facilitating understanding of the tool and #2 capturing attention

6 - agree

7 - agree

8 - v0.1.0 has been published, so c  we can do.

## 2

I want this page to attribute or to make it clear that this is a very Matt Pocock skills workflow oriented product / project / program (https://www.aihero.dev/skills, https://github.com/mattpocock/skills).

I'm kind of between a more grayscale palette and this more CMYK kind of palette. I think I'd like to start with the CMYK to lean into the printing press room aesthetic and touch a little bit more into the cyberpunk palette. /research inspiration for some best practices or libraries or reference points for this.

9 - b, I find a a quite a grand statement, so I'm wary to use it, but maybe I should just go for it. I want to be strong but not too exaggerated, but I do think this is quite cool and has potential. My brother is looking to increase automation with agents and we both like the game Factorio a lot- that can give a bit of aesthetic inspiration too, but I'm trying to build up primitives early game with this which can be pieced together to build automation / factory type work, then I can engineer the factory while the factory engineers the software.

10 - agree

11 - agree

12 - a, but do name the people and companies

13 - agree

14 - agree

## 3

15 - agree

16 - b, except the agent phrase to "Agents are doing that for code"

17 - agree

18 - 1. my dad's name is Mark Stephens, and yes, he was a pressman on the presses at Tursack printing, and worked third shift at the Morgantown plant. 2. agreed. Maybe something too about me always wondering if I would get into printing, the printing industry. so I guess this is as close as I'm getting at the moment. 3. Yes. 4. First person. I don't need to let my dad and Bob see it before it goes live, partly given almost no one will see the page.

## 4

20 - agree; 
21 - agree
22 - agree
23 - agree, though file an issue to add it later
24 - agree
25 - I want to also for a moment consider what might just best help communicate for this page, even setting aside the something new interest. Maybe even /research here.
26 - agree
27 - agree, though these and the colors and other things can be refined via a /handoff to a /prototype session with another agent, which can then /handoff back to this session
28 - agree
29 - agree

## 5

30 - Use this:

My dad, Mark Stephens, was a pressman at Tursack Printing, the print shop Robert Tursack Sr. founded in Philadelphia in 1959. My dad worked third shift at the Morgantown, Pennsylvania plant, keeping the big four-color presses running through the night. Sometimes I got to visit, and I always wondered whether I'd end up in printing myself. The lineage carries on. Bob Tursack went on to found Brilliant Graphics in Exton, where my mom still does graphic design. This is as close as I am to printing right now.

The printing press made copying words cheap. Agents are doing that for code. thirdshift is named for my dad's shift: the machines can run overnight, and I need something to keep them running.

- Jacob Stephens

31 - agreed

25 - agree

## 6

32 - agree
33 - agree
34 - agree

## 7

/handoff prototype session for the thirdshift.app visual system (Q31)

## 8 prototype

/prototype ~/Downloads/thirdshift-site-prototype-handoff.md

## 9 prototype

I'm between hero A and B. I like ink A best. I like the subtle effects best. I like the motion press, and I'm torn between A and B, as B the floor
plan seems more flexible as the process gets more complex if it does, but A looks more printing press with the rollers. Maybe the printing press
factory or something can be eventually used if needed. I'm thinking about whether or not it is a bit of an artificial constraint to use CMYK as a
process markers. The box around M and Y in floor plan makes it more clear that they are part of one agent session as compared to the side elevation
bracket. Further, the larger text of implement and review in the floor plan is easier to read than the smaller text in the side elevation.

## 10 prototype

For Press D, I like the bigger subtitles for the different roller sections, but I now prefer the bracket format of A over the box format of D.

## 11 prototype

I like the crop marks on the hero b. Hero a leaves some - perhaps too much empty space in the right middle of the layout.

The first impression I want is a proof pulled from the press. B feels more like what I would see at the print shop, whereas A feels more like a newspaper. So I'm leaning towards B.

## 12

I want a page on the site which presents the prompts that thirdshift uses, as well as the factory skills, so readers can better understand what is guiding the agents' process.

35 - agree

36 - agree

37 - I prefer the motion version of press D, but the static version of press D is okay as a fall back if the motion version cannot be played, so I agree. What I like better about the static version is the size of it, that it takes up more width on the on the page in a wider viewport such as viewing on the laptop. 

38 - a

39 - he is right for me

40 - a

## 13

41 - agree - it would be cool if the prompts for example could pull from the source code To reduce the chance for drift, but but that's not essential if that's too complicating. 

42 - agree

43 - agree

44 - agree

45 - agree

46 - agree

## 14

/to-spec