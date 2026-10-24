//! Fake `gh` for the thirdshift test harness.
//!
//! State lives in the JSON file named by $FAKE_GH_STATE:
//!
//! ```text
//! {"repo": "owner/repo",
//!  "private"?: true,
//!  "advisory_error"?: "<stderr>",
//!  "issues": {"<number>": "OPEN" | "CLOSED"},
//!  "issue_closed_at"?: {"<number>": "<ISO 8601 time>"},
//!  "titles"?: {"<number>": "<title>"},
//!  "created"?: {"<number>": "<ISO 8601 time>"},
//!  "bodies"?: {"<number>": "<body>"},
//!  "comments"?: {"<number>": ["<body>" | {"body", "by", "association"}, ...]},
//!  "author_associations"?: {"<number>": "OWNER" | "MEMBER" | "COLLABORATOR" | "NONE"},
//!  "trust_events"?: {"<number>": [<REST issue event with created_at>, ...]},
//!  "edits"?: {"<number>": [{"kind": "body" | "title", "by": "<login>" | null, "at"}, ...]},
//!  "collaborators"?: ["<login>", ...],
//!  "org_members"?: ["<login>", ...],
//!  "owner_type"?: "User" | "Organization",
//!  "prs": [{"number", "url", "head", "base", "state", "isDraft", "title", "body",
//!           "mergeable"?, "unknown_polls"?, "reviewDecision"?}],
//!  "checks": {"<sha>": [{"id", "name", "conclusion", "url", "pending_polls"?,
//!                        "steps"?: [{"status", "conclusion"}],
//!                        "rerun"?: {"conclusion", "stale_polls"?, "pending_polls"?,
//!                                   "steps"?},
//!                        "stale"?: {"id", "conclusion", "polls"}}]},
//!  "next_check_id"?: <number>,
//!  "statuses": {"<sha>": [{"context", "state", "url"}]},
//!  "runs"?: [{"databaseId", "url", "branch", "workflow", "event", "headSha", "status",
//!             "conclusion", "jobs"?, "hidden_polls"?, "pending_polls"?}],
//!  "on_ci_read"?: {"times", "script", "seen"},
//!  "on_issue_view"?: {"<number>": "<script>" | {"fields": "<--json fields>", "script": "<script>"}},
//!  "on_merge"?: {"times", "script"},
//!  "refuse_merges"?: {"times", "error"},
//!  "after_merge"?: "<script>",
//!  "failing"?: ["<command> <subcommand>", ...],
//!  "failing_stderr"?: "<stderr for failing commands>",
//!  "rate_limits"?: {"core": {"remaining", "reset"}, "graphql": {"remaining", "reset"}},
//!  "inbox_rate_limit"?: {"remaining", "resetAt"},
//!  "fail_next"?: {"call": "<command prefix>", "times": <count>, "error": "<stderr>"},
//!  "advisory_create_fails_after"?: <number of records>,
//!  "user_email"?: "<address>" | null,
//!  "sub_issues"?: {"<number>": [<number>, ...]},
//!  "labels"?: {"<number>": ["<label>", ...]},
//!  "repo_labels"?: ["<label>", ...],
//!  "issue_urls"?: {"<number>": "<full Issue URL>"},
//!  "blocked_by"?: {"<number>": [<number>, ...]},
//!  "timeline"?: {"<number>": [{"event", "label"?, "at"}, ...]}}
//! ```
//!
//! `gh pr merge` makes a real merge commit of the PR's head into its base in
//! the bare origin repo beside $FAKE_GH_STATE (`origin.git`). It closes no
//! issues, whatever the PR body says: GitHub may close them a moment later, or
//! may not.
//!
//! A PR's `headRefOid` is its head branch's tip on origin, frozen when it is
//! merged, and a merged PR records its merge commit as `mergeCommit`.
//! `--json` rejects a field the fake does not have, such as a PR's
//! `closingIssuesReferences`, with gh's `Unknown JSON field`, as gh 2.45 does.
//!
//! An issue's title is its entry in `titles`, else `Issue <number>`, and its
//! `createdAt` its entry in `created`, else the first second of 2020.
//! Trust defaults to an OWNER author and a ready-for-agent event by the
//! repository owner at the first second of 2020. `trust_events` overrides
//! that history, including an empty array for an issue never labelled
//! ready-for-agent. The REST events endpoint streams every supplied event
//! with --paginate --jq '.[]'.
//!
//! `gh api --method PUT repos/<repo>/issues/<number>/labels -f
//! labels[]=<label> ...` sets an issue's labels to exactly those given.
//! `gh api --method DELETE repos/<repo>/issues/<number>/labels/<label>` takes
//! that label, whatever its case, off the issue, failing if the issue does
//! not have it, as GitHub does.
//! `gh api --method POST repos/<repo>/issues/<parent>/sub_issues -F
//! sub_issue_id=<id>` links the issue as a native sub-issue of the parent.
//! `gh api --method POST repos/<repo>/issues/<ticket>/dependencies/blocked_by
//! -F issue_id=<id>` links the ticket as blocked by the issue. Database ids
//! are the issue numbers, and `gh api repos/<repo>/issues/<number> --jq .id`
//! answers with one.
//!
//! `gh api user` answers with the signed-in user's profile, whose public
//! `email` is `user_email`, else null. `gh fake fails 'api user'` makes it
//! fail.
//!
//! `gh api repos/<repo>/releases/generate-notes` answers with GitHub's
//! generated notes: a line for each merged PR whose merge commit is reachable
//! from `target_commitish` and not from `previous_tag_name`. `<repo>` may be
//! gh's `{owner}/{repo}` placeholder, here and in the commit endpoints.
//! `gh api repos/<repo>/actions/runs?head_sha=<sha>` answers with the workflow
//! runs on <sha>: `.workflow_runs[]` reads REST-shaped metadata from `runs`
//! (with `headSha` and `databaseId` as for `gh fake run`); the release script's
//! line query reads its check runs as the check-runs endpoint reads them.
//! `gh api repos/<repo>/commits/<sha>/pulls` answers with the PRs <sha>
//! belongs to, as GitHub does: those merged as <sha> and those whose head is
//! <sha>.
//!
//! `gh run list` answers with the workflow runs in `runs`, newest first, that
//! match its `--branch`, `--workflow`, `--event` and `--commit` (their
//! `headSha`). Runs there are only ones a test records with `gh fake run`:
//! they are separate from the check runs. A run is not listed for its first
//! `hidden_polls` reads that match it, as GitHub lists a run a moment after
//! the push that starts it, then reports as in progress for its first
//! `pending_polls` reads, then as `status` with `conclusion`. `gh run view
//! <id> --json jobs` answers with the run's `jobs`, each `{"name",
//! "conclusion"}`, for its current attempt.
//!
//! `gh release view <tag> --json url` answers with the GitHub Release's URL
//! for a tag on origin.
//!
//! `gh api graphql` answers the one query thirdshift reads a Spec's Tickets
//! with: the sub-issues of issue `$number` (its `sub_issues`, in order), each
//! with its state, its `labels`, how many sub-issues it has of its own, and
//! the issues it is `blocked_by` with their states. It answers a query that
//! asks for `timelineItems` as the one thirdshift reads a Pickup run's
//! candidate with: issue `$number`'s parent, the issue whose `sub_issues` it is
//! one of, the state of each of its sub-issues, the issues it is `blocked_by`
//! with their states, and from its `timeline`, oldest first, every `labeled`
//! or `unlabeled` event as `labelled` in pages of 100, using the `$after`
//! cursor and returning `pageInfo`, and the newest of its `sub_issue_added`,
//! `sub_issue_removed`, `blocked_by_added` and `blocked_by_removed` events as
//! `linked`. A query that asks for `userContentEdits`, or for
//! `RENAMED_TITLE_EVENT`s, answers with issue `$number`'s `body` or `title`
//! `edits`, oldest first, as `edits` in pages of 100 using the `$after`
//! cursor, each with its `at` time and its `by` account as `actor`. A query
//! that asks for `comments` with their `authorAssociation` answers with issue
//! `$number`'s title, body and `comments`, oldest first, in pages of 100 using
//! the `$after` cursor: a plain body is by the repository owner, as `OWNER`,
//! and an object names its `by` account, null for a deleted one, and its
//! `association`. Any other query exits 2.
//!
//! `gh api graphql` also answers the Inbox's batched issue and pull request query: one
//! `alias: repository(owner, name)` entry per subject, each with that
//! PR's `url`, `state` (`OPEN`, `CLOSED` or `MERGED`) and its `mergedAt` and
//! `closedAt` times when the state names them, else null, plus the query's
//! own `rateLimit`. An alias for a pull request the state does not know, or
//! for another repository, answers a null `pullRequest` and a scoped GraphQL
//! error. Like `gh`, the fake prints the partial data and exits 1.
//! `rate_limits` supplies the resources from `gh api rate_limit` (otherwise
//! 4999 remaining, resetting in an hour). `inbox_rate_limit` overrides the
//! batched GraphQL response's quota, e.g. when that query spends the reserve.
//! `next_rate_limits` replaces `rate_limits` after each quota answer, modelling
//! requests spending the REST reserve during private-storage discovery.
//! `failing` with `failing_stderr` models unreachable, signed-out and
//! rate-limited reads with gh's actual diagnostics and a nonzero exit.
//! Issues answer `state`, `closedAt` from `issue_closed_at` and label nodes
//! from `labels`, with the same null/scoped error for an unknown issue.
//!
//! `gh api --method PATCH repos/<repo>/pulls/<number> -f body=<body>` sets the
//! PR's body. `gh pr view`, `gh pr ready` and `gh pr merge` name the PR by
//! its head branch, number or URL; a number or URL selects that PR's actual head.
//!
//! `gh issue close` closes the issue, recording its `--comment`; on an issue
//! already closed it only warns, as gh does.
//!
//! `gh issue list --label <label> --json <fields>` lists the open issues with
//! that label, whatever its case, newest first, or with `--state closed` the
//! closed ones.
//! `gh api --paginate --jq '.[]' repos/<repo>/issues?state=<state>&labels=<label>&...`
//! streams REST issues with `html_url`, filtered by that state and label,
//! newest first. State may also be `all` for private Security findings.
//!
//! `gh issue create --title <title> --body <body> --label <label>,<label>`
//! opens an issue numbered one past the highest issue, with that title, body
//! (in `bodies`) and labels, and the time as its `createdAt`, and prints its
//! URL. A label the repository does not have, one not in `repo_labels`,
//! fails it, as gh does. `gh label list --json name` lists the repository's
//! labels, and `gh label create <name>` adds one, failing if the repository
//! has it already.
//!
//! A PR's mergeable state reports as UNKNOWN for its first `unknown_polls`
//! reads. A check run reports as in progress for its first `pending_polls`
//! reads, then as completed with its `conclusion`.
//!
//! A check run whose `url` is `…/actions/runs/<run>/job/<job>` belongs to the
//! GitHub Actions workflow run <run>. `gh run rerun <run> --failed` starts a
//! new attempt of each check run of <run> that failed, on any commit, and
//! fails, as gh does, if there is none. The new attempt has a new `id`, and
//! comes to what the check's `rerun` says: for its first `stale_polls` reads
//! GitHub still lists the attempt before it, failed as it was, then the new
//! one reports as in progress for its `pending_polls` reads, then as
//! completed with its `conclusion`. With no `rerun`, the new attempt fails as
//! the one before it did.
//!
//! `gh api repos/<repo>/actions/jobs/<id>` answers with the job whose check
//! run has that `id`: its status and conclusion, as the check-runs endpoint
//! last reported them, and its `steps`, else one completed step that came to
//! the check's conclusion. A new attempt takes its `rerun`'s `steps`, if any.
//!
//! Tests and fake agents change that state with `gh fake …` (not a real gh
//! command):
//!
//! ```text
//! gh fake checks <sha> '<JSON list>'      set the check runs on <sha>, each
//!                                         given the next `id` if it has none
//! gh fake statuses <sha> '<JSON list>'    set the commit statuses on <sha>
//! gh fake run '<JSON>'                    record a workflow run, newer than
//!                                         any recorded before it, numbered
//!                                         with the next `databaseId` if it
//!                                         has none; one with the `databaseId`
//!                                         of a recorded run is a new attempt
//!                                         of it, replacing the fields it has
//! gh fake pr <head> <field> '<JSON>'      set a field of <head>'s newest PR
//! gh fake issue <number> <state>          set issue <number> OPEN or CLOSED
//! gh fake on-ci-read <times> '<script>'   run <script> in bash the first time
//!                                         thirdshift reads CI on each new sha,
//!                                         for the next <times> shas, with the
//!                                         sha in $FAKE_CI_SHA
//! gh fake on-issue-view <number> '<script>'  run <script> in bash the first
//!                                         time issue <number> is viewed
//! gh fake on-merge <times> '<script>'     run <script> in bash before each of
//!                                         the next <times> `gh pr merge` calls
//! gh fake refuse-merges <times> '<error>' fail the next <times> `gh pr merge`
//!                                         calls with <error> on stderr, after
//!                                         any on-merge script
//! gh fake after-merge '<script>'          run <script> in bash after each
//!                                         successful `gh pr merge`, with the
//!                                         PR's head branch in
//!                                         $FAKE_MERGE_HEAD and the merge
//!                                         commit in $FAKE_MERGE_SHA
//! gh fake fails '<command> <subcommand>'  make every such call fail, e.g.
//!                                         'issue close'; with more words,
//!                                         every call that starts with them,
//!                                         e.g. 'api --method DELETE'
//! gh fake user-email '<JSON>'             set the public profile email, an
//!                                         address or null
//! gh fake labels <number> '<JSON list>'   set issue <number>'s labels
//! gh fake created <number> <time>         set issue <number>'s `createdAt`
//! gh fake sub-issues <number> '<JSON list>'  set issue <number>'s sub-issues,
//!                                         by number, making it a Spec
//! gh fake repo-labels '<JSON list>'       set the repository's labels
//! gh fake timeline <number> '<JSON list>' set issue <number>'s timeline
//!                                         events, oldest first
//! gh fake edits <number> '<JSON list>'    set issue <number>'s body and
//!                                         title edits, oldest first
//! ```
//!
//! Every call's argv is appended to the JSON list in $FAKE_GH_RECORD.
//!
//! Parallel child Runs call gh at once, so each call holds an exclusive lock
//! on $FAKE_GH_STATE.lock while it reads and writes the state and the record.
//! It lets go while a hook script runs, as the script may call gh itself.
//!
//! Only the subcommands thirdshift and the fake agents use are supported.
//! Anything else exits 2 so an unexpected call fails loudly.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use crate::json::{self, Array, Bool, Json, Null, number, object, string};
use crate::{die, git, python_list};

static LOCK: Mutex<Option<File>> = Mutex::new(None);

/// Wait for the state's lock and hold it until `unlock` or exit.
fn lock() {
    *LOCK.lock().unwrap() = Some(crate::lock_beside(&state_path()));
}

fn unlock() {
    *LOCK.lock().unwrap() = None;
}

fn state_path() -> PathBuf {
    crate::env_path("FAKE_GH_STATE")
}

fn load() -> Json {
    crate::read_json(&state_path())
}

fn save(state: &Json) {
    fs::write(state_path(), state.dump()).unwrap();
}

const BOOLEAN_FLAGS: [&str; 7] = [
    "draft", "undo", "paginate", "merge", "silent", "failed", "include",
];

/// `--flag` values by name; a boolean flag's is `None`.
type Flags = BTreeMap<String, Option<String>>;

/// Split args into positionals and the --flag values.
fn parse(args: &[&str]) -> (Vec<String>, Flags) {
    let (mut positional, mut flags) = (Vec::new(), Flags::new());
    let mut i = 0;
    while i < args.len() {
        let arg = args[i];
        if arg.starts_with('-') {
            let name = arg.trim_start_matches('-');
            if BOOLEAN_FLAGS.contains(&name) {
                flags.insert(name.to_owned(), None);
            } else if let Some((name, value)) = name.split_once('=') {
                flags.insert(name.to_owned(), Some(value.to_owned()));
            } else {
                i += 1;
                let value = args.get(i).unwrap_or_else(|| panic!("{arg} needs a value"));
                flags.insert(name.to_owned(), Some((*value).to_owned()));
            }
        } else {
            positional.push(arg.to_owned());
        }
        i += 1;
    }
    (positional, flags)
}

/// The value of `--name`, if it was given one.
fn flag<'a>(flags: &'a Flags, name: &str) -> Option<&'a str> {
    flags.get(name).and_then(Option::as_deref)
}

/// Fail unless `-R`/`--repo`, if given, names the fake's repo.
fn check_repo(state: &Json, flags: &Flags) {
    let repo = flag(flags, "R")
        .filter(|repo| !repo.is_empty())
        .or(flag(flags, "repo"));
    check_repo_is(state, repo);
}

/// Fail unless `repo`, if any, is the fake's. `{owner}/{repo}` is gh's
/// placeholder for the current directory's repo, which is always the fake's.
/// The local origin path stands in for that repository's remote URL.
fn check_repo_is(state: &Json, repo: Option<&str>) {
    if let Some(repo) = repo
        && repo != "{owner}/{repo}"
        && Path::new(repo) != origin_repo()
        && repo.to_lowercase() != state.at("repo").str().to_lowercase()
    {
        die(&format!("fake gh: unknown repo {repo}"), 1);
    }
}

fn prs(state: &Json) -> &[Json] {
    state.at("prs").items()
}

/// The index of the newest PR from `head`, or gh's error if there is none.
fn newest_pr_from(state: &Json, head: &str) -> usize {
    prs(state)
        .iter()
        .rposition(|pr| pr.at("head").str() == head)
        .unwrap_or_else(|| no_pr_for(head))
}

/// Resolve a number or URL, or, for fake agents, a head branch.
fn selected_pr(state: &Json, name: &str) -> usize {
    if name.starts_with("https://") {
        prs(state)
            .iter()
            .rposition(|pr| pr.at("url").str() == name)
            .unwrap_or_else(|| no_pr_for(name))
    } else if !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()) {
        prs(state)
            .iter()
            .rposition(|pr| pr.at("number").as_i64() == name.parse().ok())
            .unwrap_or_else(|| no_pr_for(name))
    } else {
        newest_pr_from(state, name)
    }
}

fn no_pr_for(name: &str) -> ! {
    die(&format!("no pull requests found for branch \"{name}\""), 1)
}

/// gh's error for acting on a PR that isn't open.
fn ensure_open(pr: &Json) {
    if pr.at("state").str() != "OPEN" {
        die(&format!("pull request #{} is not open", pr.at("number")), 1);
    }
}

fn unsupported_jq(jq: Option<&str>) -> ! {
    die(
        &format!("fake gh: unsupported --jq {}", jq.unwrap_or("None")),
        2,
    )
}

fn pr_create(state: &mut Json, flags: &Flags) {
    let n = prs(state).len() + 1;
    let url = format!("https://github.com/{}/pull/{n}", state.at("repo").str());
    state.at_mut("prs").items_mut().push(object([
        ("number", number(n)),
        ("url", string(&url)),
        ("head", string(flag(flags, "head").expect("no --head"))),
        ("base", string(flag(flags, "base").expect("no --base"))),
        ("state", string("OPEN")),
        // A PR retains its head even before the scripted session's first
        // push or after GitHub deletes its branch on merge.
        (
            "createdHead",
            string(
                String::from_utf8(
                    git(&std::env::current_dir().unwrap(), &["rev-parse", "HEAD"]).stdout,
                )
                .unwrap()
                .trim(),
            ),
        ),
        ("isDraft", Bool(flags.contains_key("draft"))),
        ("title", string(flag(flags, "title").unwrap_or(""))),
        ("body", string(flag(flags, "body").unwrap_or(""))),
    ]));
    save(state);
    println!("{url}");
}

/// By head branch, its newest PR, or by number. `--jq .<field>` prints that
/// one field raw, as gh does for a string.
fn pr_view(state: &mut Json, positional: &[String], flags: &Flags) {
    let name = &positional[0];
    let at = selected_pr(state, name);
    let pr = &prs(state)[at];
    let wanted = wanted_fields(flags);
    let mut fields = pr_fields(pr, &wanted);
    fields.set("title", pr.at("title").clone());
    let mut fields = json_fields(&fields, &wanted);
    if let Some(jq) = flag(flags, "jq") {
        match jq.strip_prefix('.') {
            Some(field) if wanted.iter().any(|w| w == field) => {
                println!("{}", fields.at(field).python());
                return;
            }
            _ => unsupported_jq(Some(jq)),
        }
    }
    let pr = &mut state.at_mut("prs").items_mut()[at];
    let unknown_polls = pr.get("unknown_polls").and_then(Json::as_i64).unwrap_or(0);
    if wanted.iter().any(|w| w == "mergeable") && unknown_polls > 0 {
        pr.set("unknown_polls", number(unknown_polls - 1));
        save(state);
        fields.set("mergeable", string("UNKNOWN"));
    }
    println!("{fields}");
}

fn wanted_fields(flags: &Flags) -> Vec<String> {
    flag(flags, "json")
        .expect("no --json")
        .split(',')
        .map(str::to_owned)
        .collect()
}

/// The `wanted` ones of `fields`, as `--json` selects them. A field the fake
/// does not have fails as gh 2.45 fails one it does not know.
fn json_fields(fields: &Json, wanted: &[String]) -> Json {
    if let Some(unknown) = wanted.iter().find(|key| !fields.has(key)) {
        let Json::Object(all) = fields else {
            unreachable!()
        };
        let available: BTreeSet<&str> = all.iter().map(|(key, _)| key.as_str()).collect();
        let available: Vec<&str> = available.into_iter().collect();
        die(
            &format!(
                "Unknown JSON field: \"{unknown}\"\nAvailable fields:\n  {}",
                available.join("\n  ")
            ),
            1,
        );
    }
    let mut selected = object([]);
    for key in wanted {
        if !selected.has(key) {
            selected.set(key, fields.at(key).clone());
        }
    }
    selected
}

fn record(args: &[String]) {
    crate::append_record(
        &crate::env_path("FAKE_GH_RECORD"),
        Array(args.iter().map(string).collect()),
    );
}

/// The PR's JSON fields. `headRefOid`, its head as merged or else its head
/// branch's tip on origin, is read only when `wanted`.
fn pr_fields(pr: &Json, wanted: &[String]) -> Json {
    let mut fields = object([
        ("url", pr.at("url").clone()),
        ("state", pr.at("state").clone()),
        ("headRefName", pr.at("head").clone()),
        ("baseRefName", pr.at("base").clone()),
        ("isDraft", pr.at("isDraft").clone()),
        ("number", pr.at("number").clone()),
        (
            "isCrossRepository",
            pr.get("isCrossRepository").cloned().unwrap_or(Bool(false)),
        ),
        (
            "mergeable",
            pr.get("mergeable").cloned().unwrap_or(string("MERGEABLE")),
        ),
        (
            "reviewDecision",
            pr.get("reviewDecision").cloned().unwrap_or(string("")),
        ),
        ("body", pr.get("body").cloned().unwrap_or(string(""))),
        (
            "mergeCommit",
            match pr.get("mergeCommit") {
                Some(commit) => object([("oid", commit.clone())]),
                None => Null,
            },
        ),
    ]);
    if wanted.iter().any(|w| w == "headRefOid") {
        let head = match pr.get("headRefOid") {
            Some(head) if head.truthy() => head.clone(),
            _ => string(
                origin_tip(pr.at("head").str())
                    .unwrap_or_else(|| pr.at("createdHead").str().to_owned()),
            ),
        };
        fields.set("headRefOid", head);
    }
    fields
}

const PR_LINE_OF_NEWEST: &str =
    r#".[0] // empty | "\(.state) \(.url) \(.headRefOid) \(.mergeCommit.oid // "")""#;

const PR_LINE_OF_NEWEST_WITH_OWNERSHIP: &str = r#".[0] // empty | "\(.isCrossRepository) \(.state) \(.url) \(.headRefOid) \(.mergeCommit.oid // "")""#;

const WORKFLOW_RUN_LINES: &str = r#".workflow_runs[] | "\(.status) \(.conclusion) \(.name)""#;

const COMMIT_PR_LINES: &str = r#".[] | "\(.merge_commit_sha) \(.number)""#;

const LATEST_RUN_LINE: &str = r#".[] | "\(.headSha) \(.status) \(.conclusion)""#;

const RELEASE_RUN_LINE: &str = r#".[] | "\(.databaseId) \(.url) \(.status) \(.conclusion)""#;

const FAILED_JOB_NAMES: &str = r#"[.jobs[] | select(.conclusion != "success" and .conclusion != "skipped" and .conclusion != "neutral") | .name] | join(", ")"#;

/// Newest first. `--search head:<prefix>` matches head branches starting with
/// <prefix>, as GitHub's search does; `--head <branch>` matches that branch
/// exactly. `--jq` supports the newest-PR line with or without its ownership.
fn pr_list(state: &Json, flags: &Flags) {
    let wanted_state = flag(flags, "state").unwrap_or("open").to_uppercase();
    let search = flag(flags, "search").unwrap_or("");
    let prefix = search.strip_prefix("head:").unwrap_or("");
    if !search.is_empty() && prefix.is_empty() {
        die(&format!("fake gh: unsupported search {search}"), 2);
    }
    let limit: usize = flag(flags, "limit").unwrap_or("30").parse().unwrap();
    let wanted = wanted_fields(flags);
    let listed: Vec<Json> = prs(state)
        .iter()
        .rev()
        .filter(|pr| {
            let head = pr.at("head").str();
            head.starts_with(prefix)
                && flag(flags, "head").is_none_or(|wanted| wanted == head)
                && (wanted_state == "ALL" || pr.at("state").str() == wanted_state)
        })
        .take(limit)
        .map(|pr| json_fields(&pr_fields(pr, &wanted), &wanted))
        .collect();
    match flag(flags, "jq") {
        None => println!("{}", Array(listed)),
        Some(jq @ (PR_LINE_OF_NEWEST | PR_LINE_OF_NEWEST_WITH_OWNERSHIP)) => {
            if let Some(pr) = listed.first() {
                let merge = match pr.at("mergeCommit") {
                    commit if commit.truthy() => {
                        commit.get("oid").map(Json::python).unwrap_or_default()
                    }
                    _ => String::new(),
                };
                let ownership = match jq {
                    PR_LINE_OF_NEWEST_WITH_OWNERSHIP => format!("{} ", pr.at("isCrossRepository")),
                    _ => String::new(),
                };
                let line = format!(
                    "{ownership}{} {} {} {merge}",
                    pr.at("state").python(),
                    pr.at("url").python(),
                    pr.at("headRefOid").python()
                );
                println!("{}", line.trim_end());
            }
        }
        jq => unsupported_jq(jq),
    }
}

fn pr_ready(state: &mut Json, positional: &[String], flags: &Flags) {
    let at = selected_pr(state, &positional[0]);
    let pr = &mut state.at_mut("prs").items_mut()[at];
    ensure_open(pr);
    pr.set("isDraft", Bool(flags.contains_key("undo")));
    save(state);
}

fn scenario_root() -> PathBuf {
    state_path().parent().unwrap().to_owned()
}

fn origin_repo() -> PathBuf {
    scenario_root().join("origin.git")
}

/// The tip of `branch` on origin, if origin has it.
fn origin_tip(branch: &str) -> Option<String> {
    let tip = git(
        &origin_repo(),
        &["rev-parse", &format!("refs/heads/{branch}")],
    );
    tip.status
        .success()
        .then(|| String::from_utf8(tip.stdout).unwrap().trim().to_owned())
}

/// The tip of `branch` on origin, which the fake's contract says it has.
fn branch_tip(branch: &str) -> String {
    origin_tip(branch).unwrap_or_else(|| panic!("origin has no branch {branch}"))
}

/// Only a merge commit on a matching head is supported: anything else, such
/// as --auto or --delete-branch, is an unexpected call.
fn pr_merge(state: &mut Json, positional: &[String], flags: &Flags) {
    let supported = ["merge", "match-head-commit", "repo", "R"];
    if !flags.contains_key("merge")
        || !flags.contains_key("match-head-commit")
        || flags.keys().any(|name| !supported.contains(&name.as_str()))
    {
        let names: Vec<&String> = flags.keys().collect();
        die(
            &format!(
                "fake gh: unsupported pr merge flags {}",
                python_list(&names)
            ),
            2,
        );
    }
    run_hook(state, "on_merge", &[]);
    if let Some(refusal) = state.get_mut("refuse_merges")
        && refusal.truthy()
        && refusal.at("times").as_i64().unwrap() > 0
    {
        let times = refusal.at("times").as_i64().unwrap();
        refusal.set("times", number(times - 1));
        let error = refusal.at("error").str().to_owned();
        save(state);
        die(&error, 1);
    }
    let at = selected_pr(state, &positional[0]);
    let pr = &prs(state)[at];
    let head = pr.at("head").str().to_owned();
    ensure_open(pr);
    let base = pr.at("base").str().to_owned();
    let head_sha = branch_tip(&head);
    if Some(head_sha.as_str()) != flag(flags, "match-head-commit") {
        die(
            "GraphQL: Head branch was modified. Review and try the merge again. (mergePullRequest)",
            1,
        );
    }
    let owner = state.at("repo").str().split('/').next().unwrap();
    let message = format!(
        "Merge pull request #{} from {owner}/{head}",
        pr.at("number")
    );
    let work = scenario_root().join(format!("tmp-merge-{}", std::process::id()));
    fs::create_dir(&work).unwrap();
    let merged = merge_in(&work, &base, &head_sha, &message);
    fs::remove_dir_all(&work).unwrap();
    if let Err(error) = merged {
        die(&error, 1);
    }
    let merge_commit = branch_tip(&base);
    let pr = &mut state.at_mut("prs").items_mut()[at];
    pr.set("state", string("MERGED"));
    pr.set("headRefOid", string(head_sha));
    pr.set("mergeCommit", string(&merge_commit));
    save(state);
    if let Some(script) = state.get("after_merge") {
        unlock();
        let env = [
            ("FAKE_MERGE_HEAD", head.as_str()),
            ("FAKE_MERGE_SHA", merge_commit.as_str()),
        ];
        assert!(
            run_script(script.str(), &env),
            "the after-merge script failed"
        );
    }
}

/// Merge `head_sha` into `base` with a merge commit in a clone of origin at
/// `work`, and push it.
fn merge_in(work: &Path, base: &str, head_sha: &str, message: &str) -> Result<(), String> {
    let origin = origin_repo();
    git(
        work,
        &["clone", "-q", "-b", base, origin.to_str().unwrap(), "."],
    );
    if !git(work, &["merge", "-q", "--no-ff", "-m", message, head_sha])
        .status
        .success()
    {
        return Err(
            "Pull request is not mergeable: the merge commit cannot be cleanly created.".into(),
        );
    }
    let pushed = git(work, &["push", "-q", "origin", &format!("HEAD:{base}")]);
    if !pushed.status.success() {
        return Err(format!(
            "fake gh: pushing the merge failed: {}",
            String::from_utf8_lossy(&pushed.stderr)
        ));
    }
    Ok(())
}

fn unknown_issue(number: &str) -> ! {
    die(
        &format!(
            "GraphQL: Could not resolve to an issue or pull request with the number of {number}."
        ),
        1,
    )
}

/// The JSON fields of issue `n`, in state `issue_state`.
fn issue_fields(state: &Json, n: &str, issue_state: &Json) -> Json {
    let title = match state.get("titles").and_then(|titles| titles.get(n)) {
        Some(title) => title.clone(),
        None => string(format!("Issue {n}")),
    };
    let url = format!("https://github.com/{}/issues/{n}", state.at("repo").str());
    let labels = issue_labels(state, n)
        .iter()
        .map(|label| object([("name", label.clone())]))
        .collect();
    let created = match state.get("created").and_then(|created| created.get(n)) {
        Some(created) => created.clone(),
        None => string("2020-01-01T00:00:00Z"),
    };
    object([
        (
            "author_association",
            state
                .get("author_associations")
                .and_then(|authors| authors.get(n))
                .cloned()
                .unwrap_or_else(|| string("OWNER")),
        ),
        ("number", number(n.parse::<i64>().unwrap())),
        // The fake's database ids are the issue numbers.
        ("id", number(n.parse::<i64>().unwrap())),
        ("state", issue_state.clone()),
        ("title", title),
        ("url", string(url)),
        ("labels", Array(labels)),
        ("createdAt", created),
        (
            "body",
            state
                .get("bodies")
                .and_then(|bodies| bodies.get(n))
                .cloned()
                .unwrap_or_else(|| string("")),
        ),
    ])
}

/// The labels of issue `n`.
fn issue_labels<'a>(state: &'a Json, n: &str) -> &'a [Json] {
    state
        .get("labels")
        .and_then(|labels| labels.get(n))
        .map(Json::items)
        .unwrap_or_default()
}

/// The time, as GitHub writes one: `2026-10-01T12:00:00Z`.
fn now() -> String {
    let date = Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .unwrap();
    String::from_utf8(date.stdout).unwrap().trim().to_owned()
}

/// `gh api --method PUT repos/<repo>/issues/<number>/labels -f
/// labels[]=<label> ...`: set the issue's labels to exactly those given.
fn issue_labels_put(state: &mut Json, args: &[&str]) {
    let (positional, _) = parse(args);
    let issue = positional.first().and_then(|path| {
        let (repo, rest) = repo_prefix(path)?;
        let n = rest.strip_prefix("issues/")?.strip_suffix("/labels")?;
        Some((repo, n))
    });
    let fields = args.windows(2).filter(|pair| pair[0] == "-f");
    let labels: Option<Vec<Json>> = fields
        .map(|pair| pair[1].strip_prefix("labels[]=").map(string))
        .collect();
    let (Some((repo, n)), Some(labels)) = (issue, labels) else {
        die(
            &format!("fake gh: unsupported api PUT {}", python_list(args)),
            2,
        )
    };
    check_repo_is(state, Some(repo));
    if !state.at("issues").has(n) {
        die("gh: Not Found (HTTP 404)", 1);
    }
    // Model the real Claim and Ready transitions consumed by Pickup, including
    // consecutive applications/removals within the same second. Other label
    // histories are supplied explicitly by fixtures that need them.
    let before = issue_labels(state, n).to_vec();
    let contains = |labels: &[Json], name: &Json| {
        labels
            .iter()
            .any(|label| label.str().eq_ignore_ascii_case(name.str()))
    };
    let at = string(now());
    let events = before
        .iter()
        .filter(|label| !contains(&labels, label))
        .map(|label| ("unlabeled", label))
        .chain(
            labels
                .iter()
                .filter(|label| !contains(&before, label))
                .map(|label| ("labeled", label)),
        )
        .filter(|(_, label)| {
            matches!(
                label.str().to_ascii_lowercase().as_str(),
                "in-progress" | "ready-for-agent"
            )
        })
        .map(|(event, label)| {
            object([
                ("event", string(event)),
                ("label", label.clone()),
                ("at", at.clone()),
            ])
        })
        .collect::<Vec<_>>();
    if !events.is_empty() {
        state
            .entry("timeline", object([]))
            .entry(n, Array(Vec::new()))
            .items_mut()
            .extend(events);
    }
    state.entry("labels", object([])).set(n, Array(labels));
    save(state);
}

/// `gh api --method POST repos/<repo>/issues/<parent>/sub_issues -F
/// sub_issue_id=<id>`: link the issue as a native sub-issue of the parent.
/// The fake's database ids are the issue numbers.
fn api_add_sub_issue(state: &mut Json, args: &[&str]) {
    let (positional, _) = parse(args);
    let parent = positional.first().and_then(|path| {
        let (repo, rest) = repo_prefix(path)?;
        check_repo_is(state, Some(repo));
        rest.strip_prefix("issues/")?
            .strip_suffix("/sub_issues")
            .filter(|parent| state.at("issues").has(parent))
    });
    let child = args
        .windows(2)
        .filter(|pair| pair[0] == "-F")
        .filter_map(|pair| pair[1].strip_prefix("sub_issue_id="))
        .next()
        .filter(|child| state.at("issues").has(child));
    let (Some(parent), Some(child)) = (parent, child) else {
        die(
            &format!("fake gh: unsupported api POST {}", python_list(args)),
            2,
        )
    };
    let mut sub = listed(state, "sub_issues", parent);
    if !sub.iter().any(|ticket| ticket.python().as_str() == child) {
        sub.push(number(child.parse::<i64>().unwrap()));
    }
    state
        .entry("sub_issues", object([]))
        .set(parent, Array(sub));
    save(state);
}

/// `gh api --method POST repos/<repo>/issues/<ticket>/dependencies/blocked_by
/// -F issue_id=<id>`: link the ticket as blocked by the issue. The fake's
/// database ids are the issue numbers.
fn api_add_blocked_by(state: &mut Json, args: &[&str]) {
    let (positional, _) = parse(args);
    let ticket = positional.first().and_then(|path| {
        let (repo, rest) = repo_prefix(path)?;
        check_repo_is(state, Some(repo));
        rest.strip_prefix("issues/")?
            .strip_suffix("/dependencies/blocked_by")
            .filter(|ticket| state.at("issues").has(ticket))
    });
    let blocker = args
        .windows(2)
        .filter(|pair| pair[0] == "-F")
        .filter_map(|pair| pair[1].strip_prefix("issue_id="))
        .next()
        .filter(|blocker| state.at("issues").has(blocker));
    let (Some(ticket), Some(blocker)) = (ticket, blocker) else {
        die(
            &format!("fake gh: unsupported api POST {}", python_list(args)),
            2,
        )
    };
    let mut blocked = listed(state, "blocked_by", ticket);
    if !blocked.iter().any(|edge| edge.python().as_str() == blocker) {
        blocked.push(number(blocker.parse::<i64>().unwrap()));
    }
    state
        .entry("blocked_by", object([]))
        .set(ticket, Array(blocked));
    save(state);
}

/// `gh api --method DELETE repos/<repo>/issues/<number>/labels/<label>`: take
/// the label, whatever its case, off the issue.
fn issue_label_delete(state: &mut Json, args: &[&str]) {
    let (positional, _) = parse(args);
    let label = positional.first().and_then(|path| {
        let (repo, rest) = repo_prefix(path)?;
        let (n, name) = rest.strip_prefix("issues/")?.split_once("/labels/")?;
        Some((repo, n, name))
    });
    let Some((repo, n, name)) = label else {
        die(
            &format!("fake gh: unsupported api DELETE {}", python_list(args)),
            2,
        )
    };
    check_repo_is(state, Some(repo));
    if !state.at("issues").has(n) {
        die("gh: Not Found (HTTP 404)", 1);
    }
    let is_named = |label: &Json| label.str().eq_ignore_ascii_case(name);
    if !issue_labels(state, n).iter().any(is_named) {
        die("gh: Label does not exist (HTTP 404)", 1);
    }
    let labels = state.at_mut("labels").at_mut(n).items_mut();
    labels.retain(|label| !is_named(label));
    save(state);
}

/// After the issue's `on-issue-view` script, if it has one still to run.
fn issue_view(state: &mut Json, positional: &[String], flags: &Flags) {
    let n = &positional[0];
    run_issue_view_hook(state, n, flag(flags, "json").unwrap_or_default());
    let Some(issue_state) = state.at("issues").get(n) else {
        unknown_issue(n)
    };
    let fields = issue_fields(state, n, issue_state);
    println!("{}", json_fields(&fields, &wanted_fields(flags)));
}

/// `gh issue list [--label <label>] --json <fields>`: the open issues with the
/// label, whatever its case, newest first, or with `--state closed` the
/// closed ones. Without `--label`, every issue in that state.
fn issue_list(state: &Json, flags: &Flags) {
    let supported = ["label", "state", "json", "limit", "repo", "R"];
    let label = flag(flags, "label");
    if flags.keys().any(|name| !supported.contains(&name.as_str()))
        || !matches!(flag(flags, "state"), None | Some("open" | "closed"))
        || label.is_some_and(|label| label.contains(','))
    {
        die(
            &format!("fake gh: unsupported issue list flags {flags:?}"),
            2,
        );
    }
    let wanted_state = flag(flags, "state").unwrap_or("open").to_uppercase();
    let limit: usize = flag(flags, "limit").unwrap_or("30").parse().unwrap();
    let wanted = wanted_fields(flags);
    let listed = matching_issues(state, &wanted_state, label)
        .into_iter()
        .take(limit)
        .map(|(n, issue_state)| {
            json_fields(&issue_fields(state, &n.to_string(), issue_state), &wanted)
        })
        .collect();
    println!("{}", Array(listed));
}

/// Issues matching a state and optional label, newest first, for both the
/// CLI listing and the REST listing.
fn matching_issues<'a>(
    state: &'a Json,
    wanted_state: &str,
    label: Option<&str>,
) -> Vec<(i64, &'a Json)> {
    let Json::Object(issues) = state.at("issues") else {
        panic!("issues is an object")
    };
    let labels = state.get("labels");
    let mut matching: Vec<(i64, &Json)> = issues
        .iter()
        .filter(|(n, issue_state)| {
            let labels = labels.and_then(|labels| labels.get(n));
            (wanted_state == "ALL" || issue_state.str() == wanted_state)
                && label.is_none_or(|label| {
                    labels
                        .map(Json::items)
                        .unwrap_or_default()
                        .iter()
                        .any(|name| name.str().eq_ignore_ascii_case(label))
                })
        })
        .map(|(n, issue_state)| (n.parse().unwrap(), issue_state))
        .collect();
    matching.sort_by_key(|(n, _)| -n);
    matching
}

fn issue_close(state: &mut Json, positional: &[String], flags: &Flags) {
    let n = &positional[0];
    let Some(issue_state) = state.at("issues").get(n) else {
        unknown_issue(n)
    };
    if issue_state.str() == "CLOSED" {
        eprintln!("! Issue #{n} is already closed");
        return;
    }
    if let Some(comment) = flag(flags, "comment") {
        state
            .entry("comments", object([]))
            .entry(n, Array(Vec::new()))
            .items_mut()
            .push(string(comment));
    }
    state.at_mut("issues").set(n, string("CLOSED"));
    save(state);
}

/// Comment on issue `n` with `--body`, open or closed.
fn issue_comment(state: &mut Json, positional: &[String], flags: &Flags) {
    let n = &positional[0];
    if state.at("issues").get(n).is_none() {
        unknown_issue(n)
    }
    let body = flag(flags, "body").expect("no --body");
    state
        .entry("comments", object([]))
        .entry(n, Array(Vec::new()))
        .items_mut()
        .push(string(body));
    save(state);
}

/// Open an issue with `--title`, `--body` and the comma-separated `--label`s,
/// numbered one past the highest issue, and print its URL.
fn issue_create(state: &mut Json, flags: &Flags) {
    use std::io::Read;
    let labels: Vec<&str> = flag(flags, "label")
        .map(|labels| labels.split(',').collect())
        .unwrap_or_default();
    for label in &labels {
        if !has_repo_label(state, label) {
            die(&format!("could not add label: '{label}' not found"), 1);
        }
    }
    let Json::Object(issues) = state.at("issues") else {
        panic!("issues is an object")
    };
    let highest = issues
        .iter()
        .filter_map(|(n, _)| n.parse::<i64>().ok())
        .max();
    let n = (highest.unwrap_or(0) + 1).to_string();
    state.at_mut("issues").set(&n, string("OPEN"));
    let title = flag(flags, "title").expect("no --title");
    state.entry("titles", object([])).set(&n, string(title));
    let mut input = String::new();
    let body = if flag(flags, "body-file") == Some("-") {
        std::io::stdin().read_to_string(&mut input).unwrap();
        &input
    } else {
        flag(flags, "body").expect("no --body")
    };
    state.entry("bodies", object([])).set(&n, string(body));
    let labels = labels.into_iter().map(string).collect();
    state.entry("labels", object([])).set(&n, Array(labels));
    state.entry("created", object([])).set(&n, string(now()));
    save(state);
    println!("https://github.com/{}/issues/{n}", state.at("repo").str());
}

/// Whether the repository has the label `name`, whatever its case.
fn has_repo_label(state: &Json, name: &str) -> bool {
    let labels = state
        .get("repo_labels")
        .map(Json::items)
        .unwrap_or_default();
    labels
        .iter()
        .any(|label| label.str().eq_ignore_ascii_case(name))
}

/// `gh label list --json name`: the repository's labels.
fn label_list(state: &Json, flags: &Flags) {
    if flag(flags, "json") != Some("name") {
        die(
            &format!("fake gh: unsupported label list flags {flags:?}"),
            2,
        );
    }
    let labels = state
        .get("repo_labels")
        .map(Json::items)
        .unwrap_or_default();
    let listed = labels
        .iter()
        .map(|label| object([("name", label.clone())]))
        .collect();
    println!("{}", Array(listed));
}

/// `gh label create <name>`: add a label to the repository, failing if it has
/// it already, as gh does without `--force`.
fn label_create(state: &mut Json, positional: &[String]) {
    let name = &positional[0];
    if has_repo_label(state, name) {
        die(
            &format!(
                "label with name \"{name}\" already exists; use `--force` to update its color and description"
            ),
            1,
        );
    }
    state
        .entry("repo_labels", Array(Vec::new()))
        .items_mut()
        .push(string(name));
    save(state);
}

/// A commit endpoint `gh api` answers.
enum Endpoint {
    CheckRuns,
    Status,
    Pulls,
    Runs,
}

/// The repo, commit and endpoint of `path`, as
/// `repos/<repo>/commits/<sha>/(check-runs|status|pulls)` or
/// `repos/<repo>/actions/runs?head_sha=<sha>`, each maybe followed by a query.
fn commit_endpoint(path: &str) -> Option<(&str, &str, Endpoint)> {
    let (repo, rest) = repo_prefix(path)?;
    let is_query = |rest: &str| rest.is_empty() || rest.starts_with(['&', '?']);
    let is_sha =
        |sha: &str| !sha.is_empty() && sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if let Some(rest) = rest.strip_prefix("commits/") {
        let (sha, rest) = rest.split_once('/')?;
        let endpoints = [
            ("check-runs", Endpoint::CheckRuns),
            ("status", Endpoint::Status),
            ("pulls", Endpoint::Pulls),
        ];
        return endpoints.into_iter().find_map(|(name, endpoint)| {
            let query = rest.strip_prefix(name)?;
            (is_sha(sha) && is_query(query)).then_some((repo, sha, endpoint))
        });
    }
    let rest = rest.strip_prefix("actions/runs?head_sha=")?;
    let end = rest
        .find(|c: char| !matches!(c, '0'..='9' | 'a'..='f'))
        .unwrap_or(rest.len());
    let (sha, query) = rest.split_at(end);
    (is_sha(sha) && is_query(query)).then_some((repo, sha, Endpoint::Runs))
}

/// `path` as `repos/<owner>/<name>/<rest>`: `<owner>/<name>` and `<rest>`.
fn repo_prefix(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("repos/")?;
    let (owner, after_owner) = rest.split_once('/')?;
    let (name, rest) = after_owner.split_once('/')?;
    if owner.is_empty() || name.is_empty() {
        return None;
    }
    Some((
        &path["repos/".len()..][..owner.len() + 1 + name.len()],
        rest,
    ))
}

/// The signed-in user, the two commit endpoints thirdshift reads CI from, the
/// workflow runs on a commit that the release script waits for, or the PRs a
/// commit belongs to that the Release notes script looks through. Each
/// answers one page, each item of the list `--jq '.<list>[]'` names on its
/// own line, for the workflow runs one `<status> <conclusion> <name>` line
/// per run with `--jq` set to `WORKFLOW_RUN_LINES`, and for the PRs one
/// `<merge commit> <number>` line per PR with `--jq` set to
/// `COMMIT_PR_LINES`.
/// Advisory create/list/update endpoints, backed by private JSON state.
fn advisory_api(state: &mut Json, positional: &[String], flags: &Flags) {
    use std::io::Read;
    let (path, query) = positional[0]
        .split_once('?')
        .unwrap_or((&positional[0], ""));
    let (repo, rest) = repo_prefix(path).unwrap();
    check_repo_is(state, Some(repo));
    if let Some(error) = state.get("advisory_error") {
        die(error.str(), 1);
    }
    if state.get("private").is_some_and(Json::truthy) {
        die("gh: Not Found (HTTP 404)", 1);
    }
    let method = flag(flags, "method").unwrap_or("GET");
    if method == "GET" {
        if let Some(id) = rest.strip_prefix("security-advisories/") {
            let advisory = state
                .at("advisories")
                .items()
                .iter()
                .find(|advisory| advisory.at("ghsa_id").as_str() == Some(id))
                .unwrap();
            println!("{advisory}");
            return;
        }
        let wanted = query
            .split('&')
            .find_map(|pair| pair.strip_prefix("state="));
        let advisories = state.get("advisories").map(Json::items).unwrap_or_default();
        let listed: Vec<Json> = advisories
            .iter()
            .filter(|record| {
                wanted.is_none_or(|wanted| record.at("state").as_str() == Some(wanted))
            })
            .cloned()
            .collect();
        if flag(flags, "jq") == Some(".[]") {
            for record in listed {
                println!("{record}");
            }
        } else {
            println!("{}", Array(listed));
        }
        return;
    }
    let mut input = String::new();
    let mut body = if flag(flags, "input") == Some("-") {
        std::io::stdin().read_to_string(&mut input).unwrap();
        parse_json(&input)
    } else {
        object([])
    };
    if method == "POST"
        && state
            .get("advisory_create_fails_after")
            .and_then(Json::as_i64)
            .is_some_and(|limit| {
                state
                    .get("advisories")
                    .map_or(0, |records| records.items().len())
                    >= limit as usize
            })
    {
        die("HTTP 422: Private candidate write-up.", 1);
    }
    let advisories = state.entry("advisories", Array(Vec::new())).items_mut();
    if method == "POST" && rest == "security-advisories" {
        if !body.has("summary") || !body.has("description") || !body.has("vulnerabilities") {
            die("HTTP 422: required advisory fields missing", 1);
        }
        let id = format!("GHSA-test-test-{:04}", advisories.len() + 1);
        body.set("ghsa_id", string(&id));
        body.set(
            "html_url",
            string(format!(
                "https://github.com/{repo}/security/advisories/{id}"
            )),
        );
        body.set("state", string("draft"));
        advisories.push(body.clone());
    } else if method == "PATCH" {
        let id = rest.strip_prefix("security-advisories/").unwrap();
        let advisory = advisories
            .iter_mut()
            .find(|advisory| advisory.at("ghsa_id").as_str() == Some(id))
            .unwrap();
        let Json::Object(fields) = body else {
            panic!("advisory update must be an object");
        };
        for (key, value) in fields {
            advisory.set(&key, value);
        }
        body = advisory.clone();
    } else {
        die("fake gh: unsupported advisory operation", 2);
    }
    save(state);
    println!("{body}");
}

/// Update a finding issue's body through the private-record operation.
fn finding_issue_patch(state: &mut Json, rest: &[&str]) {
    use std::io::Read;
    let (positional, flags) = parse(rest);
    let (repo, path) = repo_prefix(&positional[0]).unwrap();
    check_repo_is(state, Some(repo));
    let n = path.strip_prefix("issues/").unwrap();
    assert_eq!(flag(&flags, "input"), Some("-"));
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    let body = parse_json(&input);
    let issue_state = state.at("issues").at(n).clone();
    state
        .entry("bodies", object([]))
        .set(n, body.at("body").clone());
    save(state);
    println!("{}", issue_fields(state, n, &issue_state));
}

/// Answer either membership endpoint from its list of account logins.
fn membership_response(state: &Json, key: &str, login: &str) {
    let member = state.get(key).is_some_and(|users| {
        users
            .items()
            .iter()
            .any(|user| user.str().eq_ignore_ascii_case(login))
    });
    println!(
        "HTTP/2.0 {}\r\n",
        if member {
            "204 No Content"
        } else {
            "404 Not Found"
        }
    );
    if !member {
        std::process::exit(1);
    }
}

fn api(state: &mut Json, positional: &[String], flags: &Flags) {
    let path = &positional[0];
    if path == "rate_limit" {
        let reset = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 3600;
        let defaults = object([("remaining", number(4999)), ("reset", number(reset))]);
        let resource = |name| {
            state
                .get("rate_limits")
                .and_then(|limits| limits.get(name))
                .cloned()
                .unwrap_or_else(|| defaults.clone())
        };
        println!(
            "{}",
            object([(
                "resources",
                object([("core", resource("core")), ("graphql", resource("graphql")),])
            )])
        );
        if let Some(next) = state.get("next_rate_limits").cloned() {
            state.set("rate_limits", next);
            save(state);
        }
        return;
    }
    if let Some((repo, rest)) = repo_prefix(path) {
        if let Some(n) = rest
            .strip_prefix("issues/")
            .and_then(|s| s.strip_suffix("/events?per_page=100"))
        {
            check_repo_is(state, Some(repo));
            assert!(flags.contains_key("paginate"));
            assert_eq!(flag(flags, "jq"), Some(".[]"));
            let defaults = vec![object([
                ("event", string("labeled")),
                ("label", object([("name", string("ready-for-agent"))])),
                (
                    "actor",
                    object([("login", string(repo.split('/').next().unwrap()))]),
                ),
                ("created_at", string("2020-01-01T00:00:00Z")),
            ])];
            let events = state
                .get("trust_events")
                .and_then(|events| events.get(n))
                .map(Json::items)
                .unwrap_or(&defaults);
            for event in events {
                println!("{event}");
            }
            return;
        }
        if let Some(login) = rest.strip_prefix("collaborators/") {
            check_repo_is(state, Some(repo));
            assert!(flags.contains_key("include"));
            membership_response(state, "collaborators", login);
            return;
        }
    }
    if let Some(login) = path
        .strip_prefix("orgs/")
        .and_then(|s| s.split_once("/members/").map(|(_, login)| login))
    {
        assert!(flags.contains_key("include"));
        membership_response(state, "org_members", login);
        return;
    }
    if let Some((repo, rest)) = repo_prefix(path)
        && let Some(n) = rest.strip_prefix("issues/")
        && n.parse::<u64>().is_ok()
    {
        check_repo_is(state, Some(repo));
        if flag(flags, "jq") == Some(".id") {
            println!("{n}");
            return;
        }
        println!("{}", issue_fields(state, n, state.at("issues").at(n)));
        return;
    }
    if path == &format!("repos/{}", state.at("repo").str()) {
        println!(
            "{}",
            object([
                (
                    "owner",
                    object([(
                        "type",
                        state
                            .get("owner_type")
                            .cloned()
                            .unwrap_or_else(|| string("User"))
                    )])
                ),
                (
                    "private",
                    Bool(state.get("private").is_some_and(Json::truthy))
                )
            ])
        );
        return;
    }
    if let Some((repo, rest)) = repo_prefix(path)
        && rest.starts_with("issues?")
    {
        check_repo_is(state, Some(repo));
        let query: BTreeMap<_, _> = rest
            .strip_prefix("issues?")
            .unwrap()
            .split('&')
            .map(|parameter| parameter.split_once('=').unwrap())
            .collect();
        let wanted_state = query["state"].to_uppercase();
        assert!(matches!(wanted_state.as_str(), "OPEN" | "CLOSED" | "ALL"));
        assert_eq!(query["per_page"], "100");
        assert!(flags.contains_key("paginate"));
        assert_eq!(flag(flags, "jq"), Some(".[]"));
        for (n, issue_state) in matching_issues(state, &wanted_state, Some(query["labels"])) {
            let mut issue = issue_fields(state, &n.to_string(), issue_state);
            issue.set("html_url", issue.at("url").clone());
            println!("{issue}");
        }
        return;
    }
    if path.contains("/security-advisories") {
        advisory_api(state, positional, flags);
        return;
    }
    if path == "user" && flags.is_empty() {
        let email = state.get("user_email").cloned().unwrap_or(Null);
        println!(
            "{}",
            object([("login", string("runner")), ("email", email)])
        );
        return;
    }
    if let Some((repo, rest)) = repo_prefix(path)
        && let Some(id) = rest.strip_prefix("actions/jobs/")
    {
        check_repo_is(state, Some(repo));
        println!("{}", actions_job(state, id));
        return;
    }
    let Some((repo, sha, endpoint)) = commit_endpoint(path) else {
        die(&format!("fake gh: unsupported api path {path}"), 2)
    };
    check_repo_is(state, Some(repo));
    let jq = flag(flags, "jq");

    let (items, field) = match endpoint {
        Endpoint::Runs => {
            if jq == Some(".workflow_runs[]") {
                for run in state.get("runs").map(Json::items).unwrap_or_default() {
                    if run.get("headSha").and_then(Json::as_str) == Some(sha) {
                        let mut run = run.clone();
                        run.set("id", run.at("databaseId").clone());
                        println!("{run}");
                    }
                }
                return;
            }
            if jq != Some(WORKFLOW_RUN_LINES) {
                unsupported_jq(jq);
            }
            for run in read_checks(state, sha) {
                let conclusion = run.at("conclusion");
                let conclusion = if conclusion.truthy() {
                    conclusion.python()
                } else {
                    "null".to_owned()
                };
                println!(
                    "{} {conclusion} {}",
                    run.at("status").python(),
                    run.at("name").python()
                );
            }
            return;
        }
        Endpoint::Pulls => {
            if jq != Some(COMMIT_PR_LINES) {
                unsupported_jq(jq);
            }
            for pr in commit_prs(state, sha) {
                let merge = match pr.get("mergeCommit") {
                    Some(commit) if commit.truthy() => commit.python(),
                    _ => "null".to_owned(),
                };
                println!("{merge} {}", pr.at("number").python());
            }
            return;
        }
        Endpoint::CheckRuns => (read_checks(state, sha), "check_runs"),
        Endpoint::Status => {
            let statuses = state
                .get("statuses")
                .and_then(|statuses| statuses.get(sha))
                .map(Json::items)
                .unwrap_or_default();
            let items = statuses
                .iter()
                .map(|status| {
                    object([
                        ("context", status.at("context").clone()),
                        ("state", status.at("state").clone()),
                        ("target_url", status.get("url").cloned().unwrap_or(Null)),
                    ])
                })
                .collect();
            (items, "statuses")
        }
    };
    if jq != Some(format!(".{field}[]").as_str()) {
        unsupported_jq(jq);
    }
    for item in items {
        println!("{item}");
    }
}

/// The PRs merged as `sha` or whose head is `sha`, oldest first.
fn commit_prs<'a>(state: &'a Json, sha: &str) -> Vec<&'a Json> {
    let head = |pr: &Json| match pr.get("headRefOid") {
        Some(head) => head.as_str().map(str::to_owned),
        None => origin_tip(pr.at("head").str()),
    };
    prs(state)
        .iter()
        .filter(|pr| {
            pr.get("mergeCommit").and_then(Json::as_str) == Some(sha)
                || head(pr).as_deref() == Some(sha)
        })
        .collect()
}

/// The check runs on `sha`, each read once, after any `on-ci-read` script.
fn read_checks(state: &mut Json, sha: &str) -> Vec<Json> {
    run_ci_read_hook(state, sha);
    let mut runs = Vec::new();
    let checks = state
        .get_mut("checks")
        .and_then(|checks| checks.get_mut(sha));
    for check in checks.map(Json::items_mut).into_iter().flatten() {
        let details_url = check.get("url").cloned().unwrap_or(Null);
        // The attempt before a re-run, while GitHub still lists it.
        if let Some(stale) = check.get_mut("stale").filter(|stale| stale.truthy()) {
            let polls = stale.at("polls").as_i64().unwrap();
            if polls > 0 {
                stale.set("polls", number(polls - 1));
                runs.push(object([
                    ("id", stale.at("id").clone()),
                    ("name", check.at("name").clone()),
                    ("status", string("completed")),
                    ("conclusion", check.at("stale").at("conclusion").clone()),
                    ("details_url", details_url),
                ]));
                continue;
            }
        }
        let pending_polls = check
            .get("pending_polls")
            .and_then(Json::as_i64)
            .unwrap_or(0);
        let pending = pending_polls > 0;
        if pending {
            check.set("pending_polls", number(pending_polls - 1));
        }
        runs.push(object([
            ("id", check.get("id").cloned().unwrap_or(Null)),
            ("name", check.at("name").clone()),
            (
                "status",
                string(if pending { "in_progress" } else { "completed" }),
            ),
            (
                "conclusion",
                if pending {
                    Null
                } else {
                    check.at("conclusion").clone()
                },
            ),
            ("details_url", details_url),
        ]));
    }
    save(state);
    runs
}

/// The GitHub Actions job whose check run's `id` is `id`, with its steps.
fn actions_job(state: &Json, id: &str) -> Json {
    let checks = state.get("checks").cloned().unwrap_or(object([]));
    let Json::Object(commits) = &checks else {
        panic!("checks is an object")
    };
    let has_id = |run: Option<&Json>| {
        run.and_then(|run| run.get("id"))
            .is_some_and(|run_id| run_id.python() == id)
    };
    let checks: Vec<&Json> = commits.iter().flat_map(|(_, list)| list.items()).collect();
    // The attempt before a re-run failed on a failed step.
    if let Some(check) = checks.iter().find(|check| has_id(check.get("stale"))) {
        let conclusion = check.at("stale").at("conclusion").clone();
        let step = object([
            ("status", string("completed")),
            ("conclusion", conclusion.clone()),
        ]);
        return object([
            ("id", check.at("stale").at("id").clone()),
            ("status", string("completed")),
            ("conclusion", conclusion),
            ("steps", Array(vec![step])),
        ]);
    }
    let Some(check) = checks.into_iter().find(|check| has_id(Some(check))) else {
        die(&format!("gh: Not Found (HTTP 404): no job {id}"), 1)
    };
    let pending = check
        .get("pending_polls")
        .and_then(Json::as_i64)
        .is_some_and(|polls| polls > 0);
    let (status, conclusion) = if pending {
        (string("in_progress"), Null)
    } else {
        (string("completed"), check.at("conclusion").clone())
    };
    let steps = match check.get("steps").filter(|steps| steps.truthy()) {
        Some(steps) => steps.clone(),
        None => Array(vec![object([
            ("status", status.clone()),
            ("conclusion", conclusion.clone()),
        ])]),
    };
    object([
        ("id", check.at("id").clone()),
        ("status", status),
        ("conclusion", conclusion),
        ("steps", steps),
    ])
}

/// The next check run `id`, each one higher than the last.
fn next_check_id(state: &mut Json) -> Json {
    let next = state.get("next_check_id").and_then(Json::as_i64);
    let id = next.unwrap_or(1);
    state.set("next_check_id", number(id + 1));
    number(id)
}

/// `gh fake checks <sha> <list>`: set the check runs on `sha`, each given the
/// next `id` if it has none.
fn set_checks(state: &mut Json, sha: &str, mut checks: Json) {
    for check in checks.items_mut() {
        if !check.has("id") {
            check.set("id", next_check_id(state));
        }
    }
    state.entry("checks", object([])).set(sha, checks);
}

/// Whether a check run or job that came to `conclusion` failed.
fn is_failure(conclusion: &str) -> bool {
    !["success", "skipped", "neutral"].contains(&conclusion)
}

/// `gh run rerun <run> --failed`: start a new attempt of each failed check
/// run of the workflow run `<run>`, which comes to what the check's `rerun`
/// says, or fails as the attempt before it did. With no failed check run of
/// that workflow run, it fails as gh does on a run it can't re-run.
fn run_rerun(state: &mut Json, positional: &[String], flags: &Flags) {
    let supported = ["failed", "repo", "R"];
    if !flags.contains_key("failed") || flags.keys().any(|name| !supported.contains(&name.as_str()))
    {
        die(
            &format!("fake gh: unsupported run rerun flags {flags:?}"),
            2,
        );
    }
    let run = &positional[0];
    let of_run = format!("/actions/runs/{run}/job/");
    let mut checks = state.get("checks").cloned().unwrap_or(object([]));
    let Json::Object(commits) = &mut checks else {
        panic!("checks is an object")
    };
    let mut rerun_any = false;
    for check in commits.iter_mut().flat_map(|(_, list)| list.items_mut()) {
        let conclusion = check.at("conclusion").clone();
        let failed = is_failure(conclusion.str());
        let url = check.get("url").and_then(Json::as_str).unwrap_or("");
        if !failed || !url.contains(&of_run) {
            continue;
        }
        rerun_any = true;
        let rerun = check.get("rerun").filter(|rerun| rerun.truthy()).cloned();
        let polls = |name: &str| {
            let polls = rerun.as_ref().and_then(|rerun| rerun.get(name));
            number(polls.and_then(Json::as_i64).unwrap_or(0))
        };
        if let Some(rerun) = &rerun {
            check.set("conclusion", rerun.at("conclusion").clone());
        }
        let steps = rerun.as_ref().and_then(|rerun| rerun.get("steps")).cloned();
        check.set("steps", steps.unwrap_or(Null));
        let (stale_polls, pending_polls) = (polls("stale_polls"), polls("pending_polls"));
        let before = check.at("id").clone();
        check.set(
            "stale",
            object([
                ("id", before),
                ("conclusion", conclusion),
                ("polls", stale_polls),
            ]),
        );
        check.set("pending_polls", pending_polls);
        check.set("rerun", Null);
        check.set("id", next_check_id(state));
    }
    if !rerun_any {
        die(
            &format!("run {run} cannot be rerun; its workflow file may be broken"),
            1,
        );
    }
    state.set("checks", checks);
    save(state);
}

/// The `-f`/`-F` fields of a `gh api` call, by name.
fn api_fields(args: &[&str]) -> BTreeMap<String, String> {
    args.windows(2)
        .filter(|pair| ["-f", "-F", "--raw-field", "--field"].contains(&pair[0]))
        .map(|pair| {
            let (name, value) = pair[1]
                .split_once('=')
                .unwrap_or_else(|| panic!("field {:?} has no '='", pair[1]));
            (name.to_owned(), value.to_owned())
        })
        .collect()
}

/// The `<repo>` of `repos/<repo>/releases/generate-notes`.
fn generate_notes_repo(path: &str) -> Option<&str> {
    repo_prefix(path)
        .filter(|(_, rest)| *rest == "releases/generate-notes")
        .map(|(repo, _)| repo)
}

/// GitHub's generated release notes for `tag_name` at `target_commitish`, or
/// at the tag itself without one: one line per merged PR whose merge commit
/// is reachable from the target but not from `previous_tag_name`, oldest
/// first, then the compare link. Only `--jq .body` is supported.
fn generate_notes(state: &Json, args: &[&str]) {
    let fields = api_fields(args);
    let (positional, flags) = parse(args);
    let jq = flag(&flags, "jq");
    if jq != Some(".body") {
        unsupported_jq(jq);
    }
    check_repo_is(state, generate_notes_repo(&positional[0]));
    // GitHub ignores the target when the tag exists.
    let tag = &fields["tag_name"];
    let target = fields.get("target_commitish").unwrap_or(tag);
    let previous = fields
        .get("previous_tag_name")
        .filter(|previous| !previous.is_empty());
    let reachable = |commit: &str, git_ref: &str| {
        git(
            &origin_repo(),
            &["merge-base", "--is-ancestor", commit, git_ref],
        )
        .status
        .success()
    };
    let lines: Vec<String> = prs(state)
        .iter()
        .filter_map(|pr| {
            let commit = pr.get("mergeCommit")?.python();
            let listed = reachable(&commit, target)
                && !previous.is_some_and(|previous| reachable(&commit, previous));
            listed.then(|| {
                format!(
                    "* {} by @runner in {}",
                    pr.at("title").python(),
                    pr.at("url").python()
                )
            })
        })
        .collect();
    let compare = match previous {
        Some(previous) => format!("{previous}...{tag}"),
        None => format!("commits/{tag}"),
    };
    println!("## What's Changed\n{}", lines.join("\n"));
    println!(
        "\n**Full Changelog**: https://github.com/{}/compare/{compare}",
        state.at("repo").str()
    );
}

/// `gh api --method PATCH repos/<repo>/pulls/<number> -f body=<body>`: set
/// the PR's body, the only field thirdshift edits.
fn pr_patch(state: &mut Json, args: &[&str]) {
    let (positional, _) = parse(args);
    let pull = positional.first().and_then(|path| {
        let (repo, rest) = repo_prefix(path)?;
        let n = rest.strip_prefix("pulls/")?;
        (!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())).then_some((repo, n))
    });
    let fields = api_fields(args);
    let (Some((repo, n)), true) = (pull, fields.keys().eq(["body"])) else {
        die(
            &format!("fake gh: unsupported api PATCH {}", python_list(args)),
            2,
        )
    };
    check_repo_is(state, Some(repo));
    let Some(pr) = state
        .at_mut("prs")
        .items_mut()
        .iter_mut()
        .find(|pr| pr.at("number").python() == n)
    else {
        die("gh: Not Found (HTTP 404)", 1)
    };
    pr.set("body", string(&fields["body"]));
    save(state);
}

/// Only the flags the release script reads workflow runs with are
/// supported, and only its two `--jq`s: the latest CI run on a branch and the
/// release workflow run on a commit.
fn run_list(state: &mut Json, flags: &Flags) {
    let supported = [
        "branch", "workflow", "event", "commit", "limit", "json", "jq",
    ];
    let unsupported: Vec<String> = flags
        .keys()
        .filter(|name| !supported.contains(&name.as_str()))
        .cloned()
        .collect();
    if !unsupported.is_empty() {
        die(
            &format!(
                "fake gh: unsupported run list flags {}",
                python_list(&unsupported)
            ),
            2,
        );
    }
    let fields: &[&str] = match flag(flags, "jq") {
        Some(LATEST_RUN_LINE) => &["headSha", "status", "conclusion"],
        Some(RELEASE_RUN_LINE) => &["databaseId", "url", "status", "conclusion"],
        jq => unsupported_jq(jq),
    };
    let filters = [
        ("branch", "branch"),
        ("workflow", "workflow"),
        ("event", "event"),
        ("commit", "headSha"),
    ];
    let limit: usize = flag(flags, "limit").unwrap_or("20").parse().unwrap();
    let mut listed = Vec::new();
    let runs = state.get_mut("runs").map(Json::items_mut);
    for run in runs.into_iter().flatten().rev() {
        let matches = filters.iter().all(|(flag_name, field)| {
            flag(flags, flag_name).is_none_or(|wanted| run.at(field).as_str() == Some(wanted))
        });
        if !matches {
            continue;
        }
        let polls = |run: &Json, name: &str| run.get(name).and_then(Json::as_i64).unwrap_or(0);
        let hidden_polls = polls(run, "hidden_polls");
        if hidden_polls > 0 {
            run.set("hidden_polls", number(hidden_polls - 1));
            continue;
        }
        let pending_polls = polls(run, "pending_polls");
        let mut shown = run.clone();
        if pending_polls > 0 {
            run.set("pending_polls", number(pending_polls - 1));
            shown.set("status", string("in_progress"));
            shown.set("conclusion", string(""));
        }
        listed.push(shown);
    }
    save(state);
    for run in listed.iter().take(limit) {
        let line: Vec<String> = fields.iter().map(|field| run.at(field).python()).collect();
        println!("{}", line.join(" "));
    }
}

/// `gh run view <id> --json jobs`, with only the release script's `--jq`: the
/// names of the jobs of its current attempt that did not succeed.
fn run_view(state: &Json, positional: &[String], flags: &Flags) {
    if flag(flags, "json") != Some("jobs") || flag(flags, "jq") != Some(FAILED_JOB_NAMES) {
        die(&format!("fake gh: unsupported run view flags {flags:?}"), 2);
    }
    let id = &positional[0];
    let runs = state.get("runs").map(Json::items).unwrap_or_default();
    let Some(run) = runs.iter().find(|run| run.at("databaseId").python() == *id) else {
        die(&format!("could not find any workflow run with ID {id}"), 1)
    };
    let jobs = run.get("jobs").map(Json::items).unwrap_or_default();
    let failed: Vec<String> = jobs
        .iter()
        .filter(|job| is_failure(job.at("conclusion").str()))
        .map(|job| job.at("name").python())
        .collect();
    println!("{}", failed.join(", "));
}

/// `gh release view <tag> --json url --jq .url`: the Release's URL, for a tag
/// on origin.
fn release_view(state: &Json, positional: &[String], flags: &Flags) {
    if flag(flags, "json") != Some("url") || flag(flags, "jq") != Some(".url") {
        die(
            &format!("fake gh: unsupported release view flags {flags:?}"),
            2,
        );
    }
    let tag = &positional[0];
    let tagged = git(
        &origin_repo(),
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/tags/{tag}"),
        ],
    );
    if !tagged.status.success() {
        die("release not found", 1);
    }
    println!(
        "https://github.com/{}/releases/tag/{tag}",
        state.at("repo").str()
    );
}

/// Record `run`, numbered with the next `databaseId` if it has none, or
/// replace the fields `run` has of the recorded run with its `databaseId`, as
/// a new attempt of that run.
fn record_run(state: &mut Json, mut run: Json) {
    let repo = state.at("repo").str().to_owned();
    let runs = state.entry("runs", Array(Vec::new())).items_mut();
    if let Some(id) = run.get("databaseId").cloned()
        && let Some(recorded) = runs
            .iter_mut()
            .find(|recorded| recorded.get("databaseId") == Some(&id))
    {
        let Json::Object(fields) = run else {
            panic!("a run is an object")
        };
        for (key, value) in fields {
            recorded.set(&key, value);
        }
        return;
    }
    if !run.has("databaseId") {
        let newest = runs
            .iter()
            .filter_map(|recorded| recorded.get("databaseId").and_then(Json::as_i64))
            .max()
            .unwrap_or(0);
        run.set("databaseId", number(newest + 1));
    }
    if !run.has("url") {
        let url = format!(
            "https://github.com/{repo}/actions/runs/{}",
            run.at("databaseId").python()
        );
        run.set("url", string(url));
    }
    runs.push(run);
}

/// The Tickets query or the candidate query, told apart by the candidate
/// query's `timelineItems`: `-f query=...` plus `-f`/`-F` variables.
///
/// The Inbox's batched subject query, identified by its numbered issue/PR
/// lookups with a `rateLimit`: one `alias: repository(owner, name)` per subject,
/// with state, settlement times and issue labels when known, else null.
/// An alias for another repository answers null, and every answer carries the
/// query's own `rateLimit`. An unresolved item adds a scoped GraphQL error
/// and exits 1, keeping the other aliases' data on stdout as `gh` does.
fn graphql(state: &Json, args: &[&str]) {
    let mut variables = api_fields(args);
    let query = variables.remove("query").unwrap_or_default();
    if (query.contains("pullRequest(number:") || query.contains("issue(number:"))
        && query.contains("rateLimit")
    {
        inbox_states(state, &query);
        return;
    }
    let asks_for = |fields: &[&str]| fields.iter().all(|field| query.contains(field));
    let is_candidate = asks_for(&["parent", "subIssues", "blockedBy", "timelineItems"]);
    let is_comments = asks_for(&["comments(", "authorAssociation"]);
    let edits = if asks_for(&["userContentEdits"]) {
        Some(("body", "editedAt"))
    } else if asks_for(&["RENAMED_TITLE_EVENT"]) {
        Some(("title", "createdAt"))
    } else {
        None
    };
    if !is_candidate
        && !is_comments
        && edits.is_none()
        && !asks_for(&["subIssues", "blockedBy", "labels", "totalCount"])
    {
        die(&format!("fake gh: unsupported graphql query {query}"), 2);
    }
    check_repo_is(
        state,
        Some(&format!("{}/{}", variables["owner"], variables["repo"])),
    );
    let n = &variables["number"];
    if !state.at("issues").has(n) {
        die(
            &format!(
                "GraphQL: Could not resolve to an Issue with the number of {n}. (repository.issue)"
            ),
            1,
        );
    }
    let after = variables.get("after").map(String::as_str);
    let issue = if let Some((kind, field)) = edits {
        edit_history(state, n, kind, field, after)
    } else if is_comments {
        issue_comments(state, n, after)
    } else if is_candidate {
        candidate(state, n, after)
    } else {
        let nodes = listed(state, "sub_issues", n)
            .iter()
            .map(|ticket| {
                let key = ticket.python();
                let labels = listed(state, "labels", &key)
                    .into_iter()
                    .map(|label| object([("name", label)]))
                    .collect();
                let mut node = numbered_issue(state, ticket);
                node.set("labels", object([("nodes", Array(labels))]));
                node.set("subIssues", sub_issue_count(state, &key));
                node.set("blockedBy", blockers(state, &key));
                node
            })
            .collect();
        object([("subIssues", object([("nodes", Array(nodes))]))])
    };
    let answer = object([("data", object([("repository", object([("issue", issue)]))]))]);
    println!("{answer}");
}

/// The Inbox's batched query: one `alias:
/// repository(owner: "owner", name: "repo") { issue/pullRequest(number: n) { ... }
/// }` entry per subject, answered alias by alias with the query's own
/// `rateLimit`.
fn inbox_states(state: &Json, query: &str) {
    let mut data: Vec<(String, Json)> = Vec::new();
    let mut errors = Vec::new();
    let mut rest = query;
    while let Some(at) = rest.find(": repository(") {
        let alias = rest[..at]
            .rsplit(|char: char| !(char.is_alphanumeric() || char == '_'))
            .next()
            .unwrap_or_default()
            .to_owned();
        rest = &rest[at + ": repository(".len()..];
        let owner = quoted_after(rest, "owner:");
        let name = quoted_after(rest, "name:");
        let subject = rest.split(": repository(").next().unwrap_or(rest);
        let field = if subject.contains("pullRequest(number:") {
            "pullRequest"
        } else {
            "issue"
        };
        let lookup = format!("{field}(number:");
        let number = subject
            .find(&lookup)
            .and_then(|at| {
                subject[at + lookup.len()..]
                    .trim_start()
                    .split(|char: char| !char.is_ascii_digit())
                    .next()
            })
            .unwrap_or_default();
        let same_repo = owner.is_some_and(|owner| {
            name.is_some_and(|name| {
                format!("{owner}/{name}").eq_ignore_ascii_case(state.at("repo").str())
            })
        });
        let item = same_repo
            .then(|| {
                if field == "issue" {
                    return state.at("issues").get(number).map(|issue_state| {
                        let labels = listed(state, "labels", number)
                            .into_iter()
                            .map(|label| object([("name", label)]))
                            .collect();
                        object([
                            ("state", issue_state.clone()),
                            (
                                "closedAt",
                                state
                                    .get("issue_closed_at")
                                    .and_then(|times| times.get(number))
                                    .cloned()
                                    .unwrap_or(Null),
                            ),
                            ("labels", object([("nodes", Array(labels))])),
                        ])
                    });
                }
                prs(state)
                    .iter()
                    .find(|pr| pr.at("number").python() == number)
                    .map(|pr| {
                        let at = |field: &str| pr.get(field).cloned().unwrap_or(Null);
                        object([
                            ("url", pr.at("url").clone()),
                            ("state", pr.at("state").clone()),
                            ("mergedAt", at("mergedAt")),
                            ("closedAt", at("closedAt")),
                        ])
                    })
            })
            .flatten();
        if item.is_none() {
            errors.push(object([
                ("type", string("NOT_FOUND")),
                ("path", Json::Array(vec![string(&alias), string(field)])),
                (
                    "message",
                    string(format!("Could not resolve {field} {number}.")),
                ),
            ]));
        }
        data.push((alias, object([(field, item.unwrap_or(Null))])));
    }
    if data.is_empty() {
        die("fake gh: batched pull request query names no repository", 2);
    }
    data.push((
        "rateLimit".to_owned(),
        state.get("inbox_rate_limit").cloned().unwrap_or_else(|| {
            object([
                ("limit", number(5000)),
                ("cost", number(1)),
                ("remaining", number(4999)),
                ("resetAt", string(now())),
            ])
        }),
    ));
    let mut answer = Json::Object(vec![("data".to_owned(), Json::Object(data))]);
    let failed = !errors.is_empty();
    if failed {
        answer.set("errors", Json::Array(errors));
    }
    println!("{answer}");
    if failed {
        die(
            "gh: Could not resolve a pull request in the batched query.",
            1,
        );
    }
}

/// The `"value"` after `key` in a GraphQL query, if any.
fn quoted_after<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query
        .find(key)
        .and_then(|at| query[at + key.len()..].trim_start().strip_prefix('"'))
        .and_then(|rest| rest.split('"').next())
}

/// The list `map` holds for issue `key`, empty if it holds none.
fn listed(state: &Json, map: &str, key: &str) -> Vec<Json> {
    state
        .get(map)
        .and_then(|map| map.get(key))
        .map(|list| list.items().to_vec())
        .unwrap_or_default()
}

/// Issue `n` as a GraphQL node with its number and state, open if the fake
/// does not know it.
fn numbered_issue(state: &Json, n: &Json) -> Json {
    let issue_state = state.at("issues").get(&n.python()).cloned();
    object([
        ("number", n.clone()),
        (
            "url",
            state
                .get("issue_urls")
                .and_then(|urls| urls.get(&n.python()))
                .cloned()
                .unwrap_or_else(|| {
                    string(format!(
                        "https://github.com/{}/issues/{}",
                        state.at("repo").str(),
                        n.python()
                    ))
                }),
        ),
        ("state", issue_state.unwrap_or(string("OPEN"))),
    ])
}

/// How many sub-issues issue `key` has, as GraphQL's `subIssues`.
fn sub_issue_count(state: &Json, key: &str) -> Json {
    object([("totalCount", number(listed(state, "sub_issues", key).len()))])
}

/// The issues issue `key` is blocked by, as GraphQL's `blockedBy`.
fn blockers(state: &Json, key: &str) -> Json {
    let blockers = listed(state, "blocked_by", key)
        .iter()
        .map(|blocker| numbered_issue(state, blocker))
        .collect();
    object([("nodes", Array(blockers))])
}

/// Issue `n`'s `kind` edits, oldest first, each at its `field` time, as the
/// edit queries read them: the page after the `after` cursor.
fn edit_history(state: &Json, n: &str, kind: &str, field: &str, after: Option<&str>) -> Json {
    let edits: Vec<Json> = listed(state, "edits", n)
        .into_iter()
        .filter(|edit| edit.at("kind").str() == kind)
        .map(|edit| {
            let actor = match edit.at("by") {
                Null => Null,
                login => object([("login", login.clone())]),
            };
            object([(field, edit.at("at").clone()), ("actor", actor)])
        })
        .collect();
    let start = after
        .map(|cursor| cursor.parse::<usize>().expect("edit cursor"))
        .unwrap_or(0);
    let end = (start + 100).min(edits.len());
    let page_info = object([
        ("hasNextPage", Bool(end < edits.len())),
        ("endCursor", string(end.to_string())),
    ]);
    object([(
        "edits",
        object([
            ("nodes", Array(edits[start..end].to_vec())),
            ("pageInfo", page_info),
        ]),
    )])
}

/// Issue `n`'s title, body and comments, as the session issue query reads
/// them, the comments in pages of 100 from the `after` cursor.
fn issue_comments(state: &Json, n: &str, after: Option<&str>) -> Json {
    let owner = state.at("repo").str().split('/').next().unwrap().to_owned();
    let comments: Vec<Json> = listed(state, "comments", n)
        .into_iter()
        .map(|comment| {
            let (body, by, association) = match &comment {
                Json::Object(_) => (
                    comment.at("body").clone(),
                    comment.at("by").clone(),
                    comment.at("association").clone(),
                ),
                _ => (comment.clone(), string(owner.clone()), string("OWNER")),
            };
            let author = match by {
                Null => Null,
                login => object([("login", login)]),
            };
            object([
                ("author", author),
                ("authorAssociation", association),
                ("createdAt", string("2020-01-01T00:00:00Z")),
                ("body", body),
            ])
        })
        .collect();
    let start = after
        .map(|cursor| cursor.parse::<usize>().expect("comment cursor"))
        .unwrap_or(0);
    let end = (start + 100).min(comments.len());
    let fields = issue_fields(state, n, state.at("issues").at(n));
    object([
        ("title", fields.at("title").clone()),
        ("body", fields.at("body").clone()),
        (
            "comments",
            object([
                ("nodes", Array(comments[start..end].to_vec())),
                (
                    "pageInfo",
                    object([
                        ("hasNextPage", Bool(end < comments.len())),
                        ("endCursor", string(end.to_string())),
                    ]),
                ),
            ]),
        ),
    ])
}

/// Issue `n` as the candidate query reads it.
fn candidate(state: &Json, n: &str, after: Option<&str>) -> Json {
    let parent = match state.get("sub_issues") {
        Some(Json::Object(specs)) => specs
            .iter()
            .find(|(_, tickets)| tickets.items().iter().any(|ticket| ticket.python() == n))
            .map(|(spec, _)| spec.as_str()),
        _ => None,
    };
    let parent = match parent {
        Some(spec) => {
            let url = format!(
                "https://github.com/{}/issues/{spec}",
                state.at("repo").str()
            );
            object([("url", string(url))])
        }
        None => Null,
    };
    let (mut labelled, mut linked) = (Vec::new(), None);
    for event in listed(state, "timeline", n) {
        let at = event.at("at").clone();
        let link = match event.at("event").str() {
            "labeled" | "unlabeled" => {
                let label = object([("name", event.at("label").clone())]);
                let kind = if event.at("event").str() == "labeled" {
                    "LabeledEvent"
                } else {
                    "UnlabeledEvent"
                };
                labelled.push(object([
                    ("__typename", string(kind)),
                    ("createdAt", at),
                    ("label", label),
                ]));
                continue;
            }
            "sub_issue_added" => "SubIssueAddedEvent",
            "sub_issue_removed" => "SubIssueRemovedEvent",
            "blocked_by_added" => "BlockedByAddedEvent",
            "blocked_by_removed" => "BlockedByRemovedEvent",
            other => die(&format!("fake gh: unsupported timeline event {other}"), 2),
        };
        linked = Some(object([("__typename", string(link)), ("createdAt", at)]));
    }
    let start = after
        .map(|cursor| cursor.parse::<usize>().expect("label cursor"))
        .unwrap_or(0);
    let end = (start + 100).min(labelled.len());
    let page_info = object([
        ("hasNextPage", Bool(end < labelled.len())),
        ("endCursor", string(end.to_string())),
    ]);
    let labelled = labelled[start..end].to_vec();
    let sub_issues = listed(state, "sub_issues", n)
        .iter()
        .map(|ticket| object([("state", numbered_issue(state, ticket).at("state").clone())]))
        .collect();
    object([
        ("parent", parent),
        ("subIssues", object([("nodes", Array(sub_issues))])),
        ("blockedBy", blockers(state, n)),
        (
            "labelled",
            object([("nodes", Array(labelled)), ("pageInfo", page_info)]),
        ),
        (
            "linked",
            object([("nodes", Array(linked.into_iter().collect()))]),
        ),
    ])
}

/// Run the `on-ci-read` script, if it has runs left and has not yet run for
/// `sha`, e.g. to move the Base branch while thirdshift waits for CI. Leaves
/// `state` as the script left it.
fn run_ci_read_hook(state: &mut Json, sha: &str) {
    let Some(hook) = state.get_mut("on_ci_read").filter(|hook| hook.truthy()) else {
        return;
    };
    let seen = hook.at_mut("seen").items_mut();
    if seen.iter().any(|seen| seen.as_str() == Some(sha)) {
        return;
    }
    seen.push(string(sha));
    run_hook(state, "on_ci_read", &[("FAKE_CI_SHA", sha)]);
}

/// Run the `on-issue-view` script of issue `n`, if it has one that has not
/// yet run, e.g. to close the issue as thirdshift waits on it. Leaves `state`
/// as the script left it. An object hook limits it to matching `--json`
/// fields, so an unrelated title or state read does not consume it.
fn run_issue_view_hook(state: &mut Json, n: &str, fields: &str) {
    let Some(hook) = state
        .get_mut("on_issue_view")
        .and_then(|hooks| hooks.get_mut(n))
        .filter(|hook| hook.truthy())
    else {
        return;
    };
    if hook
        .get("fields")
        .is_some_and(|wanted| wanted.str() != fields)
    {
        return;
    }
    let script = hook.get("script").unwrap_or(hook).str().to_owned();
    *hook = Null;
    save(state);
    unlock();
    let succeeded = run_script(&script, &[]);
    lock();
    assert!(succeeded, "the on-issue-view script failed");
    *state = load();
}

/// Run the script of hook `name`, if it has runs left, with `env` added to
/// the environment. Leaves `state` as the script left it.
fn run_hook(state: &mut Json, name: &str, env: &[(&str, &str)]) {
    let Some(hook) = state.get_mut(name).filter(|hook| hook.truthy()) else {
        return;
    };
    let times = hook.at("times").as_i64().unwrap();
    if times == 0 {
        return;
    }
    hook.set("times", number(times - 1));
    let script = hook.at("script").str().to_owned();
    save(state);
    unlock();
    let succeeded = run_script(&script, env);
    lock();
    assert!(succeeded, "the {name} script failed");
    *state = load();
}

/// Run `script` with `bash -euc`, its stdout going to stderr so it can't mix
/// with what gh prints, and `env` added to the environment.
fn run_script(script: &str, env: &[(&str, &str)]) -> bool {
    let stderr = std::io::stderr().as_fd().try_clone_to_owned().unwrap();
    Command::new("bash")
        .args(["-euc", script])
        .envs(env.iter().copied())
        .stdout(Stdio::from(stderr))
        .status()
        .unwrap()
        .success()
}

fn parse_json(text: &str) -> Json {
    json::parse(text).unwrap_or_else(|error| panic!("{error}: {text}"))
}

/// `gh fake …`, through which the tests set up the state: checks, runs,
/// hooks and failing commands.
fn fake_command(state: &mut Json, args: &[&str]) {
    let times = |text: &str| number(text.parse::<i64>().unwrap());
    match args {
        ["checks", sha, list] => set_checks(state, sha, parse_json(list)),
        ["statuses", sha, list] => {
            state
                .entry("statuses", object([]))
                .set(sha, parse_json(list));
        }
        ["run", run] => record_run(state, parse_json(run)),
        ["on-ci-read", n, script] => state.set(
            "on_ci_read",
            object([
                ("times", times(n)),
                ("script", string(*script)),
                ("seen", Array(Vec::new())),
            ]),
        ),
        ["on-issue-view", n, script] => state
            .entry("on_issue_view", object([]))
            .set(n, string(*script)),
        ["on-merge", n, script] => state.set(
            "on_merge",
            object([("times", times(n)), ("script", string(*script))]),
        ),
        ["refuse-merges", n, error] => state.set(
            "refuse_merges",
            object([("times", times(n)), ("error", string(*error))]),
        ),
        ["after-merge", script] => state.set("after_merge", string(*script)),
        ["issue", n, issue_state] => state.at_mut("issues").set(n, string(*issue_state)),
        ["labels", n, labels] => state.entry("labels", object([])).set(n, parse_json(labels)),
        ["created", n, time] => state.entry("created", object([])).set(n, string(*time)),
        ["sub-issues", n, tickets] => state
            .entry("sub_issues", object([]))
            .set(n, parse_json(tickets)),
        ["repo-labels", labels] => state.set("repo_labels", parse_json(labels)),
        ["timeline", n, events] => state
            .entry("timeline", object([]))
            .set(n, parse_json(events)),
        ["edits", n, edits] => state.entry("edits", object([])).set(n, parse_json(edits)),
        ["user-email", email] => state.set("user_email", parse_json(email)),
        ["fails", call] => state
            .entry("failing", Array(Vec::new()))
            .items_mut()
            .push(string(*call)),
        ["pr", head, field, value] => {
            let at = newest_pr_from(state, head);
            state.at_mut("prs").items_mut()[at].set(field, parse_json(value));
        }
        _ => die(
            &format!("fake gh: unsupported fake command {}", python_list(args)),
            2,
        ),
    }
    save(state);
}

pub fn main(args: Vec<String>) {
    // Opt-in authentication at the external GitHub seam. Most fixtures do
    // not model credentials; publishing-authority tests explicitly do.
    if std::env::var_os("FAKE_GH_REQUIRE_AUTH").is_some() {
        let token = [
            "GH_TOKEN",
            "GITHUB_TOKEN",
            "GH_ENTERPRISE_TOKEN",
            "GITHUB_ENTERPRISE_TOKEN",
        ]
        .iter()
        .any(|name| std::env::var(name).is_ok_and(|value| !value.is_empty()));
        let config = std::env::var_os("GH_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| crate::env_path("HOME").join(".config/gh"));
        let stored = fs::read_to_string(config.join("hosts.yml"))
            .is_ok_and(|text| text.contains("oauth_token:"));
        if !token && !stored {
            die("gh: authentication required", 1);
        }
    }
    lock();
    record(&args);
    let mut state = load();
    let failing = state.get("failing").map(Json::items).unwrap_or_default();
    let starts_with = |call: &str| {
        let words: Vec<&str> = call.split(' ').collect();
        args.iter().map(String::as_str).take(words.len()).eq(words)
    };
    if let Some(failure) = state.get("fail_next")
        && starts_with(failure.at("call").str())
        && failure.at("times").as_i64().unwrap() > 0
    {
        let left = failure.at("times").as_i64().unwrap() - 1;
        let error = failure.at("error").str().to_owned();
        state.at_mut("fail_next").set("times", number(left));
        save(&state);
        die(&error, 1);
    }
    if failing
        .iter()
        .any(|failing| failing.as_str().is_some_and(starts_with))
    {
        die(
            state
                .get("failing_stderr")
                .and_then(Json::as_str)
                .unwrap_or("HTTP 502: Bad Gateway (https://api.github.com/graphql)"),
            1,
        );
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let parsed = |rest: &[&str]| {
        let (positional, flags) = parse(rest);
        check_repo(&state, &flags);
        (positional, flags)
    };
    match args.as_slice() {
        ["pr", "create", rest @ ..] => {
            let (_, flags) = parsed(rest);
            pr_create(&mut state, &flags);
        }
        ["pr", "view", rest @ ..] => {
            let (positional, flags) = parsed(rest);
            pr_view(&mut state, &positional, &flags);
        }
        ["pr", "list", rest @ ..] => {
            let (_, flags) = parsed(rest);
            pr_list(&state, &flags);
        }
        ["pr", "ready", rest @ ..] => {
            let (positional, flags) = parsed(rest);
            pr_ready(&mut state, &positional, &flags);
        }
        ["pr", "merge", rest @ ..] => {
            let (positional, flags) = parsed(rest);
            pr_merge(&mut state, &positional, &flags);
        }
        ["issue", "view", rest @ ..] => {
            let (positional, flags) = parsed(rest);
            issue_view(&mut state, &positional, &flags);
        }
        ["issue", "list", rest @ ..] => {
            let (_, flags) = parsed(rest);
            issue_list(&state, &flags);
        }
        ["issue", "close", rest @ ..] => {
            let (positional, flags) = parsed(rest);
            issue_close(&mut state, &positional, &flags);
        }
        ["issue", "comment", rest @ ..] => {
            let (positional, flags) = parsed(rest);
            issue_comment(&mut state, &positional, &flags);
        }
        ["issue", "create", rest @ ..] => {
            let (_, flags) = parsed(rest);
            issue_create(&mut state, &flags);
        }
        ["label", "list", rest @ ..] => {
            let (_, flags) = parsed(rest);
            label_list(&state, &flags);
        }
        ["label", "create", rest @ ..] => {
            let (positional, _) = parsed(rest);
            label_create(&mut state, &positional);
        }
        ["run", "list", rest @ ..] => {
            let (_, flags) = parsed(rest);
            run_list(&mut state, &flags);
        }
        ["run", "rerun", rest @ ..] => {
            let (positional, flags) = parsed(rest);
            run_rerun(&mut state, &positional, &flags);
        }
        ["run", "view", rest @ ..] => {
            let (positional, flags) = parsed(rest);
            run_view(&state, &positional, &flags);
        }
        ["release", "view", rest @ ..] => {
            let (positional, flags) = parsed(rest);
            release_view(&state, &positional, &flags);
        }
        ["api", "graphql", rest @ ..] => graphql(&state, rest),
        ["api", "--method", "PATCH", rest @ ..]
            if rest.first().is_some_and(|path| {
                repo_prefix(path).is_some_and(|(_, path)| {
                    path.strip_prefix("issues/")
                        .is_some_and(|number| number.parse::<u64>().is_ok())
                })
            }) =>
        {
            finding_issue_patch(&mut state, rest)
        }
        ["api", "--method", "PATCH", rest @ ..]
            if !rest
                .first()
                .is_some_and(|path| path.contains("/security-advisories")) =>
        {
            pr_patch(&mut state, rest)
        }
        ["api", "--method", "PUT", rest @ ..] => issue_labels_put(&mut state, rest),
        ["api", "--method", "DELETE", rest @ ..] => issue_label_delete(&mut state, rest),
        ["api", "--method", "POST", rest @ ..]
            if rest
                .first()
                .is_some_and(|path| path.ends_with("/sub_issues")) =>
        {
            api_add_sub_issue(&mut state, rest)
        }
        ["api", "--method", "POST", rest @ ..]
            if rest
                .first()
                .is_some_and(|path| path.contains("/dependencies/blocked_by")) =>
        {
            api_add_blocked_by(&mut state, rest)
        }
        ["api", path, ..] if generate_notes_repo(path).is_some() => {
            generate_notes(&state, &args[1..]);
        }
        ["api", rest @ ..] => {
            let (positional, flags) = parse(rest);
            api(&mut state, &positional, &flags);
        }
        ["fake", rest @ ..] => fake_command(&mut state, rest),
        _ => die(
            &format!("fake gh: unsupported command: {}", python_list(&args)),
            2,
        ),
    }
}
