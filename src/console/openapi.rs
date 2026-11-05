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
        paths.entry(route.api).or_insert_with(|| json!({}))
            [route.method.as_str().to_ascii_lowercase()] = json!({
            "description": format!("CLI twins: {}. The required scope is declared in x-scope.", route.cli.join("; ")),
            "x-scope": route.scope,
            "x-cli-twins": route.cli,
            "parameters": parameters(route),
            "responses": {
                "200": {
                    "description": "The CLI twin's JSON response.",
                    "content": {"application/json": {"schema": response_shape(route)}}
                },
                "400": refusal("Invalid path argument."),
                "401": refusal("A Console sign-in is required."),
                "422": refusal("The CLI twin refused the arguments or selected record."),
                "500": refusal("The Console could not read this view.")
            }
        });
    }
    json!({
        "openapi": "3.1.1",
        "info": {
            "title": "thirdshift Console API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Currently served /api/v1 operations. Scopes are declarations; token scope enforcement arrives in a later Console API ticket. Local --no-sign-in needs no session."
        },
        "paths": paths,
        "security": [{"ConsoleSession": []}],
        "components": {"securitySchemes": {"ConsoleSession": {
            "type": "apiKey", "in": "cookie", "name": "__Host-thirdshift-session"
        }}}
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

fn parameters(route: &PageRoute) -> Vec<Value> {
    let mut parameters: Vec<_> = route
        .api
        .split('/')
        .filter_map(|segment| {
            let name = segment.strip_prefix('{')?.strip_suffix('}')?;
            Some(parameter(name, "path", true, json!({"type": "string"})))
        })
        .collect();
    let mut query = |name, required, schema| {
        parameters.push(parameter(name, "query", required, schema));
    };
    match route.view {
        View::Inspect | View::Log => {
            query("command", true, json!({"type": "string"}));
            if matches!(route.view, View::Log) {
                query("session", false, json!({"type": "string"}));
            }
        }
        View::Spans => {
            query("command", false, json!({"type": "string"}));
            query("span", false, json!({"type": "string"}));
        }
        View::Inbox if route.path == "/waterfall" => {
            query("command", false, json!({"type": "string"}));
        }
        View::Inbox if route.path == "/" => {
            query("all", false, json!({"type": "boolean", "default": false}));
        }
        View::Scores => query("pull_request", false, json!({"type": "string"})),
        View::Metrics => {
            for name in ["command", "from", "through"] {
                query(name, false, json!({"type": "string"}));
            }
            query("repo", false, array(json!({"type": "string"})));
        }
        // These served reads take only their path arguments (if any).
        View::OpenApi
        | View::Inbox
        | View::BugOrigins
        | View::Experiments
        | View::Experiment
        | View::ExperimentSession
        | View::CronJobs
        | View::Resources
        | View::ResourceStatus
        | View::Logs
        | View::CommandLogs
        | View::Reader
        | View::ConsoleLog
        | View::ThemeExport => {}
        View::SettleBugOrigin
        | View::HumanScores
        | View::ApproveEvalTask
        | View::Changes
        | View::LogStream
        | View::Account
        | View::SignOut
        | View::SignInPage
        | View::SignIn
        | View::Look
        | View::Theme
        | View::ThemeImport
        | View::Sections
        | View::StopPage
        | View::Stop
        | View::Dismiss
        | View::UndoDismissal
        | View::SetPasswordPage
        | View::SendCode
        | View::SetPassword => unreachable!("page-only operation"),
    }
    parameters
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
fn response_shape(route: &PageRoute) -> Value {
    let text = json!({"type": "string"});
    let integer = json!({"type": "integer"});
    let record = json!({"type": "object"});
    let version = json!({"type": "integer", "const": 1});
    match route.view {
        View::OpenApi => object(json!({
            "openapi": {"type": "string", "const": "3.1.1"},
            "info": record, "paths": record, "components": record, "security": array(record.clone())
        })),
        View::Inbox if route.path == "/waterfall" => tree(),
        View::Inbox => object(json!({
            "version": version, "hostname": text, "active_runs": integer,
            "commands": array(record.clone()), "idle_passes": array(record.clone()),
            "pass_starts": array(record), "github_answer_age_seconds": {"type": ["integer", "null"]},
            "github_last_answered_at": {"type": ["string", "null"]},
            "github_failure": {"type": ["string", "null"]}
        })),
        View::Inspect => object(json!({"version": version, "hostname": text, "command": record})),
        View::Spans => json!({"oneOf": [tree(), span()]}),
        View::Log | View::Reader => object(json!({"version": version, "contents": text})),
        View::Logs | View::CommandLogs => object(json!({
            "version": version, "logs": array(object(json!({
                "id": text, "name": text, "repository": text, "kind": text,
                "live": {"type": "boolean"}, "command": text, "owners": array(text.clone())
            })))
        })),
        View::Metrics => json!({"oneOf": [
            object(json!({"version": version, "hostname": text, "command": record})),
            object(json!({"version": version, "hostname": text, "weeks": array(record)}))
        ]}),
        View::BugOrigins => array(object(json!({
            "issue": text, "closed_at": text, "state": {"enum": ["proposed", "confirmed", "rejected"]},
            "commit": {"type": ["string", "null"]}, "reason": text, "pull_requests": array(record)
        }))),
        View::Scores => array(object(json!({
            "week": text, "window_start": text, "window_end": text,
            "seed": integer, "sample": array(record), "initial_calibration": {"type": "boolean"}
        }))),
        View::Experiments => array(experiment()),
        View::Experiment => experiment(),
        View::ExperimentSession => object(json!({
            "id": text, "trace": {"type": "boolean"}, "contents": text
        })),
        View::CronJobs => object(json!({
            "jobs": array(object(json!({"schedule": text, "command": text}))), "pass_starts": array(record)
        })),
        View::Resources | View::ResourceStatus => object(json!({
            "host": object(json!({"total_memory_mb": integer, "available_memory_mb": integer,
                "logical_cpus": integer, "load_one": {"type": "number"}})),
            "limits": object(json!({"min_available_memory_mb": integer, "max_load_per_cpu_percent": integer})),
            "load_per_cpu_percent": {"type": "number"}, "ready": {"type": "boolean"},
            "reasons": array(text)
        })),
        View::ConsoleLog => array(object(
            json!({"time": integer, "email": text, "event": text}),
        )),
        View::ThemeExport => object(json!({
            "name": text, "base": text, "tokens": {"type": "object", "additionalProperties": text}
        })),
        View::SettleBugOrigin
        | View::HumanScores
        | View::ApproveEvalTask
        | View::Changes
        | View::LogStream
        | View::Account
        | View::SignOut
        | View::SignInPage
        | View::SignIn
        | View::Look
        | View::Theme
        | View::ThemeImport
        | View::Sections
        | View::StopPage
        | View::Stop
        | View::Dismiss
        | View::UndoDismissal
        | View::SetPasswordPage
        | View::SendCode
        | View::SetPassword => unreachable!("page-only operation"),
    }
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
        "format": {"type": "integer", "const": 4}, "id": {"type": "string"},
        "name": {"type": "string"}, "state": {"type": "string"}, "created_at": {"type": "string"},
        "tasks": array(json!({"type": "object"})), "results": array(json!({"type": "object"})),
        "sessions": array(json!({"type": "object"})), "running_trials": array(json!({"type": "object"}))
    }))
}
