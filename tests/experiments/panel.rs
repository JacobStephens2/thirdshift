//! The Grader panel: each seat reads every pair of process Trials blind in
//! both orders, and its calls and the panel's vote go beside the Verdict.
use super::*;

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
