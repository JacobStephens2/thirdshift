//! The document is the independent source of field names and JSON types.
use super::*;

fn contract() -> Value {
    let document = include_str!("../../docs/experiment-results.md");
    let block = document
        .split_once("<!-- result-contract -->\n```json\n")
        .expect("documented result contract")
        .1
        .split_once("\n```")
        .unwrap()
        .0;
    serde_json::from_str(block).expect("valid documented result contract")
}

pub(super) fn assert_documented(name: &str, value: &Value) {
    let contract = contract();
    if let Err(error) = check(&contract, &contract[name], value, name) {
        panic!("Experiment results documentation drift: {error}");
    }
}

fn check(contract: &Value, shape: &Value, value: &Value, path: &str) -> Result<(), String> {
    let mismatch = || format!("{path}: expected {shape}, got {value}");
    match shape {
        Value::Object(fields) => {
            let object = value.as_object().ok_or_else(mismatch)?;
            if let Some(item) = fields.get("*") {
                for (key, value) in object {
                    check(contract, item, value, &format!("{path}.{key}"))?;
                }
            } else {
                for key in object.keys() {
                    if !fields.contains_key(key) && !fields.contains_key(&format!("{key}?")) {
                        return Err(format!("{path}.{key}: undocumented field"));
                    }
                }
                for (key, field) in fields {
                    let name = key.trim_end_matches('?');
                    if let Some(value) = object.get(name) {
                        check(contract, field, value, &format!("{path}.{name}"))?;
                    } else if !key.ends_with('?') {
                        return Err(format!("{path}.{name}: missing documented field"));
                    }
                }
            }
        }
        Value::Array(items) => {
            assert_eq!(items.len(), 1, "one item type at {path}");
            for (index, item) in value.as_array().ok_or_else(mismatch)?.iter().enumerate() {
                check(contract, &items[0], item, &format!("{path}[{index}]"))?;
            }
        }
        Value::String(kind) => {
            if kind.ends_with('?') && value.is_null() {
                return Ok(());
            }
            let kind = kind.trim_end_matches('?');
            if let Some(reference) = contract.get(kind) {
                return check(contract, reference, value, path);
            }
            let matches = match kind {
                "string" => value.is_string(),
                "integer" => value.as_u64().is_some(),
                "number" => value.is_number(),
                "boolean" => value.is_boolean(),
                _ if kind.contains('|') => value
                    .as_str()
                    .is_some_and(|text| kind.split('|').any(|choice| choice == text)),
                _ => panic!("unknown documented type {kind}"),
            };
            if !matches {
                return Err(mismatch());
            }
        }
        _ if shape == value => {}
        _ => return Err(mismatch()),
    }
    Ok(())
}

#[test]
fn documented_format_matches_real_experiment_results() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    let run = fixture.run(&["--prior", "0.01"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment = fixture.experiment();
    assert_eq!(experiment["format"], 3, "format 2 shipped in v0.17.4");
    assert_documented("Experiment", &serde_json::from_str(&run.stdout).unwrap());
    assert!(
        !experiment["pending_rechecks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let id = experiment["id"].as_str().unwrap();
    let trial = experiment["pending_rechecks"][0]["id"].as_str().unwrap();
    let rechecked = fixture.scenario.run(&[
        "recheck",
        id,
        trial,
        "pass",
        "--reason",
        "Written criteria met.",
        "--wrong-finding",
        "false",
        "--json",
    ]);
    assert_eq!(rechecked.code, Some(0), "{}", rechecked.stderr);
    let checked = fixture.experiment();
    assert_documented(
        "Experiment",
        &serde_json::from_str(&rechecked.stdout).unwrap(),
    );
    assert!(checked["results"].as_array().unwrap().iter().any(|result| {
        result["score"]["rechecks"]
            .as_array()
            .is_some_and(|history| !history.is_empty())
    }));

    let session = checked["sessions"][0]["id"].as_str().unwrap();
    let contents = fixture
        .scenario
        .run(&["experiments", id, "--session", session, "--json"]);
    assert_eq!(contents.code, Some(0), "{}", contents.stderr);
    assert_documented(
        "SessionContents",
        &serde_json::from_str(&contents.stdout).unwrap(),
    );

    let directory = fixture
        .scenario
        .path(&format!("home/.thirdshift/evals/experiments/{id}/results"));
    let files: Vec<_> = fs::read_dir(directory).unwrap().collect();
    assert_eq!(files.len(), 2);
    for file in files {
        let result = serde_json::from_slice(&fs::read(file.unwrap().path()).unwrap()).unwrap();
        assert_documented("TrialResult", &result);
    }

    // Prove the guard rejects additions, removals and retypes inside a Score,
    // rather than merely checking the outer object or a list of key strings.
    let contract = contract();
    for mutation in ["add", "remove", "retype"] {
        let mut changed = checked.clone();
        let score = changed["results"][0]["score"].as_object_mut().unwrap();
        match mutation {
            "add" => {
                score.insert("undocumented".into(), json!(true));
            }
            "remove" => {
                score.remove("passed");
            }
            "retype" => {
                score.insert("passed".into(), json!("true"));
            }
            _ => unreachable!(),
        }
        assert!(
            check(&contract, &contract["Experiment"], &changed, "Experiment").is_err(),
            "{mutation}"
        );
    }
}
