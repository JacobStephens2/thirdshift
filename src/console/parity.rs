//! The CLI inventory is read from its enums, independently of the route table.
use std::collections::BTreeSet;
use std::mem::{Discriminant, discriminant};

use crate::args::Command;

const COMMANDS: &[(&str, &str)] = &[
    ("FleetControl", "start --options --json"),
    ("Help", "help"),
    ("Version", "version"),
    ("Update", "update"),
    ("Setup", "setup"),
    ("Config", "config"),
    ("Score", "score"),
    ("Scores", "scores"),
    ("BugOrigins", "bug-origins"),
    ("Evals", "evals"),
    ("Experiment", "experiment HEAD"),
    ("Experiments", "experiments"),
    ("Recheck", "recheck id --pending --json"),
    ("CronJobs", "cronjobs"),
    ("LaunchDirectories", "launch-directories"),
    ("Resources", "resources"),
    ("Console", "console"),
    ("ConsoleInstall", "console install"),
    ("ConsolePassword", "console password"),
    ("ConsoleSignOut", "console sign-out"),
    ("ConsoleLog", "console log"),
    ("ConsoleToken", "console token list"),
    ("ConsoleFleet", "console fleet list"),
    ("ConsoleReport", "console report list"),
    ("ConsoleScopes", "console scopes"),
    ("ConsoleWhoami", "console whoami"),
    ("ConsoleOpenApi", "console openapi"),
    ("ConsoleTheme", "console theme list"),
    ("ConsoleThemeImport", "console theme import theme.json"),
    ("ConsoleOtherAccounts", "console other-accounts"),
    ("ConsoleThemeExport", "console theme export focus-auto"),
    ("ConsoleSections", "console sections"),
    ("Status", "status"),
    ("Show", "show widgets#7"),
    (
        "RemoteDetails",
        "show widgets#7 --machine fleet-parity.example.invalid",
    ),
    ("Metrics", "metrics"),
    ("Spans", "show widgets#7 --spans"),
    ("Stop", "stop widgets#7"),
    ("Dismiss", "dismiss widgets#7"),
    ("Logs", "logs widgets#7"),
    ("LogsList", "logs"),
    ("LogsExport", "logs widgets#7 --export output"),
    ("LogFile", "logs --file id"),
    ("ReleaseSummary", "--release-summary"),
    ("Release", "release 1.2.3"),
    ("Skill", "skill to-spec"),
    ("Process", "process list"),
    ("Review", "review https://github.com/acme/widgets/pull/1"),
    ("Fix", "fix https://github.com/acme/widgets/pull/1"),
    ("Trial", "--trial request.json"),
    ("OomExec", "--oom-exec true"),
    ("EmailTest", "email-test"),
    ("Architect", "architect"),
    ("Pickup", "pickup"),
    ("Secure", "secure"),
    ("Triage", "triage"),
    ("Run", "https://github.com/acme/widgets/issues/7"),
];

const PROCESSES: &[(&str, &str)] = &[
    ("List", "process list"),
    ("Show", "process show inspect"),
    ("Check", "process check inspect"),
    ("Run", "process inspect issue 7"),
];

const EVALS: &[(&str, &str)] = &[
    ("List", "evals list"),
    ("ImportReplays", "evals import-replays"),
    ("ProposeProcess", "evals propose-process repository"),
    ("Approve", "evals approve suite task"),
    ("Touched", "evals touched src/main.rs"),
    ("Prepare", "evals prepare suite task output"),
    (
        "Verify",
        "evals verify repository 0123456789012345678901234567890123456789",
    ),
    (
        "PrepareArms",
        "evals prepare-arms repository 0123456789012345678901234567890123456789 codex 0123456789012345678901234567890123456789 codex record.json",
    ),
];

const FLEET: &[(&str, &str)] = &[
    ("Add", "console fleet add other https://other.example.com"),
    ("List", "console fleet list"),
    ("Remove", "console fleet remove other"),
];

const REPORT: &[(&str, &str)] = &[
    (
        "Add",
        "console report add console https://console.example.com",
    ),
    ("List", "console report list"),
    ("Remove", "console report remove console"),
];

// Distinct actions represented by fields of a CLI command, rather than enums.
const ACTIONS: &[&str] = &[
    "cronjobs --fleet",
    "metrics --fleet",
    "metrics --machine other",
    "metrics --push",
    "update --check",
    "config spec.parallel",
    "config spec.parallel 3",
    "scores 23 --mine",
    "bug-origins 8 --confirm",
    "bug-origins 8 --correct 24",
    "bug-origins 8 --none",
    "dismiss widgets#7 --undo",
    "console theme focus-auto",
    "console sections --hide done",
    "console other-accounts readable",
    "experiments id --name name",
    "experiments id --record-difficulty",
    "experiments id --session session",
    "recheck id trial pass --reason reviewed --json",
    "recheck id --batch fingerprint fail --reason reviewed --json",
];

// Keep these reasons and ADR 0026's CLI-only list together. A prefix is never
// an exclusion: each entry is parsed and covers only its command/action.
const EXCLUSIONS: &[(&str, &str)] = &[
    (
        "release --review 1.2.3",
        "Interactive release summary review opens an editor on a local terminal (#2054 Q13, Q15).",
    ),
    (
        "cronjobs --fleet",
        "Fleet schedule aggregation is an explicit client of owning-machine inventories; the API stays local and cannot lend saved Fleet credentials.",
    ),
    (
        "metrics --fleet",
        "Fleet metrics aggregation is an explicit client of owning-machine metrics and cached peer snapshots; the API stays local and cannot lend saved Fleet credentials.",
    ),
    (
        "metrics --push",
        "Outbound metrics reporting sends this machine's local summary to a configured Console using saved credentials; the receiving Console exposes the report endpoint directly.",
    ),
    (
        "console report add console https://console.example.com",
        "Report destination pairing stays in the shell so the API cannot change where this machine's metrics go or the tokens they carry.",
    ),
    (
        "console report list",
        "Report destinations are inspected through the shell alongside pairing and removal.",
    ),
    (
        "console report remove console",
        "Report destination removal stays in the shell alongside pairing and its Credentials changes.",
    ),
    (
        "console token list",
        "Token management stays in the shell so the API cannot widen its own access.",
    ),
    (
        "console scopes",
        "Address scopes stay in the shell so the API cannot widen its own access.",
    ),
    (
        "console fleet add other https://other.example.com",
        "Fleet pairing stays in the shell so the API cannot change trusted destinations or their tokens.",
    ),
    (
        "console fleet list",
        "Fleet pairing metadata is inspected through the shell alongside pairing and removal.",
    ),
    (
        "console fleet remove other",
        "Fleet removal stays in the shell alongside pairing and its Credentials changes.",
    ),
    (
        "show widgets#7 --machine fleet-parity.example.invalid",
        "Fleet detail forwarding uses the owning machine's API directly; API reads stay local and never relay through saved Fleet tokens.",
    ),
    (
        "console sign-out",
        "Console sign-in administration, including revoking sign-ins, stays in the shell.",
    ),
    (
        "console install",
        "Console installation writes the machine's proxy and service setup.",
    ),
    (
        "setup",
        "Interactive setup bootstraps the machine and its Console sign-in list.",
    ),
    (
        "console password",
        "Console passwords require the machine's shell or the separate sign-in flow.",
    ),
    (
        "logs widgets#7 --export output",
        "Log export writes to a caller-selected local path.",
    ),
    (
        "evals prepare suite task output",
        "Eval preparation writes a repository to a caller-selected local path.",
    ),
    (
        "evals prepare-arms repository 0123456789012345678901234567890123456789 codex 0123456789012345678901234567890123456789 codex record.json",
        "Arm preparation writes its record to a caller-selected local path.",
    ),
    (
        "console",
        "Console serving bootstraps the API listener itself; it is not a client operation.",
    ),
    (
        "--trial request.json",
        "The hidden Trial protocol consumes a local request and writes local Trial artifacts.",
    ),
    (
        "--oom-exec true",
        "The hidden OOM wrapper replaces its process inside an existing Trial scope.",
    ),
    (
        "--release-summary",
        "The hidden release-summary protocol belongs to the local release script's stdin exchange.",
    ),
];

fn parse(words: &str) -> Command {
    crate::args::parse(
        &words
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|error| panic!("{words}: {error}"))
}

// Identity ignores display/selection arguments, but retains different actions
// sharing one variant. API metadata cannot cover a writer with a read example.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "the independently parsed enum inventory fails on new variants; only multi-action variants need a suffix here"
)]
fn identity(words: &str) -> (Discriminant<Command>, String) {
    let command = parse(words);
    let action = match &command {
        Command::Process(action) => format!("{:?}", discriminant(action)),
        Command::Evals(action) => format!("{:?}", discriminant(action)),
        Command::ConsoleFleet(action) => format!("{:?}", discriminant(action)),
        Command::ConsoleReport(action) => format!("{:?}", discriminant(action)),
        Command::Update { check, .. } => check.to_string(),
        Command::Release(args) => args.review.to_string(),
        Command::Config { key, value, .. } => format!("{}:{}", key.is_some(), value.is_some()),
        Command::CronJobs { fleet, .. } => fleet.to_string(),
        // `--machine` reads one Fleet machine through the same client;
        // `--push` sends this machine's own summary out.
        Command::Metrics(asked) => {
            if asked.push {
                "push".to_owned()
            } else {
                (asked.fleet || asked.machine.is_some()).to_string()
            }
        }
        Command::ConsoleOtherAccounts { value, .. } => value.is_some().to_string(),
        Command::ConsoleTheme { name, .. } => name.is_some().to_string(),
        Command::ConsoleSections { hide, show, .. } => {
            (!hide.is_empty() || !show.is_empty()).to_string()
        }
        Command::Scores { mine, .. } => mine.to_string(),
        Command::BugOrigins(_) => words
            .split_whitespace()
            .find(|word| matches!(*word, "--confirm" | "--correct" | "--none"))
            .unwrap_or("read")
            .into(),
        Command::Dismiss { undo, .. } => undo.to_string(),
        Command::Experiments(args) => [
            "--name",
            "--record-difficulty",
            "--session",
            "--merge",
            "--prefer",
            "--pair",
        ]
        .into_iter()
        .find(|flag| args.iter().any(|arg| arg == flag))
        .unwrap_or("read")
        .into(),
        Command::Recheck(args) => if args.iter().any(|arg| arg == "--pending") {
            "read"
        } else if args.iter().any(|arg| arg == "--batch") {
            "batch"
        } else {
            "write"
        }
        .into(),
        _ => String::new(),
    };
    (discriminant(&command), action)
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "only the named enum is relevant among syn's non-exhaustive syntax items"
)]
fn inventory(source: &str, name: &str, examples: &[(&str, &str)]) {
    let file = syn::parse_file(source).unwrap();
    let enumeration = file
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Enum(item) if item.ident == name => Some(item),
            _ => None,
        })
        .unwrap();
    let variants: BTreeSet<_> = enumeration
        .variants
        .iter()
        .map(|v| v.ident.to_string())
        .collect();
    assert_eq!(
        variants,
        examples.iter().map(|(name, _)| name.to_string()).collect(),
        "CLI inventory for {name} needs updating"
    );
    let identities: BTreeSet<_> = examples
        .iter()
        .map(|(_, words)| format!("{:?}", identity(words)))
        .collect();
    assert_eq!(
        identities.len(),
        examples.len(),
        "inventory examples must exercise distinct {name} actions"
    );
}

fn assert_covered(
    commands: impl IntoIterator<Item = String>,
    twins: impl IntoIterator<Item = String>,
) {
    let twins: Vec<_> = twins.into_iter().map(|words| identity(&words)).collect();
    let exclusions: Vec<_> = EXCLUSIONS
        .iter()
        .map(|(words, reason)| {
            assert!(
                !reason.trim().is_empty(),
                "{words} needs an exclusion reason"
            );
            identity(words)
        })
        .collect();
    let missing: Vec<_> = commands
        .into_iter()
        .filter(|words| {
            let key = identity(words);
            !twins.contains(&key) && !exclusions.contains(&key)
        })
        .collect();
    assert!(
        missing.is_empty(),
        "CLI commands have neither an API twin nor a reasoned exclusion: {missing:?}"
    );
}

#[test]
fn every_cli_command_has_an_api_twin_or_reasoned_exclusion() {
    inventory(include_str!("../args.rs"), "Command", COMMANDS);
    inventory(include_str!("../args.rs"), "ProcessAction", PROCESSES);
    inventory(include_str!("../evals.rs"), "Action", EVALS);
    inventory(include_str!("fleet.rs"), "Action", FLEET);
    inventory(include_str!("report.rs"), "Action", REPORT);
    let description = super::openapi::document();
    let twins = description["paths"]
        .as_object()
        .unwrap()
        .values()
        .flat_map(|methods| methods.as_object().unwrap().values())
        .flat_map(|operation| operation["x-cli-twins"].as_array().unwrap())
        .map(|twin| {
            twin.as_str()
                .unwrap()
                .strip_prefix("thirdshift ")
                .unwrap()
                .to_owned()
        });
    assert_covered(
        COMMANDS
            .iter()
            .chain(PROCESSES)
            .chain(EVALS)
            .chain(FLEET)
            .chain(REPORT)
            .map(|(_, words)| words.to_string())
            .chain(ACTIONS.iter().map(|s| s.to_string()))
            .chain(["release --review 1.2.3".to_owned()]),
        twins,
    );
}

#[test]
#[should_panic(expected = "neither an API twin nor a reasoned exclusion")]
fn parity_rejects_a_cli_command_without_a_twin_or_exclusion() {
    assert_covered(["status".into()], []);
}

#[test]
fn adr_lists_every_cli_only_reason() {
    let adr = include_str!(
        "../../docs/adr/0026-each-console-serves-an-api-that-can-do-what-the-cli-can.md"
    );
    for (_, reason) in EXCLUSIONS {
        assert!(adr.contains(reason), "ADR 0026 is missing: {reason}");
    }
}
