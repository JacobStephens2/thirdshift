//! OpenCode's JSONL progress; final usage is deliberately left to session export.
use crate::harness::interpretation::{Decoder, Ended, Facts, Report, StreamUpdate, TurnOutcome};
use crate::harness::skill_load::SkillLoad;
use crate::progress::{bash, shorten};
use serde_json::Value;

#[derive(Default)]
pub struct OpenCodeProgress {
    session_id: Option<String>,
    started: bool,
    message: Option<String>,
    failure: Option<String>,
    skill_load: SkillLoad,
}

impl OpenCodeProgress {
    pub fn for_prompt(prompt: &str) -> Self {
        Self {
            skill_load: SkillLoad::for_prompt(prompt),
            ..Self::default()
        }
    }
}

impl Decoder for OpenCodeProgress {
    fn condense(&mut self, raw: &str) -> StreamUpdate {
        let Ok(event) = serde_json::from_str::<Value>(raw) else {
            return StreamUpdate::default();
        };
        if let Some(id) = event["sessionID"].as_str().filter(|id| !id.is_empty()) {
            self.session_id = Some(id.into());
        }
        let part = &event["part"];
        let progress = match event["type"].as_str() {
            Some("step_start") if !self.started => {
                self.started = true;
                vec!["session started".into()]
            }
            Some("text") => {
                if let Some(text) = part["text"].as_str() {
                    self.message = Some(text.into());
                }
                Vec::new()
            }
            Some("tool_use") => {
                let tool = part["tool"].as_str().unwrap_or("");
                let state = &part["state"];
                if tool == "skill" {
                    let skill = state["input"]["id"].as_str().unwrap_or("");
                    if state["status"] == "completed" {
                        self.skill_load.observed(skill);
                    }
                    vec![format!("skill {skill}")]
                } else if tool == "bash" {
                    state["input"]["command"]
                        .as_str()
                        .map(|command| vec![bash(command)])
                        .unwrap_or_default()
                } else if tool.is_empty() {
                    Vec::new()
                } else {
                    vec![shorten(tool)]
                }
            }
            Some("step_finish") => vec!["step finished".into()],
            Some("error") => {
                let error = &event["error"];
                let message = super::error_text(error).unwrap_or("OpenCode's turn failed");
                self.failure = Some(if let Some(kind) = error["type"].as_str() {
                    format!("{kind}: {message}")
                } else {
                    message.into()
                });
                vec![format!(
                    "error: {}",
                    shorten(self.failure.as_deref().unwrap())
                )]
            }
            _ => Vec::new(),
        };
        StreamUpdate {
            progress,
            evidence: Vec::new(),
        }
    }
    fn complete(self: Box<Self>) -> Facts {
        let outcome = TurnOutcome::from_failed(self.failure.is_some());
        Facts {
            report: Report {
                warnings: self.skill_load.warnings(),
                summary: None,
                ..Report::default()
            },
            ended: Ended {
                session_id: self.session_id,
                // Tool cancellation alone is not killed-work evidence.
                killed: Vec::new(),
                final_message: self.message,
            },
            outcome,
            diagnostic: self.failure,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::harness::{
        Harness,
        interpretation_tests::{finish, stream},
    };

    #[test]
    fn a_dropped_last_step_keeps_the_last_text_without_trusting_stream_usage() {
        let mut stream = stream(Harness::OpenCode, "/thirdshift-implement issue-url");
        for line in [
            r#"{"type":"step_start","sessionID":"s1","part":{}}"#,
            r#"{"type":"tool_use","sessionID":"s1","part":{"tool":"skill","state":{"status":"completed","input":{"id":"thirdshift-implement"}}}}"#,
            r#"{"type":"text","sessionID":"s1","part":{"text":"earlier reply"}}"#,
            r#"{"type":"step_finish","sessionID":"s1","part":{"tokens":{"input":1200,"output":300}}}"#,
            r#"{"type":"text","sessionID":"s1","part":{"text":"last reply"}}"#,
            "not JSON",
        ] {
            stream.condense(line);
        }
        let stream = finish(stream);
        assert_eq!(
            stream.outcome.as_ref().unwrap().final_message.as_deref(),
            Some("last reply")
        );
        assert_eq!(
            stream.outcome.as_ref().unwrap().session_id.as_deref(),
            Some("s1")
        );
        assert!(stream.report.as_ref().unwrap().warnings.clone().is_empty());
        assert!(stream.report.as_ref().unwrap().summary.clone().is_none());
    }

    #[test]
    fn a_wrong_or_failed_skill_load_still_warns() {
        let mut stream = stream(Harness::OpenCode, "/thirdshift-implement issue-url");
        for line in [
            r#"{"type":"tool_use","part":{"tool":"skill","state":{"status":"completed","input":{"id":"thirdshift-tdd"}}}}"#,
            r#"{"type":"tool_use","part":{"tool":"skill","state":{"status":"error","input":{"id":"thirdshift-implement"}}}}"#,
        ] {
            stream.condense(line);
        }
        let stream = finish(stream);
        assert_eq!(
            stream.report.as_ref().unwrap().warnings.clone(),
            ["warning: the session never loaded thirdshift-implement with its skill tool"]
        );
        assert!(
            stream
                .outcome
                .as_ref()
                .unwrap()
                .session_id
                .as_deref()
                .is_none()
        );
    }

    #[test]
    fn a_recorded_no_route_error_fails_and_names_the_model_or_effort_problem() {
        let mut stream = stream(Harness::OpenCode, "");
        stream.condense(r#"{"type":"error","sessionID":"s1","error":{"type":"provider.no-route","message":"Model unavailable: thirdshift-invalid-provider/invalid"}}"#);
        let stream = finish(stream);
        assert!(stream.outcome.is_err());
        assert_eq!(
            stream.outcome.as_ref().unwrap_err().to_string(),
            "opencode's turn failed: provider.no-route: Model unavailable: thirdshift-invalid-provider/invalid"
        );
    }
}
