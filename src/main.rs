mod architect;
mod args;
mod asks;
mod base_fix;
mod branch;
mod child_run;
mod ci;
mod claim;
mod command;
mod config;
mod delivery;
mod email;
mod failed_run;
mod git;
mod github;
mod harness;
mod host;
mod interrupt;
mod issue;
mod labels;
mod launch;
mod logs;
mod notification;
mod pass;
mod pickup;
mod poll;
mod process;
mod progress;
mod prompt;
#[cfg(test)]
mod prompts_page;
mod pull_request;
mod ready;
mod resend_key;
mod run;
mod run_ending;
mod security;
mod session;
mod setup;
mod skills;
mod spec_run;
#[cfg(test)]
mod test_support;
mod update;
mod worktree;

use std::process::ExitCode;

use architect::Outcome;
use args::{ArchitectArgs, Command, PassArgs, RunArgs};
use asks::Asks;
use command::{Ending, failure};
use config::UserConfig;
use logs::Begin;
use notification::About;
use pickup::Took;
use run::StartedBy;

const HELP: &str = "\
thirdshift turns a GitHub issue into a ready-for-review pull request, or a merged one, unattended.

usage: thirdshift <Issue URL>                         Run the factory on the issue, from the clone on the Base branch
       thirdshift merge <Issue URL>                   Run the factory on the issue, then merge its pull request
       thirdshift --no-merge <Issue URL>              Run the factory on the issue and leave its pull request for review
       thirdshift --email <Issue URL>                 Run the factory on the issue, then email how the Run ended
       thirdshift architect [<focus>]                 Review the Base branch's architecture, publish a plan for a refactor, and run it
       thirdshift architect [<focus>] --plan-only     Publish and mark ready the plan for a refactor, and stop there
       thirdshift architect base <branch> [<focus>]   Do either with <branch> as the Base branch, from a clone on any branch
       thirdshift pickup                              Take the lowest-numbered Ready issue in the repository and run it
       thirdshift pickup base <branch>                Do that with <branch> as the Base branch, from a clone on any branch
       thirdshift secure                              Audit the Base branch and record findings privately
       thirdshift secure base <branch>                Do that with <branch> as the Base branch
       thirdshift email-test [<address>]              Send a test email through Resend, to check the email setup
       thirdshift setup                               Choose your defaults, then write the User config with every setting
       thirdshift update                              Update thirdshift to the latest release
       thirdshift version                             Print thirdshift's version
       thirdshift help                                Print this help

merge, --no-merge, --email, --no-email, base-fix, --no-base-fix, parallel <n> (or
--parallel <n>), harness <name>, model <name> and effort <level> (or --harness, --model and
--effort) go before or after the Issue URL, in any order.

harness (claude, codex, agy, grok, muse or opencode), model and effort choose the Harness every session runs on, and its
Model and Effort, for each Ticket's Run and a Base fix too. For each, the command wins, then
the User config, then the default: claude, with its own Model and Effort. A Model and Effort
in the User config come from the chosen Harness's own section, so harness claude over a codex
default takes [harness.claude]:

    [harness]
    default = \"claude\"

    [harness.claude]
    model = \"opus\"
    effort = \"high\"

Before any work, the Harness's CLI must be on PATH. On claude, a named Model gets a minimal
test call with its Effort, which must succeed. On codex, a named Model and Effort must be in
codex debug models, matched regardless of case, a Model by slug or display name, and are
passed on as Codex names them. Grok Build checks grok models and its refreshed effort cache,
then runs grok -p <prompt> --always-approve --sandbox off --output-format streaming-messages-json with null stdin,
GROK_DISABLE_AUTOUPDATER=1 and GROK_FOLDER_TRUST=0. Resume uses -r <session id>.
A failure stops the command naming what to fix, before the
Claim, the worktree and any Command log. Codex sessions run codex exec --json
--dangerously-bypass-approvals-and-sandbox, with the skills in .agents/skills/. The Command
log, the Activity log's start line, the pull request's body, as in Built with claude · opus ·
high, and the Run notification each name all three.
On agy (Antigravity CLI), agy models checks names without a turn, regardless of case;
a bare model alias needs an Effort, and effort-suffixed model IDs are accepted. Every
session and Resume uses -p --dangerously-skip-permissions --output-format stream-json,
null stdin and AGY_CLI_DISABLE_AUTO_UPDATE=true; a Resume uses --conversation <id>.
agy reads AGENTS.md and GEMINI.md, falling back through an excluded GEMINI.md link to
root CLAUDE.md when neither exists. It does not read ~/.claude/.
On opencode (OpenCode), the Model id comes from your providers as provider/model;
Effort is its #variant suffix and needs a Model. A minimal standalone call checks both.
Every session, check and Resume uses run --standalone --format json --auto with the prompt
on stdin and OPENCODE_DISABLE_AUTOUPDATE=1; a Resume adds -s <stream session id>.
Skills load through the skill tool. OpenCode reads AGENTS.md, never CLAUDE.md or ~/.claude/;
thirdshift supplies an excluded AGENTS.md link to root CLAUDE.md when needed. Session export
supplies the last assistant text, usage and outcome even when the last stream event is lost.

--email sends one Run notification when the Run ends, whatever the outcome: ready for
review, merged, failed or interrupted. --email <address> sends it to <address>; a word
after --email is the address only if it has an @ and isn't a URL.

A check that fails on the pull request and also on the Base branch commit it last merged in
is an Inherited failure, not the branch's to fix: a Run whose only red checks are Inherited
failures fails, saying to fix the Base branch first, with each check's URL there and,
unless base-fix, --no-base-fix or base.fix decided it, the command that retries the Run
with base-fix. With base-fix, it starts a Base fix instead, once: it opens an issue for
those checks, labelled base-fix and ready-for-agent, runs a Merge run on it into the Base
branch, waits for it to merge, then merges the Base branch in and watches CI again. If the
Base fix fails, or the checks still fail on the Base branch once it has merged, the Run
fails, naming the Base fix issue. A Run that finds an open base-fix issue for the same Base
branch and checks waits for that one to close instead of starting another, and a Spec run's
Tickets that meet the same Inherited failure share one Base fix.

A failed check that is the branch's own starts a CI-fix Repair. If the Repair leaves the head
commit as it was, thirdshift asks GitHub to re-run those failed checks, once per head, and
watches CI there again: a check that failed on a flaky test then passes and the Run goes on.
If CI is red again, a failed check can't be re-run (only GitHub Actions jobs can), or GitHub
refuses the re-run, the Run fails.

A Run or a Spec run makes the Claim on its issue once its checks pass, before any work: it
labels the issue in-progress, in place of ready-for-agent if it has that, keeping its other
labels and creating the label if the repository lacks it. A Ticket's Run in a Spec run and a
Base fix make none, and an issue already in-progress is left as it is. A Run whose Claim
can't be made stops there, naming the cause. The Claim is released, the issue's labels put
back as they were, when the Run or the Spec run fails with nothing on origin to take over:
no Issue branch or Spec branch and no pull request. A failed Security fix keeps its Claim
even with nothing on origin, for the Day shift. It is removed once a Self-merge has left
the issue closed, and otherwise stays: a failure to release or remove it is a warning naming
the command to run by hand.

On a Spec, an issue with sub-issues, the Run is a Spec run: it takes every Ticket (sub-issue) it
can reach, in the order their \"blocked by\" links allow, each merged into the Spec branch. Its Spec
PR opens as a draft, with a Tickets checklist, once the first Ticket lands, and is marked ready
once every Ticket is done. An Unready Ticket, one labelled ready-for-human, needs-info, wontfix or
needs-triage, is never run, nor is a Ticket with sub-issues, a Ticket in a cycle of blockers, or
any Ticket that one of these, an open issue outside the Spec or a failed Ticket blocks. If any
Ticket is not done, the Spec run fails, with a line on each saying why, leaving the Spec PR a draft.
Running the Spec again continues its Spec branch, its Spec PR and each failed Ticket's Issue branch;
with every Ticket closed and no Spec branch, there is nothing to do.
It runs up to 3 Tickets at once; parallel <n> runs up to <n> for one Spec run, and spec.parallel
sets the default:

    [spec]
    parallel = 2

A Spec run sends one Run notification for the whole Spec, with a line on each Ticket, and its
Ticket Runs send none.

Tickets always merge into the Spec branch, whatever the command or the User config says.
merge on a Spec merges the Spec PR into the Base branch once it is ready, mergeable and green,
as does merge.always; without either, or with --no-merge, the Spec PR is left ready for review.

architect starts an Architect run from the clone on the Base branch, with no Issue URL. Its
Architecture review, an agent session in its own worktree at the Base branch's head on origin,
looks for deepening opportunities and publishes the top one as a plan: a Spec with Tickets, or
a single Ticket. thirdshift then checks that the plan is open, new and not labelled
ready-for-human, needs-info or wontfix, swaps its needs-triage label for ready-for-agent,
labels it architect-plan, creating the label if the repository lacks it, and dispatches it
as thirdshift <Issue URL> would: a Spec run on a Spec, a Run on a single Ticket. A Spec's
Tickets are never labelled architect-plan. That run makes the Claim on the plan, which keeps
architect-plan. The Architect run ends as that run does, with its exit code and its PR's URL.
merge, --no-merge, base-fix, --no-base-fix, parallel <n>, harness, model and effort apply to
that run, as do the User config's defaults; parallel <n> fails it if the plan is a single
Ticket. harness, model and effort apply to the review too. The review
itself watches no CI, so only that run can start a Base fix. With --plan-only, the
Architect run prints the plan's URL and stops instead, for you to read, edit and run with
thirdshift <Issue URL>, and takes none of those flags. <focus> is free text, one argument,
that points the review at an area:

    thirdshift architect \"the Spec run\"

base <branch> (or --base <branch>) names the Architect run's Base branch, whatever branch the
clone has checked out, so it can start from a clone on another branch, on a detached HEAD or
with uncommitted changes:

    thirdshift architect base main

The review starts at <branch>'s head on origin, and the run the plan is dispatched as branches
off <branch> and targets it with its pull request. <branch> must exist on origin, with no local
copy of it ahead, and launch.pull updates the clone only when <branch> is the branch checked
out. base goes with --plan-only too, before or after the focus and the other flags. base is
for a Pass only: thirdshift <Issue URL> doesn't take it. Without base, the Base
branch is the branch checked out.

A review that finds no Strong candidate publishes no plan. It files its top recommendation as
one idea issue labelled needs-triage, or names the open issue that already covers it, and
thirdshift prints that issue's URL instead. Its last line says which: the review filed the
idea, or it filed nothing. Either way thirdshift labels that issue an Architect idea:
architect-idea and needs-triage, in one request that keeps its other labels, creating
architect-idea if the repository lacks it. needs-triage goes back on an issue that had been
triaged. If the issue can't be labelled, the Architect run fails.

A review that fails, is interrupted, or ends without naming one of these issues fails the
Architect run and leaves any plan it published labelled needs-triage. One that finds no
deepening opportunity at all has no issue to name, so it fails the Architect run too.

A Pass is a command started with no Issue URL that decides before any work whether it is skipped.
Only one Pass per repository runs at a time on a machine. A Pass started while another Pass
on the same repository is still running, the Spec run or Run it dispatched included, is skipped:
it prints another Pass is already running on <owner>/<repo>, does nothing else and exits 0.
Nothing is left to clear once that other run ends, however it ends. Runs started on an Issue
URL are never skipped this way.

An Architect run that finds an open issue labelled architect-plan is skipped too, before
any review, with or without --plan-only: the last Architect plan is not finished. It names
each open Architect plan, prints its URL on stdout, gives the command that picks it up,
thirdshift <plan URL>, and exits 0. No flag overrides this: finish or close the Architect
plan, or remove its label. An Architect run never retries or dispatches an existing
Architect plan, so one whose run failed stays open until you pick it up.

Past that, an Architect run that finds an Architect idea open and still labelled needs-triage
is skipped as well: the factory has run out of Strong ideas. It prints each such idea's URL on
stdout, names it on stderr as waiting for triage, and exits 0. Any triage decision lets the
next Architect run go ahead: take needs-triage off, whatever replaces it, or close the issue.

Last, an Architect run is skipped while the repository has a Ready issue, by the search a
Pickup run makes, so that work a human shaped goes first. It prints that issue's URL on stdout,
names it on stderr as going first, and exits 0. A ready-for-agent issue that is not a Ready
issue doesn't skip it, but one with no Pickup run to take it keeps Architect runs from starting.

To run an Architect run on a schedule, have the operating system's scheduler, such as cron, run
thirdshift architect base main from the clone: the README's \"On a schedule\" has a crontab entry.

pickup starts a Pickup run from the clone, with no Issue URL: one pass, which takes the
lowest-numbered Ready issue in the repository and dispatches it as thirdshift <Issue URL>
would: a Spec run on a Spec, a Run otherwise. A Ready issue is an open issue labelled
ready-for-agent that has none of ready-for-human, needs-info, wontfix and needs-triage, is
not in-progress, is not a sub-issue, is not labelled base-fix or security-fix, has no open
blocker, and was
never started: no Issue branch for it is on origin, and no pull request from one exists,
open, merged or closed. A Spec whose Tickets are all closed is not one either: a Spec run
would find nothing to do. It must also be settled: ten minutes have passed since
ready-for-agent was applied to it, and since a sub-issue or a \"blocked by\" link of its
was last added or removed, so a Spec is not taken while its Tickets are being attached. A
sub-issue is reached through its Spec, when the Spec is itself a Ready issue.

Each ready-for-agent issue a pass looks at and does not take gets one line on stderr with
the first reason that applies, such as #21 is a Ticket of #20, which is not ready or #30
blocked by #29, before the line that says what the pass did. A line names the issue taken,
and the run it is dispatched as makes the Claim on it. The Pickup run ends as that run does,
with its exit code and its PR's URL. merge, --no-merge, base-fix, --no-base-fix,
parallel <n>, harness, model and effort apply to that run, as do the User config's defaults; parallel <n> is ignored
when the issue is not a Spec. base <branch> names the Pickup run's Base branch as it does an
Architect run's, and the dispatched run branches off <branch> and targets it:

    thirdshift pickup base main

pickup takes nothing else: no focus and no --plan-only.

A Pickup run is skipped, exiting 0 with nothing on stdout and one line on stderr saying why,
after any lines on issues it passed over, when the repository has no Ready issue, when the
repository is at its Claim limit, and while another Pass on the same
repository is still running on this machine.

A Pickup run takes nothing while as many open issues are labelled in-progress, whoever
started them, as the Claim limit, 3 unless set, so a broken Base branch can't fail every
Ready issue in turn, and pull requests can't pile up unreviewed. The skipped run's line names
the count and the limit. pickup.limit in the User config sets the Claim limit, a whole number
from 1 up:

    [pickup]
    limit = 5

There is no flag for it.

Each Pickup run that gets the lock first makes the Sweep: it takes in-progress off every
closed issue that still has it, keeping the issue's other labels, so an issue merged by hand
doesn't look taken. Closed issues never count against the Claim limit. A label it can't take
off is a warning: line, and the pass carries on.

--email, --email <address> and --no-email ask a Pickup run for its Run notification as they
do a Run, and email.always sets the default. A pass that took an issue sends one: the
notification the run it dispatched would send by hand, with that run's subject, outcome and
body. The dispatched run sends none of its own. A skipped pass sends none, even when asked.
The notification's checks, an address and a Resend API key, are made before any other work
on every pass, so one that would be skipped fails on them too, with exit 1.

To run a Pickup run on a schedule, have a scheduler, such as cron, run thirdshift pickup base main
from the clone: the README's \"A Pickup run on a schedule\" has a crontab entry.

--email, --email <address> and --no-email ask an Architect run for its Run notification as
they do a Run, with or without --plan-only, and email.always sets the default. It sends one
for the whole Architect run, however it ends: how the review ended, with the plan or idea
issue it named, and how the run the plan was dispatched as ended, with a line on each Ticket
of a Spec run. The run the plan is dispatched as sends none of its own. A skipped run sends
none, even when asked, so a scheduler can start one every few minutes. The notification's
checks, an address and a Resend API key, are made before any other work on every run, so
one that would be skipped fails on them too, with exit 1.

--email, --email <address> and --no-email ask a Security run for its Run notification as
they do a Run, and email.always sets the default. It sends one when its audit ends, whether
it succeeded, failed or was interrupted, listing each recorded finding's severity when
known, title and private link, never its write-up. A skipped Security run sends none.
The address and Resend API key checks come before skip checks or work; a failed send is
only a warning and never changes the run's outcome.

security-fix allows a Security run to fix one reproduced finding: the most severe first,
ties in private-record order, before auditing or after an audit reproduces a finding.
The publishing session follows the reproduction's fix size: one terse Ticket, or a Spec
with Tickets. Every new issue says what the fix changes and links the private record.
On a private repository the finding's own issue is the fix's Ticket or Spec; a single
fix needs no publishing session. thirdshift checks the issues, swaps the top issue's
needs-triage for ready-for-agent, adds security-fix to every fix issue and dispatches its
Run or Spec run. The Security run ends as that run ends, including merge when requested.
Fixing is off by default. [security] fix = true also allows it;
no-security-fix overrides the setting. Both words, with or without dashes, are accepted
on Run, Spec run, Architect run, Pickup run and Security run commands, and passed to their
Runs. Without permission, a reproduced finding waits until its record is closed or
published, or its fix Ticket is closed. A failed fix keeps its Claim even without a push,
and Security runs pause while its issue is open. Pickup never retries a Security fix,
even if its Claim could not be made. Closing the issue lets Security runs go on.
When neither the command nor the User config decided against fixing, a run that left a
reproduced finding unfixed offers security-fix and fix under [security] on stderr and in
its notification. The README's Fencing section shows the scheduled command.

The User config, ~/.thirdshift/config.toml, sets defaults for every Run on this machine;
thirdshift setup asks for your defaults and writes one listing every setting, to edit.
With merge.always set, every Run is a Merge run unless given --no-merge:

    [merge]
    always = true

With base.fix set, every Run may start a Base fix, as if given base-fix, unless given
--no-base-fix:

    [base]
    fix = true

With launch.pull set, every Run first fast-forwards the checked-out Base branch to origin:

    [launch]
    pull = true

logs.dir sets the root of the logs instead of ~/.thirdshift/logs: an absolute path, or one under ~/.
Each repository's logs go in <owner>/<repo>/ under it, named for the GitHub repository, in
folders thirdshift creates as it needs them. Session logs go in sessions/, as
<n>-<stamp>-<kind>.jsonl for a Run and architect-<stamp>-<kind>.jsonl for an Architect run.
Command logs, everything a Run, a Spec run or a Pass printed, go in
commands/issue/<n>-<stamp>.log, commands/pickup/<n>-<stamp>.log, named for the issue taken,
and commands/architect/<stamp>.log. A Pass skipped before any work
keeps no Command log, nor does a Run that fails before any work, as on the Origin match.

    [logs]
    dir = \"~/elsewhere/logs\"

Each repository's Activity log, activity.log, is a short record of what the factory did there:
a line when a Run, a Spec run or a Pass starts work, naming its Command
log, and one when it ends, with its outcome, each starting with the local date and time. A
skipped Pass writes a line only when its reason differs from the last
line of its own kind, so a repository that sits idle shows one line, not one per pass.

With activity.quiet_skips set, a skipped Pass prints nothing on stdout
or stderr, its starting line included, leaving only its Activity log line; a pass that does
work prints as ever. A crontab line can then send its output to one file, which catches only
what failed:

    [activity]
    quiet_skips = true

--email and email-test send to their <address>, else to email.to, from email.from, else
from onboarding@resend.dev, which only delivers to your own Resend account's address.
With email.always set, every Run sends a Run notification unless given --no-email:

    [email]
    always = true
    to = \"you@example.com\"
    from = \"thirdshift@your-verified-domain.com\"

The Resend API key comes from the RESEND_API_KEY environment variable, else from the
Credentials, ~/.thirdshift/credentials.toml (mode 600), never from the User config.
With Run notifications on, thirdshift setup asks for it, hidden, and saves it there:

    [resend]
    key = \"re_...\"
";

fn main() -> ExitCode {
    child_run::name_this_process();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let RunArgs {
        issue,
        flags,
        given,
    } = match args::parse(&args) {
        Ok(Command::Help) => {
            print!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Ok(Command::Version) => {
            println!("thirdshift {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Ok(Command::Update) => return outcome(update::update()),
        Ok(Command::Setup) => return outcome(setup::setup()),
        Ok(Command::EmailTest(to)) => {
            return outcome(
                UserConfig::load().and_then(|config| email::send_test(to, &config.email)),
            );
        }
        Ok(Command::Architect(architect_args)) => return architect(architect_args),
        Ok(Command::Pickup(pickup_args)) => return pickup(pickup_args),
        Ok(Command::Secure(args)) => return secure(args),
        Ok(Command::Run(run_args)) => run_args,
        Err(error) => return argument_error(format_args!("{error:#}")),
    };
    let begin = match &given {
        Some(given) => Begin::ChildRun(&given.stamp),
        None => Begin::Run(&issue),
    };
    let asks_of = |config: &UserConfig| match &given {
        Some(given) => Asks::of_child_run(given, config),
        None => Asks::of_run(&issue, &flags, config),
    };
    let ask = |config: &UserConfig| asks_of(config).notification;
    let (config, mut started) = match command::start(begin, ask, About::Issue(&issue)) {
        Ok(started) => started,
        Err(failure) => return failure,
    };
    let mut asks = asks_of(&config);
    let started_by = given
        .as_ref()
        .map_or(StartedBy::Command, |given| StartedBy::Child(&given.kind));
    let ended = run::run_to_end(&issue, &mut asks, started_by);
    started.built_with(&asks.harness);
    started.finish(Ending::Run(ended))
}

/// An Architect run: the Architecture review and its plan marked ready, then,
/// unless the command asked to stop at the plan, the plan dispatched as
/// `thirdshift <plan URL>` with the same flags would be, but on the Architect
/// run's Base branch, whatever the Launch directory has checked out. The
/// dispatched run's ending is the Architect run's, with the Base fix it took,
/// if any. One that stops at the plan, or whose review found no Strong
/// candidate and so published no plan to dispatch, puts the URL of the issue
/// it ended on on stdout: the plan, the idea issue the review filed, or the
/// issue that already covers its top recommendation. One whose review or
/// plan fails puts the cause and the session log on stderr. One that is
/// skipped says why on stderr, and is no failure: as another Pass on its
/// repository is still running, it puts nothing on stdout,
/// and as Architect plans are still open there, or Architect ideas wait for
/// triage there, the URL of each, or as it has a Ready issue, that issue's
/// URL. If asked, by the command or the User config, it sends one Run
/// notification, however it ended, short of being skipped; the run it
/// dispatched sends none of its own. The notification's checks are made
/// before any other work all the same, so a run that would be skipped fails
/// on them too. Its Command log, kept once it is past its skip checks, covers
/// that run too. The repository's Activity log records the skip, if it
/// differs from the last, or the start and end of its work. With
/// `activity.quiet_skips` set, a skipped run prints nothing at all.
fn architect(args: ArchitectArgs) -> ExitCode {
    let ask = |config: &UserConfig| args.flags.notification(config);
    let (config, mut started) = match command::start(Begin::ArchitectRun, ask, About::ArchitectRun)
    {
        Ok(started) => started,
        Err(failure) => return failure,
    };
    let mut harness = args.flags.harness(&config);
    let outcome = architect::run(
        args.focus.as_deref(),
        args.base.as_deref(),
        args.plan_only,
        &args.flags,
        &config,
        &mut harness,
    );
    started.built_with(&harness);
    started.finish(match outcome {
        Outcome::Skipped(skipped) => Ending::Skipped(skipped.into()),
        Outcome::Ran { review, dispatched } => Ending::Architect { review, dispatched },
    })
}

/// A Pickup run: the search for the lowest-numbered Ready issue in the Launch
/// directory's repository, then that issue dispatched as `thirdshift <Issue
/// URL>` with the same flags would be, but on the Pickup run's Base branch,
/// whatever the Launch directory has checked out. The dispatched run's ending
/// is the Pickup run's. One that is skipped, as when the repository is at the
/// User config's Claim limit, says why on stderr, puts nothing on stdout, and
/// is no failure. If asked, by the command or the User config, one that took
/// an issue sends one Run notification, the one the dispatched run would send
/// started by hand, and that run sends none of its own; one that is skipped
/// sends none. The notification's checks are made before any other work all
/// the same, so a pass that would be skipped fails on them too. Its Command
/// log is kept once it has taken an issue, and covers the dispatched run.
/// The repository's Activity log records the skip, if it differs from the
/// last, or the start and end of its work. With `activity.quiet_skips` set, a
/// skipped pass prints nothing at all.
fn pickup(args: PassArgs) -> ExitCode {
    let ask = |config: &UserConfig| args.flags.notification(config);
    let (config, mut started) = match command::start(Begin::PickupRun, ask, About::PickupRun) {
        Ok(started) => started,
        Err(failure) => return failure,
    };
    let mut harness = args.flags.harness(&config);
    let took = pickup::run(args.base.as_deref(), &args.flags, &config, &mut harness);
    started.built_with(&harness);
    match took {
        Ok(pickup::Outcome::Took(Took {
            issue,
            title,
            ended,
        })) => {
            started.took(&issue, title);
            started.finish(Ending::Run(ended))
        }
        Ok(pickup::Outcome::Skipped(skipped)) => started.finish(Ending::Skipped(skipped.into())),
        Err(error) => failure(&error),
    }
}

/// A Security run, with one notification when asked, unless skipped.
fn secure(args: PassArgs) -> ExitCode {
    let (config, mut started) = match command::start(
        Begin::SecurityRun,
        |config| args.flags.notification(config),
        About::SecurityRun,
    ) {
        Ok(started) => started,
        Err(failure) => return failure,
    };
    let mut harness = args.flags.security_harness(&config);
    let outcome = security::run(args.base.as_deref(), &args.flags, &config, &mut harness);
    started.built_with(&harness);
    started.finish(match outcome {
        security::Outcome::Skipped(skipped) => Ending::Skipped(command::Skip {
            reason: skipped.to_string(),
            urls: Vec::new(),
        }),
        security::Outcome::Audited(audited) => Ending::Security(audited),
        security::Outcome::Fixed { ended, findings } => Ending::SecurityFix { ended, findings },
    })
}

/// The end of a command other than a Run: the line that says how it went,
/// or its error, on stderr.
fn outcome(result: anyhow::Result<impl std::fmt::Display>) -> ExitCode {
    match result {
        Ok(outcome) => {
            progress::step(outcome);
            ExitCode::SUCCESS
        }
        Err(error) => failure(&error),
    }
}

/// An argument thirdshift can't use: the error, then the help, on stderr.
fn argument_error(error: std::fmt::Arguments) -> ExitCode {
    progress::step(error);
    eprint!("\n{HELP}");
    ExitCode::from(2)
}
