//! OpenAPI 3.1 from the same entries that register the Console's API routes.
//! Only static metadata is read: descriptions never inspect local credentials.
use serde_json::{Map, Value, json};

use super::{PAGES, PageRoute, View};

pub fn print() -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(&document())?)
}

pub(super) async fn serve() -> axum::Json<Value> {
    axum::Json(document())
}

pub(super) fn document() -> Value {
    let mut paths = Map::<String, Value>::new();
    for route in PAGES.iter().filter(|route| route.serves_api()) {
        let (parameters, schema) = operation(route);
        paths.entry(route.api).or_insert_with(|| json!({}))
            [route.method.as_str().to_ascii_lowercase()] = json!({
            "description": format!("CLI twins: {}. The required scope is declared in x-scope.", route.cli.join("; ")),
            "x-scope": route.scope.map(|scope| scope.name()).unwrap_or("authenticated"),
            "x-cli-twins": route.cli,
            "parameters": parameters,
            "responses": {
                "200": {
                    "description": "The CLI twin's JSON response, filtered by the caller's scopes.",
                    "content": {"application/json": {"schema": schema}}
                },
                "400": refusal("Invalid path argument."),
                "401": refusal("A valid Console sign-in or bearer token is required."),
                "403": refusal("The caller lacks the required scope."),
                "422": refusal("The CLI twin refused the arguments or selected record."),
                "429": {
                    "description": "Too many failed authentications from this client address.",
                    "headers": {"Retry-After": {
                        "description": "Seconds until another authentication may be attempted.",
                        "schema": {"type": "integer", "minimum": 1}
                    }},
                    "content": {"application/json": {"schema": object(json!({
                        "error": {"type": "string"},
                        "retry_after": {"type": "integer", "minimum": 1}
                    }))}}
                },
                "500": refusal("The Console could not read this view.")
            }
        });
        if route.method == axum::http::Method::POST {
            paths.get_mut(route.api).unwrap()["post"]["requestBody"] = action_body(route.view);
        }
    }
    json!({
        "openapi": "3.1.1",
        "info": {
            "title": "thirdshift Console API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Currently served /api/v1 operations. Each operation enforces its x-scope. Bearer tokens require HTTPS or a direct connection to 127.0.0.1 without proxy headers. Local --no-sign-in needs no session."
        },
        "paths": paths,
        "security": [{"ConsoleSession": []}, {"ConsoleBearer": []}],
        "components": {"securitySchemes": {
            "ConsoleSession": {
                "type": "apiKey", "in": "cookie", "name": "__Host-thirdshift-session"
            },
            "ConsoleBearer": {"type": "http", "scheme": "bearer"}
        }}
    })
}

fn refusal(description: &str) -> Value {
    json!({"description": description, "content": {"application/json": {
        "schema": object(json!({"error": {"type": "string"}}))
    }}})
}

fn parameter(name: &str, location: &str, required: bool, schema: Value) -> Value {
    json!({"name": name, "in": location, "required": required, "schema": schema})
}

fn object(properties: Value) -> Value {
    let required = properties
        .as_object()
        .expect("schema properties")
        .keys()
        .collect::<Vec<_>>();
    json!({"type": "object", "properties": properties, "required": required})
}

fn array(items: Value) -> Value {
    json!({"type": "array", "items": items})
}

/// Describe the CLI envelopes. Nested Command and Experiment records are open
/// objects so their existing versioned JSON fields remain extensible.
fn operation(route: &PageRoute) -> (Vec<Value>, Value) {
    let mut parameters: Vec<_> = route
        .api
        .split('/')
        .filter_map(|segment| {
            let name = segment.strip_prefix('{')?.strip_suffix('}')?;
            Some(parameter(name, "path", true, json!({"type": "string"})))
        })
        .collect();
    parameters.push(json!({
        "name": "X-Thirdshift-Person", "in": "header", "required": false,
        "description": "Bearer calls only: an address on the owning machine's sign-in list. Effective scopes intersect the token's and this address's scopes.",
        "schema": {"type": "string", "format": "email"}
    }));
    let mut query = |name, required, schema| {
        parameters.push(parameter(name, "query", required, schema));
    };
    let text = json!({"type": "string"});
    let integer = json!({"type": "integer"});
    let record = json!({"type": "object"});
    let version = json!({"type": "integer", "const": 1});
    let schema = match route.view {
        View::Help
        | View::Version
        | View::Processes
        | View::Process
        | View::CheckProcess
        | View::EmailTest => text,
        View::Evals => array(record),
        View::TouchedEvals => array(text),
        View::VerifyEval => object(json!({"safe":{"const":true}})),
        View::OtherAccounts | View::SetOtherAccounts => {
            json!({"oneOf":[{"type":"string","enum":["none","readable"]},array(text)]})
        }
        View::RecordDifficulty => array(record),
        View::RenameExperiment
        | View::PendingRechecks
        | View::Recheck
        | View::DecideLeak
        | View::ExperimentPair
        | View::MergeCalls
        | View::PairPreference => record,
        View::Update => object(json!({"current": text, "latest": text})),
        View::RunUpdate => text,
        View::ImportReplays => object(json!({"imported": integer, "skipped": integer})),
        View::ProposeProcess => {
            object(json!({"proposals":array(record.clone()),"skipped":array(record)}))
        }
        View::Config => object(json!({"version": version, "settings": array(setting())})),
        View::ConfigSetting | View::SetConfig => setting(),
        View::Whoami => object(json!({
            "version": version, "hostname": text, "thirdshift_version": text,
            "api_version": {"type": "string", "const": "v1"},
            "identity": {"oneOf": [
                {"type": "object", "required": ["kind", "holder"], "properties": {
                    "kind": {"const": "token"}, "holder": text, "person": text
                }},
                object(json!({"kind": {"const": "session"}, "address": text})),
                object(json!({"kind": {"const": "local"}, "uid": integer}))
            ]},
            "scopes": array(json!({"type": "string", "enum": super::Scope::TOKEN}))
        })),
        View::ReportMetrics => object(json!({
            "machine": text, "pushed_at": {"type": "string", "format": "date-time"}
        })),
        View::OpenApi => object(json!({
            "openapi": {"type": "string", "const": "3.1.1"},
            "info": record, "paths": record, "components": record, "security": array(record.clone())
        })),
        View::Inbox if route.path == "/waterfall" => {
            query("command", false, text);
            tree()
        }
        View::Inbox => {
            if route.path == "/" {
                query("all", false, json!({"type": "boolean", "default": false}));
            }
            object(json!({
                "version": {"type": "integer", "const": 2}, "hostname": text, "active_runs": integer,
                "machines": array(object(json!({
                    "name": {"type": "string"}, "hostname": {"type": ["string", "null"]},
                    "fetched_at": {"type": "string", "format": "date-time"},
                    "reachability": {"enum": ["ok", "unreachable", "refused", "needs update"]},
                    "active_runs": {"type": "integer"}, "error": {"type": ["string", "null"]}
                }))),
                "commands": array(record.clone()), "idle_passes": array(record.clone()),
                "pass_starts": array(record), "github_answer_age_seconds": {"type": ["integer", "null"]},
                "github_last_answered_at": {"type": ["string", "null"]},
                "github_failure": {"type": ["string", "null"]}
            }))
        }
        View::Inspect => {
            query("command", true, text.clone());
            object(json!({"version": version, "hostname": text, "command": record}))
        }
        View::Spans => {
            query("command", false, text.clone());
            query("span", false, text);
            json!({"oneOf": [tree(), span()]})
        }
        View::Log => {
            query("command", true, text.clone());
            query("session", false, text.clone());
            query(
                "tail",
                false,
                json!({"type": "boolean", "default": false, "description": "Read at most the final 4096 bytes."}),
            );
            object(json!({"version": version, "contents": text}))
        }
        View::Reader => object(json!({"version": version, "contents": text})),
        View::Logs | View::CommandLogs => object(json!({
            "version": version, "logs": array(object(json!({
                "id": text, "name": text, "repository": text, "kind": text,
                "live": {"type": "boolean"}, "command": text, "owners": array(text.clone())
            })))
        })),
        View::Metrics => {
            query("command", false, text.clone());
            query(
                "window",
                false,
                json!({"enum": ["7d", "30d", "12w", "all"], "description": "A preset ending today: 7d and 30d by day, 12w and all by week. Not with grain, from or through."}),
            );
            query(
                "grain",
                false,
                json!({"enum": ["day", "week"], "default": "week"}),
            );
            for name in ["from", "through"] {
                query(
                    name,
                    false,
                    json!({"type": "string", "format": "date", "description": "A Monday unless grain is day. Both bound the window and add a comparison with the preceding one."}),
                );
            }
            for name in ["repo", "kind", "harness", "model", "version", "account"] {
                query(name, false, array(text.clone()));
            }
            let nullable = json!({"type": ["string", "null"]});
            let window = object(json!({
                "preset": nullable, "grain": {"enum": ["day", "week"]},
                "from": nullable, "through": nullable,
                "prior": {"oneOf": [{"type": "null"}, object(json!({"from": text, "through": text}))]}
            }));
            let tally = json!({"type": "object", "description": "Succeeded, Failed (Died included), Paused, Stopped and Live counts; n is Succeeded + Failed; success_rate is Succeeded / n. An outcomes bucket also lists its Failed Commands' selectors as failed_commands."});
            json!({"oneOf": [
                object(json!({"version": version, "hostname": text, "command": record})),
                {
                    "type": "object",
                    "description": "Signal rows are weeks by week grain, days by day grain; their repairs, ci_rounds and total_minutes cover Succeeded Commands. Each group holds an axis of shown buckets and lines, each with its key, window totals, per-bucket values and, for a bounded window, a comparison with current, prior and delta.",
                    "properties": {
                        "version": version, "hostname": text, "window": window, "filters": record,
                        "unavailable_accounts": array(text.clone()), "outcomes": array(tally.clone()),
                        "comparison": {"oneOf": [{"type": "null"}, object(json!({
                            "current": record, "prior": record, "rows": array(record.clone())
                        }))]},
                        "where_runs_failed": {
                            "type": "object",
                            "description": "Failed Commands, died ones included, as rows by stopped_at and category, each listing its Commands' selectors; observed recorded both, missing fell back to the last stage before any trailing cleanup and died or failed.",
                            "properties": {"failed": integer, "observed": integer, "missing": integer, "rows": array(record.clone())}
                        },
                        "versions": object(json!({
                            "rows": array(tally),
                            "transitions": array(object(json!({
                                "bucket": text, "dimension": {"enum": ["thirdshift_version", "harness", "model"]},
                                "from": array(text.clone()), "to": array(text.clone())
                            })))
                        })),
                        "groups": object(json!({
                            "yield": object(json!({"pull_requests": record})),
                            "quality": object(json!({"bug_origins": record, "scores": record, "interventions": record})),
                            "efficiency": record, "friction": record
                        })),
                        "weeks": array(record.clone()), "days": array(record)
                    },
                    "required": ["version", "hostname", "window", "filters", "unavailable_accounts", "outcomes", "comparison", "where_runs_failed", "versions", "groups"]
                }
            ]})
        }
        View::BugOrigins => array(object(json!({
            "issue": text, "closed_at": text, "state": {"enum": ["proposed", "confirmed", "rejected"]},
            "commit": {"type": ["string", "null"]}, "reason": text, "pull_requests": array(record)
        }))),
        View::Scores => {
            query("pull_request", false, text.clone());
            array(object(json!({
                "week": text, "window_start": text, "window_end": text,
                "seed": integer, "sample": array(record), "initial_calibration": {"type": "boolean"}
            })))
        }
        View::Experiments => array(experiment()),
        View::Experiment => experiment(),
        View::ExperimentSession => object(json!({
            "id": text, "trace": {"type": "boolean"}, "contents": text
        })),
        View::CronJobs => object(json!({
            "jobs": array(object(json!({
                "schedule": text, "command": text, "repository": {"type": ["string", "null"]},
                "command_kind": {"type": ["string", "null"], "description": "Conservative owning-machine Command recognition; null when unknown"}
            }))), "pass_starts": array(record)
        })),
        View::LaunchDirectories => object(json!({
            "directories": array(object(json!({"repository": text, "path": text})))
        })),
        View::StartOptions => object(json!({"form": record, "passes": array(record.clone())})),
        View::StartRun => json!({"oneOf": [
            object(json!({"version": version, "name": text})),
            object(json!({"version": version, "outcome": {"const":"skipped"}, "reason": text})),
            object(json!({"version": version, "outcome": {"const":"started"}, "process_id": integer}))
        ]}),
        View::Resources | View::ResourceStatus => object(json!({
            "host": object(json!({"total_memory_mb": integer, "available_memory_mb": integer,
                "logical_cpus": integer, "load_one": {"type": "number"}, "available_disk_mb": integer})),
            "limits": object(json!({"min_available_memory_mb": integer, "max_load_per_cpu_percent": integer,
                "min_available_disk_mb": integer})),
            "load_per_cpu_percent": {"type": "number"}, "ready": {"type": "boolean"},
            "reasons": array(text)
        })),
        View::ConsoleLog => array(object(
            json!({"time": integer, "email": text, "event": text}),
        )),
        View::ThemeImport | View::ThemeExport => object(json!({
            "name": text, "base": text, "tokens": {"type": "object", "additionalProperties": text}
        })),
        View::Themes => array(text),
        View::Stop => object(json!({"version": version, "name": text, "outcome": text,
            "issue_branches": array(text.clone()), "pull_requests": array(text)})),
        View::Dismiss | View::UndoDismissal => record,
        View::HumanScores => array(record),
        View::SettleBugOrigin => record,
        View::ApproveEvalTask => {
            object(json!({"suite": text, "task": text, "state": {"const": "approved"}}))
        }
        View::Look | View::Theme | View::Sections => record,
        View::FleetRun
        | View::Changes
        | View::LogStream
        | View::Account
        | View::SignOut
        | View::SignInPage
        | View::SignIn
        | View::StopPage
        | View::SetPasswordPage
        | View::SendCode
        | View::SetPassword => unreachable!("page-only operation"),
    };
    (parameters, schema)
}

fn action_body(view: View) -> Value {
    let text = json!({"type": "string"});
    let json_request = |properties, required| {
        json!({"required":true,"content":{"application/json":{"schema":{
            "type":"object","additionalProperties":false,"properties":properties,"required":required
        }}}})
    };
    let (properties, required) = match view {
        View::CheckProcess => {
            let mut schema = start_schema()["oneOf"]
                .as_array()
                .unwrap()
                .iter()
                .find(|schema| schema["properties"]["command"]["const"] == "process")
                .unwrap()
                .clone();
            schema["required"]
                .as_array_mut()
                .unwrap()
                .retain(|value| value != "command");
            return json!({"required":true,"content":{"application/json":{"schema":schema}}});
        }
        View::TouchedEvals => return json_request(json!({"paths":array(text)}), vec!["paths"]),
        View::ProposeProcess => {
            return json_request(
                json!({"repository":{"type":"string","description":"A known Launch directory's owner/repo, not a filesystem path."},"limit":{"type":"integer","minimum":1,"maximum":100,"default":100}}),
                vec!["repository"],
            );
        }
        View::VerifyEval => {
            return json_request(
                json!({"repository":text,"base":text}),
                vec!["repository", "base"],
            );
        }
        View::SetOtherAccounts => return json_request(json!({"value":text}), vec!["value"]),
        View::EmailTest => return json_request(json!({"address":text}), vec![]),
        View::RenameExperiment => return json_request(json!({"name":text}), vec!["name"]),
        View::RecordDifficulty => (json!({}), vec![]),
        View::Recheck => {
            let mut body = json_request(
                json!({"trial":text,"fingerprint":text,"decision":{"enum":["pass","fail"]},"reason":text,"wrong_finding":{"type":"boolean"}}),
                vec!["decision", "reason"],
            );
            body["content"]["application/json"]["schema"]["oneOf"] = json!([
                {"required":["trial"],"not":{"required":["fingerprint"]}},
                {"required":["fingerprint"],"not":{"required":["trial"]}}
            ]);
            return body;
        }
        View::DecideLeak => {
            return json!({"required":true,"content":{"application/x-www-form-urlencoded":{"schema":{
                "type":"object", "properties":{"decision":{"enum":["confirm","dismiss"]},"reason":text},"required":["decision","reason"]
            }}}});
        }
        View::MergeCalls => {
            let call = json!({"enum": ["ready", "not-ready"]});
            return json!({"required":true,"content":{"application/x-www-form-urlencoded":{"schema":{
                "type":"object", "properties":{"a":call,"b":call},"required":["a","b"]
            }}}});
        }
        View::PairPreference => {
            use crate::evals::experiment::pairwise::Preference;
            let values: Vec<_> = Preference::ALL.iter().map(|(_, value, _)| *value).collect();
            let preference = json!({"enum": values});
            let mut properties = json!({"preference": preference});
            for (name, _) in crate::evals::experiment::pairwise::RUBRIC {
                properties[format!("rubric_{name}")] = preference.clone();
            }
            return json!({"required":true,"content":{"application/x-www-form-urlencoded":{"schema":{
                "type":"object", "properties":properties,"required":["preference"]
            }}}});
        }
        View::StartRun => {
            return json!({"required": true, "content": {"application/json": {"schema": start_schema()}}});
        }
        View::SetConfig => (json!({"value": text}), vec!["value"]),
        View::ThemeImport => {
            return json!({"required": true, "content": {"application/json": {"schema": {
                "type": "object", "required": ["name"], "properties": {
                    "name": text, "base": text,
                    "tokens": {"type": "object", "additionalProperties": text}
                }
            }}}});
        }
        View::Stop => (
            json!({"command": text, "requeue": {"type":"boolean", "default":false}}),
            vec!["command"],
        ),
        View::Dismiss | View::UndoDismissal => (json!({"command": text}), vec!["command"]),
        View::Theme => (json!({"theme": text}), vec!["theme"]),
        View::Sections => (json!({"hide": text, "show": text}), vec![]),
        View::SettleBugOrigin => (
            json!({"issue": text, "action": {"enum":["confirm","correct","reject"]}, "pull_request": text}),
            vec!["issue", "action"],
        ),
        View::HumanScores => {
            let mut properties = json!({"pull_request": text});
            let mut required = vec!["pull_request".to_owned()];
            for criterion in crate::scoring::Criterion::ALL {
                let name = criterion.name();
                properties[name] = json!({"enum":["pass","fail"]});
                properties[format!("{name}_reason")] = text.clone();
                required.push(name.to_owned());
                required.push(format!("{name}_reason"));
            }
            return json!({"required":true,"content":{"application/x-www-form-urlencoded":{"schema":{
                "type":"object", "properties":properties,"required":required
            }}}});
        }
        View::ReportMetrics => {
            let record = json!({"type": "object"});
            return json!({"required": true, "content": {"application/json": {"schema": {
                "type": "object", "additionalProperties": false,
                "required": ["version", "machine", "view", "summary"],
                "properties": {
                    "version": {"type": "integer", "const": 1},
                    "machine": {"type": "string", "description": "The report token's holder; any other name is refused with 403."},
                    "view": {"type": "string", "description": "The summarized view's metrics query: window and filter keys only."},
                    "summary": object(json!({"outcomes": array(record.clone()), "comparison": {"oneOf": [{"type": "null"}, record.clone()]}, "where_runs_failed": record}))
                }
            }}}});
        }
        View::ApproveEvalTask | View::RunUpdate | View::ImportReplays => (json!({}), vec![]),
        View::Help
        | View::Version
        | View::Processes
        | View::Process
        | View::Evals
        | View::OtherAccounts
        | View::PendingRechecks
        | View::Whoami
        | View::Update
        | View::Config
        | View::ConfigSetting
        | View::OpenApi
        | View::BugOrigins
        | View::Scores
        | View::Experiments
        | View::Experiment
        | View::ExperimentSession
        | View::ExperimentPair
        | View::CronJobs
        | View::LaunchDirectories
        | View::StartOptions
        | View::FleetRun
        | View::Resources
        | View::ResourceStatus
        | View::Metrics
        | View::Inbox
        | View::Inspect
        | View::Spans
        | View::Log
        | View::Changes
        | View::Logs
        | View::CommandLogs
        | View::Reader
        | View::LogStream
        | View::ConsoleLog
        | View::Account
        | View::SignOut
        | View::SignInPage
        | View::SignIn
        | View::Look
        | View::Themes
        | View::ThemeExport
        | View::StopPage
        | View::SetPasswordPage
        | View::SendCode
        | View::SetPassword => unreachable!("not a served action"),
    };
    json!({"required":!required.is_empty(),"content":{"application/x-www-form-urlencoded":{"schema":{
        "type":"object", "properties":properties,"required":required
    }}}})
}

fn start_schema() -> Value {
    let text = json!({"type":"string"});
    let boolean = json!({"type":"boolean"});
    let fields = json!({
        "repository": text, "issue": {"oneOf":[{"type":"integer","minimum":1},text]},
        "pull_request": text, "author": text, "name": text,
        "base": text, "focus": text, "report": {"type":"string","minLength":1},
        "order": {"enum":["lowest-first","highest-first"]},
        "plan_only": boolean, "recommend_only": boolean, "read_machine": boolean,
        "allow_ready_for_agent": boolean, "merge": boolean, "base_fix": boolean,
        "security_review": boolean, "security_fix": boolean, "security_close": boolean,
        "parallel": {"type":"integer","minimum":1},
        "harness": {"enum":["claude","codex","agy","grok","muse","opencode","kimi"]},
        "model": text, "effort": text,
        "email": {"oneOf":[
            {"type":"object","required":["mode"],"additionalProperties":false,
             "properties":{"mode":{"enum":["always","unless_merged"]},"address":text}},
            {"type":"object","required":["mode"],"additionalProperties":false,
             "properties":{"mode":{"const":"never"}}}
        ]},
        "experiment": {"type":"object","additionalProperties":false,"properties":super::start::experiment_properties()}
    });
    let mut variants = Vec::new();
    for kind in super::start::Kind::ALL {
        let command = kind.word();
        let mut properties = serde_json::Map::new();
        for key in ["repository"].into_iter().chain(kind.fields()) {
            properties.insert(key.into(), fields[key].clone());
        }
        properties.insert("command".into(), json!({"const":command}));
        let mut required: Vec<_> = kind
            .required()
            .iter()
            .copied()
            .chain(["repository"])
            .collect();
        if command != "ticket" {
            required.push("command");
        }
        let mut variant = json!({"type":"object","additionalProperties":false,"required":required,"properties":properties});
        if command == "review" {
            variant["oneOf"] = json!([
                {"required":["pull_request"],"not":{"required":["author"]}},
                {"required":["author"],"not":{"required":["pull_request"]}}
            ]);
        } else if command == "process" {
            variant["not"] = json!({"required":["issue","pull_request"]});
        } else if command == "fix" {
            variant["not"] = json!({"anyOf":[
                {"required":["pull_request","base"]},
                {"required":["pull_request","author"]}
            ]});
        }
        variants.push(variant);
    }
    json!({"oneOf":variants})
}

fn setting() -> Value {
    object(json!({
        "key": {"type":"string"}, "value": {}, "default": {},
        "editable": {"type":"boolean"}, "reason": {"type":["string","null"]}
    }))
}

fn tree() -> Value {
    object(
        json!({"command": {"type": "string"}, "now": {"type": "number"}, "spans": array(span())}),
    )
}

fn span() -> Value {
    object(json!({
        "id": {"type": "string"}, "parent_id": {"type": ["string", "null"]},
        "kind": {"type": "string"}, "name": {"type": "string"},
        "start": {"type": ["number", "null"]}, "end": {"type": ["number", "null"]},
        "duration_ms": {"type": ["number", "null"]}, "status": {"type": "string"},
        "error": {"type": ["string", "null"]}, "estimated": {"type": "boolean"},
        "attributes": {"type": "object"}, "input": {"type": ["string", "null"]},
        "output": {"type": ["string", "null"]}, "command": {"type": "string"},
        "command_url": {"type": "string"}, "conversation_url": {"type": ["string", "null"]},
        "run": {"type": ["integer", "null"]}, "session_kind": {"type": ["string", "null"]}
    }))
}

fn experiment() -> Value {
    object(json!({
        "format": {"type": "integer", "const": 12}, "id": {"type": "string"},
        "waiting_for": array(json!({"type":"string"})),
        "no_wait": {"type":"boolean"}, "commands_at_start": {"type":"integer"},
        "name": {"type": "string"}, "state": {"type": "string"}, "created_at": {"type": "string"},
        "tasks": array(json!({"type": "object"})), "results": array(json!({"type": "object"})),
        "sessions": array(json!({"type": "object"})), "running_trials": array(json!({"type": "object"}))
    }))
}
