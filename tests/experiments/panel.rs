//! The Grader panel: each seat reads every pair of process Trials blind in
//! both orders, and its calls and the panel's vote go beside the Verdict, or
//! count in it, with the people's, when the Bar says who decides.
use super::*;
use std::path::Path;

/// Panel Graders by seat. `candidate` and `baseline` judge consistently:
/// they prefer, and call ready, the side whose diff does or doesn't add the
/// Candidate's `extra.txt`. `first` always prefers side A, so it flips.
fn panel_graders(fixture: &Fixture, seats: &[&str]) {
    let behaviours: Value = seats
        .iter()
        .enumerate()
        .map(|(index, behaviour)| (format!("{}", index + 1), json!(behaviour)))
        .collect::<serde_json::Map<_, _>>()
        .into();
    fs::write(
        fixture.scenario.path("claude-script.sh.grader"),
        format!(
            r#"
python3 - <<'PY'
import json, os, re
behaviours = {behaviours}
seat = re.search(r'--seat-(\d+)--order-', os.getcwd()).group(1)
behaviour = behaviours[seat]
packet = json.load(open(os.environ['THIRDSHIFT_GRADER_PACKET']))
def chosen(side):
    if behaviour == 'first':
        return side['label'] == 'A'
    extra = 'extra.txt' in (side['diff'] or '')
    return extra if behaviour == 'candidate' else not extra
sides = [dict(label=side['label'], rubric=[dict(criterion=c['name'], met=chosen(side), evidence='Seen in the diff.') for c in packet['rubric']], merge_reason='Read the diff.', ready_to_merge=chosen(side)) for side in packet['sides']]
preferred = [side['label'] for side in packet['sides'] if chosen(side)][0]
json.dump(dict(sides=sides, reason='Compared both diffs.', preference=preferred), open(os.environ['THIRDSHIFT_GRADER_CALLS'], 'w'))
PY
"#
        ),
    )
    .unwrap();
}

const SEATS: [&str; 6] = [
    "--panel-seat",
    "harness=claude,model=judge-one",
    "--panel-seat",
    "harness=claude,model=judge-two",
    "--panel-seat",
    "harness=codex,model=judge-three,effort=high",
];

fn panel_pair(view: &Value) -> &Value {
    let panel = view["panel"].as_array().unwrap();
    assert_eq!(panel.len(), 1, "{view}");
    &panel[0]
}

fn preferences(pair: &Value) -> Vec<(String, String, String, bool)> {
    pair["graders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|grader| {
            (
                grader["orders"][0]["preference"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                grader["orders"][1]["preference"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                grader["preference"].as_str().unwrap().to_owned(),
                grader["votes"].as_bool().unwrap(),
            )
        })
        .collect()
}

#[test]
fn a_three_seat_panel_reports_each_graders_calls_in_both_orders_and_its_majority() {
    let fixture = super::process::fixture();
    super::pairs::pair_fixture(&fixture, super::pairs::CANDIDATE_TEST);
    panel_graders(&fixture, &["candidate", "candidate", "first"]);
    let mut args = SEATS.to_vec();
    args.extend(["--panel-vote", "model", "--panel-arm-family-rule", "off"]);
    let run = fixture.run(&args);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    super::result_format::assert_documented("Experiment", &view);
    assert_eq!(view["bar"]["panel"]["vote"], "model", "{}", view["bar"]);
    let pair = panel_pair(&view);
    assert_eq!(
        (&pair["suite"], &pair["task"], &pair["trial"]),
        (&json!("process"), &json!("fixture"), &json!(0))
    );
    let candidate = || "candidate".to_owned();
    assert_eq!(
        preferences(pair),
        [
            (candidate(), candidate(), candidate(), true),
            (candidate(), candidate(), candidate(), true),
            // Always side A: the Baseline in one order, the Candidate in the
            // other, so its call flips with the order and is a tie.
            (
                pair["graders"][2]["orders"][0]["preference"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                pair["graders"][2]["orders"][1]["preference"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                "tie".to_owned(),
                true
            ),
        ],
        "{pair}"
    );
    assert_ne!(
        pair["graders"][2]["orders"][0]["preference"],
        pair["graders"][2]["orders"][1]["preference"]
    );
    assert_eq!(
        pair["graders"][2]["merge"],
        json!({"baseline": "tie", "candidate": "tie"})
    );
    assert_eq!(pair["graders"][2]["family"], "openai");
    assert_eq!(pair["graders"][0]["family"], "anthropic");
    let order = &pair["graders"][0]["orders"][0];
    assert_eq!(order["candidate"]["ready_to_merge"], true, "{order}");
    assert_eq!(order["baseline"]["ready_to_merge"], false, "{order}");
    assert_eq!(order["reason"], "Compared both diffs.");
    assert_eq!(order["candidate"]["rubric"].as_array().unwrap().len(), 4);
    assert_ne!(
        pair["graders"][0]["orders"][0]["shown_as_a"],
        pair["graders"][0]["orders"][1]["shown_as_a"],
        "the second order swaps the sides"
    );
    // Per Model, the two same-family seats are two of three votes.
    assert_eq!(pair["votes"].as_array().unwrap().len(), 3, "{pair}");
    assert_eq!(pair["preference"], "candidate", "{pair}");
    assert_eq!(
        pair["merge"],
        json!({"baseline": "not_ready", "candidate": "ready"})
    );
    // The pair carries the panel's result, for the blind page's reveal and
    // people–panel agreement.
    assert_eq!(
        view["pairs"][0]["panel"],
        json!({"baseline_ready": false, "candidate_ready": true, "preference": "candidate"})
    );

    // Each seat read a blind packet in each order, with the sides swapped.
    let packets: Vec<Value> = fixture
        .grader_calls()
        .iter()
        .map(|call| {
            serde_json::from_slice(
                &fs::read(PathBuf::from(call["cwd"].as_str().unwrap()).join("packet.json"))
                    .unwrap(),
            )
            .unwrap()
        })
        .collect();
    assert_eq!(packets.len(), 6);
    let mut by_order = [0, 0];
    for packet in &packets {
        by_order[packet["order"].as_u64().unwrap() as usize - 1] += 1;
        assert_eq!(
            packet["ticket"],
            "# Fixture Spec\n\nReview this invented change.\n"
        );
        assert_eq!(packet["criteria"], "Report the invented finding.");
        assert_eq!(packet["rubric"].as_array().unwrap().len(), 4);
        let labels: Vec<_> = packet["sides"]
            .as_array()
            .unwrap()
            .iter()
            .map(|side| side["label"].as_str().unwrap())
            .collect();
        assert_eq!(labels, ["A", "B"]);
        for side in packet["sides"].as_array().unwrap() {
            assert!(side["diff"].as_str().unwrap().contains("+implemented"));
            assert_eq!(side["evidence"]["fails_on_base"], true, "{side}");
            assert!(side["evidence"].get("error").is_none(), "{side}");
        }
        let text = packet.to_string();
        for identifying in ["--baseline--", "--candidate--", "harnesses/", "judge-"] {
            assert!(!text.contains(identifying), "{identifying}: {text}");
        }
    }
    assert_eq!(by_order, [3, 3]);
    let extra_side = |packet: &Value| {
        packet["sides"]
            .as_array()
            .unwrap()
            .iter()
            .position(|side| side["diff"].as_str().unwrap().contains("extra.txt"))
    };
    let first = packets.iter().find(|packet| packet["order"] == 1).unwrap();
    let second = packets.iter().find(|packet| packet["order"] == 2).unwrap();
    assert_eq!(
        extra_side(first).map(|at| 1 - at),
        extra_side(second),
        "the second order shows the sides the other way round"
    );

    let shown = fixture
        .scenario
        .run_with_env(&["experiments", view["id"].as_str().unwrap()], &NO_SANDBOX);
    assert!(
        shown.stdout.contains(
            "Panel process/fixture #1 · prefers Candidate · Baseline not ready, Candidate ready"
        ),
        "{}",
        shown.stdout
    );
    assert!(
        shown
            .stdout
            .contains("Panel process/fixture #1 · codex/judge-three (openai): "),
        "{}",
        shown.stdout
    );

    // The panel sits beside the Verdict: the same Experiment without it
    // reaches the same Verdict.
    let without = super::process::fixture();
    super::pairs::pair_fixture(&without, super::pairs::CANDIDATE_TEST);
    let run = without.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let plain: Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(plain["panel"], json!([]));
    assert!(plain["bar"].get("panel").is_none(), "{}", plain["bar"]);
    // Only the measured times, and the line naming them, differ between runs.
    for field in ["verdict", "provisional_verdict", "suites"] {
        assert_eq!(view["verdict"][field], plain["verdict"][field], "{field}");
    }
    let scores = |view: &Value| -> Vec<Value> {
        view["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|result| {
                let score = &result["score"];
                json!([score["passed"], score["wrong_finding"], score["borderline"]])
            })
            .collect()
    };
    assert_eq!(scores(&view), scores(&plain));
}

#[test]
fn seats_from_an_arms_family_are_reported_but_do_not_vote() {
    let fixture = super::process::fixture();
    super::pairs::pair_fixture(&fixture, super::pairs::CANDIDATE_TEST);
    panel_graders(&fixture, &["candidate", "candidate", "baseline"]);
    let run = fixture.run(&SEATS);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(view["bar"]["panel"]["vote"], "family", "{}", view["bar"]);
    assert_eq!(view["bar"]["panel"]["arm_family_rule"], true);
    let pair = panel_pair(&view);
    // Both Arms run on Claude: its seats are reported and the other family
    // decides alone.
    let votes: Vec<_> = pair["graders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|grader| (grader["preference"].clone(), grader["votes"].clone()))
        .collect();
    assert_eq!(
        votes,
        [
            (json!("candidate"), json!(false)),
            (json!("candidate"), json!(false)),
            (json!("baseline"), json!(true)),
        ],
        "{pair}"
    );
    assert_eq!(pair["votes"].as_array().unwrap().len(), 1, "{pair}");
    assert_eq!(pair["votes"][0]["name"], "openai", "{pair}");
    assert_eq!(pair["preference"], "baseline", "{pair}");
    assert_eq!(
        pair["merge"],
        json!({"baseline": "ready", "candidate": "not_ready"})
    );
    let shown = fixture
        .scenario
        .run_with_env(&["experiments", view["id"].as_str().unwrap()], &NO_SANDBOX);
    assert!(
        shown
            .stdout
            .contains("Panel process/fixture #1 · claude/judge-one (anthropic, not voting): "),
        "{}",
        shown.stdout
    );
}

#[test]
fn with_the_rule_off_two_same_family_seats_are_one_vote_per_family() {
    let fixture = super::process::fixture();
    super::pairs::pair_fixture(&fixture, super::pairs::CANDIDATE_TEST);
    panel_graders(&fixture, &["candidate", "candidate", "baseline"]);
    let mut args = SEATS.to_vec();
    args.extend(["--panel-arm-family-rule", "off"]);
    let run = fixture.run(&args);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    let pair = panel_pair(&view);
    assert!(
        pair["graders"]
            .as_array()
            .unwrap()
            .iter()
            .all(|grader| grader["votes"] == true),
        "{pair}"
    );
    // Anthropic's two seats are one vote against OpenAI's one: a tie, and a
    // tied merge call is not ready for either Arm.
    let units: Vec<_> = pair["votes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|unit| (unit["name"].clone(), unit["preference"].clone()))
        .collect();
    assert_eq!(
        units,
        [
            (json!("anthropic"), json!("candidate")),
            (json!("openai"), json!("baseline"))
        ]
    );
    assert_eq!(pair["preference"], "tie", "{pair}");
    assert_eq!(
        pair["merge"],
        json!({"baseline": "not_ready", "candidate": "not_ready"})
    );
}

#[test]
fn panel_settings_need_a_seat_and_named_values() {
    let fixture = super::process::fixture();
    for (args, error) in [
        (
            vec!["--panel-vote", "model"],
            "need at least one --panel-seat",
        ),
        (
            vec!["--panel-seat", "harness=claude"],
            "invalid --panel-seat",
        ),
        (
            vec![
                "--panel-seat",
                "harness=claude,model=a",
                "--panel-vote",
                "seat",
            ],
            "invalid --panel-vote seat",
        ),
        (
            vec![
                "--panel-seat",
                "harness=claude,model=a",
                "--panel-seat",
                "harness=claude,model=a",
            ],
            "repeated panel seat",
        ),
    ] {
        let run = fixture.run(&args);
        assert_eq!(run.code, Some(2), "{args:?}");
        assert!(run.stderr.contains(error), "{args:?}: {}", run.stderr);
    }
}

#[test]
fn identifying_lines_an_arm_commits_stay_out_of_the_packet() {
    let fixture = super::process::fixture();
    super::pairs::pair_fixture(&fixture, super::pairs::CANDIDATE_TEST);
    let script = fs::read_to_string(fixture.scenario.path("claude-script.sh")).unwrap();
    fixture.scenario.agent_does(&script.replace(
        "git add extra.txt test_new_candidate.py",
        "printf '# Notes\\nModel: secret-model\\nKept as evidence.\\n' > notes.md\n    git add extra.txt test_new_candidate.py notes.md",
    ));
    panel_graders(&fixture, &["candidate"]);
    let run = fixture.run(&["--panel-seat", "harness=codex,model=judge-one"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let calls = fixture.grader_calls();
    assert_eq!(calls.len(), 2);
    for call in calls {
        let packet =
            fs::read_to_string(PathBuf::from(call["cwd"].as_str().unwrap()).join("packet.json"))
                .unwrap();
        assert!(packet.contains("+Kept as evidence."), "{packet}");
        assert!(!packet.contains("secret-model"), "{packet}");
    }
}

#[test]
fn a_stand_in_bar_whose_voting_panel_spans_too_few_families_is_refused_at_start() {
    let fixture = super::process::fixture();
    super::pairs::pair_fixture(&fixture, super::pairs::CANDIDATE_TEST);
    panel_graders(&fixture, &["candidate", "candidate", "candidate"]);
    // Both Arms run on Claude, so only the OpenAI seat votes: one family.
    let mut args = SEATS.to_vec();
    args.extend([
        "--panel",
        "counted",
        "--people",
        "counted",
        "--people-reader",
        "ann@example.com",
    ]);
    let run = fixture.run(&args);
    assert_ne!(run.code, Some(0), "{}", run.stdout);
    assert!(
        run.stderr.contains(
            "the voting Grader panel spans 1 family (openai), but standing in for people needs 2"
        ),
        "{}",
        run.stderr
    );
    let experiments = fixture.scenario.path("home/.thirdshift/evals/experiments");
    assert!(
        fs::read_dir(&experiments).map_or(true, |mut entries| entries.next().is_none()),
        "no Experiment is created"
    );
    assert!(fixture.grader_calls().is_empty());

    // With the rule off, Anthropic's seats vote too: two families suffice.
    args.extend(["--panel-arm-family-rule", "off"]);
    let run = fixture.run(&args);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
}

#[test]
fn counted_people_hold_the_verdict_until_their_blind_calls_then_decide_it() {
    let fixture = super::process::fixture();
    // No leak hits wait ahead of the people.
    super::process::private_fix(&fixture);
    super::pairs::pair_fixture(&fixture, super::pairs::CANDIDATE_TEST);
    let reader = std::process::Command::new("id")
        .arg("-un")
        .output()
        .unwrap();
    let reader = String::from_utf8(reader.stdout).unwrap().trim().to_owned();
    let run = fixture.run(&["--people", "counted", "--people-reader", &reader]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    super::result_format::assert_documented("Experiment", &view);
    assert_eq!(
        view["bar"]["people"],
        json!({"role": "counted", "readers": [reader], "sample": null, "overlap": 0})
    );
    assert_eq!(view["verdict"]["verdict"], "waiting_on_people", "{view}");
    assert_eq!(view["verdict"]["deciders"]["waiting_pairs"], 1);
    let pair = view["people"]["queues"][0]["pairs"][0]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(view["people"]["queues"][0]["reader"], reader.as_str());
    let id = view["id"].as_str().unwrap();
    let shown = fixture
        .scenario
        .run_with_env(&["experiments", id], &NO_SANDBOX);
    assert!(
        shown
            .stdout
            .contains(&format!("Queue {reader} · 1 pairs: {pair}")),
        "{}",
        shown.stdout
    );

    // The reader calls the Baseline ready and the Candidate not: the
    // Candidate's Trial fails though its code Grader passed.
    let page = fixture.scenario.run_with_env(
        &[
            "experiments",
            id,
            "--pair",
            &pair,
            "--merge",
            "ready",
            "not-ready",
        ],
        &NO_SANDBOX,
    );
    assert_eq!(page.code, Some(0), "{}", page.stderr);
    let shown = fixture
        .scenario
        .run_with_env(&["experiments", id, "--json"], &NO_SANDBOX);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    super::result_format::assert_documented("Experiment", &shown);
    let verdict = &shown["verdict"];
    assert_ne!(verdict["verdict"], "waiting_on_people", "{verdict}");
    assert_eq!(verdict["deciders"]["people_pairs"], 1, "{verdict}");
    assert_eq!(verdict["deciders"]["waiting_pairs"], 0, "{verdict}");
    let sides = &shown["results"];
    assert!(
        sides
            .as_array()
            .unwrap()
            .iter()
            .all(|result| result["score"]["passed"] == true),
        "the code Grader's Scores stay as they were: {sides}"
    );
    assert_eq!(
        verdict["suites"]["process"]["accuracy"]["difference_counts"],
        json!([1, 0, 0]),
        "{verdict}"
    );
}

#[test]
fn a_counted_peoples_call_that_settles_the_verdict_releases_trial_repositories() {
    let fixture = super::process::fixture();
    super::process::private_fix(&fixture);
    super::pairs::pair_fixture(&fixture, super::pairs::CANDIDATE_TEST);
    let reader = std::process::Command::new("id")
        .arg("-un")
        .output()
        .unwrap();
    let reader = String::from_utf8(reader.stdout).unwrap().trim().to_owned();
    let run = fixture.run(&["--people", "counted", "--people-reader", &reader]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(view["verdict"]["verdict"], "waiting_on_people", "{view}");
    let id = view["id"].as_str().unwrap().to_owned();
    let pair = view["people"]["queues"][0]["pairs"][0]
        .as_str()
        .unwrap()
        .to_owned();
    let trials = fixture
        .scenario
        .path("home/.thirdshift/evals/experiments")
        .join(&id)
        .join("trials");
    let repositories = || -> Vec<PathBuf> {
        fs::read_dir(&trials)
            .unwrap()
            .map(|trial| trial.unwrap().path())
            .filter(|trial| trial.is_dir())
            .flat_map(|trial| fs::read_dir(trial).unwrap())
            .map(|attempt| attempt.unwrap().path())
            .filter(|attempt| {
                attempt
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("attempt-")
            })
            .map(|attempt| attempt.join("repository"))
            .filter(|repository| repository.exists())
            .collect()
    };
    assert!(!repositories().is_empty(), "kept while waiting on people");
    let page = fixture.scenario.run_with_env(
        &[
            "experiments",
            &id,
            "--pair",
            &pair,
            "--merge",
            "ready",
            "ready",
        ],
        &NO_SANDBOX,
    );
    assert_eq!(page.code, Some(0), "{}", page.stderr);
    let shown = fixture
        .scenario
        .run_with_env(&["experiments", &id, "--json"], &NO_SANDBOX);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    assert_ne!(shown["verdict"]["verdict"], "waiting_on_people");
    assert!(
        repositories().is_empty(),
        "still retained: {:?}",
        repositories()
    );
}

/// [`panel_graders`] whose every session reports 10,000 input and 1,000
/// output tokens of `fixture-model`: $0.03 at the fixture's frozen prices.
fn priced_panel_graders(fixture: &Fixture, seats: &[&str]) {
    panel_graders(fixture, seats);
    let script = fixture.scenario.path("claude-script.sh.grader");
    let calls = fs::read_to_string(&script).unwrap();
    fs::write(
        &script,
        format!(
            "echo '{{\"type\":\"assistant\",\"message\":{{\"id\":\"panel\",\"model\":\"fixture-model\",\"usage\":{{\"input_tokens\":10000,\"output_tokens\":1000}},\"content\":[]}}}}'\n{calls}"
        ),
    )
    .unwrap();
}

#[test]
fn the_estimate_lists_each_panel_seats_expected_cost() {
    let fixture = super::process::fixture();
    super::pairs::pair_fixture(&fixture, super::pairs::CANDIDATE_TEST);
    priced_panel_graders(&fixture, &["candidate", "candidate"]);
    let run = fixture.run(&[
        "--panel-seat",
        "harness=claude,model=fixture-model",
        "--panel-seat",
        "harness=claude,model=judge-two",
    ]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    // 400k input tokens, 320k of them cache reads, and 15k output tokens at
    // $2, $0.20 and $10 per million: $0.374 a session, two sessions a pair.
    for line in [
        "Grader panel seat 1 (claude/fixture-model): 2 sessions (1 pair × 2 orders), estimate $0.748000 at $0.374000 per assumed 400k-input, 15k-output session",
        "Grader panel seat 2 (claude/judge-two): 2 sessions (1 pair × 2 orders), no frozen price for Model judge-two",
        "Grader panel estimate: $0.748000 for priced seats, apart from the Arms",
    ] {
        assert!(run.stderr.contains(line), "{line}\n{}", run.stderr);
    }
}

#[test]
fn a_low_spending_limit_pauses_grading_and_a_resume_continues_from_completed_calls() {
    let fixture = super::process::fixture();
    super::pairs::pair_fixture(&fixture, super::pairs::CANDIDATE_TEST);
    priced_panel_graders(&fixture, &["candidate"]);
    let args = [
        "--panel-seat",
        "harness=claude,model=fixture-model",
        "--panel-spending-limit",
        "0.01",
    ];
    let paused = fixture.run(&args);
    assert_eq!(paused.code, Some(1), "{}", paused.stderr);
    let view = fixture.experiment();
    assert_eq!(view["state"], "paused", "{view}");
    assert_eq!(view["bar"]["panel"]["spending_limit"], 0.01);
    let reason = view["pause_reason"].as_str().unwrap();
    assert!(
        reason.contains("Grader panel reached its $0.010000 spending limit"),
        "{reason}"
    );
    // The first session spent $0.03, past the limit, so the second order
    // waits.
    assert_eq!(fixture.grader_calls().len(), 1);
    assert_eq!(view["panel"], json!([]));
    assert_eq!(view["panel_spend"]["sessions"], 1, "{view}");
    assert_eq!(view["panel_spend"]["cost_usd"], 0.03, "{view}");
    // The panel's spend is its own: no Arm's sessions or cost include it.
    let arm_sessions: Vec<&Value> = view["accounting"]["trials"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|trial| trial["sessions"].as_array().unwrap())
        .collect();
    assert!(!arm_sessions.is_empty());
    for session in arm_sessions {
        assert!(
            !session["path"].as_str().unwrap().contains("graders/"),
            "{session}"
        );
    }
    let arms = |view: &Value| {
        json!([
            view["accounting"]["baseline"],
            view["accounting"]["candidate"],
            view["accounting"]["paired_median_ratios"]
        ])
    };
    let before = arms(&view);

    // Each run of the command gets the limit afresh; the saved call is kept.
    let resumed = fixture.run(&args);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    let view = fixture.experiment();
    assert_eq!(view["state"], "completed", "{view}");
    assert_eq!(
        fixture.grader_calls().len(),
        2,
        "a completed call was graded again"
    );
    assert_eq!(panel_pair(&view)["preference"], "candidate");
    assert_eq!(view["panel_spend"]["sessions"], 2, "{view}");
    assert_eq!(view["panel_spend"]["cost_usd"], 0.06, "{view}");
    assert_eq!(
        view["panel_spend"]["seats"],
        json!([{"harness": "claude", "model": "fixture-model", "effort": null, "sessions": 2, "cost_usd": 0.06}])
    );
    assert_eq!(arms(&view), before, "panel spend changed the Arms' cost");
    let shown = fixture
        .scenario
        .run_with_env(&["experiments", view["id"].as_str().unwrap()], &NO_SANDBOX);
    assert!(
        shown.stdout.contains(
            "Grader panel spend, apart from the Arms: $0.060000 over 2 sessions\nGrader panel claude/fixture-model: 2 sessions, $0.060000"
        ),
        "{}",
        shown.stdout
    );
}

#[test]
fn sparse_multi_seat_calls_resume_with_matching_estimates_and_reports() {
    let fixture = super::process::fixture();
    super::pairs::pair_fixture(&fixture, super::pairs::CANDIDATE_TEST);
    panel_graders(&fixture, &["candidate", "candidate"]);
    let seats = &SEATS[..4];
    let run = fixture.run(seats);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let before: Value = serde_json::from_str(&run.stdout).unwrap();
    let directory = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/experiments/{}",
        before["id"].as_str().unwrap()
    ));
    let pair = fs::read_dir(directory.join("panel"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    for name in [
        "seat-1-order-1.json",
        "seat-2-order-1.json",
        "seat-2-order-2.json",
    ] {
        fs::remove_file(pair.join(name)).unwrap();
    }
    let retained = fs::read(pair.join("seat-1-order-2.json")).unwrap();
    let trials = fixture.trial_calls().len();
    assert_eq!(fixture.grader_calls().len(), 4);
    assert_eq!(fixture.experiment()["panel"], json!([]));
    let resumed = fixture.run(seats);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    for expected in [
        "Grader panel seat 1 (claude/judge-one): 1 session,",
        "Grader panel seat 2 (claude/judge-two): 2 sessions (1 pair × 2 orders)",
    ] {
        assert!(
            resumed.stderr.contains(expected),
            "{expected}: {}",
            resumed.stderr
        );
    }
    let calls = fixture.grader_calls();
    assert_eq!(calls.len(), 7);
    let resumed_names: Vec<_> = calls[4..]
        .iter()
        .map(|call| {
            Path::new(call["cwd"].as_str().unwrap())
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert!(resumed_names[0].ends_with("--seat-1--order-1"));
    assert!(resumed_names[1].ends_with("--seat-2--order-1"));
    assert!(resumed_names[2].ends_with("--seat-2--order-2"));
    assert_eq!(fixture.trial_calls().len(), trials);
    assert_eq!(
        fs::read(pair.join("seat-1-order-2.json")).unwrap(),
        retained
    );
    let after: Value = serde_json::from_str(&resumed.stdout).unwrap();
    assert_eq!(after["panel"], before["panel"]);
}

#[test]
fn corrupt_later_panel_calls_stop_resume_before_any_more_sessions() {
    let fixture = super::process::fixture();
    super::pairs::pair_fixture(&fixture, super::pairs::CANDIDATE_TEST);
    priced_panel_graders(&fixture, &["candidate"]);
    let args = [
        "--panel-seat",
        "harness=claude,model=fixture-model",
        "--panel-spending-limit",
        "0.01",
    ];
    let paused = fixture.run(&args);
    assert_eq!(paused.code, Some(1), "{}", paused.stderr);
    let view = fixture.experiment();
    let directory = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/experiments/{}",
        view["id"].as_str().unwrap()
    ));
    let pair = fs::read_dir(directory.join("panel"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::remove_file(pair.join("seat-1-order-1.json")).unwrap();
    let corrupt = pair.join("seat-1-order-2.json");
    fs::write(&corrupt, "malformed retained call").unwrap();
    let before = fixture.grader_calls().len();
    for extra in [vec![], vec!["--yes"]] {
        let mut resume = args.to_vec();
        resume.extend(extra);
        let refused = fixture.run(&resume);
        assert_eq!(refused.code, Some(1), "{}", refused.stderr);
        assert_eq!(fixture.grader_calls().len(), before, "{}", refused.stderr);
        assert!(
            refused.stderr.contains("seat 1, order 2"),
            "{}",
            refused.stderr
        );
        assert!(!pair.join("seat-1-order-1.json").exists());
        assert_eq!(
            fs::read_to_string(&corrupt).unwrap(),
            "malformed retained call"
        );
    }
}

#[test]
fn a_panel_spending_limit_needs_a_seat_and_a_positive_amount() {
    let fixture = super::process::fixture();
    for (args, error) in [
        (
            vec!["--panel-spending-limit", "5"],
            "need at least one --panel-seat",
        ),
        (
            vec![
                "--panel-seat",
                "harness=claude,model=a",
                "--panel-spending-limit",
                "0",
            ],
            "--panel-spending-limit must be a positive finite amount in USD",
        ),
    ] {
        let run = fixture.run(&args);
        assert_eq!(run.code, Some(2), "{args:?}");
        assert!(run.stderr.contains(error), "{args:?}: {}", run.stderr);
    }
}

#[test]
fn an_unpriceable_panel_session_under_a_limit_keeps_its_completed_call() {
    let fixture = super::process::fixture();
    super::pairs::pair_fixture(&fixture, super::pairs::CANDIDATE_TEST);
    priced_panel_graders(&fixture, &["candidate"]);
    // The sessions report a Model the frozen table doesn't price.
    let script = fixture.scenario.path("claude-script.sh.grader");
    let calls = fs::read_to_string(&script).unwrap();
    fs::write(&script, calls.replace("fixture-model", "unpriced-model")).unwrap();
    let args = [
        "--panel-seat",
        "harness=claude,model=judge-two",
        "--panel-spending-limit",
        "5",
    ];
    let paused = fixture.run(&args);
    assert_eq!(paused.code, Some(1), "{}", paused.stderr);
    let view = fixture.experiment();
    let reason = view["pause_reason"].as_str().unwrap();
    assert!(
        reason.contains("can't price a Grader panel session against the spending limit"),
        "{reason}"
    );
    for _ in 0..2 {
        fixture.run(&args);
    }
    // Each order's call is kept once made: no session is paid for twice.
    assert_eq!(
        fixture.grader_calls().len(),
        2,
        "a completed call was graded again"
    );
    let view = fixture.experiment();
    assert_eq!(view["panel"].as_array().unwrap().len(), 1, "{view}");
    assert_eq!(view["panel_spend"]["cost_usd"], Value::Null, "{view}");
}

#[test]
fn a_resume_with_only_panel_calls_left_asks_about_the_panel_alone() {
    let fixture = super::process::fixture();
    super::pairs::pair_fixture(&fixture, super::pairs::CANDIDATE_TEST);
    priced_panel_graders(&fixture, &["candidate"]);
    let args = [
        "--panel-seat",
        "harness=claude,model=fixture-model",
        "--panel-spending-limit",
        "0.01",
    ];
    let paused = fixture.run(&args);
    assert_eq!(paused.code, Some(1), "{}", paused.stderr);

    let declined = fixture.run_on_terminal(
        &args,
        &[("Run the remaining Grader panel sessions? [y/N] ", "n")],
    );
    assert_eq!(declined.code, Some(1), "{}", declined.stderr);
    for shown in [
        "0 of 2 Trials remain",
        "Grader panel seat 1 (claude/fixture-model): 1 session, estimate $0.030000 at $0.030000 per session measured in this Experiment",
        "Grader panel spending limit: $0.010000 per run, then grading pauses",
        "Experiment declined; no Grader panel sessions were run",
    ] {
        assert!(
            declined.stderr.contains(shown),
            "{shown}: {}",
            declined.stderr
        );
    }
    assert!(
        !declined.stderr.contains("Worst case"),
        "{}",
        declined.stderr
    );
    assert_eq!(fixture.grader_calls().len(), 1);
}
