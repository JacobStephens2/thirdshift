//! Structured Experiment options, shared by admission and OpenAPI.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(in crate::console) fn properties() -> Value {
    let mut properties = serde_json::Map::new();
    for key in [
        "candidate",
        "baseline",
        "baseline_harness",
        "candidate_harness",
        "baseline_model",
        "candidate_model",
        "baseline_effort",
        "candidate_effort",
        "name",
    ] {
        properties.insert(key.into(), json!({"type":"string"}));
    }
    for key in ["margin", "pass_level", "fail_level", "prior"] {
        properties.insert(key.into(), json!({"type":"number"}));
    }
    for key in [
        "seed",
        "draws",
        "min_discriminating_tasks",
        "timeout",
        "void_retries",
        "trials_per_task",
    ] {
        properties.insert(key.into(), json!({"type":"integer"}));
    }
    for key in ["release", "yes"] {
        properties.insert(key.into(), json!({"type":"boolean"}));
    }
    properties.insert(
        "suite".into(),
        json!({"type":"array","items":{"type":"string"}}),
    );
    Value::Object(properties)
}

pub(super) fn arguments(options: Option<&BTreeMap<String, Value>>) -> Result<Vec<String>> {
    let properties = properties();
    let mut args = vec!["--json".into()];
    for (key, value) in options.into_iter().flatten() {
        let schema = properties
            .get(key)
            .ok_or_else(|| anyhow::anyhow!("unknown Experiment option: {key}"))?;
        let valid = match schema["type"].as_str().unwrap() {
            "string" => value.is_string(),
            "number" => value.is_number(),
            "integer" => value.is_i64() || value.is_u64(),
            "boolean" => value.is_boolean(),
            "array" => value
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_string)),
            _ => unreachable!(),
        };
        ensure!(valid, "invalid Experiment option: {key}");
        if key == "candidate" {
            let candidate = value.as_str().unwrap();
            ensure!(
                !candidate.starts_with('-'),
                "candidate must be a Git reference"
            );
            args.push(candidate.into());
        } else if value.is_boolean() {
            if value == true {
                args.push(format!("--{key}"));
            }
        } else if let Some(items) = value.as_array() {
            for item in items {
                args.extend(["--suite".into(), item.as_str().unwrap().into()]);
            }
        } else {
            args.extend([
                format!("--{}", key.replace('_', "-")),
                value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string()),
            ]);
        }
    }
    crate::evals::experiment::parse(&args)?;
    Ok(args)
}
