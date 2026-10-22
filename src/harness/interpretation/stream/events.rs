//! Claude/Grok protocol details for the normalized event interface.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::super::event::{Agents, ErrorKind, Event, Outcome, Tools, Usage, UsageScope};
use super::{TaskState, assistant_error, grok_cost, message_text, result_failed};

#[derive(Default)]
pub(super) struct Events {
    models: Vec<ModelCall>,
    seen_models: HashSet<(Option<String>, String)>,
    tools: Tools,
    next_id: u64,
    agents: Agents,
    background_agents: HashSet<String>,
    task_agents: HashMap<String, String>,
    agent_parents: HashMap<String, Option<String>>,
    ended_work: HashSet<String>,
    last_text: HashMap<Option<String>, String>,
}

struct ModelCall {
    id: String,
    parent_id: Option<String>,
    model: Option<String>,
    usage: Usage,
    cost_usd: Option<f64>,
}

fn parent(event: &Value) -> Option<String> {
    event["parent_tool_use_id"].as_str().map(String::from)
}

fn interrupted_result(event: &Value) -> bool {
    matches!(
        event["terminal_reason"].as_str(),
        Some("aborted_streaming" | "aborted_tools")
    )
}

impl Events {
    pub(super) fn decode(&mut self, event: &Value, grok: bool) -> Vec<Event> {
        let mut output = Vec::new();
        self.errors(event, &mut output);
        match event["type"].as_str() {
            Some("assistant") => {
                if assistant_error(event) {
                    // Error envelopes can be synthetic replies, not another
                    // request. A request already observed in partial frames
                    // still gets its failed end and the usage read so far.
                    self.close_models(&parent(event), Outcome::Failed, &mut output);
                } else {
                    self.assistant(event, &mut output);
                }
            }
            Some("user") => {
                self.close_models(&parent(event), Outcome::Succeeded, &mut output);
                self.content(&event["message"], parent(event), "user", &mut output);
            }
            Some("result") => {
                let outcome = if interrupted_result(event) {
                    Outcome::Interrupted
                } else if result_failed(event) {
                    Outcome::Failed
                } else {
                    Outcome::Succeeded
                };
                self.close_models(&parent(event), outcome, &mut output);
                self.result_work(event, outcome, &mut output);
                if outcome == Outcome::Succeeded
                    && let Some(text) = event["result"].as_str()
                {
                    let parent_id = parent(event);
                    if self
                        .last_text
                        .get(&parent_id)
                        .is_none_or(|last| last != text)
                    {
                        output.push(Event::Message {
                            role: "assistant".into(),
                            text: text.into(),
                            parent_id: parent_id.clone(),
                        });
                        self.last_text.insert(parent_id, text.into());
                    }
                }
                let usage = Usage::messages(&event["usage"]);
                // Grok's zero cost is an unknown-cost placeholder.
                let cost_usd = if grok {
                    grok_cost(event)
                } else {
                    event["total_cost_usd"].as_f64()
                };
                if usage.reported() || cost_usd.is_some() {
                    output.push(Event::Usage {
                        scope: UsageScope::Session,
                        model: None,
                        usage,
                        cost_usd,
                    });
                }
                if !grok && let Some(models) = event["modelUsage"].as_object() {
                    for (model, usage) in models {
                        output.push(Event::Usage {
                            scope: UsageScope::Model,
                            model: Some(model.clone()),
                            usage: Usage {
                                input: usage["inputTokens"].as_u64(),
                                output: usage["outputTokens"].as_u64(),
                                cache_read: usage["cacheReadInputTokens"].as_u64(),
                                cache_write: usage["cacheCreationInputTokens"].as_u64(),
                            },
                            cost_usd: usage["costUSD"].as_f64(),
                        });
                    }
                }
            }
            Some("system") => self.task(event, &mut output),
            Some("stream_event") => self.partial(event, &mut output),
            _ => {}
        }
        output
    }

    fn assistant(&mut self, event: &Value, output: &mut Vec<Event>) {
        let message = &event["message"];
        let parent_id = parent(event);
        let id = message["id"]
            .as_str()
            .or(event["uuid"].as_str())
            .map(String::from)
            .unwrap_or_else(|| {
                self.next_id += 1;
                format!("message-{}", self.next_id)
            });
        if self
            .models
            .iter()
            .any(|call| call.parent_id == parent_id && call.id != id)
        {
            self.close_models(&parent_id, Outcome::Succeeded, output);
        }
        if self.seen_models.insert((parent_id.clone(), id.clone())) {
            self.last_text.insert(parent_id.clone(), String::new());
            let model = message["model"].as_str().map(String::from);
            output.push(Event::ModelCallStarted {
                id: id.clone(),
                parent_id: parent_id.clone(),
                model: model.clone(),
            });
            self.models.push(ModelCall {
                id: id.clone(),
                parent_id: parent_id.clone(),
                model,
                usage: Usage::default(),
                cost_usd: None,
            });
        }
        if let Some(call) = self
            .models
            .iter_mut()
            .find(|call| call.id == id && call.parent_id == parent_id)
        {
            call.usage.merge(Usage::messages(&message["usage"]));
            call.cost_usd = message["cost_usd"].as_f64().or(call.cost_usd);
        }
        self.content(message, parent_id, "assistant", output);
    }

    fn content(
        &mut self,
        message: &Value,
        parent_id: Option<String>,
        default_role: &str,
        output: &mut Vec<Event>,
    ) {
        let role = message["role"].as_str().unwrap_or(default_role);
        let mut text = |text: &str| {
            output.push(Event::Message {
                role: role.into(),
                text: text.into(),
                parent_id: parent_id.clone(),
            })
        };
        if let Some(content) = message["content"].as_str() {
            text(content);
            return;
        }
        for block in message["content"].as_array().into_iter().flatten() {
            match block["type"].as_str() {
                Some("text") => {
                    if let Some(text) = block["text"].as_str() {
                        if role == "assistant" {
                            self.last_text
                                .entry(parent_id.clone())
                                .or_default()
                                .push_str(text);
                        }
                        output.push(Event::Message {
                            role: role.into(),
                            text: text.into(),
                            parent_id: parent_id.clone(),
                        });
                    }
                }
                Some("tool_use") => {
                    if let (Some(id), Some(tool)) = (block["id"].as_str(), block["name"].as_str()) {
                        output.extend(self.tools.start(
                            id,
                            parent_id.clone(),
                            tool.into(),
                            block["input"].clone(),
                        ));
                        if matches!(tool, "Agent" | "Task") {
                            self.agent_parents.insert(id.into(), parent_id.clone());
                            output.extend(self.agents.start(
                                id,
                                parent_id.clone(),
                                block["input"]["description"].as_str().map(String::from),
                            ));
                            if block["input"]["run_in_background"] == true {
                                self.background_agents.insert(id.into());
                            }
                        }
                    }
                }
                Some("tool_result") => {
                    if let Some(id) = block["tool_use_id"].as_str() {
                        let content = block["content"].to_string().to_ascii_lowercase();
                        let denied = block["is_denied"] == true
                            || (block["is_error"] == true
                                && (content.contains("permission") && content.contains("denied")));
                        let outcome = if denied {
                            Outcome::Denied
                        } else if block["is_error"] == true {
                            Outcome::Failed
                        } else {
                            Outcome::Succeeded
                        };
                        output.extend(self.tools.end(
                            id,
                            parent_id.clone(),
                            block["content"].clone(),
                            outcome,
                        ));
                        if !self.background_agents.contains(id) {
                            self.end_agent(id, outcome, block["content"].clone(), output);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn close_models(
        &mut self,
        parent_id: &Option<String>,
        outcome: Outcome,
        output: &mut Vec<Event>,
    ) {
        let mut index = 0;
        while index < self.models.len() {
            if &self.models[index].parent_id == parent_id {
                let call = self.models.remove(index);
                output.push(Event::ModelCallEnded {
                    id: call.id,
                    parent_id: call.parent_id,
                    model: call.model,
                    usage: call.usage,
                    cost_usd: call.cost_usd,
                    outcome,
                });
            } else {
                index += 1;
            }
        }
    }

    fn errors(&self, event: &Value, output: &mut Vec<Event>) {
        let rate_limit = event["type"] == "rate_limit_event";
        let error = event["type"] == "error"
            || (event["type"] == "assistant" && assistant_error(event))
            || (event["type"] == "result"
                && (event["is_error"] == true
                    || interrupted_result(event)
                    || event["subtype"]
                        .as_str()
                        .is_some_and(|subtype| subtype != "success")));
        if !error && !rate_limit {
            return;
        }
        let message = event["message"]
            .as_str()
            .or(event["result"].as_str())
            .map(String::from)
            .unwrap_or_else(|| {
                if let Some(text) = message_text(event) {
                    text
                } else {
                    event["errors"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            });
        output.push(Event::Error {
            kind: if rate_limit {
                ErrorKind::RateLimit
            } else {
                ErrorKind::from_text(&format!(
                    "{} {} {} {}",
                    event["api_error"], event["error"], event["api_error_status"], message
                ))
            },
            message,
            parent_id: parent(event),
            details: event.clone(),
        });
    }

    fn task(&mut self, event: &Value, output: &mut Vec<Event>) {
        let Some(id) = event["task_id"].as_str() else {
            return;
        };
        let state = TaskState::read(event);
        if matches!(state, TaskState::Started) && event["task_type"] == "local_agent" {
            let agent = event["tool_use_id"].as_str().unwrap_or(id);
            self.task_agents.insert(id.into(), agent.into());
            self.agent_parents
                .entry(agent.into())
                .or_insert_with(|| parent(event));
            self.background_agents.insert(agent.into());
            output.extend(self.agents.start(
                agent,
                parent(event),
                event["description"].as_str().map(String::from),
            ));
        }
        let outcome = match state.status() {
            Some("killed" | "stopped") => Outcome::Killed,
            Some("interrupted") => Outcome::Interrupted,
            Some("completed") => Outcome::Succeeded,
            Some("failed") => Outcome::Failed,
            _ => return,
        };
        if matches!(outcome, Outcome::Killed | Outcome::Interrupted)
            && self.ended_work.insert(id.into())
        {
            output.push(Event::WorkEnded {
                id: id.into(),
                outcome,
            });
        }
        if let Some(agent) = self.task_agents.get(id).cloned() {
            self.end_agent(&agent, outcome, event["summary"].clone(), output);
        }
    }

    fn end_agent(&mut self, id: &str, outcome: Outcome, result: Value, output: &mut Vec<Event>) {
        if !self.agent_parents.contains_key(id) {
            return;
        }
        let mut scopes = vec![id.to_string()];
        let mut index = 0;
        while index < scopes.len() {
            let mut children: Vec<_> = self
                .agent_parents
                .iter()
                .filter(|(child, parent)| {
                    parent.as_deref() == Some(scopes[index].as_str()) && !scopes.contains(child)
                })
                .map(|(child, _)| child.clone())
                .collect();
            children.sort();
            scopes.extend(children);
            index += 1;
        }
        // Inner work ends before the agent owning it; other agents keep running.
        for scope in scopes.into_iter().rev() {
            self.close_models(&Some(scope.clone()), outcome, output);
            let stopped = if outcome == Outcome::Succeeded {
                Outcome::Killed
            } else {
                outcome
            };
            output.extend(self.tools.stop_scope(&Some(scope.clone()), stopped));
            output.extend(self.agents.end(
                &scope,
                outcome,
                if scope == id {
                    result.clone()
                } else {
                    Value::Null
                },
            ));
        }
    }

    fn result_work(&mut self, event: &Value, outcome: Outcome, output: &mut Vec<Event>) {
        let parent_id = parent(event);
        for denial in event["permission_denials"].as_array().into_iter().flatten() {
            if let (Some(id), Some(tool)) =
                (denial["tool_use_id"].as_str(), denial["tool_name"].as_str())
            {
                output.extend(self.tools.start(
                    id,
                    parent_id.clone(),
                    tool.into(),
                    denial["tool_input"].clone(),
                ));
                output.extend(self.tools.end(
                    id,
                    parent_id.clone(),
                    denial.clone(),
                    Outcome::Denied,
                ));
                self.end_agent(id, Outcome::Denied, denial.clone(), output);
            }
        }
        if outcome == Outcome::Interrupted {
            let mut agents: Vec<_> = match &parent_id {
                Some(id) => vec![id.clone()],
                None => self
                    .agent_parents
                    .iter()
                    .filter(|(_, parent)| parent.is_none())
                    .map(|(id, _)| id.clone())
                    .collect(),
            };
            agents.sort();
            for agent in agents {
                self.end_agent(&agent, outcome, Value::Null, output);
            }
            output.extend(self.tools.stop_scope(&parent_id, outcome));
        }
    }

    fn partial(&mut self, event: &Value, output: &mut Vec<Event>) {
        let frame = &event["event"];
        let parent_id = parent(event);
        match frame["type"].as_str() {
            Some("message_start") => {
                // Full assistant blocks follow this frame; both carry the same
                // message id, so they update one call rather than starting two.
                let mut start = event.clone();
                start["message"] = frame["message"].clone();
                self.assistant(&start, output);
            }
            Some("message_delta") => {
                if let Some(call) = self
                    .models
                    .iter_mut()
                    .find(|call| call.parent_id == parent_id)
                {
                    call.usage.merge(Usage::messages(&frame["usage"]));
                }
            }
            Some("message_stop") => self.close_models(&parent_id, Outcome::Succeeded, output),
            Some("error") => {
                let mut error = frame.clone();
                error["parent_tool_use_id"] = event["parent_tool_use_id"].clone();
                self.errors(&error, output);
                self.close_models(&parent_id, Outcome::Failed, output);
            }
            _ => {}
        }
    }
}
