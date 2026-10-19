# Observing Commands

## Machine status

`thirdshift status` names this host and lists live Commands across every repository in `logs.dir`, with their issue and title when present, Harness, Model, Effort, current stage and its age. Architect runs, Pickup runs and Security runs nest the Runs or Spec runs they dispatch beneath themselves, each Spec's Tickets and their Runs, and each Run's Base fix. Every Ticket is listed: running, merged into the Spec branch, failed, waiting on named blockers, or not started. A Pass with no issue is named by its kind and Command log stamp, such as `architect-20261018T120000-0400` or `secure-20261018T120000-0400`. After the live Commands it lists those that ended in the last 24 hours, with their outcomes. `thirdshift status --all` lists every Command this machine has logs for, newest start first. A Command or child Run with no outcome whose process has gone shows as `died at <stage>` in either view. Process start identifiers distinguish a reused process id on Linux and macOS. Records remain beside their Command logs, as `*.command.json`, with child records named for their issue as `*.issue-<n>.child.json`. Each process writes only its own files, replacing them atomically. A record write failure warns once per recording process and does not change its outcome.

History uses each source only for Commands missing from the preceding sources: Command records, then Activity log start/end lines with their Command logs, then Session logs in the current `<logs.dir>/<owner>/<repo>/commands/<kind>/` layout. Activity entries have start and end times and outcomes, with no stages. Session entries group logs by issue and Command stamp; they have no known end or outcome and appear only with `--all`. Sessions covered by a Spec run or Pass's stamp belong to that Command. The flat layout from before [ADR 0011](../../docs/adr/0011-thirdshift-keeps-a-per-repository-activity-log.md) is ignored.

For each repository and kind of Pass whose latest Activity log line is a skip, status also shows that Pass as idle with its reason, even when the skip is older than another kind's activity. A later start or end for that kind clears its idle entry. For example:

```text
Architect run idle on acme/widgets: Ready issue #7 "Make widgets" goes first: https://github.com/acme/widgets/issues/7
```

`thirdshift status --json` is the versioned API for agents and the coming [Console](../../docs/adr/0016-the-console-is-a-long-running-reader.md). It runs outside a repository too. Version 1 has this shape:

```json
{
  "version": 1,
  "hostname": "factory",
  "commands": [{
    "version": 1,
    "kind": "Run",
    "repository": "acme/widgets",
    "name": "acme/widgets#7",
    "issue": {"number": 7, "url": "https://github.com/acme/widgets/issues/7", "title": "Make widgets"},
    "hostname": "factory",
    "process_id": 1234,
    "process_start": "<OS process start identifier>",
    "harness": "claude",
    "model": "opus",
    "effort": "high",
    "started_at": "2026-10-18T12:00:00-04:00",
    "ended_at": null,
    "stages": [{"name": "implement", "began_at": "2026-10-18T12:00:01-04:00", "session_log": "/logs/acme/widgets/commands/issue/7-stamp-implement.jsonl"}],
    "session_log": "/logs/acme/widgets/commands/issue/7-stamp-implement.jsonl",
    "issue_branch": "issue-7",
    "pull_request": null,
    "outcome": null,
    "cause": null,
    "source": "command_record",
    "command_log": "/logs/acme/widgets/commands/issue/7-stamp.log",
    "dispatched": [],
    "state": "live",
    "stage": {"name": "implement", "began_at": "2026-10-18T12:00:01-04:00", "age_seconds": 52},
    "parent_record": null,
    "parent_dispatch": null,
    "tickets": [],
    "children": []
  }],
  "idle_passes": [{
    "repository": "acme/widgets",
    "kind": "Pickup run",
    "reason": "no Ready issue on acme/widgets"
  }]
}
```

Times are RFC 3339; ages are nonnegative seconds. `model` and `effort` are null when left to the Harness. `issue` is null for a Pass with no issue; `session_log`, `issue_branch`, `pull_request`, `outcome`, `cause`, `ended_at`, `parent_record` and `parent_dispatch` are nullable. Command kinds are `Run`, `Spec run`, `Architect run`, `Pickup run`, `Security run` and `Base fix`. `name` identifies an issue or a Pass's kind and stamp. `dispatched` is an array of the Runs or Spec runs a Pass dispatched, with the same fields as its parent (and an empty `dispatched` array); it shares the Pass's process identity and Command log. It retains its own outcome, end time, cause, Issue branch and pull request. The retained Command record has the same fields, including dispatched work, without the computed `state`, `stage`, `source`, `command_log` and `children` or empty leaf `dispatched` arrays. Older records without an end time use their last modification time. `idle_passes` contains one entry per repository and kind whose latest Activity log line is a skip; it is independent of `commands`, and never creates a Command record or log for a skipped Pass.

`state` is `live`, `died`, `ended`, or `unknown` when historical evidence has no ending. `stage` is null before the first known stage. Readers skip unknown stage names. Known stages are `implement`, `Resume`, `Security review`, `pull request`, `CI watch`, `Repair`, `Self-merge`, `cleanup`, `Tickets`, `Spec review`, `Architecture review`, `Security audit`, `Security reproduction` and `Security fix publishing`. A Pass retains its own stages while its dispatched work records its stages separately.

`children` recursively contains the immediate child Runs in separate processes, with the same fields as their parent. A Pickup's `dispatched` contains its Run or Spec run. A Spec's `tickets` is its complete roster in issue-number order; each entry has `issue` (`number`, `url`, nullable `title`), `state` (`running`, `merged`, `failed`, `waiting`, or `not started`) and `blockers` (the named open blocking issues, with the same issue fields). Match a Ticket's `issue.url` to the Run in that Spec's `children` for its stage, age, liveness and Base fix descendants. Tickets without a Run remain in the roster. Completed child Runs stay in the tree while its Command is listed. A killed child's `state: "died"` remains independent of the Spec's Ticket accounting. `parent_record` links each child to its parent's exact file; `parent_dispatch`, when present, names the dispatched issue's URL within that file. A top-level Command has null for both. Records from before these fields existed read as having no children or Tickets.

For example, this tree excerpt omits the common Run fields shown above:

```json
{
  "kind": "Pickup run",
  "dispatched": [{
    "kind": "Spec run",
    "stage": {"name": "Tickets", "began_at": "2026-10-18T12:00:01-04:00", "age_seconds": 52},
    "tickets": [
      {"issue": {"number": 21, "url": "https://github.com/acme/widgets/issues/21", "title": null}, "state": "running", "blockers": []},
      {"issue": {"number": 22, "url": "https://github.com/acme/widgets/issues/22", "title": null}, "state": "waiting", "blockers": [{"number": 21, "url": "https://github.com/acme/widgets/issues/21", "title": null}]},
      {"issue": {"number": 23, "url": "https://github.com/acme/widgets/issues/23", "title": null}, "state": "merged", "blockers": []},
      {"issue": {"number": 24, "url": "https://github.com/acme/widgets/issues/24", "title": null}, "state": "not started", "blockers": []}
    ],
    "children": [{
      "kind": "Run",
      "issue": {"number": 21, "url": "https://github.com/acme/widgets/issues/21", "title": "Make widgets"},
      "state": "live",
      "stage": {"name": "implement", "began_at": "2026-10-18T12:00:01-04:00", "age_seconds": 52},
      "children": []
    }]
  }]
}
```
Every entry has `source`: `command_record`, `activity_log` or `session_log`. The latter two carry `repository`, `kind`, nullable `issue` (number, URL, nullable title), `harness`, `model`, `effort`, `started_at`, nullable `ended_at` and `outcome`, nullable `command_log`, `session_logs` (paths), `state`, `stages: []` and `stage: null`. They omit record-only fields such as process identity and branch information. Activity times use the UTC offset in the Command log's stamp; their outcome is the Activity log's full ending text. A Session entry has null Harness, Model, Effort, end time and outcome, and lists its grouped Session logs. Both flags can be combined as `status --all --json`.
## Show and read one Command

Name a Command by `widgets#7`, `acme/widgets#7`, or its full Issue URL. The owner is needed only when repositories on this machine share the name; an ambiguous short name is refused with the matching repositories. When several Commands worked on the issue, the live one wins, otherwise the latest start wins. A Ticket's issue names its Run within the Spec run. A Pass with no issue is named by kind and stamp, such as `architect-20261018T120000-0400`, as listed by `status`.

```sh
thirdshift show 'widgets#7'
thirdshift show https://github.com/acme/widgets/issues/7 --json
thirdshift logs 'widgets#7'
thirdshift logs 'widgets#7' --session implement
thirdshift logs 'widgets#7' --session repair --follow
```

`show` includes the Runs, stage history with start times, outcomes, and Command and Session log paths. Each live Run also gets its latest event, such as `Bash: cargo test`, from the last 64 KiB of its current Session log. It uses the Harness's stream interpretation, including Claude and Codex, and ignores malformed or unknown lines. Missing or unreadable Session logs leave the event unavailable.

`show --json` is a versioned API, usable outside a repository. Version 1 wraps the selected Command in this shape:

```json
{
  "version": 1,
  "hostname": "factory",
  "command": {
    "version": 1,
    "kind": "Run",
    "repository": "acme/widgets",
    "name": "acme/widgets#7",
    "state": "live",
    "latest_event": "Bash: cargo test",
    "stages": [{"name": "implement", "began_at": "2026-10-18T12:00:01-04:00", "session_log": "/logs/acme/widgets/commands/issue/7-stamp-implement.jsonl"}],
    "command_log": "/logs/acme/widgets/commands/issue/7-stamp.log",
    "session_log": "/logs/acme/widgets/commands/issue/7-stamp-implement.jsonl",
    "session_logs": ["/logs/acme/widgets/commands/issue/7-stamp-implement.jsonl"],
    "children": [],
    "dispatched": []
  }
}
```

The Command excerpt omits the common fields documented for `status --json`; all those fields are retained, including process identity, issue, Harness, Model, Effort and nullable outcome. `children` and `dispatched` retain the same recursive shape. `session_logs` lists the selected Run's Session log paths, including original sessions and Resumes that share a stage. `latest_event` is optional, present only when a live Run has a readable event, including in child Runs. History without a Command record uses the same Activity-log or Session-log shape as `status --all --json`, with Session logs discovered by issue/kind and Command stamp, without invented stages or liveness.

`logs` writes the Command log verbatim. `--session <kind>` writes that kind's latest Session log as raw JSON lines; `implement` and `spec-review` include a later Resume, and `repair` selects the latest numbered Repair. A missing log or session kind gives an error. `--follow` first prints the selected log, then flushes appended bytes until the owning Command ends or dies, including its final output. A Ticket shares its Command's Command log but selects its own Session logs. The selected Command and Session log stay fixed while following.
## Stopping a Command

`thirdshift stop widgets#7` stops a live Run and waits for its cleanup. It accepts
the same Command names as `show`: `owner/repo#number`, an Issue URL, or a Pass's
`kind-stamp` name. It works from any directory and never prompts. Ended and died
Commands are refused; Stop sends SIGTERM only to the recorded process, with no
forced kill.

Stopping a Spec run ends its Tickets' Runs too, saving their unfinished work.
Only the Spec's issue receives Stop labels; its Tickets' labels stay as they
were. A Stop on a Pickup run applies to the issue it took, whether a Run or a
Spec run. A Pass still without an issue, such as an Architect run in its
Architecture review or a Security run in its audit, ends without changing
labels. Stop refuses a Ticket's issue and names the Spec run to stop instead.

By default, Stop leaves the issue for the Day shift: it adds `needs-triage` and
does not put `ready-for-agent` back. It removes `in-progress` when nothing reached
`origin`, and keeps it when an Issue branch or pull request remains. Add
`--requeue` to release the Claim and put `ready-for-agent` back even after a push;
the next Run continues that Issue branch. Ordinary interruption without a Stop
keeps its usual Claim rules.

The Stop request is kept beside the Command record with the Unix user who asked
and whether to requeue. The Command writes `<subject> stopped from the CLI by
<who>` to its Activity log before its `ended:` line. Stop prints `still ending`
each minute while cleanup runs, then the outcome and any Issue branch or pull
request left behind.

## Logs

Everything thirdshift logs goes under `~/.thirdshift/logs/`, or the `logs.dir` set in the [User config](configuration.md#user-config), in a folder per repository, `<owner>/<repo>/`, named for the GitHub repository rather than your checkout, so a fork and its upstream never share one and every checkout or worktree of a repository logs to the same place. thirdshift creates each folder as it needs it, so a repository's first pass needs nothing made by hand:

```
~/.thirdshift/logs/<owner>/<repo>/
├── activity.log              the Activity log
├── audits/                   each Security audit's report, with its evidence
├── security-reviews/         each Security review's private report of old findings
└── commands/
    ├── architect/            <stamp>.log and Session logs
    ├── pickup/               <n>-<stamp>.log and Session logs
    ├── secure/               <stamp>.log and Session logs
    └── issue/                <n>-<stamp>.log and Session logs
```

The folder already names the repository, so no file name repeats it. `audits/` and `security-reviews/` hold vulnerability evidence, so keep the logs private (see [Security runs](passes.md#security-runs)). A **Session log** is one session's full transcript, as the Harness's JSON output, such as Claude Code's `stream-json`:

```
~/.thirdshift/logs/<owner>/<repo>/commands/issue/<n>-<stamp>-implement.jsonl
~/.thirdshift/logs/<owner>/<repo>/commands/issue/<n>-<stamp>-repair-<i>.jsonl
~/.thirdshift/logs/<owner>/<repo>/commands/architect/architect-<stamp>-architecture-review.jsonl
```

Session logs share the owning command's folder: a Pickup run's dispatched Run logs in `commands/pickup/`, an Architect run's in `commands/architect/`, and a Security run's in `commands/secure/`. Tickets and Base fixes inherit that folder too.

A Resume is logged as its session's kind plus `-resume`, e.g. `implement-resume.jsonl`.

A **Command log** is everything one command printed, stderr and stdout in the order printed, while the terminal still shows all of it. Its folder is the command typed: `thirdshift <Issue URL>`, a Run or a Spec run, in `issue/`, `thirdshift pickup` in `pickup/`, named for the issue it took, and `thirdshift architect` in `architect/`. A Spec run's covers its Tickets' Runs, a Run's covers its [Base fix](runs.md#base-fix), and a Pass's covers the Spec run or Run it dispatched: none of those keeps one of its own. So `ls -t ~/.thirdshift/logs/acme/widgets/commands/pickup/*.log | head` lists the recent passes on acme/widgets that took an issue. A command keeps its Command log once it starts work: a Pass skipped before doing any work keeps none, nor does a Run that fails before it starts work, as on the [Origin match](runs.md#what-a-run-does), and `setup`, `email-test`, `update`, `version` and `help` never keep one. Lines printed before the Command log's name is known, such as a Pickup run's lines on the issues it [passed over](passes.md#why-an-issue-was-passed-over) before it took one, are written first, and once it is created a progress line says `logging this command to <path>`. A Command log that can't be written produces one `warning:` line on stderr. A failure confined to the Command log leaves the outcome, stdout and exit code unchanged; if the shared folder also prevents writing a Session log, the Run fails before starting that session.

The stamp is the local time the command started, with its UTC offset, as in `20261003T120000-0400`, in the machine's time zone, or `TZ`'s if set, so it agrees with `date` and `ls -l`. A command's Command log and all of its Session logs, its Tickets' Runs' and its Base fix's included, share that one stamp, so they sort together and each can be found from the other. When a Run fails, stderr ends with the path of its most recent Session log, the place to start looking, then that of its Command log.

The **Activity log**, `activity.log`, is a short running record of what the factory did on the repository ([ADR-0011](../../docs/adr/0011-thirdshift-keeps-a-per-repository-activity-log.md)). A Run, a Spec run or a Pass writes a line when it starts work, naming its Command log, and one when it ends, with its outcome; a Run you type by hand writes them too, and the run a Pass dispatched, a Spec run's Tickets and a Base fix write none of their own. A skipped Pass writes a line only when its reason differs from the last line of its own kind, so a repository that sits idle shows one line, not one per pass, and the first skip after a pass that did work always shows: that is when the repository went idle. Every line starts with the local date and time:

```
2026-10-03 02:00:01 Pickup run skipped: no Ready issue on acme/widgets
2026-10-03 09:30:01 Pickup run #41 started: commands/pickup/41-20261003T093001-0400.log
2026-10-03 09:52:17 Pickup run #41 ended: PR https://github.com/acme/widgets/pull/42 is merged
2026-10-03 10:00:01 Pickup run skipped: no Ready issue on acme/widgets
```

Each line is appended whole, so passes that run at once on one repository never garble it. One that can't be written is one `warning:` line on stderr, and the command carries on as for a Command log. thirdshift never rotates it: collapsed skips keep it small. Existing logs, including Session logs in `<owner>/<repo>/sessions/` and logs from before the per-repository layout, are not moved.

With `quiet_skips = true` in the `[activity]` section of the [User config](configuration.md#user-config), a skipped Pass prints nothing on stdout or stderr, its dated first line included, and leaves only its Activity log line. A pass that does work, or fails, prints as ever, so a scheduler's log file catches only what went wrong. Without it, a skipped pass prints as it always has, for a pass you type by hand.
